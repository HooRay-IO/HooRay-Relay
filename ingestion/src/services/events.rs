//! Event storage service — persists a new [`WebhookEvent`] to the
//! `webhook_events` DynamoDB table.
//!
//! ## DynamoDB item layout
//!
//! Each call to [`create_event`] writes a single **metadata item**:
//!
//! ```text
//! PK  = EVENT#{event_id}
//! SK  = v0
//! ttl = created_at + 2_592_000   (30 days, auto-cleaned by DynamoDB TTL)
//! ```
//!
//! Delivery attempt records (`SK = ATTEMPT#{n}`) are written by the worker,
//! not by ingestion.
//!
//! ## Payload handling
//!
//! The JSON payload from the caller is stored as a `String` containing
//! a compact, normalized JSON representation. Handlers typically convert
//! an inbound `serde_json::Value` into this string form, so insignificant
//! formatting details (whitespace, field ordering) may differ from the
//! original request body, while preserving the same JSON semantics for
//! passthrough, auditing, and future replay.

use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_dynamodb::types::AttributeValue;
use serde_dynamo::aws_sdk_dynamodb_1::{from_item, to_item};
use tracing::{debug, info};

use crate::model::{IngestionError, WebhookEvent};

/// 30-day TTL offset in seconds (30 × 24 × 60 × 60).
const EVENT_TTL_SECS: i64 = 2_592_000;
/// GSI partition key value for orphaned events that need reconciliation.
const ORPHANED_GSI_PK: &str = "ORPHANED";
/// GSI index name used for orphaned-event lookups.
const ORPHANED_GSI_INDEX: &str = "gsi1-retry-index";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Persist a new webhook event to DynamoDB (`webhook_events` table, SK = `v0`).
///
/// The item includes a `ttl` attribute set to `created_at + 30 days` so that
/// DynamoDB auto-expires old records without manual cleanup.
///
/// # Arguments
/// - `client`  — DynamoDB client
/// - `table`   — name of the `webhook_events` table
/// - `event`   — the event to persist; must be in [`EventStatus::Pending`]
///
/// # Errors
/// Returns [`IngestionError::DynamoDb`] on any AWS SDK error.
pub async fn create_event(
    client: &DynamoClient,
    table: &str,
    event: &WebhookEvent,
) -> Result<(), IngestionError> {
    debug!(
        event_id = %event.event_id,
        customer_id = %event.customer_id,
        "persisting new webhook event"
    );

    // Serialize the struct into a DynamoDB attribute map.
    let mut item = to_item(event).map_err(|e| IngestionError::Serialization(e.to_string()))?;

    // Inject the PK / SK keys (not fields on the struct itself).
    item.insert("pk".to_string(), AttributeValue::S(event.pk()));
    item.insert(
        "sk".to_string(),
        AttributeValue::S(WebhookEvent::metadata_sk().to_string()),
    );

    // Inject a 30-day TTL so DynamoDB auto-expires the record.
    let ttl = event.created_at + EVENT_TTL_SECS;
    item.insert("ttl".to_string(), AttributeValue::N(ttl.to_string()));

    client
        .put_item()
        .table_name(table)
        .set_item(Some(item))
        .send()
        .await
        .map_err(|e| IngestionError::DynamoDb(e.to_string()))?;

    info!(
        event_id = %event.event_id,
        customer_id = %event.customer_id,
        ttl,
        "webhook event persisted successfully"
    );

    Ok(())
}

/// Mark an event as orphaned so the worker can reconcile and re-enqueue it.
///
/// This writes the event into the GSI (`gsi1-retry-index`) using
/// `gsi1pk = ORPHANED` and a time-ordered `gsi1sk` so that the worker can
/// query for old orphaned events without scanning the full table.
pub async fn mark_event_orphaned(
    client: &DynamoClient,
    table: &str,
    event_id: &str,
    created_at: i64,
) -> Result<(), IngestionError> {
    let pk = format!("EVENT#{event_id}");
    let sk = WebhookEvent::metadata_sk().to_string();
    let orphaned_sort_key = orphaned_gsi_sort_key(created_at, event_id);

    client
        .update_item()
        .table_name(table)
        .key("pk", AttributeValue::S(pk))
        .key("sk", AttributeValue::S(sk))
        .update_expression("SET gsi1pk = :gsi1pk, gsi1sk = :gsi1sk")
        .expression_attribute_values(":gsi1pk", AttributeValue::S(ORPHANED_GSI_PK.to_string()))
        .expression_attribute_values(":gsi1sk", AttributeValue::S(orphaned_sort_key))
        .send()
        .await
        .map_err(|e| IngestionError::DynamoDb(e.to_string()))?;

    Ok(())
}

/// Fetch orphaned events (persisted but not enqueued) up to a cutoff timestamp.
pub async fn fetch_orphaned_events(
    client: &DynamoClient,
    table: &str,
    cutoff_timestamp: i64,
    limit: i32,
) -> Result<Vec<WebhookEvent>, IngestionError> {
    let upper_bound = orphaned_gsi_upper_bound(cutoff_timestamp);

    let resp = client
        .query()
        .table_name(table)
        .index_name(ORPHANED_GSI_INDEX)
        .key_condition_expression("gsi1pk = :pk AND gsi1sk <= :sk")
        .expression_attribute_values(":pk", AttributeValue::S(ORPHANED_GSI_PK.to_string()))
        .expression_attribute_values(":sk", AttributeValue::S(upper_bound))
        .limit(limit)
        .send()
        .await
        .map_err(|e| IngestionError::DynamoDb(e.to_string()))?;

    let mut events = Vec::new();
    if let Some(items) = resp.items {
        for item in items {
            let event: WebhookEvent = from_item(item)?;
            events.push(event);
        }
    }

    Ok(events)
}

/// Clear the orphaned marker once the event is successfully re-enqueued.
pub async fn clear_orphaned_marker(
    client: &DynamoClient,
    table: &str,
    event_id: &str,
) -> Result<(), IngestionError> {
    let pk = format!("EVENT#{event_id}");
    let sk = WebhookEvent::metadata_sk().to_string();

    client
        .update_item()
        .table_name(table)
        .key("pk", AttributeValue::S(pk))
        .key("sk", AttributeValue::S(sk))
        .update_expression("REMOVE gsi1pk, gsi1sk")
        .send()
        .await
        .map_err(|e| IngestionError::DynamoDb(e.to_string()))?;

    Ok(())
}

/// Serialize an arbitrary JSON payload value to a compact `String`.
///
/// Used by handlers to convert the `data` field from the inbound
/// [`WebhookReceiveRequest`](crate::model::WebhookReceiveRequest) into the
/// `payload` string stored on [`WebhookEvent`].
///
/// # Errors
/// Returns [`IngestionError::Serialization`] if serialization fails (in
/// practice this should never happen for a well-formed `serde_json::Value`).
pub fn serialize_payload(data: &serde_json::Value) -> Result<String, IngestionError> {
    serde_json::to_string(data).map_err(|e| IngestionError::Serialization(e.to_string()))
}

fn orphaned_gsi_sort_key(created_at: i64, event_id: &str) -> String {
    format!("CREATED#{:010}#EVENT#{event_id}", created_at)
}

fn orphaned_gsi_upper_bound(cutoff_timestamp: i64) -> String {
    format!("CREATED#{:010}#EVENT#~", cutoff_timestamp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventStatus;

    // --- serialize_payload ---

    #[test]
    fn serialize_payload_produces_compact_json() {
        let data = serde_json::json!({"order_id": "ord_123", "amount": 99.99});
        let s = serialize_payload(&data).expect("should serialize");
        // Must be valid JSON and round-trippable.
        let back: serde_json::Value = serde_json::from_str(&s).expect("should parse back");
        assert_eq!(back["order_id"], "ord_123");
    }

    #[test]
    fn serialize_payload_handles_nested_objects() {
        let data = serde_json::json!({
            "user": {"id": "u_1", "email": "a@b.com"},
            "items": [1, 2, 3]
        });
        let s = serialize_payload(&data).expect("should serialize");
        assert!(s.contains("\"email\""));
        assert!(s.contains("[1,2,3]") || s.contains("[1, 2, 3]"));
    }

    #[test]
    fn serialize_payload_handles_empty_object() {
        let data = serde_json::json!({});
        let s = serialize_payload(&data).expect("should serialize");
        assert_eq!(s, "{}");
    }

    // --- create_event DynamoDB item shape (offline / unit) ---

    #[test]
    fn webhook_event_new_sets_pending_status() {
        let event = WebhookEvent::new(
            "evt_abc123".to_string(),
            "cust_xyz".to_string(),
            r#"{"order_id":"ord_1"}"#.to_string(),
            1_707_840_000,
        );
        assert_eq!(event.status, EventStatus::Pending);
        assert_eq!(event.attempt_count, 0);
        assert!(event.delivered_at.is_none());
        assert!(event.next_retry_at.is_none());
    }

    #[test]
    fn event_ttl_is_30_days_after_created_at() {
        let created_at = 1_707_840_000_i64;
        let ttl = created_at + EVENT_TTL_SECS;
        assert_eq!(ttl - created_at, 2_592_000); // 30 * 24 * 3600
    }

    #[test]
    fn webhook_event_pk_and_sk_match_contract() {
        let event = WebhookEvent::new(
            "evt_pk_test".to_string(),
            "cust_1".to_string(),
            "{}".to_string(),
            1_707_840_000,
        );
        assert_eq!(event.pk(), "EVENT#evt_pk_test");
        assert_eq!(WebhookEvent::metadata_sk(), "v0");
    }

    #[test]
    fn webhook_event_serializes_for_dynamo() {
        let event = WebhookEvent::new(
            "evt_dynamo_test".to_string(),
            "cust_dynamo".to_string(),
            r#"{"x":1}"#.to_string(),
            1_707_840_000,
        );
        let item = serde_dynamo::aws_sdk_dynamodb_1::to_item(&event);
        assert!(
            item.is_ok(),
            "serde_dynamo serialization should succeed: {:?}",
            item.err()
        );
    }
}

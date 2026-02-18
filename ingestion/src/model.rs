use serde::{Deserialize, Serialize};
use thiserror::Error;

// Re-export shared types from common crate
pub use common::{EventStatus, WebhookConfig, WebhookConfigResponse, WebhookEvent, QueueMessage};

// ---------------------------------------------------------------------------
// Inbound request / response types (API boundary)
// ---------------------------------------------------------------------------

/// The JSON body POSTed to `POST /webhooks/receive`.
///
/// `idempotency_key` is a caller-supplied token that prevents the same logical
/// event from being stored and delivered more than once within the 24-hour
/// idempotency window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookReceiveRequest {
    /// Caller-supplied unique token (e.g. `req_abc123`).
    pub idempotency_key: String,
    /// Identifies which customer's delivery config to use.
    pub customer_id: String,
    /// Raw event payload forwarded verbatim to the customer endpoint.
    ///
    /// Stored as an opaque `serde_json::Value` so the ingestion layer never
    /// has to understand the schema — it simply passes the bytes through.
    pub data: serde_json::Value,
}

/// Successful response for `POST /webhooks/receive`.
///
/// HTTP 202 Accepted — event was stored and queued for delivery.
/// HTTP 200 OK       — duplicate detected; existing `event_id` returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookReceiveResponse {
    pub event_id: String,
    pub status: ReceiveStatus,
    /// Unix timestamp (seconds) at which the event record was first created.
    pub created_at: i64,
}

/// Discriminates between a freshly accepted event and an idempotent duplicate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiveStatus {
    /// New event — stored and enqueued for delivery.
    Accepted,
    /// Duplicate `idempotency_key` — existing event returned unchanged.
    Duplicate,
}

// ---------------------------------------------------------------------------
// Config management types
// ---------------------------------------------------------------------------

/// JSON body for `POST /webhooks/configs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateConfigRequest {
    pub customer_id: String,
    pub url: String,
    /// Signing secret for HMAC-SHA256.  Auto-generated (prefix `whsec_`) when
    /// omitted by the caller.
    pub secret: Option<String>,
}

// WebhookConfigResponse and WebhookConfig are imported from common crate

// ---------------------------------------------------------------------------
// Stored entity — webhook_configs DynamoDB table
// ---------------------------------------------------------------------------

// WebhookConfig is now imported from common crate

// ---------------------------------------------------------------------------
// Stored entity — webhook_events DynamoDB table
// ---------------------------------------------------------------------------

// WebhookEvent and EventStatus are now imported from common crate

// ---------------------------------------------------------------------------
// Idempotency record — webhook_idempotency DynamoDB table
// ---------------------------------------------------------------------------

/// Record written to the `webhook_idempotency` table when a unique
/// `idempotency_key` is first seen.
///
/// DynamoDB key layout:
/// - PK  = `IDEM#{idempotency_key}`
///
/// The item carries a `ttl` attribute (Unix seconds) that DynamoDB uses to
/// auto-delete the record 24 hours after creation — no manual cleanup required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    /// DynamoDB partition key: `IDEM#{idempotency_key}`.
    pub pk: String,
    /// The `event_id` that was assigned to this idempotency key.
    pub event_id: String,
    /// Unix timestamp (seconds) when the record was created.
    pub created_at: i64,
    /// Unix timestamp (seconds) at which DynamoDB will auto-delete this item.
    /// Set to `created_at + 86_400` (24 hours).
    pub ttl: i64,
}

impl IdempotencyRecord {
    /// Build the DynamoDB PK for a given idempotency key.
    pub fn pk_for(idempotency_key: &str) -> String {
        format!("IDEM#{}", idempotency_key)
    }
}

// ---------------------------------------------------------------------------
// SQS message payload
// ---------------------------------------------------------------------------

// QueueMessage is now imported from common crate

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum IngestionError {
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("config not found for customer: {0}")]
    ConfigNotFound(String),
    #[error("event already exists: {0}")]
    AlreadyExists(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("dynamodb error: {0}")]
    DynamoDb(String),
    #[error("sqs error: {0}")]
    Sqs(String),
    #[error(transparent)]
    DynamoDbDecoding(#[from] common::DynamoDbError),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- WebhookReceiveRequest ---

    #[test]
    fn webhook_receive_request_round_trip() {
        let json = r#"{
            "idempotency_key": "req_abc123",
            "customer_id": "cust_xyz123",
            "data": {"order_id": "ord_123", "amount": 99.99}
        }"#;

        let req: WebhookReceiveRequest =
            serde_json::from_str(json).expect("request should deserialize");

        assert_eq!(req.idempotency_key, "req_abc123");
        assert_eq!(req.customer_id, "cust_xyz123");

        // Serialize and round-trip.
        let encoded = serde_json::to_string(&req).expect("request should serialize");
        let decoded: WebhookReceiveRequest =
            serde_json::from_str(&encoded).expect("round-trip should deserialize");
        assert_eq!(decoded, req);
    }

    // --- ReceiveStatus ---

    #[test]
    fn receive_status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ReceiveStatus::Accepted).unwrap(),
            "\"accepted\""
        );
        assert_eq!(
            serde_json::to_string(&ReceiveStatus::Duplicate).unwrap(),
            "\"duplicate\""
        );
    }

    // --- WebhookConfig ---

    #[test]
    fn webhook_config_to_response_copies_all_fields() {
        let config = WebhookConfig {
            customer_id: "cust_xyz123".to_string(),
            url: "https://example.com/hook".to_string(),
            secret: "whsec_test".to_string(),
            max_retries: 3,
            active: true,
            created_at: 1_707_840_000,
            updated_at: 1_707_840_000,
        };

        let resp = config.to_response();
        assert_eq!(resp.customer_id, config.customer_id);
        assert_eq!(resp.secret, config.secret);
        assert_eq!(resp.max_retries, 3);
        assert!(resp.active);
    }

    // --- IdempotencyRecord ---

    #[test]
    fn idempotency_record_pk_matches_dynamodb_contract() {
        assert_eq!(
            IdempotencyRecord::pk_for("req_abc123"),
            "IDEM#req_abc123"
        );
    }

    #[test]
    fn idempotency_ttl_is_24_hours_after_created_at() {
        let created_at = 1_707_840_000_i64;
        let record = IdempotencyRecord {
            pk: IdempotencyRecord::pk_for("req_test"),
            event_id: "evt_test".to_string(),
            created_at,
            ttl: created_at + 86_400,
        };

        assert_eq!(record.ttl - record.created_at, 86_400);
    }
}

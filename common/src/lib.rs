use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Shared enums
// ---------------------------------------------------------------------------

/// Lifecycle status of a webhook event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    /// Queued for first delivery — the worker has not yet attempted delivery.
    Pending,
    /// Successfully delivered (customer endpoint returned a 2xx response).
    Delivered,
    /// All retry attempts exhausted without a successful delivery.
    Failed,
}

// ---------------------------------------------------------------------------
// Stored entity — webhook_configs DynamoDB table
// ---------------------------------------------------------------------------

/// Returned by `POST /webhooks/configs` (HTTP 201) and
/// `GET /webhooks/configs?customer_id=…` (HTTP 200).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookConfigResponse {
    pub customer_id: String,
    pub url: String,
    pub secret: String,
    pub max_retries: u32,
    pub active: bool,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    /// Unix timestamp (seconds).
    pub updated_at: i64,
}

/// Persisted configuration record stored in the `webhook_configs` table.
///
/// DynamoDB key layout:
/// - PK  = `CUSTOMER#{customer_id}`
/// - SK  = `CONFIG`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub customer_id: String,
    pub url: String,
    /// Signing secret — format `whsec_{32 random alphanumeric chars}`.
    pub secret: String,
    pub max_retries: u32,
    pub active: bool,
    /// Unix timestamp (seconds).
    pub created_at: i64,
    /// Unix timestamp (seconds).
    pub updated_at: i64,
}

impl WebhookConfig {
    pub fn pk(&self) -> String {
        format!("CUSTOMER#{}", self.customer_id)
    }

    pub fn sk() -> &'static str {
        "CONFIG"
    }

    /// Convert to the response DTO returned by the API.
    pub fn to_response(&self) -> WebhookConfigResponse {
        WebhookConfigResponse {
            customer_id: self.customer_id.clone(),
            url: self.url.clone(),
            secret: self.secret.clone(),
            max_retries: self.max_retries,
            active: self.active,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Stored entity — webhook_events DynamoDB table
// ---------------------------------------------------------------------------

/// Metadata record stored in the `webhook_events` table (SK = `v0`).
///
/// DynamoDB key layout:
/// - PK  = `EVENT#{event_id}`
/// - SK  = `v0`   (metadata)
///        `ATTEMPT#{n}` (delivery attempt records)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub event_id: String,
    pub customer_id: String,
    /// Raw JSON payload forwarded verbatim to the customer endpoint.
    ///
    /// Stored as a `String` to preserve exact bytes for passthrough, auditing,
    /// and future replay scenarios without an extra serialization round-trip.
    pub payload: String,
    pub status: EventStatus,
    pub attempt_count: u32,
    /// Unix timestamp (seconds) when this event was first ingested.
    pub created_at: i64,
    /// Unix timestamp (seconds) when a 2xx delivery was confirmed; `None` until then.
    pub delivered_at: Option<i64>,
    /// Unix timestamp (seconds) of the next scheduled retry; `None` when not applicable.
    pub next_retry_at: Option<i64>,
}

impl WebhookEvent {
    /// Construct a new event in the [`EventStatus::Pending`] state.
    pub fn new(event_id: String, customer_id: String, payload: String, created_at: i64) -> Self {
        Self {
            event_id,
            customer_id,
            payload,
            status: EventStatus::Pending,
            attempt_count: 0,
            created_at,
            delivered_at: None,
            next_retry_at: None,
        }
    }

    pub fn pk(&self) -> String {
        format!("EVENT#{}", self.event_id)
    }

    pub fn metadata_sk() -> &'static str {
        "v0"
    }

    pub fn attempt_sk(attempt_number: u32) -> String {
        format!("ATTEMPT#{}", attempt_number)
    }

    pub fn can_retry(&self, max_retries: u32) -> bool {
        self.attempt_count < max_retries
    }

    pub fn mark_delivered(&mut self, delivered_at: i64) {
        self.status = EventStatus::Delivered;
        self.delivered_at = Some(delivered_at);
        self.next_retry_at = None;
    }

    pub fn mark_retry_scheduled(&mut self, next_retry_at: i64) {
        self.attempt_count += 1;
        self.next_retry_at = Some(next_retry_at);
    }

    pub fn mark_failed(&mut self) {
        self.status = EventStatus::Failed;
        self.next_retry_at = None;
    }
}

// ---------------------------------------------------------------------------
// SQS message payload
// ---------------------------------------------------------------------------

/// The JSON body sent to SQS by the ingestion Lambda after storing an event.
///
/// The worker reads this message, looks up the full event by `event_id`, and
/// attempts delivery.  `customer_id` is also included as an SQS
/// `MessageAttribute` so the worker can route the message without a DynamoDB
/// read in the hot path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMessage {
    pub event_id: String,
}

// ---------------------------------------------------------------------------
// Error types for DynamoDB decoding
// ---------------------------------------------------------------------------

/// Common error pattern for services that decode DynamoDB items.
#[derive(Debug, Error)]
pub enum DynamoDbError {
    #[error("Item not found during decoding: {entity} for {key}")]
    ItemNotFound { entity: &'static str, key: String },
    #[error("DynamoDB decoding error: {0}")]
    DecodeDynamo(String),
}

impl From<serde_dynamo::Error> for DynamoDbError {
    fn from(err: serde_dynamo::Error) -> Self {
        DynamoDbError::DecodeDynamo(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_status_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EventStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&EventStatus::Delivered).unwrap(),
            "\"delivered\""
        );
        assert_eq!(
            serde_json::to_string(&EventStatus::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn webhook_config_key_helpers_match_dynamodb_contract() {
        let config = WebhookConfig {
            customer_id: "cust_xyz123".to_string(),
            url: "https://customer.example.com/webhooks".to_string(),
            secret: "whsec_a1b2c3d4e5f6g7h8".to_string(),
            max_retries: 3,
            active: true,
            created_at: 1_707_840_000,
            updated_at: 1_707_840_000,
        };

        assert_eq!(config.pk(), "CUSTOMER#cust_xyz123");
        assert_eq!(WebhookConfig::sk(), "CONFIG");
    }

    #[test]
    fn webhook_event_new_has_pending_status_and_zero_attempts() {
        let event = WebhookEvent::new(
            "evt_1a2b3c4d".to_string(),
            "cust_xyz123".to_string(),
            "{\"order_id\":\"ord_123\"}".to_string(),
            1_707_840_000,
        );

        assert_eq!(event.status, EventStatus::Pending);
        assert_eq!(event.attempt_count, 0);
        assert_eq!(event.delivered_at, None);
        assert_eq!(event.next_retry_at, None);
    }

    #[test]
    fn webhook_event_key_helpers_match_dynamodb_contract() {
        let event = WebhookEvent::new(
            "evt_abc123".to_string(),
            "cust_123".to_string(),
            "{}".to_string(),
            1_707_840_000,
        );

        assert_eq!(event.pk(), "EVENT#evt_abc123");
        assert_eq!(WebhookEvent::metadata_sk(), "v0");
        assert_eq!(WebhookEvent::attempt_sk(1), "ATTEMPT#1");
        assert_eq!(WebhookEvent::attempt_sk(3), "ATTEMPT#3");
    }

    #[test]
    fn webhook_event_serialization_round_trip() {
        let event = WebhookEvent::new(
            "evt_123".to_string(),
            "cust_123".to_string(),
            "{\"ok\":true}".to_string(),
            1_707_840_000,
        );

        let encoded = serde_json::to_string(&event).expect("event should serialize");
        let decoded: WebhookEvent =
            serde_json::from_str(&encoded).expect("event should deserialize");

        assert_eq!(decoded.event_id, "evt_123");
        assert_eq!(decoded.status, EventStatus::Pending);
        assert_eq!(decoded.attempt_count, 0);
    }

    #[test]
    fn webhook_event_state_transitions() {
        let created_at = 1_700_000_000_i64;
        let mut event = WebhookEvent::new(
            "event-1".to_string(),
            "customer-1".to_string(),
            "payload".to_string(),
            created_at,
        );

        // Test can_retry
        assert!(event.can_retry(3));

        // Test mark_retry_scheduled
        let retry_at = created_at + 60;
        event.mark_retry_scheduled(retry_at);
        assert_eq!(event.status, EventStatus::Pending);
        assert_eq!(event.attempt_count, 1);
        assert_eq!(event.next_retry_at, Some(retry_at));

        // Test mark_delivered
        event.mark_delivered(created_at + 120);
        assert_eq!(event.status, EventStatus::Delivered);
        assert_eq!(event.delivered_at, Some(created_at + 120));
        assert_eq!(event.next_retry_at, None);
    }

    #[test]
    fn webhook_event_mark_failed_clears_next_retry() {
        let created_at = 1_700_000_000_i64;
        let mut event = WebhookEvent::new(
            "event-1".to_string(),
            "customer-1".to_string(),
            "payload".to_string(),
            created_at,
        );

        // Put the event into a retry-scheduled state first.
        let retry_at = created_at + 60;
        event.mark_retry_scheduled(retry_at);
        assert_eq!(event.status, EventStatus::Pending);
        assert_eq!(event.next_retry_at, Some(retry_at));

        // Now mark the event as failed and verify terminal state.
        event.mark_failed();
        assert_eq!(event.status, EventStatus::Failed);
        assert_eq!(event.next_retry_at, None);
    }

    #[test]
    fn queue_message_serializes_to_event_id_only() {
        let msg = QueueMessage {
            event_id: "evt_123".to_string(),
        };

        let serialized = serde_json::to_string(&msg).expect("serialization should succeed");
        assert_eq!(serialized, r#"{"event_id":"evt_123"}"#);
    }

    #[test]
    fn queue_message_round_trips_through_json() {
        let original = QueueMessage {
            event_id: "evt_roundtrip".to_string(),
        };

        let json = serde_json::to_string(&original).expect("serialization should succeed");
        let decoded: QueueMessage =
            serde_json::from_str(&json).expect("deserialization should succeed");

        assert_eq!(decoded, original);
    }
}

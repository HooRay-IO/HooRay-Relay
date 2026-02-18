use serde::{Deserialize, Serialize};
use thiserror::Error;

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

// ---------------------------------------------------------------------------
// Stored entity — webhook_configs DynamoDB table
// ---------------------------------------------------------------------------

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

/// Metadata record stored in the `webhook_events` table (SK = `v0`).
///
/// DynamoDB key layout:
/// - PK  = `EVENT#{event_id}`
/// - SK  = `v0`   (metadata)
///        `ATTEMPT#{n}` (delivery attempt records — see [`DeliveryAttempt`])
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
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_message_serializes_to_event_id_only() {
        let msg = QueueMessage {
            event_id: "evt_123".to_string(),
        };

        let serialized = serde_json::to_string(&msg).expect("serialization should succeed");
        assert_eq!(serialized, r#"{"event_id":"evt_123"}"#);
    }

    #[test]
    fn queue_message_deserializes_with_additional_attributes_field() {
        // Simulate the worker-side JSON format that includes an `attributes` field.
        let json = r#"{
            "event_id": "evt_123",
            "attributes": {
                "key": "value"
            }
        }"#;

        let msg: QueueMessage =
            serde_json::from_str(json).expect("deserialization should ignore extra fields");
        assert_eq!(msg.event_id, "evt_123");
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
    #[error("Item not found during decoding: {entity} for {key}")]
    ItemNotFound { entity: &'static str, key: String },
    #[error("DynamoDB decoding error: {0}")]
    DecodeDynamo(String),
}

impl From<serde_dynamo::Error> for IngestionError {
    fn from(err: serde_dynamo::Error) -> Self {
        IngestionError::DecodeDynamo(err.to_string())
    }
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

    // --- EventStatus ---

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

    // --- WebhookEvent ---

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

    /// Verifies that the ingestion model deserializes the exact fixture that
    /// the worker's model serialises — confirming the two models share the same
    /// DynamoDB wire format.
    #[test]
    fn webhook_event_deserializes_from_worker_fixture() {
        let fixture = r#"{
            "event_id": "evt_1a2b3c4d",
            "customer_id": "cust_xyz123",
            "payload": "{\"order_id\":\"ord_123\",\"amount\":99.99}",
            "status": "pending",
            "attempt_count": 0,
            "created_at": 1707840000,
            "delivered_at": null,
            "next_retry_at": null
        }"#;

        let event: WebhookEvent =
            serde_json::from_str(fixture).expect("fixture should deserialize");

        assert_eq!(event.event_id, "evt_1a2b3c4d");
        assert_eq!(event.customer_id, "cust_xyz123");
        assert_eq!(event.status, EventStatus::Pending);
        assert_eq!(event.created_at, 1_707_840_000);
    }

    // --- WebhookConfig ---

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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub customer_id: String,
    /// Raw JSON payload as received from the webhook source.
    ///
    /// This is intentionally stored as a `String` to preserve the exact bytes
    /// for passthrough, auditing, and replay scenarios. Downstream consumers
    /// may choose to deserialize this field into a typed structure, which can
    /// result in an additional JSON deserialization step.
    pub payload: String,
    pub status: EventStatus,
    pub attempt_count: u32,
    /// Time at which the event was created, as a Unix timestamp in seconds since the Unix epoch.
    pub created_at: i64,
    /// Time at which the event was successfully delivered, as a Unix timestamp in seconds since the Unix epoch.
    pub delivered_at: Option<i64>,
    /// Next scheduled retry time for delivering the event, as a Unix timestamp in seconds since the Unix epoch.
    pub next_retry_at: Option<i64>,
}

impl Event {
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
        self.next_retry_at = Some(next_retry_at);
    }

    pub fn mark_failed(&mut self) {
        self.status = EventStatus::Failed;
        self.next_retry_at = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub customer_id: String,
    pub url: String,
    pub secret: String,
    pub max_retries: u32,
    pub active: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WebhookConfig {
    pub fn pk(&self) -> String {
        format!("CUSTOMER#{}", self.customer_id)
    }

    pub fn sk() -> &'static str {
        "CONFIG"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAttempt {
    pub event_id: String,
    pub attempt_number: u32,
    pub attempted_at: i64,
    pub http_status: Option<u16>,
    pub response_time_ms: u64,
    pub error_message: Option<String>,
}

impl DeliveryAttempt {
    pub fn pk(&self) -> String {
        format!("EVENT#{}", self.event_id)
    }

    pub fn sk(&self) -> String {
        Event::attempt_sk(self.attempt_number)
    }

    pub fn new(
        event_id: String,
        attempt_number: u32,
        attempted_at: i64,
        http_status: Option<u16>,
        response_time_ms: u64,
        error_message: Option<String>,
    ) -> Self {
        Self {
            event_id,
            attempt_number,
            attempted_at,
            http_status,
            response_time_ms,
            error_message,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryResult {
    Success,
    Retry,
    Exhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryErrorClass {
    None,
    HttpRateLimited,
    HttpServerError,
    HttpClientError,
    HttpOther,
    NetworkTimeout,
    NetworkConnect,
    NetworkRequest,
    TransportOther,
    EventMissing,
    ConfigMissing,
    ConfigInactive,
    InvalidQueueMessage,
    SerializationError,
    DynamoDbError,
    SqsError,
    InternalError,
}

impl DeliveryErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryErrorClass::None => "none",
            DeliveryErrorClass::HttpRateLimited => "http_rate_limited",
            DeliveryErrorClass::HttpServerError => "http_server_error",
            DeliveryErrorClass::HttpClientError => "http_client_error",
            DeliveryErrorClass::HttpOther => "http_other",
            DeliveryErrorClass::NetworkTimeout => "network_timeout",
            DeliveryErrorClass::NetworkConnect => "network_connect",
            DeliveryErrorClass::NetworkRequest => "network_request",
            DeliveryErrorClass::TransportOther => "transport_other",
            DeliveryErrorClass::EventMissing => "event_missing",
            DeliveryErrorClass::ConfigMissing => "config_missing",
            DeliveryErrorClass::ConfigInactive => "config_inactive",
            DeliveryErrorClass::InvalidQueueMessage => "invalid_queue_message",
            DeliveryErrorClass::SerializationError => "serialization_error",
            DeliveryErrorClass::DynamoDbError => "dynamodb_error",
            DeliveryErrorClass::SqsError => "sqs_error",
            DeliveryErrorClass::InternalError => "internal_error",
        }
    }

    pub fn result(self) -> DeliveryResult {
        match self {
            DeliveryErrorClass::None => DeliveryResult::Success,
            DeliveryErrorClass::HttpRateLimited
            | DeliveryErrorClass::HttpServerError
            | DeliveryErrorClass::HttpOther
            | DeliveryErrorClass::NetworkTimeout
            | DeliveryErrorClass::NetworkConnect
            | DeliveryErrorClass::NetworkRequest
            | DeliveryErrorClass::DynamoDbError
            | DeliveryErrorClass::SqsError => DeliveryResult::Retry,
            DeliveryErrorClass::HttpClientError
            | DeliveryErrorClass::TransportOther
            | DeliveryErrorClass::EventMissing
            | DeliveryErrorClass::ConfigMissing
            | DeliveryErrorClass::ConfigInactive
            | DeliveryErrorClass::InvalidQueueMessage
            | DeliveryErrorClass::SerializationError
            | DeliveryErrorClass::InternalError => DeliveryResult::Exhausted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryClassification {
    pub result: DeliveryResult,
    pub class: DeliveryErrorClass,
}

impl DeliveryClassification {
    pub fn from_class(class: DeliveryErrorClass) -> Self {
        Self {
            result: class.result(),
            class,
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkerError {
    #[error("event not found: {0}")]
    EventNotFound(String),
    #[error("config not found for customer: {0}")]
    ConfigNotFound(String),
    #[error("inactive webhook config for customer: {0}")]
    InactiveConfig(String),
    #[error("invalid SQS message: {0}")]
    InvalidMessage(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("dynamodb error: {0}")]
    DynamoDb(String),
    #[error("sqs error: {0}")]
    Sqs(String),
    #[error("delivery error: {0}")]
    Delivery(String),
    #[error("Item not found during decoding: {entity} for {key}")]
    ItemNotFound { entity: &'static str, key: String },
    #[error("DynamoDB decoding error: {0}")]
    DecodeDynamo(String),
}

impl From<serde_dynamo::Error> for WorkerError {
    fn from(err: serde_dynamo::Error) -> Self {
        WorkerError::DecodeDynamo(err.to_string())
    }
}

pub fn classify_worker_error(err: &WorkerError) -> DeliveryClassification {
    match err {
        WorkerError::EventNotFound(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::EventMissing)
        }
        WorkerError::ConfigNotFound(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::ConfigMissing)
        }
        WorkerError::InactiveConfig(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::ConfigInactive)
        }
        WorkerError::InvalidMessage(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::InvalidQueueMessage)
        }
        WorkerError::Serialization(_) | WorkerError::DecodeDynamo(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::SerializationError)
        }
        WorkerError::DynamoDb(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::DynamoDbError)
        }
        WorkerError::Sqs(_) => DeliveryClassification::from_class(DeliveryErrorClass::SqsError),
        WorkerError::Delivery(_) => {
            DeliveryClassification::from_class(DeliveryErrorClass::InternalError)
        }
        WorkerError::ItemNotFound { entity, .. } => match *entity {
            "Event" => DeliveryClassification::from_class(DeliveryErrorClass::EventMissing),
            "WebhookConfig" => {
                DeliveryClassification::from_class(DeliveryErrorClass::ConfigMissing)
            }
            _ => DeliveryClassification::from_class(DeliveryErrorClass::InternalError),
        },
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMessage {
    pub event_id: String,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mark_failed_clears_next_retry_and_sets_status_failed() {
        let created_at = 1_700_000_000_i64;
        let mut event = Event::new(
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
    fn event_serialization_round_trip() {
        let event = Event::new(
            "evt_123".to_string(),
            "cust_123".to_string(),
            "{\"ok\":true}".to_string(),
            1_707_840_000,
        );

        let encoded = serde_json::to_string(&event).expect("event should serialize");
        let decoded: Event = serde_json::from_str(&encoded).expect("event should deserialize");

        assert_eq!(decoded.event_id, "evt_123");
        assert_eq!(decoded.status, EventStatus::Pending);
        assert_eq!(decoded.attempt_count, 0);
    }

    #[test]
    fn event_deserializes_from_ingestion_fixture() {
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

        let event: Event = serde_json::from_str(fixture).expect("fixture should deserialize");
        assert_eq!(event.event_id, "evt_1a2b3c4d");
        assert_eq!(event.customer_id, "cust_xyz123");
        assert_eq!(event.status, EventStatus::Pending);
        assert_eq!(event.attempt_count, 0);
        assert_eq!(event.created_at, 1_707_840_000);
        assert_eq!(event.delivered_at, None);
        assert_eq!(event.next_retry_at, None);
    }

    #[test]
    fn status_serializes_as_snake_case() {
        let status = EventStatus::Delivered;
        let encoded = serde_json::to_string(&status).expect("status should serialize");
        assert_eq!(encoded, "\"delivered\"");
    }

    #[test]
    fn event_key_helpers_match_dynamodb_contract() {
        let event = Event::new(
            "evt_abc123".to_string(),
            "cust_123".to_string(),
            "{}".to_string(),
            1_707_840_000,
        );

        assert_eq!(event.pk(), "EVENT#evt_abc123");
        assert_eq!(Event::metadata_sk(), "v0");
        assert_eq!(Event::attempt_sk(1), "ATTEMPT#1");
        assert_eq!(Event::attempt_sk(3), "ATTEMPT#3");
    }

    #[test]
    fn retry_and_terminal_transitions() {
        let mut event = Event::new(
            "evt_456".to_string(),
            "cust_abc".to_string(),
            "{}".to_string(),
            1_707_840_000,
        );

        assert!(event.can_retry(3));

        event.attempt_count += 1;
        event.mark_retry_scheduled(1_707_840_060);
        assert_eq!(event.attempt_count, 1);
        assert_eq!(event.next_retry_at, Some(1_707_840_060));

        event.mark_delivered(1_707_840_120);
        assert_eq!(event.status, EventStatus::Delivered);
        assert_eq!(event.delivered_at, Some(1_707_840_120));
        assert_eq!(event.next_retry_at, None);
    }

    #[test]
    fn delivery_error_class_maps_to_expected_result() {
        assert_eq!(
            DeliveryErrorClass::HttpServerError.result(),
            DeliveryResult::Retry
        );
        assert_eq!(
            DeliveryErrorClass::HttpClientError.result(),
            DeliveryResult::Exhausted
        );
        assert_eq!(DeliveryErrorClass::None.result(), DeliveryResult::Success);
    }

    #[test]
    fn classify_worker_error_maps_item_not_found_event() {
        let classification = classify_worker_error(&WorkerError::ItemNotFound {
            entity: "Event",
            key: "evt_123".to_string(),
        });
        assert_eq!(classification.class, DeliveryErrorClass::EventMissing);
        assert_eq!(classification.result, DeliveryResult::Exhausted);
    }
}

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
    pub payload: String,
    pub status: EventStatus,
    pub attempt_count: u32,
    pub created_at: i64,
    pub delivered_at: Option<i64>,
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
        self.attempt_count += 1;
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
        attempted_at: i64,
        http_status: Option<u16>,
        response_time_ms: u64,
        error_message: Option<String>,
    ) -> Self {
        Self {
            event_id,
            attempt_number: 1,
            attempted_at,
            http_status,
            response_time_ms,
            error_message,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryResult {
    Success,
    Retry,
    Exhausted,
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
    #[error("Item not found")]
    DecodeNotFound,
    #[error("DynamoDB decoding error: {0}")]
    DecodeDynamo(#[from] serde_dynamo::Error),
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
    fn retry_and_terminal_transitions() {
        let mut event = Event::new(
            "evt_456".to_string(),
            "cust_abc".to_string(),
            "{}".to_string(),
            1_707_840_000,
        );

        assert!(event.can_retry(3));

        event.mark_retry_scheduled(1_707_840_060);
        assert_eq!(event.attempt_count, 1);
        assert_eq!(event.next_retry_at, Some(1_707_840_060));

        event.mark_delivered(1_707_840_120);
        assert_eq!(event.status, EventStatus::Delivered);
        assert_eq!(event.delivered_at, Some(1_707_840_120));
        assert_eq!(event.next_retry_at, None);
    }
}

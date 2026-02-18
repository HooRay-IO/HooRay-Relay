use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

// Re-export shared types from common crate
pub use common::{EventStatus, WebhookConfig, WebhookEvent};

// Type alias for backward compatibility in worker code
pub type Event = WebhookEvent;

// Worker-specific QueueMessage with attributes field
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueMessage {
    pub event_id: String,
    #[serde(default)]
    pub attributes: HashMap<String, String>,
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
        WebhookEvent::attempt_sk(self.attempt_number)
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
    #[error(transparent)]
    DynamoDbDecoding(#[from] common::DynamoDbError),
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

        event.mark_retry_scheduled(1_707_840_060);
        assert_eq!(event.attempt_count, 1);
        assert_eq!(event.next_retry_at, Some(1_707_840_060));

        event.mark_delivered(1_707_840_120);
        assert_eq!(event.status, EventStatus::Delivered);
        assert_eq!(event.delivered_at, Some(1_707_840_120));
        assert_eq!(event.next_retry_at, None);
    }
}

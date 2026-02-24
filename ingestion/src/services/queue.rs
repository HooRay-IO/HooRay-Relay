//! SQS queue service — enqueues a [`QueueMessage`] for delivery by the worker.
//!
//! ## Message layout
//!
//! Each call to [`enqueue_event`] sends a single SQS message:
//!
//! ```text
//! Body            = { "event_id": "<event_id>" }   (JSON)
//! MessageAttribute "customer_id" = <customer_id>   (String)
//! ```
//!
//! The `customer_id` attribute allows the worker to route the message to the
//! correct delivery configuration **without** an extra DynamoDB read on the
//! hot path — it reads the attribute directly from the SQS message metadata.

use aws_sdk_sqs::Client as SqsClient;
use aws_sdk_sqs::types::MessageAttributeValue;
use tracing::{debug, info};

use crate::model::{IngestionError, QueueMessage};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enqueue a webhook event for asynchronous delivery by the worker.
///
/// Sends a single SQS message whose body is the JSON-serialized
/// [`QueueMessage`] and whose `customer_id` message attribute lets the worker
/// skip a DynamoDB lookup in the hot path.
///
/// # Arguments
/// - `client`      — SQS client
/// - `queue_url`   — fully-qualified SQS queue URL
/// - `event_id`    — the event to deliver (stored in the message body)
/// - `customer_id` — routing hint added as a `String` message attribute
///
/// # Errors
/// Returns [`IngestionError::Sqs`] on any AWS SDK error or serialization
/// failure.
pub async fn enqueue_event(
    client: &SqsClient,
    queue_url: &str,
    event_id: &str,
    customer_id: &str,
) -> Result<(), IngestionError> {
    debug!(
        event_id,
        customer_id, "serializing SQS message body"
    );

    // Build the JSON body.
    let msg = QueueMessage {
        event_id: event_id.to_string(),
    };
    let body =
        serde_json::to_string(&msg).map_err(|e| IngestionError::Sqs(e.to_string()))?;

    // Build the customer_id message attribute.
    let customer_id_attr = MessageAttributeValue::builder()
        .data_type("String")
        .string_value(customer_id)
        .build()
        .map_err(|e| IngestionError::Sqs(e.to_string()))?;

    debug!(
        event_id,
        customer_id,
        body = %body,
        "sending message to SQS"
    );

    client
        .send_message()
        .queue_url(queue_url)
        .message_body(&body)
        .message_attributes("customer_id", customer_id_attr)
        .send()
        .await
        .map_err(|e| IngestionError::Sqs(e.to_string()))?;

    info!(
        event_id,
        customer_id,
        queue_url,
        "event enqueued for delivery"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::model::QueueMessage;

    // -----------------------------------------------------------------------
    // Message body serialization
    // -----------------------------------------------------------------------

    #[test]
    fn queue_message_body_contains_only_event_id() {
        let msg = QueueMessage {
            event_id: "evt_abc123".to_string(),
        };
        let body = serde_json::to_string(&msg).expect("serialization should succeed");
        assert_eq!(body, r#"{"event_id":"evt_abc123"}"#);
    }

    #[test]
    fn queue_message_body_is_valid_json() {
        let msg = QueueMessage {
            event_id: "evt_test_456".to_string(),
        };
        let body = serde_json::to_string(&msg).expect("serialization should succeed");
        // Must parse back to JSON without error.
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("body must be valid JSON");
        assert_eq!(parsed["event_id"], "evt_test_456");
    }

    #[test]
    fn queue_message_body_has_no_extra_fields() {
        let msg = QueueMessage {
            event_id: "evt_xyz789".to_string(),
        };
        let body = serde_json::to_string(&msg).expect("serialization should succeed");
        let parsed: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&body).expect("body must be a JSON object");
        // Only "event_id" — no customer_id or other fields in the body.
        assert_eq!(parsed.len(), 1, "body must contain exactly one field");
        assert!(parsed.contains_key("event_id"));
    }

    // -----------------------------------------------------------------------
    // MessageAttribute contract
    // -----------------------------------------------------------------------

    #[test]
    fn customer_id_attribute_data_type_is_string() {
        // The worker reads MessageAttribute["customer_id"].string_value().
        // Confirm the builder produces a "String" DataType (not "Number" etc.).
        use aws_sdk_sqs::types::MessageAttributeValue;

        let attr = MessageAttributeValue::builder()
            .data_type("String")
            .string_value("cust_xyz123")
            .build()
            .expect("attribute should build successfully");

        assert_eq!(attr.data_type(), "String");
        assert_eq!(
            attr.string_value(),
            Some("cust_xyz123"),
            "string_value must match the customer_id passed in"
        );
    }

    #[test]
    fn customer_id_attribute_value_round_trips() {
        use aws_sdk_sqs::types::MessageAttributeValue;

        let customer_id = "cust_abc-def_456";
        let attr = MessageAttributeValue::builder()
            .data_type("String")
            .string_value(customer_id)
            .build()
            .expect("attribute should build successfully");

        assert_eq!(attr.string_value(), Some(customer_id));
    }

    #[test]
    fn customer_id_attribute_string_value_is_none_when_omitted() {
        use aws_sdk_sqs::types::MessageAttributeValue;

        // The SQS SDK builder does NOT enforce string_value at build time for
        // a "String" DataType attribute; omitting it results in string_value()
        // returning None.  Our enqueue_event() always supplies a value, so
        // this test documents the SDK behaviour rather than relying on a panic.
        let attr = MessageAttributeValue::builder()
            .data_type("String")
            // intentionally omit .string_value(...)
            .build()
            .expect("SDK builder accepts a String attribute without string_value");

        assert!(
            attr.string_value().is_none(),
            "string_value() should be None when no value was provided"
        );
    }

    // -----------------------------------------------------------------------
    // Event ID format constraints
    // -----------------------------------------------------------------------

    #[test]
    fn event_id_in_message_body_preserves_exact_value() {
        // Any string passed as event_id must appear verbatim in the JSON body.
        let event_id = "evt_1a2b3c4d5e6f";
        let msg = QueueMessage {
            event_id: event_id.to_string(),
        };
        let body = serde_json::to_string(&msg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["event_id"].as_str().unwrap(), event_id);
    }
}

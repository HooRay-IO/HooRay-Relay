pub mod model;
pub mod services;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use aws_sdk_sqs as sqs;
use services::{delivery::DeliveryService, dynamodb::DynamoDbService};
use sqs::types::Message;
use std::{env, time::Duration};
use tracing::{error, info, warn};

use crate::model::{DeliveryResult, EventStatus, QueueMessage, WorkerError};

#[derive(Clone, Copy)]
enum MessageAction {
    Delete,
    KeepForRetry,
}

struct Worker {
    delivery_service: DeliveryService,
    sqs_client: sqs::Client,
    queue_url: String,
    dynamodb_service: DynamoDbService,
}

impl Worker {
    async fn new() -> Result<Self, WorkerError> {
        let queue_url = env::var("QUEUE_URL")
            .or_else(|_| env::var("WEBHOOK_QUEUE_URL"))
            .map_err(|_| {
                WorkerError::InvalidMessage(
                    "QUEUE_URL or WEBHOOK_QUEUE_URL must be set".to_string(),
                )
            })?;

        let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let events_table = env::var("EVENTS_TABLE")
            .or_else(|_| env::var("WEBHOOK_EVENTS_TABLE"))
            .map_err(|_| {
                WorkerError::InvalidMessage(
                    "EVENTS_TABLE or WEBHOOK_EVENTS_TABLE must be set".to_string(),
                )
            })?;
        let configs_table = env::var("CONFIGS_TABLE")
            .or_else(|_| env::var("WEBHOOK_CONFIGS_TABLE"))
            .map_err(|_| {
                WorkerError::InvalidMessage(
                    "CONFIGS_TABLE or WEBHOOK_CONFIGS_TABLE must be set".to_string(),
                )
            })?;
        Ok(Worker {
            delivery_service: DeliveryService::new(),
            sqs_client: sqs::Client::new(&config),
            queue_url,
            dynamodb_service: DynamoDbService::new(
                DynamoDbClient::new(&config),
                events_table,
                configs_table,
            ),
        })
    }

    async fn run(&self) {
        info!("worker is running");
        loop {
            if let Err(err) = self.poll_and_process().await {
                error!(error = %err, "poll_and_process failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    async fn poll_and_process(&self) -> Result<(), WorkerError> {
        info!("polling SQS");
        let response = self
            .sqs_client
            .receive_message()
            .queue_url(&self.queue_url)
            .wait_time_seconds(20)
            .max_number_of_messages(10)
            .send()
            .await
            .map_err(|e| WorkerError::Sqs(format!("failed to receive messages: {e}")))?;

        let messages = response.messages();
        if messages.is_empty() {
            info!("no messages received");
            return Ok(());
        }

        info!(
            message_count = messages.len(),
            "processing received messages"
        );
        for message in messages {
            if let Err(err) = self.process_message(message).await {
                error!(error = %err, "failed to process message");
            }
        }

        Ok(())
    }

    async fn deliver_event(&self, event_id: &str) -> Result<MessageAction, WorkerError> {
        let mut event = match self.dynamodb_service.get_event(event_id).await {
            Ok(event) => event,
            Err(WorkerError::ItemNotFound {
                entity: "Event", ..
            }) => {
                warn!(event_id = %event_id, "event not found, deleting message");
                return Ok(MessageAction::Delete);
            }
            Err(err) => return Err(err),
        };

        if matches!(event.status, EventStatus::Delivered | EventStatus::Failed) {
            info!(event_id = %event.event_id, status = ?event.status, "event already terminal; skipping duplicate");
            return Ok(MessageAction::Delete);
        }

        let config = match self.dynamodb_service.get_config(&event.customer_id).await {
            Ok(config) => config,
            Err(WorkerError::ItemNotFound {
                entity: "WebhookConfig",
                ..
            }) => {
                warn!(event_id = %event.event_id, customer_id = %event.customer_id, "missing config, marking failed");
                event.mark_failed();
                self.dynamodb_service.update_event_status(&event).await?;
                return Ok(MessageAction::Delete);
            }
            Err(err) => return Err(err),
        };

        if !config.active {
            warn!(event_id = %event.event_id, customer_id = %event.customer_id, "inactive config, marking failed");
            event.mark_failed();
            self.dynamodb_service.update_event_status(&event).await?;
            return Ok(MessageAction::Delete);
        }

        let (result, attempt) = self.delivery_service.deliver(&event, &config).await?;
        self.dynamodb_service.record_attempt(attempt).await?;
        event.attempt_count += 1;
        self.dynamodb_service
            .increment_attempt_count(&event)
            .await?;

        match result {
            DeliveryResult::Success => {
                event.mark_delivered(chrono::Utc::now().timestamp());
                self.dynamodb_service.update_event_status(&event).await?;
                info!(event_id = %event.event_id, "event delivered successfully");
                Ok(MessageAction::Delete)
            }
            DeliveryResult::Retry => {
                if event.attempt_count >= config.max_retries {
                    warn!(event_id = %event.event_id, attempt_count = event.attempt_count, max_retries = config.max_retries, "retry exhausted, marking failed");
                    event.mark_failed();
                    self.dynamodb_service.update_event_status(&event).await?;
                    Ok(MessageAction::Delete)
                } else {
                    info!(event_id = %event.event_id, attempt_count = event.attempt_count, max_retries = config.max_retries, "retryable failure, keeping SQS message");
                    let next_retry_at = chrono::Utc::now().timestamp() + (60 * 5);
                    event.mark_retry_scheduled(next_retry_at);
                    self.dynamodb_service.update_event_status(&event).await?;
                    Ok(MessageAction::KeepForRetry)
                }
            }
            DeliveryResult::Exhausted => {
                event.mark_failed();
                self.dynamodb_service.update_event_status(&event).await?;
                info!(event_id = %event.event_id, "event delivery exhausted");
                Ok(MessageAction::Delete)
            }
        }
    }

    async fn process_message(&self, message: &Message) -> Result<(), WorkerError> {
        let receipt_handle = message
            .receipt_handle()
            .ok_or_else(|| WorkerError::InvalidMessage("missing receipt_handle".to_string()))?;
        let body = match message.body() {
            Some(body) => body,
            None => {
                warn!("message missing body; deleting");
                self.delete_message(receipt_handle).await?;
                return Ok(());
            }
        };

        let queue_message: QueueMessage = match serde_json::from_str(body) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(error = %err, body = %body, "invalid queue payload; deleting poison message");
                self.delete_message(receipt_handle).await?;
                return Ok(());
            }
        };

        info!(event_id = %queue_message.event_id, "received queue message");
        match self.deliver_event(&queue_message.event_id).await? {
            MessageAction::Delete => self.delete_message(receipt_handle).await?,
            MessageAction::KeepForRetry => {}
        }
        Ok(())
    }

    async fn delete_message(&self, receipt_handle: &str) -> Result<(), WorkerError> {
        self.sqs_client
            .delete_message()
            .queue_url(&self.queue_url)
            .receipt_handle(receipt_handle)
            .send()
            .await
            .map_err(|e| WorkerError::Sqs(format!("failed to delete message: {e}")))?;

        info!("deleted SQS message");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_sqs::types::Message;
    use std::sync::{Arc, Mutex};

    /// Test double for exercising the message-processing logic without hitting AWS.
    #[derive(Clone)]
    struct TestWorker {
        deleted_receipt_handles: Arc<Mutex<Vec<String>>>,
        delivered_event_ids: Arc<Mutex<Vec<String>>>,
        next_action: Arc<Mutex<MessageAction>>,
    }

    impl TestWorker {
        fn new(next_action: MessageAction) -> Self {
            Self {
                deleted_receipt_handles: Arc::new(Mutex::new(Vec::new())),
                delivered_event_ids: Arc::new(Mutex::new(Vec::new())),
                next_action: Arc::new(Mutex::new(next_action)),
            }
        }

        async fn delete_message(&self, receipt_handle: &str) -> Result<(), WorkerError> {
            self.deleted_receipt_handles
                .lock()
                .unwrap()
                .push(receipt_handle.to_string());
            Ok(())
        }

        async fn deliver_event(&self, event_id: &str) -> Result<MessageAction, WorkerError> {
            self.delivered_event_ids
                .lock()
                .unwrap()
                .push(event_id.to_string());
            Ok(*self.next_action.lock().unwrap())
        }

        /// Copy of the production `process_message` logic, but using the test double's
        /// in-memory `delete_message` and `deliver_event` implementations.
        async fn process_message(&self, message: &Message) -> Result<(), WorkerError> {
            let receipt_handle = message
                .receipt_handle()
                .ok_or_else(|| WorkerError::InvalidMessage("missing receipt_handle".to_string()))?;
            let body = match message.body() {
                Some(body) => body,
                None => {
                    // message missing body; deleting
                    self.delete_message(receipt_handle).await?;
                    return Ok(());
                }
            };

            let queue_message: QueueMessage = match serde_json::from_str(body) {
                Ok(parsed) => parsed,
                Err(_) => {
                    // invalid queue payload; deleting poison message
                    self.delete_message(receipt_handle).await?;
                    return Ok(());
                }
            };

            match self.deliver_event(&queue_message.event_id).await? {
                MessageAction::Delete => self.delete_message(receipt_handle).await?,
                MessageAction::KeepForRetry => {}
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn process_message_deletes_when_body_missing() {
        let worker = TestWorker::new(MessageAction::KeepForRetry);
        let message = Message::builder()
            .receipt_handle("rh-1")
            // no body set
            .build();

        let result = worker.process_message(&message).await;
        assert!(result.is_ok(), "expected Ok for missing body case");

        let deleted = worker.deleted_receipt_handles.lock().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "rh-1");
    }

    #[tokio::test]
    async fn process_message_deletes_on_invalid_json() {
        let worker = TestWorker::new(MessageAction::KeepForRetry);
        let message = Message::builder()
            .receipt_handle("rh-2")
            .body("not valid json")
            .build();

        let result = worker.process_message(&message).await;
        assert!(result.is_ok(), "expected Ok for invalid JSON case");

        let deleted = worker.deleted_receipt_handles.lock().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "rh-2");
    }

    #[tokio::test]
    async fn process_message_calls_deliver_event_and_keeps_for_retry() {
        let worker = TestWorker::new(MessageAction::KeepForRetry);
        let payload = serde_json::json!({
            "event_id": "event-123"
        })
        .to_string();

        let message = Message::builder()
            .receipt_handle("rh-3")
            .body(payload)
            .build();

        let result = worker.process_message(&message).await;
        assert!(result.is_ok(), "expected Ok for valid message");

        let delivered = worker.delivered_event_ids.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0], "event-123");

        let deleted = worker.deleted_receipt_handles.lock().unwrap();
        assert!(deleted.is_empty(), "message should be kept for retry");
    }

    #[tokio::test]
    async fn process_message_calls_deliver_event_and_deletes_on_success() {
        let worker = TestWorker::new(MessageAction::Delete);
        let payload = serde_json::json!({
            "event_id": "event-456"
        })
        .to_string();

        let message = Message::builder()
            .receipt_handle("rh-4")
            .body(payload)
            .build();

        let result = worker.process_message(&message).await;
        assert!(result.is_ok(), "expected Ok for valid message");

        let delivered = worker.delivered_event_ids.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0], "event-456");

        let deleted = worker.deleted_receipt_handles.lock().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0], "rh-4");
    }
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let worker = match Worker::new().await {
        Ok(worker) => worker,
        Err(err) => {
            warn!(error = %err, "worker initialization failed");
            return;
        }
    };

    worker.run().await;
}

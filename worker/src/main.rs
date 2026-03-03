pub mod model;
pub mod observability;
pub mod resilience;
pub mod services;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use aws_sdk_sqs::types::Message;
use chrono::Duration as ChronoDuration;
use observability::Observability;
use resilience::ResilienceConfig;
use resilience::breaker::{on_failure, on_success, should_allow_request};
use resilience::retry::{build_resilience_outcome_from_delivery_result, calculate_retry_decision};
use services::{delivery::DeliveryService, dynamodb::DynamoDbService, sqs::SqsService};
use std::collections::HashMap;
use std::{env, time::Duration};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::model::{DeliveryResult, EventStatus, QueueMessage, WorkerError};

#[derive(Clone, Copy)]
enum MessageAction {
    Delete,
    KeepForRetry { retry_delay_secs: i64 },
}

struct Worker {
    delivery_service: DeliveryService,
    sqs_service: SqsService,
    dynamodb_service: DynamoDbService,
    observability: Observability,
    resilience_config: ResilienceConfig,
    breaker_states: Mutex<HashMap<String, resilience::BreakerState>>,
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
        let resilience_config = ResilienceConfig::default();
        Ok(Worker {
            delivery_service: DeliveryService::new(),
            sqs_service: SqsService::new(
                aws_sdk_sqs::Client::new(&config),
                queue_url.clone(),
                resilience_config.clone(),
            ),
            observability: Observability::new(&queue_url),
            dynamodb_service: DynamoDbService::new(
                DynamoDbClient::new(&config),
                events_table,
                configs_table,
            ),
            resilience_config,
            breaker_states: Mutex::new(HashMap::new()),
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
        self.emit_queue_depth_metric().await;

        let messages = self.sqs_service.receive_messages().await?;
        if messages.is_empty() {
            info!("no messages received");
            return Ok(());
        }

        info!(
            message_count = messages.len(),
            "processing received messages"
        );
        for message in &messages {
            if let Err(err) = self.process_message(message).await {
                error!(error = %err, "failed to process message");
            }
        }
        self.emit_queue_depth_metric().await;

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

        let endpoint_key = event.customer_id.clone();
        let now = chrono::Utc::now().timestamp();
        let mut breaker_state = self.get_breaker_state(&endpoint_key).await;
        if !should_allow_request(&mut breaker_state, now) {
            let retry_delay_secs = self.retry_delay_from_breaker(now, &breaker_state).max(1);
            self.set_breaker_state(endpoint_key.clone(), breaker_state)
                .await;
            return self
                .schedule_retry(
                    &mut event,
                    retry_delay_secs,
                    "breaker blocked delivery; rescheduling retry",
                )
                .await;
        }
        self.set_breaker_state(endpoint_key.clone(), breaker_state.clone())
            .await;

        let (result, attempt) = self.delivery_service.deliver(&event, &config).await?;
        self.observability
            .emit_delivery_attempt(&event, &attempt, &result);
        self.dynamodb_service.record_attempt(attempt).await?;
        event.attempt_count += 1;
        self.dynamodb_service
            .increment_attempt_count(&event)
            .await?;

        match result {
            DeliveryResult::Success => {
                on_success(
                    &mut breaker_state,
                    chrono::Utc::now().timestamp(),
                    &self.resilience_config,
                );
                self.set_breaker_state(endpoint_key.clone(), breaker_state)
                    .await;
                event.mark_delivered(chrono::Utc::now().timestamp());
                self.dynamodb_service.update_event_status(&event).await?;
                info!(event_id = %event.event_id, "event delivered successfully");
                Ok(MessageAction::Delete)
            }
            DeliveryResult::Retry => {
                on_failure(
                    &mut breaker_state,
                    chrono::Utc::now().timestamp(),
                    &self.resilience_config,
                );
                self.set_breaker_state(endpoint_key.clone(), breaker_state.clone())
                    .await;
                let outcome = build_resilience_outcome_from_delivery_result(DeliveryResult::Retry);
                let retry_decision =
                    calculate_retry_decision(&outcome, &breaker_state, &self.resilience_config);

                if !retry_decision.should_retry {
                    warn!(
                        event_id = %event.event_id,
                        attempt_count = event.attempt_count,
                        "retry policy declined retry; marking failed"
                    );
                    event.mark_failed();
                    self.dynamodb_service.update_event_status(&event).await?;
                    return Ok(MessageAction::Delete);
                }

                let retry_delay_secs = retry_decision
                    .next_attempt_delay
                    .map(|delay| delay.num_seconds().max(1))
                    .unwrap_or_else(|| {
                        self.resilience_config
                            .backoff_base_delay
                            .num_seconds()
                            .max(1)
                    });
                self.schedule_retry(
                    &mut event,
                    retry_delay_secs,
                    "retryable failure, keeping SQS message",
                )
                .await
            }
            DeliveryResult::Exhausted => {
                on_failure(
                    &mut breaker_state,
                    chrono::Utc::now().timestamp(),
                    &self.resilience_config,
                );
                self.set_breaker_state(endpoint_key, breaker_state).await;
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
                self.sqs_service.delete_message(receipt_handle).await?;
                return Ok(());
            }
        };

        let queue_message: QueueMessage = match serde_json::from_str(body) {
            Ok(parsed) => parsed,
            Err(err) => {
                warn!(error = %err, body = %body, "invalid queue payload; deleting poison message");
                self.sqs_service.delete_message(receipt_handle).await?;
                return Ok(());
            }
        };

        info!(event_id = %queue_message.event_id, "received queue message");
        match self.deliver_event(&queue_message.event_id).await? {
            MessageAction::Delete => self.sqs_service.delete_message(receipt_handle).await?,
            MessageAction::KeepForRetry { retry_delay_secs } => {
                self.sqs_service
                    .change_visibility_for_retry(
                        receipt_handle,
                        ChronoDuration::seconds(retry_delay_secs),
                    )
                    .await?
            }
        }
        Ok(())
    }

    async fn emit_queue_depth_metric(&self) {
        match self.sqs_service.approximate_queue_depth().await {
            Ok(Some(depth)) => self.observability.emit_queue_depth(depth),
            Ok(None) => {}
            Err(err) => warn!(error = %err, "failed to sample queue depth"),
        }
    }

    async fn get_breaker_state(&self, endpoint_key: &str) -> resilience::BreakerState {
        let mut states = self.breaker_states.lock().await;
        states
            .entry(endpoint_key.to_string())
            .or_insert_with(Self::closed_breaker_state)
            .clone()
    }

    async fn set_breaker_state(
        &self,
        endpoint_key: String,
        breaker_state: resilience::BreakerState,
    ) {
        let mut states = self.breaker_states.lock().await;
        states.insert(endpoint_key, breaker_state);
    }

    fn retry_delay_from_breaker(&self, now: i64, breaker_state: &resilience::BreakerState) -> i64 {
        let fallback = self
            .resilience_config
            .breaker_recovery_timeout
            .num_seconds()
            .max(1);
        breaker_state
            .next_probe_at
            .map(|next_probe_at| (next_probe_at - now).max(1))
            .unwrap_or(fallback)
    }

    fn closed_breaker_state() -> resilience::BreakerState {
        resilience::BreakerState {
            mode: resilience::BreakerMode::Closed,
            consecutive_failures: 0,
            consecutive_successes: 0,
            opened_at: None,
            next_probe_at: None,
            last_failure_at: None,
            last_success_at: None,
            half_open_in_flight: false,
            version: 0,
        }
    }

    async fn schedule_retry(
        &self,
        event: &mut crate::model::Event,
        retry_delay_secs: i64,
        reason: &str,
    ) -> Result<MessageAction, WorkerError> {
        let next_retry_at = chrono::Utc::now().timestamp() + retry_delay_secs.max(1);
        event.mark_retry_scheduled(next_retry_at);
        self.dynamodb_service.update_event_status(event).await?;
        info!(
            event_id = %event.event_id,
            attempt_count = event.attempt_count,
            retry_delay_secs,
            reason,
        );
        Ok(MessageAction::KeepForRetry { retry_delay_secs })
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
                MessageAction::KeepForRetry { .. } => {}
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn process_message_deletes_when_body_missing() {
        let worker = TestWorker::new(MessageAction::KeepForRetry {
            retry_delay_secs: 300,
        });
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
        let worker = TestWorker::new(MessageAction::KeepForRetry {
            retry_delay_secs: 300,
        });
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
        let worker = TestWorker::new(MessageAction::KeepForRetry {
            retry_delay_secs: 300,
        });
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

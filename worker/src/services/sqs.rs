use aws_sdk_sqs as sqs;
use aws_sdk_sqs::types::{Message, QueueAttributeName};
use chrono::Duration;

use crate::model::WorkerError;
use crate::resilience::{ResilienceConfig, retry::compute_visibility_timeout};

#[derive(Clone)]
pub struct SqsService {
    client: sqs::Client,
    queue_url: String,
    resilience_config: ResilienceConfig,
}

impl SqsService {
    pub fn new(
        client: sqs::Client,
        queue_url: String,
        resilience_config: ResilienceConfig,
    ) -> Self {
        Self {
            client,
            queue_url,
            resilience_config,
        }
    }

    pub async fn receive_messages(&self) -> Result<Vec<Message>, WorkerError> {
        let baseline_visibility =
            compute_visibility_timeout(Duration::zero(), &self.resilience_config);

        let output = self
            .client
            .receive_message()
            .queue_url(&self.queue_url)
            .wait_time_seconds(20)
            .max_number_of_messages(10)
            .visibility_timeout(self.to_sqs_visibility_seconds(baseline_visibility))
            .send()
            .await
            .map_err(|e| WorkerError::Sqs(format!("failed to receive messages: {e}")))?;

        Ok(output.messages().iter().cloned().collect())
    }

    pub async fn delete_message(&self, receipt_handle: &str) -> Result<(), WorkerError> {
        self.client
            .delete_message()
            .queue_url(&self.queue_url)
            .receipt_handle(receipt_handle)
            .send()
            .await
            .map_err(|e| WorkerError::Sqs(format!("failed to delete message: {e}")))?;

        Ok(())
    }

    pub async fn change_visibility_for_retry(
        &self,
        receipt_handle: &str,
        retry_delay: Duration,
    ) -> Result<(), WorkerError> {
        let visibility_timeout = compute_visibility_timeout(retry_delay, &self.resilience_config);

        self.client
            .change_message_visibility()
            .queue_url(&self.queue_url)
            .receipt_handle(receipt_handle)
            .visibility_timeout(self.to_sqs_visibility_seconds(visibility_timeout))
            .send()
            .await
            .map_err(|e| WorkerError::Sqs(format!("failed to change message visibility: {e}")))?;

        Ok(())
    }

    pub async fn approximate_queue_depth(&self) -> Result<Option<i64>, WorkerError> {
        let output = self
            .client
            .get_queue_attributes()
            .queue_url(&self.queue_url)
            .attribute_names(QueueAttributeName::ApproximateNumberOfMessages)
            .send()
            .await
            .map_err(|e| WorkerError::Sqs(format!("failed to get queue attributes: {e}")))?;

        let attributes = match output.attributes() {
            Some(attributes) => attributes,
            None => return Ok(None),
        };
        let raw_depth = match attributes.get(&QueueAttributeName::ApproximateNumberOfMessages) {
            Some(raw_depth) => raw_depth,
            None => return Ok(None),
        };
        let depth = raw_depth.parse::<i64>().map_err(|e| {
            WorkerError::Sqs(format!("failed to parse queue depth '{raw_depth}': {e}"))
        })?;

        Ok(Some(depth))
    }

    fn to_sqs_visibility_seconds(&self, timeout: Duration) -> i32 {
        timeout.num_seconds().clamp(0, 43_200) as i32
    }
}

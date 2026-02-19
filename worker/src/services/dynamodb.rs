use crate::model::{DeliveryAttempt, Event, EventStatus, WebhookConfig, WorkerError};
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::types::AttributeValue;
use std::collections::HashMap;
use tracing::{error, info, instrument, warn};

fn required_attr<'a>(
    item: &'a HashMap<String, AttributeValue>,
    key: &str,
) -> Result<&'a AttributeValue, WorkerError> {
    item.get(key).ok_or(WorkerError::ItemNotFound {
        entity: "DynamoDB attribute",
        key: key.to_string(),
    })
}

fn optional_number_attr(
    item: &HashMap<String, AttributeValue>,
    key: &str,
) -> Result<Option<i64>, WorkerError> {
    match item.get(key) {
        Some(attr) => Ok(Some(get_number_attr(attr)? as i64)),
        None => Ok(None),
    }
}

fn parse_event_item(item: Option<HashMap<String, AttributeValue>>) -> Result<Event, WorkerError> {
    let item = item.ok_or(WorkerError::ItemNotFound {
        entity: "Event",
        key: "pk/sk".to_string(),
    })?;

    Ok(Event {
        event_id: get_string_attr(required_attr(&item, "event_id")?)?,
        customer_id: get_string_attr(required_attr(&item, "customer_id")?)?,
        payload: get_string_attr(required_attr(&item, "payload")?)?,
        status: parse_status_attr(required_attr(&item, "status")?)?,
        attempt_count: get_number_attr(required_attr(&item, "attempt_count")?)? as u32,
        created_at: get_number_attr(required_attr(&item, "created_at")?)? as i64,
        delivered_at: optional_number_attr(&item, "delivered_at")?,
        next_retry_at: optional_number_attr(&item, "next_retry_at")?,
    })
}

fn parse_config_item(
    item: Option<HashMap<String, AttributeValue>>,
) -> Result<WebhookConfig, WorkerError> {
    let item = item.ok_or(WorkerError::ItemNotFound {
        entity: "WebhookConfig",
        key: "pk/sk".to_string(),
    })?;

    Ok(WebhookConfig {
        customer_id: get_string_attr(required_attr(&item, "customer_id")?)?,
        url: get_string_attr(required_attr(&item, "url")?)?,
        secret: get_string_attr(required_attr(&item, "secret")?)?,
        max_retries: get_number_attr(required_attr(&item, "max_retries")?)? as u32,
        active: get_bool_attr(required_attr(&item, "active")?)?,
        created_at: get_number_attr(required_attr(&item, "created_at")?)? as i64,
        updated_at: get_number_attr(required_attr(&item, "updated_at")?)? as i64,
    })
}

fn get_string_attr(attr: &AttributeValue) -> Result<String, WorkerError> {
    if let AttributeValue::S(s) = attr {
        Ok(s.clone())
    } else {
        Err(WorkerError::DecodeDynamo(
            "Expected string attribute".to_string(),
        ))
    }
}

fn get_number_attr(attr: &AttributeValue) -> Result<u64, WorkerError> {
    if let AttributeValue::N(n) = attr {
        n.parse::<u64>().map_err(|e| {
            WorkerError::DecodeDynamo(format!("Failed to parse number attribute: {}", e))
        })
    } else {
        Err(WorkerError::DecodeDynamo(
            "Expected number attribute".to_string(),
        ))
    }
}

fn get_bool_attr(attr: &AttributeValue) -> Result<bool, WorkerError> {
    if let AttributeValue::Bool(b) = attr {
        Ok(*b)
    } else {
        Err(WorkerError::DecodeDynamo(
            "Expected boolean attribute".to_string(),
        ))
    }
}

fn parse_status_attr(attr: &AttributeValue) -> Result<EventStatus, WorkerError> {
    if let AttributeValue::S(s) = attr {
        match s.as_str() {
            "pending" => Ok(EventStatus::Pending),
            "delivered" => Ok(EventStatus::Delivered),
            "failed" => Ok(EventStatus::Failed),
            _ => Err(WorkerError::DecodeDynamo(format!(
                "Invalid status value: {}",
                s
            ))),
        }
    } else {
        Err(WorkerError::DecodeDynamo(
            "Expected string attribute for status".to_string(),
        ))
    }
}

pub struct DynamoDbService {
    client: Client,
    webhook_events_table: String,
    // webhook_idempotency_table: String,
    webhook_configs_table: String,
}

impl DynamoDbService {
    pub fn new(
        client: Client,
        webhook_events_table: String,
        // webhook_idempotency_table: String,
        webhook_configs_table: String,
    ) -> Self {
        Self {
            client,
            webhook_events_table,
            // webhook_idempotency_table,
            webhook_configs_table,
        }
    }

    /// Fetches an event by its ID from DynamoDB.
    #[instrument(skip(self), fields(event_id = %event_id))]
    pub async fn get_event(&self, event_id: &str) -> Result<Event, WorkerError> {
        info!("fetching event");
        let pk = format!("EVENT#{}", event_id);
        let resp = self
            .client
            .get_item()
            .table_name(&self.webhook_events_table)
            .key("pk", AttributeValue::S(pk))
            .key("sk", AttributeValue::S(Event::metadata_sk().to_string()))
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "failed to fetch event");
                WorkerError::DynamoDb(format!("Failed to fetch event with ID {}: {}", event_id, e))
            })?;

        let event = parse_event_item(resp.item)?;
        info!(attempt_count = event.attempt_count, "fetched event");
        Ok(event)
    }

    /// Fetch webhook config
    #[instrument(skip(self), fields(customer_id = %customer_id))]
    pub async fn get_config(&self, customer_id: &str) -> Result<WebhookConfig, WorkerError> {
        info!("fetching webhook config");
        let pk = format!("CUSTOMER#{}", customer_id);
        let sk = WebhookConfig::sk().to_string();
        let resp = self
            .client
            .get_item()
            .table_name(&self.webhook_configs_table)
            .key("pk", AttributeValue::S(pk))
            .key("sk", AttributeValue::S(sk))
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "failed to fetch webhook config");
                WorkerError::DynamoDb(format!(
                    "Failed to fetch config for customer_id {}: {}",
                    customer_id, e
                ))
            })?;

        let config = parse_config_item(resp.item)?;
        if config.active {
            info!("fetched active webhook config");
        } else {
            warn!("fetched inactive webhook config");
        }
        Ok(config)
    }

    /// Records a delivery attempt for an event in DynamoDB.
    /// This creates a new item for the attempt in the webhook events table.
    #[instrument(skip(self, attempt), fields(event_id = %attempt.event_id, attempt_number = attempt.attempt_number))]
    pub async fn record_attempt(&self, attempt: DeliveryAttempt) -> Result<(), WorkerError> {
        // Create a new item for the delivery attempt with pk=EVENT#<event_id> and sk=ATTEMPT#<attempt_number>
        let mut request = self
            .client
            .put_item()
            .table_name(&self.webhook_events_table)
            .item("pk", AttributeValue::S(attempt.pk()))
            .item("sk", AttributeValue::S(attempt.sk()))
            .item(
                "attempt_number",
                AttributeValue::N(attempt.attempt_number.to_string()),
            )
            .item(
                "attempted_at",
                AttributeValue::N(attempt.attempted_at.to_string()),
            )
            .item(
                "response_time_ms",
                AttributeValue::N(attempt.response_time_ms.to_string()),
            );

        if let Some(status) = attempt.http_status {
            request = request.item("http_status", AttributeValue::N(status.to_string()));
        }

        if let Some(error_message) = &attempt.error_message {
            request = request.item("error_message", AttributeValue::S(error_message.clone()));
        }

        request.send().await.map_err(|e| {
            error!(error = %e, "failed to record delivery attempt");
            WorkerError::DynamoDb(format!(
                "Failed to record delivery attempt for event_id {}: {}",
                attempt.event_id, e
            ))
        })?;
        info!("recorded delivery attempt");
        Ok(())
    }

    /// Updates the status of an event in DynamoDB after a delivery attempt.
    #[instrument(skip(self, event), fields(event_id = %event.event_id))]
    pub async fn update_event_status(&self, event: &Event) -> Result<(), WorkerError> {
        // Implementation would involve updating the event item with the new status and next retry time if needed
        let pk = event.pk();
        let sk = Event::metadata_sk().to_string();
        let mut request = self
            .client
            .update_item()
            .table_name(&self.webhook_events_table)
            .key("pk", AttributeValue::S(pk))
            .key("sk", AttributeValue::S(sk))
            .update_expression("SET #status = :status")
            .expression_attribute_names("#status", "status")
            .expression_attribute_values(
                ":status",
                AttributeValue::S(
                    match event.status {
                        EventStatus::Pending => "pending",
                        EventStatus::Delivered => "delivered",
                        EventStatus::Failed => "failed",
                    }
                    .to_string(),
                ),
            );

        if let Some(next_retry_at) = event.next_retry_at {
            request = request
                .update_expression("SET #status = :status, next_retry_at = :next_retry_at")
                .expression_attribute_values(
                    ":next_retry_at",
                    AttributeValue::N(next_retry_at.to_string()),
                );
        }

        request.send().await.map_err(|e| {
            error!(error = %e, "failed to update event status");
            WorkerError::DynamoDb(format!(
                "Failed to update event status for event_id {}: {}",
                event.event_id, e
            ))
        })?;

        info!(status = ?event.status, "updated event status");
        Ok(())
    }

    #[instrument(skip(self, event), fields(event_id = %event.event_id))]
    pub async fn increment_attempt_count(&self, event: &Event) -> Result<(), WorkerError> {
        let pk = event.pk();
        let sk = Event::metadata_sk().to_string();
        self.client
            .update_item()
            .table_name(&self.webhook_events_table)
            .key("pk", AttributeValue::S(pk))
            .key("sk", AttributeValue::S(sk))
            .update_expression("SET attempt_count = :attempt_count")
            .expression_attribute_values(
                ":attempt_count",
                AttributeValue::N(event.attempt_count.to_string()),
            )
            .send()
            .await
            .map_err(|e| {
                error!(error = %e, "failed to update attempt count");
                WorkerError::DynamoDb(format!(
                    "Failed to update attempt count for event_id {}: {}",
                    event.event_id, e
                ))
            })?;

        info!("updated attempt count");
        Ok(())
    }
}

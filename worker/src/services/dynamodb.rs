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
        Some(AttributeValue::Null(_)) => Ok(None),
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
        } else {
            request = request.update_expression("SET #status = :status REMOVE next_retry_at");
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

#[cfg(test)]
mod tests {
    use super::*;
    use aws_config::BehaviorVersion;
    use std::env;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn must_env(name: &str) -> String {
        env::var(name).unwrap_or_else(|_| panic!("{} must be set for integration test", name))
    }

    fn now_unix_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is before unix epoch")
            .as_secs() as i64
    }

    fn attr_s(item: &HashMap<String, AttributeValue>, key: &str) -> String {
        match item.get(key) {
            Some(AttributeValue::S(value)) => value.clone(),
            _ => panic!("missing or invalid string attr: {}", key),
        }
    }

    fn attr_n_u32(item: &HashMap<String, AttributeValue>, key: &str) -> u32 {
        match item.get(key) {
            Some(AttributeValue::N(value)) => {
                value.parse::<u32>().expect("invalid u32 number attr")
            }
            _ => panic!("missing or invalid number attr: {}", key),
        }
    }

    fn attr_n_i64(item: &HashMap<String, AttributeValue>, key: &str) -> i64 {
        match item.get(key) {
            Some(AttributeValue::N(value)) => {
                value.parse::<i64>().expect("invalid i64 number attr")
            }
            _ => panic!("missing or invalid number attr: {}", key),
        }
    }

    #[test]
    fn optional_number_attr_treats_null_as_none() {
        let mut item = HashMap::new();
        item.insert("delivered_at".to_string(), AttributeValue::Null(true));
        item.insert("next_retry_at".to_string(), AttributeValue::Null(true));

        let delivered = optional_number_attr(&item, "delivered_at")
            .expect("NULL delivered_at should decode as None");
        let next_retry = optional_number_attr(&item, "next_retry_at")
            .expect("NULL next_retry_at should decode as None");

        assert_eq!(delivered, None);
        assert_eq!(next_retry, None);
    }

    #[test]
    fn parse_event_item_accepts_null_optional_timestamps() {
        let mut item = HashMap::new();
        item.insert("event_id".to_string(), AttributeValue::S("evt_nulls".to_string()));
        item.insert(
            "customer_id".to_string(),
            AttributeValue::S("cust_nulls".to_string()),
        );
        item.insert("payload".to_string(), AttributeValue::S("{\"ok\":true}".to_string()));
        item.insert("status".to_string(), AttributeValue::S("pending".to_string()));
        item.insert("attempt_count".to_string(), AttributeValue::N("0".to_string()));
        item.insert("created_at".to_string(), AttributeValue::N("1707840000".to_string()));
        item.insert("delivered_at".to_string(), AttributeValue::Null(true));
        item.insert("next_retry_at".to_string(), AttributeValue::Null(true));

        let event = parse_event_item(Some(item)).expect("event with NULL optional attrs should parse");
        assert_eq!(event.event_id, "evt_nulls");
        assert_eq!(event.delivered_at, None);
        assert_eq!(event.next_retry_at, None);
    }

    struct CleanupItem {
        table: String,
        pk: String,
        sk: String,
    }

    struct CleanupGuard {
        client: Client,
        items: Vec<CleanupItem>,
    }

    impl CleanupGuard {
        fn new(client: Client) -> Self {
            Self {
                client,
                items: Vec::new(),
            }
        }

        fn track(&mut self, table: &str, pk: &str, sk: &str) {
            self.items.push(CleanupItem {
                table: table.to_string(),
                pk: pk.to_string(),
                sk: sk.to_string(),
            });
        }
    }

    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            if self.items.is_empty() {
                return;
            }

            let client = self.client.clone();
            let items = std::mem::take(&mut self.items);
            let join_handle = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();

                let Ok(runtime) = runtime else {
                    eprintln!("cleanup guard: failed to build Tokio runtime");
                    return;
                };

                runtime.block_on(async move {
                    for item in items.into_iter().rev() {
                        if let Err(err) = client
                            .delete_item()
                            .table_name(&item.table)
                            .key("pk", AttributeValue::S(item.pk))
                            .key("sk", AttributeValue::S(item.sk))
                            .send()
                            .await
                        {
                            eprintln!(
                                "cleanup guard: failed to delete pk/sk from {}: {}",
                                item.table, err
                            );
                        }
                    }
                });
            });

            if join_handle.join().is_err() {
                eprintln!("cleanup guard: cleanup thread panicked");
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials, network access, and DynamoDB tables from env"]
    async fn validates_attempt_recording_and_event_updates() {
        let events_table = must_env("WEBHOOK_EVENTS_TABLE");
        let configs_table = must_env("WEBHOOK_CONFIGS_TABLE");
        let _region = must_env("AWS_REGION");
        let ts_now = now_unix_secs();
        let unique = format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is before unix epoch")
                .as_millis()
        );
        let event_id = format!("evt_service_test_{}", unique);
        let customer_id = format!("cust_service_test_{}", unique);
        let event_pk = format!("EVENT#{}", event_id);
        let config_pk = format!("CUSTOMER#{}", customer_id);

        let shared_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let client = Client::new(&shared_config);
        let service =
            DynamoDbService::new(client.clone(), events_table.clone(), configs_table.clone());
        let mut cleanup = CleanupGuard::new(client.clone());

        client
            .put_item()
            .table_name(&events_table)
            .condition_expression("attribute_not_exists(pk) AND attribute_not_exists(sk)")
            .item("pk", AttributeValue::S(event_pk.clone()))
            .item("sk", AttributeValue::S(Event::metadata_sk().to_string()))
            .item("event_id", AttributeValue::S(event_id.clone()))
            .item("customer_id", AttributeValue::S(customer_id.clone()))
            .item(
                "payload",
                AttributeValue::S(
                    "{\"order_id\":\"ord_service_test\",\"amount\":42.0}".to_string(),
                ),
            )
            .item("status", AttributeValue::S("pending".to_string()))
            .item("attempt_count", AttributeValue::N("0".to_string()))
            .item("created_at", AttributeValue::N(ts_now.to_string()))
            .send()
            .await
            .expect("seed event put-item should succeed");
        cleanup.track(&events_table, &event_pk, Event::metadata_sk());

        client
            .put_item()
            .table_name(&configs_table)
            .condition_expression("attribute_not_exists(pk) AND attribute_not_exists(sk)")
            .item("pk", AttributeValue::S(config_pk.clone()))
            .item("sk", AttributeValue::S(WebhookConfig::sk().to_string()))
            .item("customer_id", AttributeValue::S(customer_id.clone()))
            .item(
                "url",
                AttributeValue::S("https://webhook.site/service-test".to_string()),
            )
            .item(
                "secret",
                AttributeValue::S("whsec_service_test".to_string()),
            )
            .item("max_retries", AttributeValue::N("3".to_string()))
            .item("active", AttributeValue::Bool(true))
            .item("created_at", AttributeValue::N(ts_now.to_string()))
            .item("updated_at", AttributeValue::N(ts_now.to_string()))
            .send()
            .await
            .expect("seed config put-item should succeed");
        cleanup.track(&configs_table, &config_pk, WebhookConfig::sk());

        let config = service
            .get_config(&customer_id)
            .await
            .expect("get_config should return seeded config");
        assert_eq!(config.customer_id, customer_id);
        assert_eq!(config.url, "https://webhook.site/service-test");
        assert_eq!(config.secret, "whsec_service_test");
        assert_eq!(config.max_retries, 3);
        assert!(config.active);
        let mut event = service
            .get_event(&event_id)
            .await
            .expect("get_event should return seeded event");
        assert_eq!(event.attempt_count, 0);

        // `increment_attempt_count` persists the current `attempt_count` field
        // from the event into DynamoDB; the caller is responsible for updating
        // `event.attempt_count` before calling it.
        event.attempt_count = 1;
        service
            .increment_attempt_count(&event)
            .await
            .expect("increment_attempt_count should persist updated attempt_count");

        let event_after_count = client
            .get_item()
            .table_name(&events_table)
            .consistent_read(true)
            .key("pk", AttributeValue::S(event_pk.clone()))
            .key("sk", AttributeValue::S(Event::metadata_sk().to_string()))
            .send()
            .await
            .expect("get_item should read updated event")
            .item
            .expect("event item should exist");
        assert_eq!(attr_n_u32(&event_after_count, "attempt_count"), 1);

        event.status = EventStatus::Failed;
        event.next_retry_at = Some(ts_now + 60);
        service
            .update_event_status(&event)
            .await
            .expect("update_event_status should update status fields");

        let event_after_status = client
            .get_item()
            .table_name(&events_table)
            .consistent_read(true)
            .key("pk", AttributeValue::S(event_pk.clone()))
            .key("sk", AttributeValue::S(Event::metadata_sk().to_string()))
            .send()
            .await
            .expect("get_item should read status-updated event")
            .item
            .expect("event item should exist");
        assert_eq!(attr_s(&event_after_status, "status"), "failed");
        assert_eq!(
            attr_n_i64(&event_after_status, "next_retry_at"),
            ts_now + 60
        );

        let attempt = DeliveryAttempt::new(
            event_id.clone(),
            1,
            ts_now + 5,
            Some(500),
            250,
            Some("validation failure".to_string()),
        );
        service
            .record_attempt(attempt)
            .await
            .expect("record_attempt should create attempt item");
        cleanup.track(&events_table, &event_pk, &Event::attempt_sk(1));

        let attempt_item = client
            .get_item()
            .table_name(&events_table)
            .consistent_read(true)
            .key("pk", AttributeValue::S(event_pk.clone()))
            .key("sk", AttributeValue::S(Event::attempt_sk(1)))
            .send()
            .await
            .expect("get_item should read attempt item")
            .item
            .expect("attempt item should exist");
        assert_eq!(attr_n_u32(&attempt_item, "attempt_number"), 1);
        assert_eq!(attr_n_i64(&attempt_item, "attempted_at"), ts_now + 5);
        assert_eq!(attr_n_u32(&attempt_item, "http_status"), 500u32);
        assert_eq!(attr_s(&attempt_item, "error_message"), "validation failure");
    }
}

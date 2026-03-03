use crate::model::{DeliveryAttempt, DeliveryResult, Event};
use chrono::Utc;
use serde_json::{Map, Value, json};
use std::env;

const DEFAULT_METRIC_NAMESPACE: &str = "HoorayRelay/Worker";
const DEFAULT_ENVIRONMENT: &str = "dev";

#[derive(Clone, Debug)]
pub struct Observability {
    environment: String,
    queue_name: String,
    namespace: String,
}

impl Observability {
    pub fn new(queue_url: &str) -> Self {
        let environment =
            env::var("ENVIRONMENT").unwrap_or_else(|_| DEFAULT_ENVIRONMENT.to_string());
        let namespace =
            env::var("METRIC_NAMESPACE").unwrap_or_else(|_| DEFAULT_METRIC_NAMESPACE.to_string());

        Self {
            environment,
            queue_name: queue_name_from_url(queue_url),
            namespace,
        }
    }

    pub fn emit_delivery_attempt(
        &self,
        event: &Event,
        attempt: &DeliveryAttempt,
        result: &DeliveryResult,
    ) {
        let result_text = result.as_metric_label();
        let status_code = attempt
            .http_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "none".to_string());

        println!(
            "{}",
            json!({
                "event_type": "delivery_attempt",
                "event_id": event.event_id,
                "customer_id": event.customer_id,
                "attempt_number": attempt.attempt_number,
                "result": result_text,
                "http_status": attempt.http_status,
                "latency_ms": attempt.response_time_ms,
                "error": attempt.error_message,
            })
        );

        let detailed_dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("queue_name".to_string(), self.queue_name.clone()),
            ("customer_id".to_string(), event.customer_id.clone()),
            ("status_code".to_string(), status_code.clone()),
        ];
        let aggregate_dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("queue_name".to_string(), self.queue_name.clone()),
        ];
        let status_dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("queue_name".to_string(), self.queue_name.clone()),
            ("status_code".to_string(), status_code),
        ];

        self.emit_metric("webhook.delivery.attempt", "Count", 1.0, &detailed_dims);
        self.emit_metric("webhook.delivery.attempt", "Count", 1.0, &aggregate_dims);
        self.emit_metric(
            "webhook.delivery.http_status_code",
            "Count",
            1.0,
            &status_dims,
        );

        match result {
            DeliveryResult::Success => {
                self.emit_metric("webhook.delivery.success", "Count", 1.0, &detailed_dims);
                self.emit_metric("webhook.delivery.success", "Count", 1.0, &aggregate_dims);
            }
            DeliveryResult::Retry | DeliveryResult::Exhausted => {
                self.emit_metric("webhook.delivery.failure", "Count", 1.0, &detailed_dims);
                self.emit_metric("webhook.delivery.failure", "Count", 1.0, &aggregate_dims);
            }
        }

        self.emit_metric(
            "webhook.delivery.latency_ms",
            "Milliseconds",
            attempt.response_time_ms as f64,
            &detailed_dims,
        );
        self.emit_metric(
            "webhook.delivery.latency_ms",
            "Milliseconds",
            attempt.response_time_ms as f64,
            &aggregate_dims,
        );
    }

    pub fn emit_queue_depth(&self, depth: i64) {
        let dimensions = vec![
            ("environment".to_string(), self.environment.clone()),
            ("queue_name".to_string(), self.queue_name.clone()),
        ];
        self.emit_metric("webhook.queue.depth", "Count", depth as f64, &dimensions);
    }

    fn emit_metric(
        &self,
        metric_name: &str,
        unit: &str,
        value: f64,
        dimensions: &[(String, String)],
    ) {
        println!(
            "{}",
            build_metric_payload(&self.namespace, metric_name, unit, value, dimensions,)
        );
    }
}

fn queue_name_from_url(queue_url: &str) -> String {
    queue_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown-queue")
        .to_string()
}

fn build_metric_payload(
    namespace: &str,
    metric_name: &str,
    unit: &str,
    value: f64,
    dimensions: &[(String, String)],
) -> Value {
    let mut root = Map::new();
    let dimension_keys: Vec<Value> = dimensions
        .iter()
        .map(|(key, _)| Value::String(key.clone()))
        .collect();

    for (key, value_text) in dimensions {
        root.insert(key.clone(), Value::String(value_text.clone()));
    }

    root.insert(metric_name.to_string(), Value::from(value));
    root.insert(
        "_aws".to_string(),
        json!({
            "Timestamp": Utc::now().timestamp_millis(),
            "CloudWatchMetrics": [{
                "Namespace": namespace,
                "Dimensions": [dimension_keys],
                "Metrics": [{
                    "Name": metric_name,
                    "Unit": unit
                }]
            }]
        }),
    );

    Value::Object(root)
}

trait DeliveryResultMetricLabel {
    fn as_metric_label(&self) -> &'static str;
}

impl DeliveryResultMetricLabel for DeliveryResult {
    fn as_metric_label(&self) -> &'static str {
        match self {
            DeliveryResult::Success => "success",
            DeliveryResult::Retry => "retry",
            DeliveryResult::Exhausted => "exhausted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_name_parser_extracts_name() {
        let queue_url = "https://sqs.us-west-2.amazonaws.com/123456789012/webhook_delivery_dev";
        assert_eq!(queue_name_from_url(queue_url), "webhook_delivery_dev");
    }

    #[test]
    fn queue_name_parser_falls_back_when_missing_segments() {
        assert_eq!(queue_name_from_url(""), "unknown-queue");
    }

    #[test]
    fn metric_payload_contains_name_and_dimensions() {
        let payload = build_metric_payload(
            "HoorayRelay/Worker",
            "webhook.delivery.success",
            "Count",
            1.0,
            &[
                ("environment".to_string(), "dev".to_string()),
                ("queue_name".to_string(), "queue-a".to_string()),
            ],
        );

        assert_eq!(payload["webhook.delivery.success"], 1.0);
        assert_eq!(payload["environment"], "dev");
        assert_eq!(payload["queue_name"], "queue-a");
        assert_eq!(
            payload["_aws"]["CloudWatchMetrics"][0]["Metrics"][0]["Name"],
            "webhook.delivery.success"
        );
    }
}

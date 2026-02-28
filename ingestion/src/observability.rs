//! Structured CloudWatch Embedded Metric Format (EMF) observability for the
//! ingestion Lambda.
//!
//! ## Emitted metrics
//!
//! | Metric name                          | Unit         | When emitted                        |
//! |--------------------------------------|--------------|-------------------------------------|
//! | `webhook.receive.count`              | Count        | Every `POST /webhooks/receive` call |
//! | `webhook.idempotency.duplicate.count`| Count        | Duplicate idempotency key detected  |
//! | `webhook.enqueue.failure.count`      | Count        | SQS `enqueue_event` returns `Err`   |
//! | `webhook.receive.latency_ms`         | Milliseconds | End-to-end handler latency          |
//!
//! ## Dimensions (applied consistently across all metrics)
//!
//! | Dimension      | Source                                  |
//! |----------------|-----------------------------------------|
//! | `environment`  | `ENVIRONMENT` env-var (default: `dev`)  |
//! | `customer_id`  | Request `customer_id` field             |
//! | `status_code`  | Final HTTP status code as string        |
//!
//! ## EMF format
//!
//! Each metric is printed as a single JSON log line that CloudWatch Logs
//! automatically extracts into CloudWatch Metrics (no agent required).
//! The format follows the [AWS EMF spec](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html).

use chrono::Utc;
use serde_json::{Map, Value, json};
use std::env;

const DEFAULT_METRIC_NAMESPACE: &str = "HoorayRelay/Ingestion";
const DEFAULT_ENVIRONMENT: &str = "dev";

// ---------------------------------------------------------------------------
// Public struct
// ---------------------------------------------------------------------------

/// Emits structured EMF metric log lines for the ingestion Lambda.
///
/// Construct once at cold-start (cheap — only reads env-vars) and store in
/// [`crate::handlers::webhook::AppState`].
#[derive(Clone, Debug)]
pub struct Observability {
    environment: String,
    namespace: String,
}

impl Observability {
    /// Build from environment variables.
    ///
    /// - `ENVIRONMENT` → `environment` dimension (default `"dev"`)
    /// - `METRIC_NAMESPACE` → EMF namespace (default `"HoorayRelay/Ingestion"`)
    pub fn new() -> Self {
        let environment =
            env::var("ENVIRONMENT").unwrap_or_else(|_| DEFAULT_ENVIRONMENT.to_string());
        let namespace = env::var("METRIC_NAMESPACE")
            .unwrap_or_else(|_| DEFAULT_METRIC_NAMESPACE.to_string());
        Self {
            environment,
            namespace,
        }
    }

    // -----------------------------------------------------------------------
    // High-level emit helpers (called from handlers)
    // -----------------------------------------------------------------------

    /// Emit metrics for a completed `POST /webhooks/receive` call.
    ///
    /// - Always emits `webhook.receive.count` (1).
    /// - Always emits `webhook.receive.latency_ms` (end-to-end handler latency).
    /// - Emits `webhook.idempotency.duplicate.count` when `is_duplicate = true`.
    /// - Emits `webhook.enqueue.failure.count` when `enqueue_failed = true`.
    ///
    /// # Arguments
    ///
    /// * `customer_id`    — the request `customer_id` field.
    /// * `status_code`    — the HTTP status code returned to the caller.
    /// * `latency_ms`     — end-to-end handler latency in milliseconds.
    /// * `is_duplicate`   — true when the idempotency check returned `Duplicate`.
    /// * `enqueue_failed` — true when `queue::enqueue_event` returned `Err`.
    pub fn emit_receive(
        &self,
        customer_id: &str,
        status_code: u16,
        latency_ms: u64,
        is_duplicate: bool,
        enqueue_failed: bool,
    ) {
        let status_str = status_code.to_string();
        let detailed_dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("customer_id".to_string(), customer_id.to_string()),
            ("status_code".to_string(), status_str.clone()),
        ];
        let aggregate_dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("status_code".to_string(), status_str),
        ];

        // Always emit receive count + latency at both detailed and aggregate.
        self.emit_metric("webhook.receive.count", "Count", 1.0, &detailed_dims);
        self.emit_metric("webhook.receive.count", "Count", 1.0, &aggregate_dims);
        self.emit_metric(
            "webhook.receive.latency_ms",
            "Milliseconds",
            latency_ms as f64,
            &detailed_dims,
        );
        self.emit_metric(
            "webhook.receive.latency_ms",
            "Milliseconds",
            latency_ms as f64,
            &aggregate_dims,
        );

        if is_duplicate {
            self.emit_metric(
                "webhook.idempotency.duplicate.count",
                "Count",
                1.0,
                &detailed_dims,
            );
            self.emit_metric(
                "webhook.idempotency.duplicate.count",
                "Count",
                1.0,
                &aggregate_dims,
            );
        }

        if enqueue_failed {
            self.emit_metric(
                "webhook.enqueue.failure.count",
                "Count",
                1.0,
                &detailed_dims,
            );
            self.emit_metric(
                "webhook.enqueue.failure.count",
                "Count",
                1.0,
                &aggregate_dims,
            );
        }
    }

    /// Emit metrics for a `POST /webhooks/configs` call.
    ///
    /// Emits `webhook.config.create.count` with `customer_id` and `status_code`
    /// dimensions.
    pub fn emit_config_create(&self, customer_id: &str, status_code: u16) {
        let status_str = status_code.to_string();
        let dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("customer_id".to_string(), customer_id.to_string()),
            ("status_code".to_string(), status_str),
        ];
        self.emit_metric("webhook.config.create.count", "Count", 1.0, &dims);
    }

    /// Emit metrics for a `GET /webhooks/configs` call.
    ///
    /// Emits `webhook.config.get.count` with `customer_id` and `status_code`
    /// dimensions.
    pub fn emit_config_get(&self, customer_id: &str, status_code: u16) {
        let status_str = status_code.to_string();
        let dims = vec![
            ("environment".to_string(), self.environment.clone()),
            ("customer_id".to_string(), customer_id.to_string()),
            ("status_code".to_string(), status_str),
        ];
        self.emit_metric("webhook.config.get.count", "Count", 1.0, &dims);
    }

    // -----------------------------------------------------------------------
    // Internal EMF emit
    // -----------------------------------------------------------------------

    fn emit_metric(
        &self,
        metric_name: &str,
        unit: &str,
        value: f64,
        dimensions: &[(String, String)],
    ) {
        println!(
            "{}",
            build_emf_payload(&self.namespace, metric_name, unit, value, dimensions)
        );
    }
}

impl Default for Observability {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// EMF payload builder (pure function — easy to unit-test)
// ---------------------------------------------------------------------------

/// Build a CloudWatch EMF-formatted JSON [`Value`] for a single metric.
///
/// The returned value is a flat JSON object that contains:
/// - one key per dimension (e.g. `"environment": "dev"`)
/// - one key for the metric value (e.g. `"webhook.receive.count": 1.0`)
/// - a `_aws` envelope required by the EMF spec
pub fn build_emf_payload(
    namespace: &str,
    metric_name: &str,
    unit: &str,
    value: f64,
    dimensions: &[(String, String)],
) -> Value {
    let mut root = Map::new();

    // Dimension keys for the `_aws.CloudWatchMetrics[].Dimensions` array.
    let dimension_keys: Vec<Value> = dimensions
        .iter()
        .map(|(key, _)| Value::String(key.clone()))
        .collect();

    // Flatten dimension key/value pairs at the root level.
    for (key, val) in dimensions {
        root.insert(key.clone(), Value::String(val.clone()));
    }

    // Metric value at root level.
    root.insert(metric_name.to_string(), Value::from(value));

    // Required `_aws` EMF envelope.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn base_dims() -> Vec<(String, String)> {
        vec![
            ("environment".to_string(), "test".to_string()),
            ("customer_id".to_string(), "cust_abc".to_string()),
            ("status_code".to_string(), "202".to_string()),
        ]
    }

    #[test]
    fn emf_payload_contains_metric_value() {
        let payload = build_emf_payload(
            "HoorayRelay/Ingestion",
            "webhook.receive.count",
            "Count",
            1.0,
            &base_dims(),
        );
        assert_eq!(payload["webhook.receive.count"], 1.0);
    }

    #[test]
    fn emf_payload_contains_all_dimensions_at_root() {
        let payload = build_emf_payload(
            "HoorayRelay/Ingestion",
            "webhook.receive.count",
            "Count",
            1.0,
            &base_dims(),
        );
        assert_eq!(payload["environment"], "test");
        assert_eq!(payload["customer_id"], "cust_abc");
        assert_eq!(payload["status_code"], "202");
    }

    #[test]
    fn emf_payload_has_aws_envelope() {
        let payload = build_emf_payload(
            "HoorayRelay/Ingestion",
            "webhook.receive.count",
            "Count",
            1.0,
            &base_dims(),
        );
        let aws = &payload["_aws"];
        assert!(aws["Timestamp"].is_number(), "_aws.Timestamp must be a number");
        let metrics = &aws["CloudWatchMetrics"][0];
        assert_eq!(metrics["Namespace"], "HoorayRelay/Ingestion");
        assert_eq!(metrics["Metrics"][0]["Name"], "webhook.receive.count");
        assert_eq!(metrics["Metrics"][0]["Unit"], "Count");
    }

    #[test]
    fn emf_payload_dimension_keys_in_envelope() {
        let payload = build_emf_payload(
            "HoorayRelay/Ingestion",
            "webhook.receive.latency_ms",
            "Milliseconds",
            42.0,
            &base_dims(),
        );
        let dim_keys = &payload["_aws"]["CloudWatchMetrics"][0]["Dimensions"][0];
        // Dimensions array should contain all three key names.
        let keys: Vec<&str> = dim_keys
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(keys.contains(&"environment"));
        assert!(keys.contains(&"customer_id"));
        assert!(keys.contains(&"status_code"));
    }

    #[test]
    fn emf_payload_latency_value() {
        let payload = build_emf_payload(
            "HoorayRelay/Ingestion",
            "webhook.receive.latency_ms",
            "Milliseconds",
            123.0,
            &base_dims(),
        );
        assert_eq!(payload["webhook.receive.latency_ms"], 123.0);
    }

    #[test]
    fn observability_new_uses_env_default() {
        // Without env-vars set, Observability::new() should not panic.
        let obs = Observability::new();
        // environment defaults to "dev" when ENVIRONMENT is not set.
        // (env-var may be set in the process, so just confirm it's non-empty.)
        assert!(!obs.environment.is_empty());
        assert!(!obs.namespace.is_empty());
    }
}

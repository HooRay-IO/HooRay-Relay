//! DynamoDB client factory and application configuration.
//!
//! This module provides two things:
//!
//! 1. [`AppConfig`] — reads all table names and the SQS queue URL from
//!    environment variables.  This is the single source of truth for
//!    infrastructure coordinates at runtime.  Both `idempotency.rs` and
//!    `events.rs` receive these values as plain `&str` arguments so they
//!    remain independently testable without touching this module.
//!
//! 2. [`build_dynamo_client`] — constructs an `aws_sdk_dynamodb::Client`
//!    using the ambient AWS configuration (env vars, ~/.aws/credentials, EC2
//!    instance profile, Lambda execution role — whichever is present).
//!    For local development the `AWS_ENDPOINT_URL` override makes it point at
//!    DynamoDB Local or LocalStack automatically.

use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::Client as DynamoClient;

use crate::model::IngestionError;

// ---------------------------------------------------------------------------
// Environment variable names (single definition — no magic strings scattered)
// ---------------------------------------------------------------------------

const ENV_EVENTS_TABLE: &str = "EVENTS_TABLE";
const ENV_IDEMPOTENCY_TABLE: &str = "IDEMPOTENCY_TABLE";
const ENV_CONFIGS_TABLE: &str = "CONFIGS_TABLE";
const ENV_QUEUE_URL: &str = "QUEUE_URL";

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

/// Runtime coordinates for every shared infrastructure resource.
///
/// Construct with [`AppConfig::from_env`] at Lambda cold-start time and pass
/// references into each service call.  Keep this as a plain data struct —
/// no Arc, no Mutex — callers that need shared ownership wrap it themselves.
///
/// # Environment variables
///
/// | Variable | Description |
/// |---|---|
/// | `EVENTS_TABLE` | `webhook_events_{env}` DynamoDB table |
/// | `IDEMPOTENCY_TABLE` | `webhook_idempotency_{env}` DynamoDB table |
/// | `CONFIGS_TABLE` | `webhook_configs_{env}` DynamoDB table |
/// | `QUEUE_URL` | SQS delivery queue URL |
///
/// All four variables are injected by `template.yaml` via the `Globals`
/// `Environment` block and are therefore always present in the Lambda runtime.
/// In local development, export them yourself before running `cargo run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    /// Name of the `webhook_events_{env}` DynamoDB table.
    pub events_table: String,
    /// Name of the `webhook_idempotency_{env}` DynamoDB table.
    pub idempotency_table: String,
    /// Name of the `webhook_configs_{env}` DynamoDB table.
    pub configs_table: String,
    /// SQS delivery queue URL.
    pub queue_url: String,
}

impl AppConfig {
    /// Build an [`AppConfig`] from the process environment.
    ///
    /// Returns [`IngestionError::MissingField`] for the first absent variable.
    ///
    /// # Errors
    ///
    /// - `EVENTS_TABLE` not set
    /// - `IDEMPOTENCY_TABLE` not set
    /// - `CONFIGS_TABLE` not set
    /// - `QUEUE_URL` not set
    pub fn from_env() -> Result<Self, IngestionError> {
        let events_table = require_env(ENV_EVENTS_TABLE)?;
        let idempotency_table = require_env(ENV_IDEMPOTENCY_TABLE)?;
        let configs_table = require_env(ENV_CONFIGS_TABLE)?;
        let queue_url = require_env(ENV_QUEUE_URL)?;

        Ok(Self {
            events_table,
            idempotency_table,
            configs_table,
            queue_url,
        })
    }
}

/// Pull a required environment variable or return [`IngestionError::MissingField`].
fn require_env(key: &str) -> Result<String, IngestionError> {
    std::env::var(key).map_err(|_| IngestionError::MissingField(key.to_owned()))
}

// ---------------------------------------------------------------------------
// DynamoDB client factory
// ---------------------------------------------------------------------------

/// Build an AWS SDK DynamoDB client from the ambient environment.
///
/// Resolution order (standard AWS SDK chain):
/// 1. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`
/// 2. `~/.aws/credentials` + `~/.aws/config`
/// 3. ECS / EC2 instance metadata (IMDSv2)
/// 4. Lambda execution role
///
/// For local development against DynamoDB Local or LocalStack, set:
/// ```bash
/// export AWS_ENDPOINT_URL=http://localhost:8000   # DynamoDB Local
/// export AWS_ENDPOINT_URL=http://localhost:4566   # LocalStack
/// export AWS_ACCESS_KEY_ID=local
/// export AWS_SECRET_ACCESS_KEY=local
/// export AWS_REGION=us-east-1
/// ```
/// The SDK picks up `AWS_ENDPOINT_URL` automatically — no code changes needed.
///
/// This function is `async` because loading `~/.aws/config` and refreshing
/// IMDS credentials are async operations in the SDK.
pub async fn build_dynamo_client() -> DynamoClient {
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    DynamoClient::new(&config)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // Global mutex to serialize environment variable mutations in tests.
    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    // Helper: set multiple env vars, run a closure, then restore the originals.
    // This avoids test-order dependencies when running with `cargo test`.
    fn with_env_vars<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        let _guard = env_lock().lock().unwrap();

        let originals: Vec<(&str, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var(k).ok()))
            .collect();

        for (k, v) in vars {
            std::env::set_var(k, v);
        }

        f();

        for (k, original) in originals {
            match original {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }

    // Remove a list of vars, run a closure, then restore whatever was there.
    fn without_env_vars<F: FnOnce()>(keys: &[&str], f: F) {
        let _guard = env_lock().lock().unwrap();

        let originals: Vec<(&str, Option<String>)> =
            keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();

        for k in keys {
            std::env::remove_var(k);
        }

        f();

        for (k, original) in originals {
            if let Some(v) = original {
                std::env::set_var(k, v);
            }
        }
    }

    #[test]
    fn app_config_loads_all_env_vars() {
        with_env_vars(
            &[
                (ENV_EVENTS_TABLE, "webhook_events_test"),
                (ENV_IDEMPOTENCY_TABLE, "webhook_idempotency_test"),
                (ENV_CONFIGS_TABLE, "webhook_configs_test"),
                (
                    ENV_QUEUE_URL,
                    "https://sqs.us-east-1.amazonaws.com/123/test",
                ),
            ],
            || {
                let cfg = AppConfig::from_env().expect("should succeed with all vars set");
                assert_eq!(cfg.events_table, "webhook_events_test");
                assert_eq!(cfg.idempotency_table, "webhook_idempotency_test");
                assert_eq!(cfg.configs_table, "webhook_configs_test");
                assert_eq!(
                    cfg.queue_url,
                    "https://sqs.us-east-1.amazonaws.com/123/test"
                );
            },
        );
    }

    #[test]
    fn app_config_missing_events_table_returns_error() {
        with_env_vars(
            &[
                (ENV_IDEMPOTENCY_TABLE, "webhook_idempotency_test"),
                (ENV_CONFIGS_TABLE, "webhook_configs_test"),
                (
                    ENV_QUEUE_URL,
                    "https://sqs.us-east-1.amazonaws.com/123/test",
                ),
            ],
            || {
                without_env_vars(&[ENV_EVENTS_TABLE], || {
                    let err = AppConfig::from_env().expect_err("should fail");
                    assert!(
                        matches!(err, IngestionError::MissingField(ref k) if k == ENV_EVENTS_TABLE),
                        "expected MissingField(EVENTS_TABLE), got {err:?}"
                    );
                });
            },
        );
    }

    #[test]
    fn app_config_missing_idempotency_table_returns_error() {
        with_env_vars(
            &[
                (ENV_EVENTS_TABLE, "webhook_events_test"),
                (ENV_CONFIGS_TABLE, "webhook_configs_test"),
                (
                    ENV_QUEUE_URL,
                    "https://sqs.us-east-1.amazonaws.com/123/test",
                ),
            ],
            || {
                without_env_vars(&[ENV_IDEMPOTENCY_TABLE], || {
                    let err = AppConfig::from_env().expect_err("should fail");
                    assert!(
                        matches!(err, IngestionError::MissingField(ref k) if k == ENV_IDEMPOTENCY_TABLE),
                        "expected MissingField(IDEMPOTENCY_TABLE), got {err:?}"
                    );
                });
            },
        );
    }

    #[test]
    fn app_config_missing_configs_table_returns_error() {
        with_env_vars(
            &[
                (ENV_EVENTS_TABLE, "webhook_events_test"),
                (ENV_IDEMPOTENCY_TABLE, "webhook_idempotency_test"),
                (
                    ENV_QUEUE_URL,
                    "https://sqs.us-east-1.amazonaws.com/123/test",
                ),
            ],
            || {
                without_env_vars(&[ENV_CONFIGS_TABLE], || {
                    let err = AppConfig::from_env().expect_err("should fail");
                    assert!(
                        matches!(err, IngestionError::MissingField(ref k) if k == ENV_CONFIGS_TABLE),
                        "expected MissingField(CONFIGS_TABLE), got {err:?}"
                    );
                });
            },
        );
    }

    #[test]
    fn app_config_missing_queue_url_returns_error() {
        with_env_vars(
            &[
                (ENV_EVENTS_TABLE, "webhook_events_test"),
                (ENV_IDEMPOTENCY_TABLE, "webhook_idempotency_test"),
                (ENV_CONFIGS_TABLE, "webhook_configs_test"),
            ],
            || {
                without_env_vars(&[ENV_QUEUE_URL], || {
                    let err = AppConfig::from_env().expect_err("should fail");
                    assert!(
                        matches!(err, IngestionError::MissingField(ref k) if k == ENV_QUEUE_URL),
                        "expected MissingField(QUEUE_URL), got {err:?}"
                    );
                });
            },
        );
    }

    #[test]
    fn app_config_clone_equality() {
        let cfg = AppConfig {
            events_table: "t1".to_string(),
            idempotency_table: "t2".to_string(),
            configs_table: "t3".to_string(),
            queue_url: "https://sqs.test/q".to_string(),
        };
        assert_eq!(cfg.clone(), cfg);
    }
}

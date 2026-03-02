//! Lambda entry point for the ingestion service.
//!
//! ## Cold-start sequence
//!
//! 1. Initialise structured JSON logging (CloudWatch-compatible).
//! 2. Load [`AppConfig`] from environment variables.
//! 3. Build AWS clients (DynamoDB + SQS) using the Lambda execution role.
//! 4. Construct [`AppState`] and wrap it in an `Arc`.
//! 5. Register all Axum routes.
//! 6. Hand the router to `lambda_http` — one invocation per HTTP request (API Gateway/Lambda HTTP).
//!
//! ## Routes
//!
//! | Method | Path                              | Handler             |
//! |--------|-----------------------------------|---------------------|
//! | POST   | `/webhooks/receive`               | [`webhook::receive_webhook`] |
//! | POST   | `/webhooks/configs`               | [`config::create_config`]    |
//! | GET    | `/webhooks/configs`               | [`config::get_config`]       |

pub mod handlers;
pub mod model;
pub mod observability;
pub mod services;

use std::sync::Arc;

use aws_config::BehaviorVersion;
use aws_sdk_sqs::Client as SqsClient;
use axum::Router;
use axum::routing::{get, post};
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use handlers::config::{create_config, get_config};
use handlers::webhook::{AppState, receive_webhook};
use observability::Observability;
use services::dynamodb::{AppConfig, build_dynamo_client};

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    // ------------------------------------------------------------------
    // 1. Structured JSON logging — CloudWatch reads this natively.
    // ------------------------------------------------------------------
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_span_events(FmtSpan::CLOSE)
        .with_target(true)
        .with_current_span(false)
        .init();

    // ------------------------------------------------------------------
    // 2. Load runtime configuration from environment variables.
    // ------------------------------------------------------------------
    let config = AppConfig::from_env().expect("all required env vars must be set at cold-start");

    // ------------------------------------------------------------------
    // 3. Build AWS clients — reused across all warm invocations.
    // ------------------------------------------------------------------
    let aws_cfg = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let dynamo = build_dynamo_client(&aws_cfg).await;
    let sqs = SqsClient::new(&aws_cfg);

    info!(
        events_table = %config.events_table,
        idempotency_table = %config.idempotency_table,
        configs_table = %config.configs_table,
        "ingestion Lambda cold-start complete"
    );

    // ------------------------------------------------------------------
    // 4. Build shared application state.
    // ------------------------------------------------------------------
    let state = Arc::new(AppState {
        dynamo,
        sqs,
        config,
        observability: Observability::new(),
    });

    // ------------------------------------------------------------------
    // 5. Define the Axum router.
    // ------------------------------------------------------------------
    let app = Router::new()
        .route("/webhooks/receive", post(receive_webhook))
        .route("/webhooks/configs", post(create_config))
        .route("/webhooks/configs", get(get_config))
        // API Gateway REST/Lambda sometimes forwards stage-prefixed paths
        // (for example, `/Prod/webhooks/...`) to the handler.
        .route("/{stage}/webhooks/receive", post(receive_webhook))
        .route("/{stage}/webhooks/configs", post(create_config))
        .route("/{stage}/webhooks/configs", get(get_config))
        .with_state(state);

    // ------------------------------------------------------------------
    // 6. Run under the Lambda HTTP adapter.
    //    `lambda_http::run` blocks until the Lambda process is terminated.
    // ------------------------------------------------------------------
    lambda_http::run(app).await
}

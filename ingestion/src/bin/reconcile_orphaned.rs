use aws_config::BehaviorVersion;
use tracing::info;
use tracing_subscriber::EnvFilter;

use ingestion::services::dynamodb::AppConfig;
use ingestion::services::reconcile::reconcile_orphaned_events;

const DEFAULT_MIN_AGE_SECS: i64 = 120;
const DEFAULT_LIMIT: i32 = 25;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(true)
        .init();

    let config = AppConfig::from_env()?;
    let aws_cfg = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let dynamo = aws_sdk_dynamodb::Client::new(&aws_cfg);
    let sqs = aws_sdk_sqs::Client::new(&aws_cfg);

    let min_age_secs = std::env::var("RECONCILE_MIN_AGE_SECS")
        .ok()
        .and_then(|val| val.parse::<i64>().ok())
        .unwrap_or(DEFAULT_MIN_AGE_SECS);

    let limit = std::env::var("RECONCILE_LIMIT")
        .ok()
        .and_then(|val| val.parse::<i32>().ok())
        .unwrap_or(DEFAULT_LIMIT);

    let requeued = reconcile_orphaned_events(
        &dynamo,
        &sqs,
        &config.events_table,
        &config.queue_url,
        min_age_secs,
        limit,
    )
    .await?;

    info!(requeued, "orphaned event reconciliation complete");
    Ok(())
}

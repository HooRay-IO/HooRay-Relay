//! Orphaned event reconciliation helpers.
//!
//! Orphaned events are persisted to DynamoDB but never enqueued due to a
//! transient SQS failure. These helpers re-enqueue them using the GSI
//! marker set by `events::mark_event_orphaned`.

use chrono::Utc;

use crate::model::{EventStatus, IngestionError};
use crate::services::{events, queue};

/// Re-enqueue orphaned events that are older than `min_age_secs`.
///
/// Returns the number of events re-enqueued.
pub async fn reconcile_orphaned_events(
    dynamo: &aws_sdk_dynamodb::Client,
    sqs: &aws_sdk_sqs::Client,
    events_table: &str,
    queue_url: &str,
    min_age_secs: i64,
    limit: i32,
) -> Result<usize, IngestionError> {
    let now = Utc::now().timestamp();
    let cutoff = now.saturating_sub(min_age_secs);

    let orphaned = events::fetch_orphaned_events(dynamo, events_table, cutoff, limit).await?;
    let mut requeued = 0usize;

    for event in orphaned {
        if event.status != EventStatus::Pending || event.attempt_count > 0 {
            continue;
        }

        queue::enqueue_event(sqs, queue_url, &event.event_id, &event.customer_id).await?;
        events::clear_orphaned_marker(dynamo, events_table, &event.event_id).await?;
        requeued += 1;
    }

    Ok(requeued)
}

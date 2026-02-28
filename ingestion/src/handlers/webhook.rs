//! Webhook receive handler — `POST /webhooks/receive`.
//!
//! ## Request pipeline
//!
//! ```text
//! JSON body deserialized
//!   └─ 1. validate()                — 422 on bad input
//!   └─ 2. idempotency::check_and_record()
//!           ├─ Duplicate → 200 with existing event_id
//!           └─ New  ─────────────────────────────────┐
//!   └─ 3. events::create_event()    — 500 on DynamoDB error
//!   └─ 4. queue::enqueue_event()    — 500 on SQS error
//!   └─ 5. 202 Accepted with new event_id
//! ```
//!
//! ## AppState
//!
//! The handler receives an [`AppState`] via Axum's `State` extractor.
//! `AppState` groups the two AWS clients and the application config so the
//! handler can stay pure and testable without touching global state.

use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::{error, info, warn};

use crate::model::{
    IngestionError, ReceiveStatus, WebhookReceiveRequest, WebhookReceiveResponse,
    WebhookValidationError,
};
use crate::observability::Observability;
use crate::services::dynamodb::AppConfig;
use crate::services::idempotency::IdempotencyOutcome;
use crate::services::{events, idempotency, queue};

// ---------------------------------------------------------------------------
// Shared application state (injected via Axum State extractor)
// ---------------------------------------------------------------------------

/// Shared state made available to every Axum handler via `State<Arc<AppState>>`.
///
/// Constructed once at Lambda cold-start and shared across concurrent requests.
pub struct AppState {
    /// DynamoDB client (shared — SDK client is cheaply cloneable internally).
    pub dynamo: aws_sdk_dynamodb::Client,
    /// SQS client (shared).
    pub sqs: aws_sdk_sqs::Client,
    /// Runtime configuration loaded from environment variables.
    pub config: AppConfig,
    /// CloudWatch EMF metric emitter.
    pub observability: Observability,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `POST /webhooks/receive` — ingest a webhook event.
///
/// Validates the request, deduplicates via idempotency check, persists the
/// event to DynamoDB, and enqueues it on SQS for delivery by the worker.
///
/// # Responses
///
/// | Status | Condition |
/// |--------|-----------|
/// | 202    | New event accepted and enqueued |
/// | 200    | Duplicate `idempotency_key` — existing `event_id` returned |
/// | 422    | Request validation failed |
/// | 500    | Unexpected infrastructure error (DynamoDB / SQS) |
pub async fn receive_webhook(
    State(state): State<Arc<AppState>>,
    Json(req): Json<WebhookReceiveRequest>,
) -> Response {
    // Start wall-clock timer for end-to-end latency metric.
    let start = Instant::now();

    // ------------------------------------------------------------------
    // Step 1 — Validate the incoming request.
    // ------------------------------------------------------------------
    if let Err(e) = req.validate() {
        warn!(error = %e, "webhook request failed validation");
        let resp = validation_error_response(e);
        state.observability.emit_receive(
            &req.customer_id,
            422,
            start.elapsed().as_millis() as u64,
            false,
            false,
        );
        return resp;
    }

    // ------------------------------------------------------------------
    // Step 2 — Generate event_id + timestamp, then idempotency check.
    // ------------------------------------------------------------------
    let event_id = generate_event_id();
    let created_at = unix_now_secs();

    let outcome = match idempotency::check_and_record(
        &state.dynamo,
        &state.config.idempotency_table,
        &req.idempotency_key,
        &event_id,
        created_at,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            error!(error = %e, "idempotency check failed");
            let resp = ingestion_error_response(e);
            state.observability.emit_receive(
                &req.customer_id,
                500,
                start.elapsed().as_millis() as u64,
                false,
                false,
            );
            return resp;
        }
    };

    // Early return for duplicates — re-use the originally assigned event_id.
    if let IdempotencyOutcome::Duplicate {
        event_id: existing_id,
    } = outcome
    {
        warn!(
            idempotency_key = %req.idempotency_key,
            existing_event_id = %existing_id,
            "duplicate webhook request — returning existing event_id"
        );

        let body = WebhookReceiveResponse {
            event_id: existing_id,
            status: ReceiveStatus::Duplicate,
            created_at: unix_now_secs(),
        };
        state.observability.emit_receive(
            &req.customer_id,
            200,
            start.elapsed().as_millis() as u64,
            true,
            false,
        );
        return (StatusCode::OK, Json(body)).into_response();
    }

    // ------------------------------------------------------------------
    // Step 3 — Persist the event to DynamoDB.
    // ------------------------------------------------------------------
    let payload = match events::serialize_payload(&req.data) {
        Ok(p) => p,
        Err(e) => {
            let resp = ingestion_error_response(e);
            state.observability.emit_receive(
                &req.customer_id,
                500,
                start.elapsed().as_millis() as u64,
                false,
                false,
            );
            return resp;
        }
    };

    let event = crate::model::WebhookEvent::new(
        event_id.clone(),
        req.customer_id.clone(),
        payload,
        created_at,
    );

    if let Err(e) = events::create_event(&state.dynamo, &state.config.events_table, &event).await {
        error!(error = %e, event_id = %event_id, "failed to persist event to DynamoDB");
        let resp = ingestion_error_response(e);
        state.observability.emit_receive(
            &req.customer_id,
            500,
            start.elapsed().as_millis() as u64,
            false,
            false,
        );
        return resp;
    }

    // ------------------------------------------------------------------
    // Step 4 — Enqueue on SQS.
    // ------------------------------------------------------------------
    if let Err(e) = queue::enqueue_event(
        &state.sqs,
        &state.config.queue_url,
        &event_id,
        &req.customer_id,
    )
    .await
    {
        // At this point the event has already been persisted to DynamoDB and the
        // idempotency record has been written. If SQS enqueue fails, this creates
        // an "orphaned" event that will not be delivered via the queue, and
        // retrying the same request (with the same idempotency key) will be
        // treated as a duplicate and will not enqueue a new message.
        //
        // Recovery strategy: rely on logging/monitoring for this error and
        // perform manual re-queueing or run a separate reconciliation job that
        // scans for persisted-but-not-enqueued events and enqueues them.
        error!(error = %e, event_id = %event_id, "failed to enqueue event on SQS");
        let resp = ingestion_error_response(e);
        state.observability.emit_receive(
            &req.customer_id,
            500,
            start.elapsed().as_millis() as u64,
            false,
            true, // enqueue_failed = true
        );
        return resp;
    }

    // ------------------------------------------------------------------
    // Step 5 — Return 202 Accepted.
    // ------------------------------------------------------------------
    info!(
        event_id = %event_id,
        customer_id = %req.customer_id,
        idempotency_key = %req.idempotency_key,
        "webhook event accepted and enqueued"
    );

    let latency_ms = start.elapsed().as_millis() as u64;
    state.observability.emit_receive(
        &req.customer_id,
        202,
        latency_ms,
        false,
        false,
    );

    let body = WebhookReceiveResponse {
        event_id,
        status: ReceiveStatus::Accepted,
        created_at,
    };
    (StatusCode::ACCEPTED, Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a unique event ID with the `evt_` prefix using nanoid.
fn generate_event_id() -> String {
    format!("evt_{}", nanoid::nanoid!(16))
}

/// Current time as Unix seconds.
fn unix_now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(err) => {
            error!("system clock is before Unix epoch: {}", err);
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Error → HTTP response mapping
// ---------------------------------------------------------------------------

fn validation_error_response(e: WebhookValidationError) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
        .into_response()
}

fn ingestion_error_response(e: IngestionError) -> Response {
    let status = match &e {
        IngestionError::MissingField(_) => StatusCode::INTERNAL_SERVER_ERROR,
        IngestionError::ConfigNotFound(_) => StatusCode::NOT_FOUND,
        IngestionError::AlreadyExists(_) => StatusCode::CONFLICT,
        IngestionError::Serialization(_) => StatusCode::INTERNAL_SERVER_ERROR,
        IngestionError::DynamoDb(_) => StatusCode::INTERNAL_SERVER_ERROR,
        IngestionError::Sqs(_) => StatusCode::INTERNAL_SERVER_ERROR,
        IngestionError::ItemNotFound { .. } => StatusCode::NOT_FOUND,
        IngestionError::DecodeDynamo(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IngestionError, ReceiveStatus, WebhookReceiveResponse};

    // -----------------------------------------------------------------------
    // generate_event_id
    // -----------------------------------------------------------------------

    #[test]
    fn event_id_has_evt_prefix() {
        let id = generate_event_id();
        assert!(
            id.starts_with("evt_"),
            "event_id must start with 'evt_', got: {id}"
        );
    }

    #[test]
    fn event_id_minimum_length() {
        // "evt_" (4) + nanoid(16) = 20 chars minimum.
        let id = generate_event_id();
        assert!(
            id.len() >= 20,
            "event_id must be at least 20 chars, got: {}",
            id.len()
        );
    }

    #[test]
    fn event_ids_are_unique() {
        let ids: Vec<String> = (0..100).map(|_| generate_event_id()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(
            ids.len(),
            unique.len(),
            "all 100 generated IDs must be unique"
        );
    }

    // -----------------------------------------------------------------------
    // unix_now_secs
    // -----------------------------------------------------------------------

    #[test]
    fn unix_now_secs_is_reasonable() {
        let now = unix_now_secs();
        // Must be after 2024-01-01T00:00:00Z and before 2100-01-01T00:00:00Z.
        assert!(now > 1_704_067_200, "timestamp must be after 2024-01-01");
        assert!(now < 4_102_444_800, "timestamp must be before 2100-01-01");
    }

    // -----------------------------------------------------------------------
    // ingestion_error_response HTTP status mapping
    // -----------------------------------------------------------------------

    #[test]
    fn missing_field_maps_to_500() {
        let resp = ingestion_error_response(IngestionError::MissingField("x".into()));
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn config_not_found_maps_to_404() {
        let resp = ingestion_error_response(IngestionError::ConfigNotFound("c".into()));
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn already_exists_maps_to_409() {
        let resp = ingestion_error_response(IngestionError::AlreadyExists("e".into()));
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn dynamodb_error_maps_to_500() {
        let resp = ingestion_error_response(IngestionError::DynamoDb("boom".into()));
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn sqs_error_maps_to_500() {
        let resp = ingestion_error_response(IngestionError::Sqs("boom".into()));
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn item_not_found_maps_to_404() {
        let resp = ingestion_error_response(IngestionError::ItemNotFound {
            entity: "Event",
            key: "evt_123".into(),
        });
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // WebhookReceiveResponse serialization contract
    // -----------------------------------------------------------------------

    #[test]
    fn accepted_response_serializes_correctly() {
        let resp = WebhookReceiveResponse {
            event_id: "evt_abc123".to_string(),
            status: ReceiveStatus::Accepted,
            created_at: 1_707_840_000,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["event_id"], "evt_abc123");
        assert_eq!(parsed["status"], "accepted");
        assert_eq!(parsed["created_at"], 1_707_840_000_i64);
    }

    #[test]
    fn duplicate_response_serializes_correctly() {
        let resp = WebhookReceiveResponse {
            event_id: "evt_existing".to_string(),
            status: ReceiveStatus::Duplicate,
            created_at: 1_707_840_000,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["status"], "duplicate");
    }

    // -----------------------------------------------------------------------
    // Validation error response
    // -----------------------------------------------------------------------

    #[test]
    fn validation_error_maps_to_422() {
        let err = WebhookValidationError::InvalidIdempotencyKey {
            reason: "must not be empty".to_string(),
        };
        let resp = validation_error_response(err);
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}

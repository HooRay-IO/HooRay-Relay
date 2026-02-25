//! Config management handlers — `POST /webhooks/configs` and
//! `GET /webhooks/configs`.
//!
//! ## Endpoints
//!
//! | Method | Path                  | Description                        |
//! |--------|-----------------------|------------------------------------|
//! | POST   | `/webhooks/configs`   | Register a new customer config     |
//! | GET    | `/webhooks/configs`   | Retrieve an existing config by ID  |
//!
//! ## Secret generation
//!
//! When the caller omits `secret` in the POST body, `create_config` generates
//! a signing secret with the format `whsec_{32 random alphanumeric chars}`.
//! The worker uses this value for HMAC-SHA256 signing — it must be treated as
//! a credential and never logged.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

use crate::handlers::webhook::AppState;
use crate::model::{CreateConfigRequest, IngestionError, WebhookConfig};
use crate::services::configs;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters accepted by `GET /webhooks/configs`.
#[derive(Debug, Deserialize)]
pub struct GetConfigQuery {
    pub customer_id: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /webhooks/configs` — register a new customer delivery configuration.
///
/// If `secret` is omitted from the request body, a `whsec_`-prefixed signing
/// secret is generated automatically.  If a config for the given `customer_id`
/// already exists it is overwritten (last-write-wins; callers can rotate their
/// secrets this way).
///
/// # Responses
///
/// | Status | Condition |
/// |--------|-----------|
/// | 201    | Config created / updated successfully |
/// | 500    | DynamoDB write failed |
pub async fn create_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateConfigRequest>,
) -> Response {
    let now = unix_now_secs();

    let secret = req
        .secret
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(generate_secret);

    let config = WebhookConfig {
        customer_id: req.customer_id.clone(),
        url: req.url.clone(),
        secret,
        max_retries: 3,
        active: true,
        created_at: now,
        updated_at: now,
    };

    match configs::put_config(&state.dynamo, &state.config.configs_table, &config).await {
        Ok(()) => {
            info!(
                customer_id = %config.customer_id,
                url = %config.url,
                "webhook config created"
            );
            (StatusCode::CREATED, Json(config.to_response())).into_response()
        }
        Err(e) => {
            error!(
                error = %e,
                customer_id = %req.customer_id,
                "failed to persist webhook config"
            );
            config_error_response(e)
        }
    }
}

/// `GET /webhooks/configs?customer_id=…` — retrieve a customer's delivery config.
///
/// # Responses
///
/// | Status | Condition |
/// |--------|-----------|
/// | 200    | Config found and returned |
/// | 404    | No config registered for this `customer_id` |
/// | 500    | DynamoDB read failed |
pub async fn get_config(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GetConfigQuery>,
) -> Response {
    match configs::fetch_config(
        &state.dynamo,
        &state.config.configs_table,
        &params.customer_id,
    )
    .await
    {
        Ok(config) => {
            info!(customer_id = %params.customer_id, "webhook config retrieved");
            (StatusCode::OK, Json(config.to_response())).into_response()
        }
        Err(e) => {
            error!(
                error = %e,
                customer_id = %params.customer_id,
                "failed to retrieve webhook config"
            );
            config_error_response(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn config_error_response(e: IngestionError) -> Response {
    let status = match &e {
        IngestionError::ConfigNotFound(_) => StatusCode::NOT_FOUND,
        IngestionError::ItemNotFound { .. } => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the current Unix time in seconds.
fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_secs() as i64
}

/// Generate a `whsec_`-prefixed 32-character alphanumeric signing secret.
fn generate_secret() -> String {
    // alphabet: uppercase + lowercase + digits = 62 chars (no ambiguous symbols)
    const ALPHABET: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J',
        'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1',
        '2', '3', '4', '5', '6', '7', '8', '9',
    ];
    let random_part = nanoid::nanoid!(32, ALPHABET);
    format!("whsec_{}", random_part)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // generate_secret
    // -----------------------------------------------------------------------

    #[test]
    fn secret_has_whsec_prefix() {
        let secret = generate_secret();
        assert!(
            secret.starts_with("whsec_"),
            "secret must start with 'whsec_', got: {secret}"
        );
    }

    #[test]
    fn secret_total_length_is_38() {
        // "whsec_" (6) + 32 random chars = 38
        let secret = generate_secret();
        assert_eq!(
            secret.len(),
            38,
            "secret must be 38 chars total, got: {}",
            secret.len()
        );
    }

    #[test]
    fn secret_random_part_is_alphanumeric() {
        let secret = generate_secret();
        let random_part = &secret[6..]; // strip "whsec_"
        assert!(
            random_part.chars().all(|c| c.is_ascii_alphanumeric()),
            "random part must be alphanumeric, got: {random_part}"
        );
    }

    #[test]
    fn secrets_are_unique() {
        let secrets: Vec<String> = (0..50).map(|_| generate_secret()).collect();
        let unique: std::collections::HashSet<_> = secrets.iter().collect();
        assert_eq!(
            secrets.len(),
            unique.len(),
            "all 50 generated secrets must be unique"
        );
    }

    // -----------------------------------------------------------------------
    // config_error_response HTTP status mapping
    // -----------------------------------------------------------------------

    #[test]
    fn config_not_found_maps_to_404() {
        let e = IngestionError::ConfigNotFound("cust_xyz".to_string());
        let resp = config_error_response(e);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn item_not_found_maps_to_404() {
        let e = IngestionError::ItemNotFound {
            entity: "WebhookConfig",
            key: "CUSTOMER#cust_xyz".to_string(),
        };
        let resp = config_error_response(e);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn dynamodb_error_maps_to_500() {
        let e = IngestionError::DynamoDb("connection refused".to_string());
        let resp = config_error_response(e);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn serialization_error_maps_to_500() {
        let e = IngestionError::Serialization("bad json".to_string());
        let resp = config_error_response(e);
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // -----------------------------------------------------------------------
    // unix_now_secs
    // -----------------------------------------------------------------------

    #[test]
    fn unix_now_secs_is_reasonable() {
        let ts = unix_now_secs();
        // 2024-01-01 in Unix time
        assert!(ts > 1_704_067_200, "timestamp looks too old: {ts}");
        // 2035-01-01 in Unix time
        assert!(
            ts < 2_051_222_400,
            "timestamp looks too far in the future: {ts}"
        );
    }

    // -----------------------------------------------------------------------
    // CreateConfigRequest — caller-supplied secret is preserved
    // -----------------------------------------------------------------------

    #[test]
    fn caller_supplied_secret_is_used_as_is() {
        let req = CreateConfigRequest {
            customer_id: "cust_test".to_string(),
            url: "https://example.com/hook".to_string(),
            secret: Some("whsec_mycustomsecret12345678901234".to_string()),
        };

        let secret = req
            .secret
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(generate_secret);

        assert_eq!(secret, "whsec_mycustomsecret12345678901234");
    }

    #[test]
    fn empty_secret_triggers_auto_generation() {
        let req = CreateConfigRequest {
            customer_id: "cust_test".to_string(),
            url: "https://example.com/hook".to_string(),
            secret: Some(String::new()),
        };

        let secret = req
            .secret
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(generate_secret);

        assert!(
            secret.starts_with("whsec_"),
            "should auto-generate: {secret}"
        );
        assert_eq!(secret.len(), 38);
    }

    #[test]
    fn none_secret_triggers_auto_generation() {
        let req = CreateConfigRequest {
            customer_id: "cust_test".to_string(),
            url: "https://example.com/hook".to_string(),
            secret: None,
        };

        let secret = req
            .secret
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(generate_secret);

        assert!(
            secret.starts_with("whsec_"),
            "should auto-generate: {secret}"
        );
        assert_eq!(secret.len(), 38);
    }
}

use crate::{
    model::{DeliveryAttempt, DeliveryResult, Event, WebhookConfig, WorkerError},
    services::signature::SignatureService,
};
use std::time::{Duration, Instant};

const DELIVERY_TIMEOUT_SECS: u64 = 30;

pub struct DeliveryService;

impl DeliveryService {
    pub async fn deliver(
        event: &Event,
        config: &WebhookConfig,
    ) -> Result<(DeliveryResult, DeliveryAttempt), WorkerError> {
        let attempt_number = event.attempt_count + 1;
        let attempted_at = chrono::Utc::now().timestamp();
        let (timestamp, signature) =
            SignatureService::generate_for_now(&config.secret, &event.payload);
        let client = reqwest::Client::new();
        let start = Instant::now();
        let res = client
            .post(&config.url)
            .timeout(Duration::from_secs(DELIVERY_TIMEOUT_SECS))
            .header("Content-Type", "application/json")
            .header("X-Webhook-Signature", signature)
            .header("X-Webhook-Id", &event.event_id)
            .header("X-Webhook-Timestamp", timestamp.to_string())
            .body(event.payload.clone())
            .send()
            .await;
        let response_time_ms = start.elapsed().as_millis() as u64;

        let (result, http_status, error_message) = match res {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    (DeliveryResult::Success, Some(status.as_u16()), None)
                } else {
                    let result = classify_http_status(status.as_u16());
                    (
                        result,
                        Some(status.as_u16()),
                        Some(format!("HTTP {}", status.as_u16())),
                    )
                }
            }
            Err(err) => {
                let result = classify_reqwest_error(&err);
                (result, None, Some(err.to_string()))
            }
        };

        let attempt = DeliveryAttempt::new(
            event.event_id.clone(),
            attempt_number,
            attempted_at,
            http_status,
            response_time_ms,
            error_message,
        );

        Ok((result, attempt))
    }
}

fn classify_http_status(status: u16) -> DeliveryResult {
    match status {
        200..=299 => DeliveryResult::Success,
        408 | 409 | 429 | 500..=599 => DeliveryResult::Retry,
        400 | 401 | 403 | 404 | 422 => DeliveryResult::Exhausted,
        402 | 405..=407 | 410..=421 | 423..=428 | 430..=499 => DeliveryResult::Exhausted,
        _ => DeliveryResult::Retry,
    }
}

fn classify_reqwest_error(err: &reqwest::Error) -> DeliveryResult {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        DeliveryResult::Retry
    } else {
        DeliveryResult::Exhausted
    }
}

#[cfg(test)]
mod tests {
    use super::classify_http_status;
    use crate::model::DeliveryResult;

    #[test]
    fn classify_http_status_matches_contract() {
        assert_eq!(classify_http_status(200), DeliveryResult::Success);
        assert_eq!(classify_http_status(500), DeliveryResult::Retry);
        assert_eq!(classify_http_status(429), DeliveryResult::Retry);
        assert_eq!(classify_http_status(404), DeliveryResult::Exhausted);
        assert_eq!(classify_http_status(422), DeliveryResult::Exhausted);
    }
}

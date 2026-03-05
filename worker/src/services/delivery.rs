use crate::{
    model::{
        DeliveryAttempt, DeliveryClassification, DeliveryErrorClass, DeliveryResult, Event,
        WebhookConfig, WorkerError,
    },
    services::signature::SignatureService,
};
use std::time::{Duration, Instant};

const DELIVERY_TIMEOUT_SECS: u64 = 30;

pub struct DeliveryService {
    client: reqwest::Client,
}

impl Default for DeliveryService {
    fn default() -> Self {
        Self::new()
    }
}

impl DeliveryService {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    pub async fn deliver(
        &self,
        event: &Event,
        config: &WebhookConfig,
    ) -> Result<(DeliveryClassification, DeliveryAttempt), WorkerError> {
        let attempt_number = event.attempt_count + 1;
        let attempted_at = chrono::Utc::now().timestamp();
        let (timestamp, signature) =
            SignatureService::generate_for_now(&config.secret, &event.payload);
        let start = Instant::now();
        let res = self
            .client
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

        let (classification, http_status, error_message) = match res {
            Ok(response) => {
                let status = response.status();
                let classification = classify_http_status(status.as_u16());
                let message = if classification.result == DeliveryResult::Success {
                    None
                } else {
                    Some(format!(
                        "[{}] HTTP {}",
                        classification.class.as_str(),
                        status.as_u16()
                    ))
                };
                (classification, Some(status.as_u16()), message)
            }
            Err(err) => {
                let classification = classify_reqwest_error(&err);
                let class = classification.class.as_str();
                (classification, None, Some(format!("[{}] {}", class, err)))
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

        Ok((classification, attempt))
    }
}

fn classify_http_status(status: u16) -> DeliveryClassification {
    match status {
        200..=299 => DeliveryClassification::from_class(DeliveryErrorClass::None),
        408 | 409 => DeliveryClassification::from_class(DeliveryErrorClass::HttpOther),
        429 => DeliveryClassification::from_class(DeliveryErrorClass::HttpRateLimited),
        500..=599 => DeliveryClassification::from_class(DeliveryErrorClass::HttpServerError),
        400..=499 => DeliveryClassification::from_class(DeliveryErrorClass::HttpClientError),
        _ => DeliveryClassification::from_class(DeliveryErrorClass::HttpOther),
    }
}

fn classify_reqwest_error(err: &reqwest::Error) -> DeliveryClassification {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        let class = if err.is_timeout() {
            DeliveryErrorClass::NetworkTimeout
        } else if err.is_connect() {
            DeliveryErrorClass::NetworkConnect
        } else {
            DeliveryErrorClass::NetworkRequest
        };
        DeliveryClassification::from_class(class)
    } else {
        DeliveryClassification::from_class(DeliveryErrorClass::TransportOther)
    }
}

#[cfg(test)]
mod tests {
    use super::{DeliveryErrorClass, classify_http_status};
    use crate::model::DeliveryResult;

    #[test]
    fn classify_http_status_matches_contract() {
        let ok = classify_http_status(200);
        assert_eq!(ok.result, DeliveryResult::Success);
        assert_eq!(ok.class, DeliveryErrorClass::None);

        let server = classify_http_status(500);
        assert_eq!(server.result, DeliveryResult::Retry);
        assert_eq!(server.class, DeliveryErrorClass::HttpServerError);

        let throttled = classify_http_status(429);
        assert_eq!(throttled.result, DeliveryResult::Retry);
        assert_eq!(throttled.class, DeliveryErrorClass::HttpRateLimited);

        let client = classify_http_status(404);
        assert_eq!(client.result, DeliveryResult::Exhausted);
        assert_eq!(client.class, DeliveryErrorClass::HttpClientError);
    }
}

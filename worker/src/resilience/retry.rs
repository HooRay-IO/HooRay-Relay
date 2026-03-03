use chrono::Duration;
use rand::RngExt;

use crate::{
    model::DeliveryResult,
    resilience::{BreakerMode, BreakerState, ResilienceConfig, ResilienceOutcome, RetryDecision},
};

pub fn compute_visibility_timeout(retry_delay: Duration, config: &ResilienceConfig) -> Duration {
    let base_timeout = retry_delay + config.processing_overhead;
    std::cmp::min(
        std::cmp::max(base_timeout, config.min_visibility),
        config.max_visibility,
    )
}

pub fn exponential_backoff(attempt: u32, resilience_config: &ResilienceConfig) -> Duration {
    let mut delay = resilience_config.backoff_base_delay;
    let multiplier = resilience_config.backoff_multiplier.max(1);
    let multiplier_i32 = i32::try_from(multiplier).unwrap_or(i32::MAX);

    for _ in 0..attempt {
        delay = match delay.checked_mul(multiplier_i32) {
            Some(next) => next,
            None => return resilience_config.backoff_max_delay,
        };
        if delay >= resilience_config.backoff_max_delay {
            return resilience_config.backoff_max_delay;
        }
    }

    std::cmp::min(delay, resilience_config.backoff_max_delay)
}

pub fn jitter(delay: Duration, jitter_max: Duration) -> Duration {
    let max_ms = jitter_max.num_milliseconds();
    if max_ms <= 0 {
        return delay;
    }

    let jitter_ms = rand::rng().random_range(0..=max_ms);
    delay + Duration::milliseconds(jitter_ms as i64)
}

pub fn build_resilience_outcome_from_delivery_result(result: DeliveryResult) -> ResilienceOutcome {
    match result {
        DeliveryResult::Success => ResilienceOutcome {
            success: true,
            error: None,
        },
        DeliveryResult::Retry | DeliveryResult::Exhausted => ResilienceOutcome {
            success: false,
            error: Some(format!("{result:?}")),
        },
    }
}

pub fn calculate_retry_decision(
    outcome: &ResilienceOutcome,
    breaker_state: &BreakerState,
    config: &ResilienceConfig,
) -> RetryDecision {
    if breaker_state.mode == BreakerMode::Open {
        return RetryDecision {
            should_retry: false,
            next_attempt_delay: None,
        };
    }

    if outcome.success {
        return RetryDecision {
            should_retry: false,
            next_attempt_delay: None,
        };
    }

    if breaker_state.consecutive_failures >= config.retry_attempts {
        return RetryDecision {
            should_retry: false,
            next_attempt_delay: None,
        };
    }

    let base_delay = exponential_backoff(breaker_state.consecutive_failures, config);
    let next_attempt_delay = jitter(base_delay, config.jitter_max);

    RetryDecision {
        should_retry: true,
        next_attempt_delay: Some(next_attempt_delay),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resilience::{BreakerMode, BreakerState, ResilienceConfig};

    fn closed_breaker_state() -> BreakerState {
        BreakerState {
            mode: BreakerMode::Closed,
            consecutive_failures: 0,
            consecutive_successes: 0,
            opened_at: None,
            next_probe_at: None,
            last_failure_at: None,
            last_success_at: None,
            half_open_in_flight: false,
            version: 0,
        }
    }

    #[test]
    fn retry_decision_retries_on_retry_outcome_when_closed() {
        let config = ResilienceConfig::default();
        let mut state = closed_breaker_state();
        state.consecutive_failures = 1;
        let outcome = build_resilience_outcome_from_delivery_result(DeliveryResult::Retry);

        let decision = calculate_retry_decision(&outcome, &state, &config);

        assert!(decision.should_retry);
        let delay = decision.next_attempt_delay.expect("expected retry delay");
        assert!(delay >= config.backoff_base_delay);
        assert!(delay <= config.backoff_max_delay + config.jitter_max);
    }

    #[test]
    fn retry_decision_stops_when_breaker_open() {
        let config = ResilienceConfig::default();
        let mut state = closed_breaker_state();
        state.mode = BreakerMode::Open;
        let outcome = build_resilience_outcome_from_delivery_result(DeliveryResult::Retry);

        let decision = calculate_retry_decision(&outcome, &state, &config);

        assert!(!decision.should_retry);
        assert!(decision.next_attempt_delay.is_none());
    }

    #[test]
    fn visibility_timeout_is_clamped_with_overhead() {
        let config = ResilienceConfig::default();

        let low = compute_visibility_timeout(Duration::seconds(1), &config);
        assert_eq!(low, config.min_visibility);

        let high = compute_visibility_timeout(Duration::hours(8), &config);
        assert_eq!(high, config.max_visibility);
    }
}

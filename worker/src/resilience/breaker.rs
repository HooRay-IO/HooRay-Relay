use crate::resilience::{BreakerMode, BreakerState, ResilienceConfig, retry::exponential_backoff};

pub fn should_allow_request(breaker_state: &mut BreakerState, now: i64) -> bool {
    match breaker_state.mode {
        BreakerMode::Closed => true,
        BreakerMode::Open => {
            let can_probe = breaker_state
                .next_probe_at
                .map(|next_probe_at| now >= next_probe_at)
                .unwrap_or(false);
            if can_probe {
                breaker_state.mode = BreakerMode::HalfOpen;
                breaker_state.half_open_in_flight = true;
                true
            } else {
                false
            }
        }
        BreakerMode::HalfOpen => {
            if breaker_state.half_open_in_flight {
                false
            } else {
                breaker_state.half_open_in_flight = true;
                true
            }
        }
    }
}

pub fn on_success(breaker_state: &mut BreakerState, now: i64, _config: &ResilienceConfig) {
    breaker_state.consecutive_failures = 0;
    breaker_state.consecutive_successes += 1;
    breaker_state.last_success_at = Some(now);

    if breaker_state.mode == BreakerMode::HalfOpen {
        // Transition back to closed on success in half-open mode
        breaker_state.mode = BreakerMode::Closed;
        breaker_state.opened_at = None;
        breaker_state.next_probe_at = None;
        breaker_state.half_open_in_flight = false;
    }
}

pub fn on_failure(breaker_state: &mut BreakerState, now: i64, config: &ResilienceConfig) {
    breaker_state.consecutive_failures += 1;
    breaker_state.consecutive_successes = 0;
    breaker_state.last_failure_at = Some(now);

    if breaker_state.mode == BreakerMode::HalfOpen {
        // Transition back to open on failure in half-open mode
        breaker_state.mode = BreakerMode::Open;
        breaker_state.opened_at = Some(now);
        breaker_state.next_probe_at = Some(now + config.breaker_recovery_timeout.num_seconds());
        breaker_state.half_open_in_flight = false;
    } else if breaker_state.mode == BreakerMode::Closed
        && breaker_state.consecutive_failures >= config.breaker_failure_threshold
    {
        // Transition to open if failure threshold is reached in closed mode
        breaker_state.mode = BreakerMode::Open;
        breaker_state.opened_at = Some(now);
        breaker_state.next_probe_at = Some(now + config.breaker_recovery_timeout.num_seconds());
        breaker_state.half_open_in_flight = false;
    }
}

pub fn open_until(
    breaker_state: &mut BreakerState,
    resilience_config: &ResilienceConfig,
    now: i64,
) {
    let attempt = breaker_state
        .consecutive_failures
        .saturating_sub(resilience_config.breaker_failure_threshold);
    let backoff = exponential_backoff(attempt, resilience_config);
    let cooldown = std::cmp::max(backoff, resilience_config.breaker_recovery_timeout);

    breaker_state.mode = BreakerMode::Open;
    breaker_state.opened_at = Some(now);
    breaker_state.next_probe_at = Some(now + cooldown.num_seconds());
    breaker_state.half_open_in_flight = false;
}

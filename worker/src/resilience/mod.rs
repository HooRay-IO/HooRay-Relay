pub mod breaker;
pub mod retry;
use chrono::Duration;

pub type UnixTimestamp = i64;

#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    pub retry_attempts: u32,
    pub backoff_base_delay: Duration,
    pub backoff_max_delay: Duration,
    pub backoff_multiplier: u32,
    pub jitter_max: Duration,
    pub breaker_failure_threshold: u32,
    pub breaker_recovery_timeout: Duration,
    pub min_visibility: Duration,
    pub max_visibility: Duration,
    pub processing_overhead: Duration,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            retry_attempts: 5,
            backoff_base_delay: Duration::seconds(5),
            backoff_max_delay: Duration::minutes(5),
            backoff_multiplier: 2,
            jitter_max: Duration::seconds(1),
            breaker_failure_threshold: 5,
            breaker_recovery_timeout: Duration::minutes(1),
            min_visibility: Duration::seconds(30),
            max_visibility: Duration::hours(1),
            processing_overhead: Duration::seconds(15),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryDecision {
    pub should_retry: bool,
    pub next_attempt_delay: Option<Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerMode {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct BreakerState {
    pub mode: BreakerMode,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub opened_at: Option<UnixTimestamp>,
    pub next_probe_at: Option<UnixTimestamp>,
    pub last_failure_at: Option<UnixTimestamp>,
    pub last_success_at: Option<UnixTimestamp>,
    pub half_open_in_flight: bool,
    pub version: u64,
}

#[derive(Debug, Clone)]
pub struct ResilienceOutcome {
    pub success: bool,
    pub error: Option<String>,
}

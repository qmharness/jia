//! Circuit breaker for provider failover.
//!
//! Standard three-state breaker: CLOSED → OPEN → HALF_OPEN → CLOSED.
//! Internal to the provider layer — 天盘 and 人盘 never interact with it.

use std::time::Instant;

/// Circuit breaker state machine.
#[derive(Debug, Clone)]
pub(crate) struct CircuitBreaker {
    state: BreakerState,
    failure_count: u32,
    last_failure: Option<Instant>,
    /// Consecutive failures to transition CLOSED → OPEN.
    failure_threshold: u32,
    /// Cooldown duration before transitioning OPEN → HALF_OPEN.
    cooldown: std::time::Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerState {
    /// Normal — requests pass through.
    Closed,
    /// Open — requests are rejected immediately.
    Open,
    /// Allowing one probe request after cooldown.
    HalfOpen,
}

impl CircuitBreaker {
    pub(crate) fn new(failure_threshold: u32, cooldown_secs: u64) -> Self {
        Self {
            state: BreakerState::Closed,
            failure_count: 0,
            last_failure: None,
            failure_threshold,
            cooldown: std::time::Duration::from_secs(cooldown_secs),
        }
    }

    /// Check if a request is allowed through.
    pub(crate) fn is_open(&mut self, now: Instant) -> bool {
        match self.state {
            BreakerState::Closed => false,
            BreakerState::Open => {
                if let Some(last) = self.last_failure {
                    if now.duration_since(last) >= self.cooldown {
                        self.state = BreakerState::HalfOpen;
                        return false; // allow probe
                    }
                }
                true // still cooling down
            }
            BreakerState::HalfOpen => false, // allow probe
        }
    }

    /// Record a successful request.
    pub(crate) fn record_success(&mut self) {
        self.failure_count = 0;
        self.last_failure = None;
        self.state = BreakerState::Closed;
    }

    /// Record a failed request.
    pub(crate) fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure = Some(Instant::now());
        if self.failure_count >= self.failure_threshold || self.state == BreakerState::HalfOpen {
            self.state = BreakerState::Open;
        }
    }

    /// Test-only: observe the consecutive-failure counter (S1 tests assert a
    /// cancelled turn does NOT reset it via record_llm_success).
    #[cfg(test)]
    pub(crate) fn failure_count(&self) -> u32 {
        self.failure_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_starts_closed() {
        let mut b = CircuitBreaker::new(3, 60);
        assert!(!b.is_open(Instant::now()));
    }

    #[test]
    fn breaker_opens_after_threshold_failures() {
        let mut b = CircuitBreaker::new(2, 60);
        let now = Instant::now();
        b.record_failure();
        assert!(!b.is_open(now));
        b.record_failure();
        assert!(b.is_open(now));
    }

    #[test]
    fn breaker_half_open_after_cooldown() {
        let mut b = CircuitBreaker::new(1, 0); // 0s cooldown
        b.record_failure();
        // After failure + 0s cooldown, half-open on next check
        assert!(!b.is_open(std::time::Instant::now()));
    }

    #[test]
    fn breaker_resets_on_success() {
        let mut b = CircuitBreaker::new(2, 60);
        b.record_failure();
        b.record_success();
        assert_eq!(b.failure_count, 0);
        assert!(!b.is_open(Instant::now()));
    }
}

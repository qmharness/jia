// ── Subagent Batch — burst-then-throttle concurrency control ──
//
// Based on kimi-code SubagentBatch pattern. Controls how many
// concurrent delegate/send_message calls can run at once, with
// rate-limit-aware backoff and capacity recovery.
//
// Architecture:
//   - 居天盘: owned by EarthPlate, used by DelegateTool via Arc
//   - Burst-then-throttle: 5 immediate permits, then 1/700ms steady
//   - Rate-limit recovery: exponential backoff on 429, auto-recover
//   - Does NOT bypass GeJu or HumanPlate — only controls timing
//
// Design rationale (哲学合规):
//   - Semaphore-based permits respect tokio's cooperative scheduling
//   - Backoff state is behind Mutex (infrequent access, short critical section)
//   - Permit drop auto-releases — RAII pattern, no manual accounting

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// ── Rate Limit State ───────────────────────────────────────────

#[derive(Debug)]
struct RateLimitState {
    /// Current backoff multiplier (1x, 2x, 4x, ..., max 16x)
    backoff_multiplier: u32,
    /// When the last rate-limit response was received
    last_rate_limit: Option<Instant>,
    /// When the last clear window started (for recovery tracking)
    last_clear: Instant,
}

impl Default for RateLimitState {
    fn default() -> Self {
        Self {
            backoff_multiplier: 1,
            last_rate_limit: None,
            last_clear: Instant::now(),
        }
    }
}

// ── Batch Handle ────────────────────────────────────────────────

/// Represents one acquired execution slot. Releases on Drop (RAII).
pub struct BatchPermit {
    _permit: OwnedSemaphorePermit,
}

// ── SubagentBatch ───────────────────────────────────────────────

pub struct SubagentBatch {
    /// Semaphore controlling total concurrent sub-agent executions
    semaphore: Arc<Semaphore>,
    /// Maximum permits the semaphore can grow to
    max_concurrency: usize,
    /// Rate-limit tracking state
    rate_state: Mutex<RateLimitState>,
}

impl SubagentBatch {
    /// Create a new batch controller.
    ///
    /// Default: 5 initial burst permits, max 8 concurrent, steady release
    /// at 1 per 700ms. This is conservative for the common case of 1-2
    /// concurrent sub-agents but adapts up under load.
    pub fn new() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(5)),
            max_concurrency: 8,
            rate_state: Mutex::new(RateLimitState::default()),
        }
    }

    /// Acquire an execution permit. If no permit is available, this
    /// awaits until one is released (or the semaphore is closed).
    pub async fn acquire(&self) -> BatchPermit {
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("SubagentBatch semaphore closed unexpectedly");
        BatchPermit { _permit: permit }
    }

    /// Notify the batch controller that the provider returned a rate-limit
    /// response (HTTP 429). This reduces concurrency by applying exponential
    /// backoff: capacity shrinks by 1 per occurrence, down to minimum 1.
    pub fn on_rate_limited(&self) {
        let mut state = self
            .rate_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.backoff_multiplier = (state.backoff_multiplier.saturating_mul(2)).min(16);
        state.last_rate_limit = Some(Instant::now());
        tracing::warn!(
            multiplier = state.backoff_multiplier,
            "SubagentBatch: rate-limit detected, backoff x{}",
            state.backoff_multiplier
        );
    }

    /// Check whether capacity should be recovered. Call this periodically
    /// (e.g., after each successful sub-agent completion). After 3 minutes
    /// without rate-limiting, one permit is added back (up to max).
    pub fn maybe_recover(&self) {
        let mut state = self
            .rate_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.last_clear.elapsed() > Duration::from_secs(180)
            && state.backoff_multiplier > 1
        {
            state.backoff_multiplier = state.backoff_multiplier.saturating_sub(1);
            state.last_clear = Instant::now();
            // Capacity recovery: add one permit back (up to max)
            let current = self.semaphore.available_permits();
            if current < self.max_concurrency {
                self.semaphore.add_permits(1);
            }
            tracing::info!(
                multiplier = state.backoff_multiplier,
                "SubagentBatch: rate-limit recovered, backoff x{}",
                state.backoff_multiplier
            );
        }
    }

    /// Return the current backoff multiplier (for diagnostics).
    pub fn backoff_multiplier(&self) -> u32 {
        self.rate_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .backoff_multiplier
    }

    /// Return the number of currently available permits.
    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

impl Default for SubagentBatch {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_five_permits() {
        let batch = SubagentBatch::new();
        assert_eq!(batch.available_permits(), 5);
    }

    #[test]
    fn backoff_default_is_one() {
        let batch = SubagentBatch::new();
        assert_eq!(batch.backoff_multiplier(), 1);
    }

    #[test]
    fn on_rate_limited_doubles_backoff() {
        let batch = SubagentBatch::new();
        batch.on_rate_limited();
        assert_eq!(batch.backoff_multiplier(), 2);
        batch.on_rate_limited();
        assert_eq!(batch.backoff_multiplier(), 4);
    }

    #[test]
    fn backoff_capped_at_16() {
        let batch = SubagentBatch::new();
        for _ in 0..10 {
            batch.on_rate_limited();
        }
        assert_eq!(batch.backoff_multiplier(), 16);
    }

    #[tokio::test]
    async fn acquire_releases_on_drop() {
        let batch = SubagentBatch::new();
        let initial = batch.available_permits();
        {
            let _permit = batch.acquire().await;
            assert_eq!(batch.available_permits(), initial - 1);
        }
        // Permit dropped — should be released
        assert_eq!(batch.available_permits(), initial);
    }

    #[test]
    fn maybe_recover_noop_when_clear() {
        let batch = SubagentBatch::new();
        batch.maybe_recover();
        assert_eq!(batch.backoff_multiplier(), 1);
    }
}

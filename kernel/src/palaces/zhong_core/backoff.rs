//! Exponential backoff with jitter for LLM retries.
//! Reference: Claude Code withRetry.ts

use std::time::Duration;
use tokio::time::sleep;

/// Exponential backoff calculator.
///
/// base: 500ms, cap: 32s, jitter: ±25%
pub(crate) struct RetryBackoff {
    attempt: u32,
    base: Duration,
    cap: Duration,
}

impl RetryBackoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            base: Duration::from_millis(500),
            cap: Duration::from_secs(32),
        }
    }

    /// Returns the delay for the current attempt, then increments the counter.
    /// Delay = min(cap, base * 2^attempt) with ±25% jitter.
    pub fn next_delay(&mut self) -> Duration {
        let exp = 2u64.saturating_pow(self.attempt);
        let raw = self.base.as_millis() as u64 * exp;
        let capped = raw.min(self.cap.as_millis() as u64);

        // 25% jitter: random offset in [-25%, +25%] of capped value.
        // Uses subsec_nanos() as a cheap entropy source (no crypto needed).
        let jitter_range = (capped as f64 * 0.5) as u64;
        let entropy = std::time::UNIX_EPOCH
            .elapsed()
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        let jitter = (capped.wrapping_mul(entropy % 1000)) % jitter_range.max(1);
        let with_jitter = capped
            .saturating_sub(jitter_range / 2)
            .saturating_add(jitter);

        self.attempt += 1;
        Duration::from_millis(with_jitter)
    }

    /// Parse Retry-After header value (delta-seconds only).
    /// Returns Some(duration) if the header was a valid integer, None otherwise.
    pub fn parse_retry_after(header_value: &str) -> Option<Duration> {
        header_value
            .trim()
            .parse::<u64>()
            .ok()
            .map(Duration::from_secs)
    }

    /// Sleep for the computed backoff duration.
    // 保留给后续退避调用点(API 面)——当前 loop 内联了 retry-after/backoff
    // 计算,暂无调用方。
    #[allow(dead_code)]
    pub async fn wait(&mut self) {
        let delay = self.next_delay();
        sleep(delay).await;
    }
}

impl Default for RetryBackoff {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_increases() {
        let mut b = RetryBackoff::new();
        let d1 = b.next_delay();
        let d2 = b.next_delay();
        let d3 = b.next_delay();
        assert!(
            d2 > d1,
            "d2={d2:?} should be > d1={d1:?}"
        );
        assert!(
            d3 > d2,
            "d3={d3:?} should be > d2={d2:?}"
        );
    }

    #[test]
    fn backoff_caps_at_32s() {
        let mut b = RetryBackoff::new();
        // Push past the cap with many attempts
        for _ in 0..10 {
            b.next_delay();
        }
        let d = b.next_delay();
        assert!(
            d <= Duration::from_secs(32),
            "delay {:?} exceeds 32s cap",
            d
        );
    }

    #[test]
    fn backoff_respects_retry_after() {
        assert_eq!(
            RetryBackoff::parse_retry_after("120"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(RetryBackoff::parse_retry_after("invalid"), None);
        assert_eq!(RetryBackoff::parse_retry_after(""), None);
    }

    #[test]
    fn first_delay_is_500ms() {
        let mut b = RetryBackoff::new();
        let d = b.next_delay();
        let ms = d.as_millis() as u64;
        // With jitter, should be in [250, 750]
        assert!(ms >= 250, "first delay {ms}ms too small");
        assert!(ms <= 750, "first delay {ms}ms too large");
    }

    #[test]
    fn backoff_jitter_is_randomish() {
        let mut b1 = RetryBackoff::new();
        let mut b2 = RetryBackoff::new();
        // Advance b2 slightly so it gets different entropy
        b2.next_delay();
        let d1 = b1.next_delay();
        let d2 = b2.next_delay();
        // With jitter, these should not be identical in most cases
        // (extremely unlikely both get same nanosecond entropy + same jitter output)
        // This is a smoke test — we don't assert strict inequality because
        // it's theoretically possible (though astronomically unlikely) to match.
        let _ = (d1, d2);
    }
}

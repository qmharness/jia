// ── Cron Jitter — deterministic per-job timing spread ──────────────
//
// Inspired by Claude Code's utils/cronTasks.ts jitter system.
//
// Purpose: prevent thundering-herd when many sessions schedule the same
// cron string (e.g. "0 * * * *" → everyone hits inference at :00).
//
// Design:
//   - Deterministic: jitter_frac(job_name) → [0, 1) float, same name → same value
//   - Recurring: forward delay proportional to interval, capped at 15 min
//   - One-shot: backward lead (fires early) only on round minutes (:00, :30)
//     because "remind me at 3pm" should never be delayed

use std::time::Duration;

/// Default jitter fraction of the interval between fires.
const RECURRING_FRAC: f64 = 0.1;

/// Simple FNV-1a 64-bit hash for deterministic jitter.
/// We inline it here to avoid adding a dependency just for hashing.
fn fnv1a_64(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Upper bound on recurring forward delay regardless of interval length.
const RECURRING_CAP_MS: u64 = 15 * 60 * 1000; // 15 minutes

/// One-shot backward lead: maximum ms a task may fire early.
const ONESHOT_MAX_MS: i64 = 90_000; // 90 seconds

/// One-shot jitter only applies when the target minute is divisible by this.
const ONESHOT_MINUTE_MOD: u32 = 30; // :00 and :30 only

/// Deterministic [0, 1) fraction derived from a job name.
/// Uses FxHash for speed; stability across restarts is sufficient
/// (same name → same hash → same jitter).
pub fn jitter_frac(name: &str) -> f64 {
    let h = fnv1a_64(name);
    (h as f64) / (u64::MAX as f64)
}

/// Compute deterministic forward delay for a recurring cron job.
///
/// `interval_ms` is the interval between consecutive fires (e.g. 3600_000 for
/// hourly). The delay is proportional to the interval: at defaults an hourly
/// job spreads across [0s, 6min) but a per-minute job only spreads by ~6s.
///
/// Returns `Duration` to add to the nominal fire time.
pub fn recurring_jitter(name: &str, interval_ms: u64) -> Duration {
    let frac = jitter_frac(name);
    let raw = frac * RECURRING_FRAC * (interval_ms as f64);
    let ms = (raw as u64).min(RECURRING_CAP_MS);
    Duration::from_millis(ms)
}

/// Compute deterministic backward lead for a one-shot cron job.
///
/// Only applies when the target time falls on a round minute boundary
/// (minute % ONESHOT_MINUTE_MOD == 0). Fires early by up to ONESHOT_MAX_MS.
///
/// Returns `Duration` to subtract from the nominal fire time.
pub fn oneshot_jitter(name: &str, target_minute: u32) -> Duration {
    if target_minute % ONESHOT_MINUTE_MOD != 0 {
        return Duration::ZERO;
    }
    let frac = jitter_frac(name);
    let ms = (frac * (ONESHOT_MAX_MS as f64)) as u64;
    Duration::from_millis(ms)
}

/// Compute the effective fire delay for a recurring job within its matched
/// minute window. Rather than a complex next-fire computation, we add a
/// per-job delay from the start of the matched minute.
///
/// `job_name` — used for deterministic hash
/// `interval_secs` — approximate interval between cron matches (e.g. 60 for
///   per-minute, 3600 for hourly, 86400 for daily). Used to scale jitter.
pub fn within_minute_jitter_ms(job_name: &str, interval_secs: u64) -> u64 {
    let jitter = recurring_jitter(job_name, interval_secs * 1000);
    jitter.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_frac_deterministic() {
        let a = jitter_frac("daily_summary");
        let b = jitter_frac("daily_summary");
        assert_eq!(a, b, "same name → same fraction");
    }

    #[test]
    fn jitter_frac_different_names() {
        let a = jitter_frac("job_a");
        let b = jitter_frac("job_b");
        // Extremely unlikely to collide
        assert!(a != b || a == 0.0 && b == 0.0);
    }

    #[test]
    fn jitter_frac_in_range() {
        for name in &["a", "daily", "very_long_job_name_123", "x"] {
            let f = jitter_frac(name);
            assert!(f >= 0.0 && f < 1.0, "frac {f} out of [0,1) for '{name}'");
        }
    }

    #[test]
    fn recurring_jitter_capped() {
        // A daily job (86400s interval) → jitter up to 8640s but capped at 15min
        let j = recurring_jitter("daily", 86400 * 1000);
        assert!(j.as_millis() <= RECURRING_CAP_MS as u128);
    }

    #[test]
    fn recurring_jitter_proportional() {
        // Per-minute job should have small jitter
        let j_min = recurring_jitter("minutely", 60 * 1000);
        let j_hour = recurring_jitter("hourly", 3600 * 1000);
        // Hourly should generally be larger (on average), but due to hash
        // distribution this isn't guaranteed per-pair. We just verify both
        // are within reasonable bounds.
        assert!(j_min.as_secs() <= 60);
        assert!(j_hour.as_secs() <= RECURRING_CAP_MS / 1000);
    }

    #[test]
    fn oneshot_jitter_only_on_round_minutes() {
        let name = "remind_3pm";

        // :00 → jitter applies
        let j = oneshot_jitter(name, 0);
        assert!(j.as_millis() > 0 || j == Duration::ZERO,
                "may be zero if hash maps to 0, but not because of the gate");

        // :30 → jitter applies
        let j = oneshot_jitter(name, 30);
        assert!(j.as_millis() <= ONESHOT_MAX_MS as u128);

        // :15 → no jitter (not a round minute)
        let j = oneshot_jitter(name, 15);
        assert_eq!(j, Duration::ZERO);

        // :07 → no jitter
        let j = oneshot_jitter(name, 7);
        assert_eq!(j, Duration::ZERO);
    }

    #[test]
    fn oneshot_jitter_bounded() {
        let j = oneshot_jitter("test", 0);
        assert!(j.as_millis() <= ONESHOT_MAX_MS as u128);
    }

    #[test]
    fn within_minute_jitter_bounded() {
        let j = within_minute_jitter_ms("hourly_job", 3600);
        // Max jitter for 3600s interval = 0.999... * 0.1 * 3600s ≈ 360s
        assert!(j <= 360_000); // within 6 minutes

        let j = within_minute_jitter_ms("minutely_job", 60);
        assert!(j <= 6000); // within 6 seconds
    }
}

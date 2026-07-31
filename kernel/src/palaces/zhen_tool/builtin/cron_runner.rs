use std::sync::Arc;
// ── Cron Runner — Background task that fires scheduled jobs ──

use std::time::SystemTime;

use super::cron::CronStore;
use super::cron::jitter;

/// Get current local time components: (minute, hour, day, month, weekday)
fn now_local_components() -> (u32, u32, u32, u32, u32) {
    let utc_ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let offset_secs = time::UtcOffset::current_local_offset()
        .unwrap_or(time::UtcOffset::UTC)
        .whole_seconds() as i64;
    let local_ts = (utc_ts + offset_secs) as u64;

    let days_since_epoch = (local_ts / 86400) as i64;
    let time_of_day = local_ts % 86400;
    let hour = (time_of_day / 3600) as u32;
    let minute = ((time_of_day % 3600) / 60) as u32;

    // civil_from_days algorithm (Howard Hinnant)
    let z = days_since_epoch + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    let _year = year;

    let weekday = ((days_since_epoch + 4) % 7) as u32;

    (minute, hour, day, month, weekday)
}

/// Parse a 5-field cron expression and determine if it matches current local time.
///
/// Fields: minute hour day-of-month month day-of-week
/// Supports: exact values, wildcards (*), step values (*/N)
fn cron_matches(expr: &str) -> bool {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }

    let (minute, hour, day, month, weekday) = now_local_components();
    let current = [minute, hour, day, month, weekday];

    for (i, field) in fields.iter().enumerate() {
        if !field_matches(field, current[i]) {
            return false;
        }
    }
    true
}

/// Check whether a one-shot ISO datetime has been reached.
///
/// `schedule` is a local datetime like `2026-05-31T21:14:00`.
/// `job_name` is used to apply deterministic backward jitter — if the target
/// lands on a round minute (:00 or :30), the job fires slightly early to
/// avoid all one-shot tasks hitting inference at the exact same instant.
///
/// Returns true when current local time >= (target_time - jitter).
fn once_matches(schedule: &str, job_name: &str) -> bool {
    let format =
        match time::format_description::parse("[year]-[month]-[day]T[hour]:[minute]:[second]") {
            Ok(f) => f,
            Err(_) => return false,
        };
    let target = match time::PrimitiveDateTime::parse(schedule, &format) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let now = match time::OffsetDateTime::now_local() {
        Ok(n) => n,
        Err(_) => return false,
    };
    let now_local = time::PrimitiveDateTime::new(now.date(), now.time());

    // Backward jitter: if target lands on a round minute, fire up to 90s early.
    let target_minute = target.minute() as u32;
    let jitter_back = jitter::oneshot_jitter(job_name, target_minute);
    let jittered_target = target - jitter_back;

    now_local >= jittered_target
}

fn field_matches(field: &str, current: u32) -> bool {
    if field == "*" {
        return true;
    }
    if let Some(rest) = field.strip_prefix("*/") {
        let step: u32 = match rest.parse() {
            Ok(s) if s > 0 => s,
            _ => return false,
        };
        return current.is_multiple_of(step);
    }
    // Exact value
    if let Ok(v) = field.parse::<u32>() {
        return v == current;
    }
    // Unsupported pattern (range, list, name, etc.) — silently never matches.
    // The CronTool layer validates these at add-time, but a manually-placed
    // file could contain one. Log once so the user knows why it never fires.
    tracing::warn!(
        field = %field,
        "Unsupported cron field pattern (only *, */N, and exact values are supported)"
    );
    false
}

/// Estimate the interval between cron matches in seconds.
/// Used to scale jitter proportionally (hourly → wider spread than per-minute).
fn estimate_cron_interval_secs(expr: &str) -> u64 {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return 60; // default: per-minute
    }
    // If minute field is a step (*/N), interval = N minutes
    if let Some(rest) = fields[0].strip_prefix("*/") {
        if let Ok(n) = rest.parse::<u64>() {
            return n * 60;
        }
    }
    // If hour field is a step, interval ≥ 1 hour
    if let Some(rest) = fields[1].strip_prefix("*/") {
        if let Ok(n) = rest.parse::<u64>() {
            return n * 3600;
        }
    }
    // If minute is wildcard, it's per-minute
    // If hour is wildcard, it's per-minute matching
    if fields[0] == "*" && fields[1] == "*" {
        return 60;
    }
    // If minute is exact and hour is wildcard → every hour at that minute
    if fields[1] == "*" && !fields[0].contains('*') {
        return 3600; // hourly
    }
    // If both minute and hour are exact → daily
    if !fields[1].contains('*') && !fields[0].contains('*') {
        if fields[2] == "*" {
            return 86400; // daily
        }
    }
    // Default: hourly (most common for non-trivial crons)
    3600
}

/// On startup, detect one-shot cron jobs that were scheduled to fire while
/// jia was not running. These jobs are removed from the store and logged.
/// A notification should be surfaced to the user on their next interaction.
fn detect_and_notify_missed(store: &CronStore) {
    let jobs = store.enabled_jobs();
    let now_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let missed: Vec<_> = jobs
        .iter()
        .filter(|j| matches!(j.trigger, crate::palaces::zhen_tool::builtin::cron::TriggerMode::Once))
        .filter(|j| {
            // Parse the ISO datetime and check if it's in the past.
            // The schedule is a LOCAL datetime — use the local offset, not UTC.
            if let Ok(format) =
                time::format_description::parse("[year]-[month]-[day]T[hour]:[minute]:[second]")
            {
                if let Ok(target) = time::PrimitiveDateTime::parse(&j.schedule, &format) {
                    let local_offset = time::UtcOffset::current_local_offset()
                        .unwrap_or(time::UtcOffset::UTC);
                    let target_dt = target.assume_offset(local_offset);
                    return target_dt.unix_timestamp() as u64 + 120 < now_secs;
                    // +120s grace: don't flag jobs whose target was <2min ago (clock skew)
                }
            }
            false
        })
        .collect();

    if !missed.is_empty() {
        for job in &missed {
            tracing::info!(
                job = %job.name,
                schedule = %job.schedule,
                "Missed one-shot cron job — auto-disabling"
            );
            let _ = store.set_enabled(&job.name, false);
        }
        tracing::warn!(
            count = missed.len(),
            jobs = ?missed.iter().map(|j| &j.name).collect::<Vec<_>>(),
            "Detected missed one-shot cron jobs on startup"
        );
    }
}

/// Spawn a background task that checks cron jobs every 15 seconds
/// and fires the injected `spawn` callback for matching jobs.
///
/// P2-2 · C13 解:会话编排(构造 Agent/RunContext)已上天盘
/// (tian_heaven::spawn);此 runner 只负责调度,经闭包回调触发,
/// 不再持有 EarthPlate(震宫 → 地盘方向违规消除)。
pub fn spawn_cron_runner(
    store: Arc<CronStore>,
    spawn: Arc<dyn Fn(String, String) + Send + Sync>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Run an initial missed-task check on startup before entering the loop.
        detect_and_notify_missed(&store);

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;

            // ── Hot-reload: scan for external changes ──
            {
                let (added_or_modified, removed_names) = store.file_store.scan_changes_sync();
                if !removed_names.is_empty() {
                    store.remove_by_names(&removed_names);
                }
                for job_file in added_or_modified {
                    let existing = store
                        .list()
                        .ok()
                        .and_then(|jobs| jobs.into_iter().find(|j| j.name == job_file.name));
                    store.upsert(job_file.into_job(existing.as_ref()));
                }
            }

            let jobs = store.enabled_jobs();

            let now_secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            for job in &jobs {
                let is_once = matches!(
                    job.trigger,
                    crate::palaces::zhen_tool::builtin::cron::TriggerMode::Once
                );
                let matches_now = if is_once {
                    once_matches(&job.schedule, &job.name)
                } else {
                    cron_matches(&job.schedule)
                };
                if !matches_now {
                    continue;
                }

                // Tick-resolution dedup: 15s tick can land multiple times within
                // the same cron-matched minute. Skip if already fired less than 45s ago.
                if let Some(last) = job.last_fired_at
                    && now_secs - last < 45
                {
                    continue;
                }

                // Cooldown: minimum gap between firings (default 20h).
                // Skip for one-shot jobs — they fire once then disable.
                if !is_once
                    && let Some(last) = job.last_fired_at
                    && now_secs - last < job.effective_cooldown()
                {
                    continue;
                }

                // ── Jitter: spread fleet-wide fire times ──
                // For recurring jobs: sleep a deterministic per-job delay
                // to avoid thundering herd on :00 boundary.
                //
                // IMPORTANT: record_fired is called BEFORE the jitter sleep so
                // last_fired_at reflects the tick time, not the post-jitter time.
                // This keeps the 45s dedup guard working correctly (otherwise
                // now_secs - last_fired_at wraps when jitter > 15s).
                store.record_fired(&job.name);

                if !is_once {
                    let interval_secs = estimate_cron_interval_secs(&job.schedule);
                    let jitter_ms = jitter::within_minute_jitter_ms(&job.name, interval_secs);
                    if jitter_ms > 0 {
                        tracing::debug!(
                            job = %job.name,
                            jitter_ms = jitter_ms,
                            "Cron jitter delay"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(jitter_ms)).await;
                    }
                }
                tracing::info!(
                    job = %job.name,
                    schedule = %job.schedule,
                    "Cron job fired"
                );
                spawn(job.name.clone(), job.prompt.clone());

                // One-shot jobs auto-disable after firing.
                if is_once {
                    let _ = store.set_enabled(&job.name, false);
                    tracing::info!(
                        job = %job.name,
                        "One-shot cron job auto-disabled"
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_matches() {
        assert!(field_matches("*", 5));
        assert!(field_matches("*", 0));
    }

    #[test]
    fn test_exact_match() {
        assert!(field_matches("30", 30));
        assert!(!field_matches("30", 15));
    }

    #[test]
    fn test_step_match() {
        assert!(field_matches("*/15", 30));
        assert!(field_matches("*/15", 45));
        assert!(!field_matches("*/15", 31));
    }

    #[test]
    fn test_valid_cron_expr() {
        assert!(cron_matches("* * * * *")); // every minute always matches
    }

    #[test]
    fn test_invalid_field_count() {
        assert!(!cron_matches("* * * *")); // 4 fields
        assert!(!cron_matches(""));
    }
}

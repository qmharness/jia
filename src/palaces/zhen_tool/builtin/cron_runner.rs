// ── Cron Runner — Background task that fires scheduled jobs ──

use std::sync::Arc;
use std::time::SystemTime;

use crate::plates::di_earth::EarthPlate;

use super::cron::CronStore;

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
/// Returns true when current local time >= target time.
fn once_matches(schedule: &str) -> bool {
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
    // Compare as PrimitiveDateTime (local time, no offset)
    let now_local = time::PrimitiveDateTime::new(now.date(), now.time());
    now_local >= target
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
    field.parse::<u32>().map(|v| v == current).unwrap_or(false)
}

/// Spawn a background task that checks cron jobs every 30 seconds
/// and spawns background agent tasks for matching jobs.
pub fn spawn_cron_runner(
    store: Arc<CronStore>,
    earth: Arc<EarthPlate>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
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
                    once_matches(&job.schedule)
                } else {
                    cron_matches(&job.schedule)
                };
                if !matches_now {
                    continue;
                }

                // Tick-resolution dedup: 30s tick can land twice within the same
                // cron-matched minute. Skip if already fired less than 60s ago.
                if let Some(last) = job.last_fired_at
                    && now_secs - last < 60
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

                store.record_fired(&job.name);
                tracing::info!(
                    job = %job.name,
                    schedule = %job.schedule,
                    "Cron job fired"
                );
                earth.spawn_cron_agent(job.name.clone(), job.prompt.clone());

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

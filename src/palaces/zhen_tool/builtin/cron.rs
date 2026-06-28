// ── Cron Tool — Schedule recurring tasks ─────────────────────

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::intent::ExecAction;

const MAX_JOBS: usize = 100;

// ── TriggerMode ─────────────────────────────────────────────

/// Scheduling trigger mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum TriggerMode {
    /// Recurring 5-field cron expression (minute hour day month weekday).
    #[default]
    #[serde(rename = "schedule")]
    Schedule,
    /// One-shot: fires once at an ISO 8601 local datetime (e.g. `2026-05-31T21:14:00`),
    /// then auto-disables.
    #[serde(rename = "once")]
    Once,
}

// ── CronJob ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CronJob {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub enabled: bool,
    pub last_fired_at: Option<u64>,
    pub last_response: Option<String>,
    pub cooldown_secs: Option<u64>,
    pub trigger: TriggerMode,
}

impl CronJob {
    pub fn effective_cooldown(&self) -> u64 {
        self.cooldown_secs.unwrap_or(72_000)
    }

    fn to_file(&self) -> CronJobFile {
        CronJobFile {
            name: self.name.clone(),
            schedule: self.schedule.clone(),
            prompt: self.prompt.clone(),
            enabled: self.enabled,
            cooldown_secs: self.cooldown_secs,
            trigger: self.trigger,
            last_fired_at: self.last_fired_at,
        }
    }
}

// ── CronJobFile (on-disk representation) ─────────────────────

/// On-disk representation of a cron job. Persists runtime fields
/// (`last_fired_at`) so execution history survives restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJobFile {
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub enabled: bool,
    #[serde(default)]
    pub cooldown_secs: Option<u64>,
    #[serde(default)]
    pub trigger: TriggerMode,
    #[serde(default)]
    pub last_fired_at: Option<u64>,
}

impl CronJobFile {
    /// Merge disk state into a `CronJob`, preserving runtime-only fields
    /// from the existing in-memory job. Takes the later `last_fired_at`
    /// (disk or memory) so restarts don't lose execution history.
    pub fn into_job(self, existing: Option<&CronJob>) -> CronJob {
        let last_response = existing.and_then(|j| j.last_response.clone());
        // Take the later of disk and in-memory last_fired_at
        let last_fired_at = match existing.and_then(|j| j.last_fired_at) {
            Some(mem_ts) => match self.last_fired_at {
                Some(disk_ts) => Some(disk_ts.max(mem_ts)),
                None => Some(mem_ts),
            },
            None => self.last_fired_at,
        };
        CronJob {
            name: self.name,
            schedule: self.schedule,
            prompt: self.prompt,
            enabled: self.enabled,
            last_fired_at,
            last_response,
            cooldown_secs: self.cooldown_secs,
            trigger: self.trigger,
        }
    }
}

// ── CronFileStore (persistence + hot-reload) ─────────────────

/// Persistence layer for cron jobs. Each job is a standalone JSON file
/// in `cron/<name>.json`. Tracks mtimes for hot-reload detection.
pub struct CronFileStore {
    dir: PathBuf,
    mtimes: std::sync::Mutex<HashMap<String, u64>>,
}

impl CronFileStore {
    /// `dir` must already exist (created by `CronStore::new`).
    fn new(dir: PathBuf) -> std::io::Result<Self> {
        Ok(Self {
            dir,
            mtimes: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Load all `.json` files from the directory, validate filenames,
    /// and populate the initial mtime cache.
    fn load_all_sync(&self) -> std::io::Result<Vec<CronJob>> {
        let mut jobs = Vec::new();
        let mut mtimes = self.mtimes.lock().unwrap();

        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(jobs),
            Err(e) => return Err(e),
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }

            let file_stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let job_file: CronJobFile = match serde_json::from_str(&content) {
                Ok(j) => j,
                Err(e) => {
                    tracing::warn!(file = %path.display(), error = %e, "Failed to parse cron job file, skipping");
                    continue;
                }
            };

            if job_file.name != file_stem {
                tracing::warn!(
                    file = %path.display(),
                    expected = %file_stem,
                    found = %job_file.name,
                    "Cron job filename/name mismatch, skipping"
                );
                continue;
            }

            // Record mtime
            if let Ok(meta) = entry.metadata()
                && let Ok(modified) = meta.modified()
                && let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH)
            {
                mtimes.insert(file_stem.clone(), dur.as_secs());
            }

            let job = job_file.into_job(None);
            jobs.push(job);
        }

        Ok(jobs)
    }

    /// Write a single job file and update the mtime cache.
    fn persist_one(&self, job: &CronJobFile) {
        let path = self.dir.join(format!("{}.json", job.name));
        if let Ok(json) = serde_json::to_string_pretty(job)
            && std::fs::write(&path, &json).is_ok()
            && let Ok(meta) = std::fs::metadata(&path)
            && let Ok(modified) = meta.modified()
            && let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            self.mtimes
                .lock()
                .unwrap()
                .insert(job.name.clone(), dur.as_secs());
        }
    }

    /// Delete a job file from disk and remove from mtime cache.
    fn remove_file(&self, name: &str) {
        let path = self.dir.join(format!("{name}.json"));
        let _ = std::fs::remove_file(&path);
        self.mtimes.lock().unwrap().remove(name);
    }

    /// Scan the directory for changes since last scan.
    /// Returns (added_or_modified, removed_names). Updates mtime cache.
    pub fn scan_changes_sync(&self) -> (Vec<CronJobFile>, Vec<String>) {
        let mut added_or_modified = Vec::new();
        let mut removed_names = Vec::new();
        let mut current_files: HashMap<String, u64> = HashMap::new();

        // Scan directory
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                    continue;
                };
                let mtime = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                current_files.insert(name, mtime);
            }
        }

        // Detect added/modified
        let mtimes = self.mtimes.lock().unwrap();
        for (name, mtime) in &current_files {
            match mtimes.get(name) {
                None => {
                    // New file
                    let path = self.dir.join(format!("{name}.json"));
                    if let Ok(content) = std::fs::read_to_string(&path)
                        && let Ok(job_file) = serde_json::from_str::<CronJobFile>(&content)
                        && job_file.name == *name
                    {
                        added_or_modified.push(job_file);
                    }
                }
                Some(cached_mtime) if mtime != cached_mtime => {
                    // Modified file
                    let path = self.dir.join(format!("{name}.json"));
                    if let Ok(content) = std::fs::read_to_string(&path)
                        && let Ok(job_file) = serde_json::from_str::<CronJobFile>(&content)
                        && job_file.name == *name
                    {
                        added_or_modified.push(job_file);
                    }
                }
                _ => {}
            }
        }

        // Detect removed
        for name in mtimes.keys() {
            if !current_files.contains_key(name) {
                removed_names.push(name.clone());
            }
        }

        drop(mtimes);

        // Update mtime cache
        let mut mtimes = self.mtimes.lock().unwrap();
        // Remove deleted
        for name in &removed_names {
            mtimes.remove(name);
        }
        // Update/add for current files
        for (name, mtime) in &current_files {
            mtimes.insert(name.clone(), *mtime);
        }

        (added_or_modified, removed_names)
    }
}

// ── Name validation ─────────────────────────────────────────

fn validate_cron_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Cron job name must not be empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "Invalid cron job name '{name}': must not contain path separators"
        ));
    }
    if name.contains(char::is_whitespace) {
        return Err(format!(
            "Invalid cron job name '{name}': must not contain whitespace"
        ));
    }
    Ok(())
}

/// Validate a 5-field cron schedule expression.
///
/// Checks: exactly 5 fields, each field is `*`, `*/N` (N > 0), or a number
/// within the standard range for that position.
fn validate_cron_schedule(schedule: &str) -> Result<(), String> {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "Cron schedule must have 5 fields (minute hour day month weekday), got {}",
            fields.len()
        ));
    }

    let ranges: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
    let field_names = ["minute", "hour", "day", "month", "weekday"];

    for (i, field) in fields.iter().enumerate() {
        let (min, max) = ranges[i];
        if *field == "*" {
            continue;
        }
        if let Some(rest) = field.strip_prefix("*/") {
            let step: u32 = rest.parse().map_err(|_| {
                format!(
                    "Invalid step value in {field_name} field: '{field}'",
                    field_name = field_names[i]
                )
            })?;
            if step == 0 {
                return Err(format!(
                    "Step value must be > 0 in {field_name} field: '{field}'",
                    field_name = field_names[i]
                ));
            }
            continue;
        }
        let value: u32 = field.parse().map_err(|_| {
            format!(
                "Invalid {field_name} field '{field}': must be *, */N, or a number",
                field_name = field_names[i]
            )
        })?;
        if value < min || value > max {
            return Err(format!(
                "Value {value} in {field_name} field out of range [{min}, {max}]",
                field_name = field_names[i]
            ));
        }
    }
    Ok(())
}

/// Validate an ISO 8601 local datetime string for one-shot cron jobs.
///
/// Accepts format: `YYYY-MM-DDTHH:MM:SS` (local time, no timezone).
fn validate_once_schedule(schedule: &str) -> Result<(), String> {
    let format = time::format_description::parse("[year]-[month]-[day]T[hour]:[minute]:[second]")
        .map_err(|e| format!("Internal error: {e}"))?;
    time::PrimitiveDateTime::parse(schedule, &format).map_err(|e| {
        format!("Invalid one-shot datetime: {e}. Expected format: YYYY-MM-DDTHH:MM:SS (local time)")
    })?;
    Ok(())
}

// ── CronStore ───────────────────────────────────────────────

/// In-memory cron job store with file-backed persistence.
/// Shared between CronTool, CronRunner, and REST API.
pub struct CronStore {
    pub jobs: std::sync::Mutex<Vec<CronJob>>,
    pub file_store: CronFileStore,
}

impl CronStore {
    /// Create a new `CronStore` with file persistence in `persist_dir`.
    /// Creates the directory if it doesn't exist, loads any existing job files.
    pub fn new(persist_dir: PathBuf) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&persist_dir);
        let file_store = CronFileStore::new(persist_dir).expect("CronFileStore::new");
        let jobs = file_store.load_all_sync().unwrap_or_default();
        Arc::new(Self {
            jobs: std::sync::Mutex::new(jobs),
            file_store,
        })
    }

    fn lock_jobs(&self) -> Result<std::sync::MutexGuard<'_, Vec<CronJob>>, String> {
        self.jobs
            .lock()
            .map_err(|e| format!("Cron store poisoned: {e}"))
    }

    pub fn add(
        &self,
        name: &str,
        schedule: &str,
        prompt: &str,
        cooldown_secs: Option<u64>,
        trigger: TriggerMode,
    ) -> Result<(), String> {
        validate_cron_name(name)?;
        match trigger {
            TriggerMode::Schedule => validate_cron_schedule(schedule)?,
            TriggerMode::Once => validate_once_schedule(schedule)?,
        }
        let job_file;
        {
            let mut guard = self.lock_jobs()?;
            if guard.len() >= MAX_JOBS {
                return Err(format!("Max {MAX_JOBS} cron jobs reached"));
            }
            guard.retain(|j| j.name != name);
            let job = CronJob {
                name: name.into(),
                schedule: schedule.into(),
                prompt: prompt.into(),
                enabled: true,
                last_fired_at: None,
                last_response: None,
                cooldown_secs,
                trigger,
            };
            job_file = job.to_file();
            guard.push(job);
        }
        self.file_store.persist_one(&job_file);
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<CronJob>, String> {
        let guard = self.lock_jobs()?;
        Ok(guard.clone())
    }

    pub fn remove(&self, name: &str) -> Result<(), String> {
        {
            let mut guard = self.lock_jobs()?;
            let len_before = guard.len();
            guard.retain(|j| j.name != name);
            if guard.len() >= len_before {
                return Err(format!("Cron job '{}' not found", name));
            }
        }
        self.file_store.remove_file(name);
        Ok(())
    }

    pub fn update(
        &self,
        name: &str,
        schedule: Option<&str>,
        prompt: Option<&str>,
        cooldown_secs: Option<u64>,
    ) -> Result<(), String> {
        let job_file;
        {
            let mut guard = self.lock_jobs()?;
            let job = guard
                .iter_mut()
                .find(|j| j.name == name)
                .ok_or_else(|| format!("Cron job '{}' not found", name))?;
            if let Some(s) = schedule {
                validate_cron_schedule(s)?;
                job.schedule = s.into();
            }
            if let Some(p) = prompt {
                job.prompt = p.into();
            }
            if cooldown_secs.is_some() {
                job.cooldown_secs = cooldown_secs;
            }
            job_file = job.to_file();
        }
        self.file_store.persist_one(&job_file);
        Ok(())
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        let job_file;
        {
            let mut guard = self.lock_jobs()?;
            let job = guard
                .iter_mut()
                .find(|j| j.name == name)
                .ok_or_else(|| format!("Cron job '{}' not found", name))?;
            job.enabled = enabled;
            job_file = job.to_file();
        }
        self.file_store.persist_one(&job_file);
        Ok(())
    }

    pub fn record_fired(&self, name: &str) {
        let job_file;
        {
            let mut guard = self.lock_jobs().unwrap();
            if let Some(job) = guard.iter_mut().find(|j| j.name == name) {
                job.last_fired_at = Some(crate::utils::unix_now() as u64);
                job_file = Some(job.to_file());
            } else {
                job_file = None;
            }
        }
        if let Some(jf) = job_file {
            self.file_store.persist_one(&jf);
        }
    }

    pub fn set_last_response(&self, name: &str, response: String) {
        if let Ok(mut guard) = self.lock_jobs()
            && let Some(job) = guard.iter_mut().find(|j| j.name == name)
        {
            job.last_response = Some(response);
        }
    }

    /// Upsert a job from hot-reload. If the job already exists in memory,
    /// replace its config fields but preserve runtime fields.
    pub fn upsert(&self, job: CronJob) {
        let mut guard = self.lock_jobs().unwrap();
        if let Some(existing) = guard.iter_mut().find(|j| j.name == job.name) {
            existing.schedule = job.schedule;
            existing.prompt = job.prompt;
            existing.enabled = job.enabled;
            existing.cooldown_secs = job.cooldown_secs;
            existing.trigger = job.trigger;
        } else {
            guard.push(job);
        }
    }

    /// Remove jobs by name (for hot-reload deletions).
    pub fn remove_by_names(&self, names: &[String]) {
        let mut guard = self.lock_jobs().unwrap();
        guard.retain(|j| !names.contains(&j.name));
    }

    /// Return a snapshot of all enabled jobs.
    pub fn enabled_jobs(&self) -> Vec<CronJob> {
        let guard = self.lock_jobs().unwrap();
        guard.iter().filter(|j| j.enabled).cloned().collect()
    }
}

// ── CronTool ────────────────────────────────────────────────

pub struct CronTool {
    store: Arc<CronStore>,
}

impl CronTool {
    pub fn new(store: Arc<CronStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl BaseTool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> String {
        "Schedule recurring tasks. Actions: add, list, remove, enable, disable.".to_string()
    }

    fn category(&self) -> &str {
        "cron"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Geng(ExecAction {
            command: "cron".into(),
        })
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove", "enable", "disable"],
                    "description": "Action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Job name (required for add/remove/enable/disable)"
                },
                "schedule": {
                    "type": "string",
                    "description": "For trigger=schedule: 5-field cron (min hour dom month dow). For trigger=once: ISO local datetime like '2026-05-31T21:14:00'. Required for add."
                },
                "prompt": {
                    "type": "string",
                    "description": "Prompt to enqueue when job fires. Required for add."
                },
                "cooldown_secs": {
                    "type": "integer",
                    "description": "Minimum seconds between firings (default 72000 = 20h). Set to 0 to disable. Ignored for one-shot jobs."
                },
                "trigger": {
                    "type": "string",
                    "enum": ["schedule", "once"],
                    "description": "Trigger mode. 'schedule' = recurring (default), 'once' = fire once at the given datetime then auto-disable."
                }
            },
            "required": ["action"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> Result<String, String> {
        let action = input["action"]
            .as_str()
            .ok_or("Missing 'action' parameter")?;

        match action {
            "add" => {
                let name = input["name"].as_str().ok_or("Missing 'name' for add")?;
                let schedule = input["schedule"]
                    .as_str()
                    .ok_or("Missing 'schedule' for add")?;
                let prompt = input["prompt"].as_str().ok_or("Missing 'prompt' for add")?;
                let cooldown_secs = input["cooldown_secs"].as_u64();
                let trigger = match input["trigger"].as_str() {
                    Some("once") => TriggerMode::Once,
                    _ => TriggerMode::Schedule,
                };

                self.store
                    .add(name, schedule, prompt, cooldown_secs, trigger)?;
                let mode = match trigger {
                    TriggerMode::Once => " (one-shot)",
                    _ => "",
                };
                Ok(format!("Cron job '{}' added: {}{}", name, schedule, mode))
            }
            "list" => {
                let jobs = self.store.list()?;
                if jobs.is_empty() {
                    return Ok("No cron jobs configured.".into());
                }
                let lines: Vec<String> = jobs
                    .iter()
                    .map(|j| {
                        let fired = j.last_fired_at.map_or("never".into(), |t| format!("{t}"));
                        format!(
                            "  {} [{}] {}: {}  last_fired: {}",
                            j.name,
                            if j.enabled { "on" } else { "off" },
                            j.schedule,
                            j.prompt,
                            fired,
                        )
                    })
                    .collect();
                Ok(format!("Cron jobs:\n{}", lines.join("\n")))
            }
            "remove" => {
                let name = input["name"].as_str().ok_or("Missing 'name' for remove")?;
                self.store.remove(name)?;
                Ok(format!("Removed cron job '{}'", name))
            }
            "enable" | "disable" => {
                let name = input["name"].as_str().ok_or("Missing 'name'")?;
                let enabled = action == "enable";
                self.store.set_enabled(name, enabled)?;
                Ok(format!(
                    "Cron job '{}' {}",
                    name,
                    if enabled { "enabled" } else { "disabled" }
                ))
            }
            _ => Err(format!(
                "Unknown action: {action}. Valid: add, list, remove, enable, disable"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_cron_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_path_separators() {
        assert!(validate_cron_name("foo/bar").is_err());
        assert!(validate_cron_name("foo\\bar").is_err());
        assert!(validate_cron_name("foo..bar").is_err());
        assert!(validate_cron_name("../escape").is_err());
    }

    #[test]
    fn validate_name_rejects_whitespace() {
        assert!(validate_cron_name("my job").is_err());
        assert!(validate_cron_name("job\tname").is_err());
    }

    #[test]
    fn validate_name_accepts_valid() {
        assert!(validate_cron_name("daily_summary").is_ok());
        assert!(validate_cron_name("my-job").is_ok());
        assert!(validate_cron_name("task_123").is_ok());
    }

    #[test]
    fn cron_job_file_round_trip() {
        let job = CronJob {
            name: "test".into(),
            schedule: "0 9 * * *".into(),
            prompt: "hello".into(),
            enabled: true,
            last_fired_at: Some(12345),
            last_response: Some("resp".into()),
            cooldown_secs: Some(3600),
            trigger: TriggerMode::Schedule,
        };
        let file = job.to_file();
        let restored = file.into_job(Some(&job));
        assert_eq!(restored.name, "test");
        assert_eq!(restored.schedule, "0 9 * * *");
        assert_eq!(restored.prompt, "hello");
        assert!(restored.enabled);
        assert_eq!(restored.last_fired_at, Some(12345));
        assert_eq!(restored.last_response, Some("resp".into()));
        assert_eq!(restored.cooldown_secs, Some(3600));
    }

    #[test]
    fn cron_job_file_into_job_preserves_runtime_fields() {
        let existing = CronJob {
            name: "test".into(),
            schedule: "0 9 * * *".into(),
            prompt: "old".into(),
            enabled: true,
            last_fired_at: Some(99999),
            last_response: Some("old_resp".into()),
            cooldown_secs: None,
            trigger: TriggerMode::Schedule,
        };
        let file = CronJobFile {
            name: "test".into(),
            schedule: "0 12 * * *".into(),
            prompt: "new".into(),
            enabled: false,
            cooldown_secs: Some(7200),
            trigger: TriggerMode::Schedule,
            last_fired_at: None,
        };
        let merged = file.into_job(Some(&existing));
        // Config fields from file
        assert_eq!(merged.schedule, "0 12 * * *");
        assert_eq!(merged.prompt, "new");
        assert!(!merged.enabled);
        assert_eq!(merged.cooldown_secs, Some(7200));
        // Runtime fields from existing
        assert_eq!(merged.last_fired_at, Some(99999));
        assert_eq!(merged.last_response, Some("old_resp".into()));
    }

    #[test]
    fn cron_job_file_into_job_no_existing() {
        let file = CronJobFile {
            name: "newjob".into(),
            schedule: "* * * * *".into(),
            prompt: "p".into(),
            enabled: true,
            cooldown_secs: None,
            trigger: TriggerMode::Schedule,
            last_fired_at: None,
        };
        let job = file.into_job(None);
        assert_eq!(job.name, "newjob");
        assert!(job.last_fired_at.is_none());
        assert!(job.last_response.is_none());
    }

    #[test]
    fn effective_defaults() {
        let job = CronJob {
            name: "t".into(),
            schedule: "* * * * *".into(),
            prompt: "p".into(),
            enabled: true,
            last_fired_at: None,
            last_response: None,
            cooldown_secs: None,
            trigger: TriggerMode::Schedule,
        };
        assert_eq!(job.effective_cooldown(), 72_000);
    }

    #[test]
    fn effective_custom_cooldown() {
        let job = CronJob {
            name: "t".into(),
            schedule: "* * * * *".into(),
            prompt: "p".into(),
            enabled: true,
            last_fired_at: None,
            last_response: None,
            cooldown_secs: Some(60),
            trigger: TriggerMode::Schedule,
        };
        assert_eq!(job.effective_cooldown(), 60);
    }

    #[test]
    fn cron_store_add_list_remove() {
        let dir = std::env::temp_dir().join(format!("jia_cron_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        // Add
        store
            .add("job1", "0 9 * * *", "prompt 1", None, TriggerMode::Schedule)
            .unwrap();
        store
            .add(
                "job2",
                "0 12 * * *",
                "prompt 2",
                Some(3600),
                TriggerMode::Schedule,
            )
            .unwrap();

        // List
        let jobs = store.list().unwrap();
        assert_eq!(jobs.len(), 2);
        let j1 = jobs.iter().find(|j| j.name == "job1").unwrap();
        assert!(j1.enabled);
        assert_eq!(j1.cooldown_secs, None);
        let j2 = jobs.iter().find(|j| j.name == "job2").unwrap();
        assert_eq!(j2.cooldown_secs, Some(3600));

        // Remove
        store.remove("job1").unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.remove("nonexistent").is_err());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_enabled_jobs() {
        let dir = std::env::temp_dir().join(format!("jia_cron_enabled_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        store
            .add("a", "0 9 * * *", "p", None, TriggerMode::Schedule)
            .unwrap();
        store
            .add("b", "0 12 * * *", "p", None, TriggerMode::Schedule)
            .unwrap();
        store.set_enabled("b", false).unwrap();

        let enabled = store.enabled_jobs();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "a");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_persistence_round_trip() {
        let dir = std::env::temp_dir().join(format!("jia_cron_persist_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        store
            .add(
                "persist_test",
                "0 9 * * *",
                "hello",
                Some(123),
                TriggerMode::Schedule,
            )
            .unwrap();

        // Load a new store from the same dir
        let store2 = CronStore::new(dir.clone());
        let jobs = store2.list().unwrap();
        let job = jobs.iter().find(|j| j.name == "persist_test").unwrap();
        assert_eq!(job.schedule, "0 9 * * *");
        assert_eq!(job.prompt, "hello");
        assert_eq!(job.cooldown_secs, Some(123));
        assert!(job.enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_set_enabled_persists() {
        let dir =
            std::env::temp_dir().join(format!("jia_cron_enabled_persist_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        store
            .add("toggle", "0 9 * * *", "p", None, TriggerMode::Schedule)
            .unwrap();
        store.set_enabled("toggle", false).unwrap();

        let store2 = CronStore::new(dir.clone());
        let jobs = store2.list().unwrap();
        assert!(!jobs.iter().find(|j| j.name == "toggle").unwrap().enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_max_jobs() {
        let dir = std::env::temp_dir().join(format!("jia_cron_max_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        for i in 0..MAX_JOBS {
            store
                .add(
                    &format!("job{i}"),
                    "* * * * *",
                    "p",
                    None,
                    TriggerMode::Schedule,
                )
                .unwrap();
        }
        assert!(
            store
                .add(
                    "one_too_many",
                    "* * * * *",
                    "p",
                    None,
                    TriggerMode::Schedule
                )
                .is_err()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_upsert_merges_runtime() {
        let dir = std::env::temp_dir().join(format!("jia_cron_upsert_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        store
            .add(
                "upsert_test",
                "0 9 * * *",
                "original",
                None,
                TriggerMode::Schedule,
            )
            .unwrap();
        store.record_fired("upsert_test");

        // Upsert a modified version
        let upserted = CronJob {
            name: "upsert_test".into(),
            schedule: "0 12 * * *".into(),
            prompt: "modified".into(),
            enabled: false,
            last_fired_at: None, // would be overwritten from existing
            last_response: None,
            cooldown_secs: Some(999),
            trigger: TriggerMode::Schedule,
        };
        store.upsert(upserted);

        let jobs = store.list().unwrap();
        let job = jobs.iter().find(|j| j.name == "upsert_test").unwrap();
        assert_eq!(job.schedule, "0 12 * * *");
        assert_eq!(job.prompt, "modified");
        assert!(!job.enabled);
        assert_eq!(job.cooldown_secs, Some(999));
        assert!(job.last_fired_at.is_some(), "runtime field preserved");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_remove_by_names() {
        let dir = std::env::temp_dir().join(format!("jia_cron_rm_names_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        store
            .add("x", "* * * * *", "p", None, TriggerMode::Schedule)
            .unwrap();
        store
            .add("y", "* * * * *", "p", None, TriggerMode::Schedule)
            .unwrap();
        store.remove_by_names(&["x".into()]);
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.list().unwrap()[0].name, "y");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trigger_mode_serde() {
        let json = serde_json::to_string(&TriggerMode::Schedule).unwrap();
        assert_eq!(json, r#""schedule""#);

        let parsed: TriggerMode = serde_json::from_str(r#""schedule""#).unwrap();
        assert!(matches!(parsed, TriggerMode::Schedule));

        // Missing field → default
        let parsed: TriggerMode = serde_json::from_str("null").unwrap_or_default();
        assert!(matches!(parsed, TriggerMode::Schedule));
    }

    #[test]
    fn cron_job_file_deser_defaults() {
        let json = r#"{"name":"j","schedule":"* * * * *","prompt":"p","enabled":true}"#;
        let file: CronJobFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.name, "j");
        assert!(file.cooldown_secs.is_none());
        assert!(matches!(file.trigger, TriggerMode::Schedule));
    }

    #[test]
    fn cron_store_add_rejects_invalid_name() {
        let dir = std::env::temp_dir().join(format!("jia_cron_badname_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());
        assert!(
            store
                .add("bad/name", "* * * * *", "p", None, TriggerMode::Schedule)
                .is_err()
        );
        assert!(
            store
                .add("bad..name", "* * * * *", "p", None, TriggerMode::Schedule)
                .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_file_store_hot_reload_detect_new() {
        let dir = std::env::temp_dir().join(format!("jia_cron_hotreload_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        // Initially empty
        assert_eq!(store.list().unwrap().len(), 0);

        // Simulate external file creation
        let job_file = CronJobFile {
            name: "external".into(),
            schedule: "0 9 * * *".into(),
            prompt: "ext".into(),
            enabled: true,
            cooldown_secs: None,
            trigger: TriggerMode::Schedule,
            last_fired_at: None,
        };
        let path = dir.join("external.json");
        std::fs::write(&path, serde_json::to_string_pretty(&job_file).unwrap()).unwrap();

        // scan_changes_sync should detect it
        let (added, removed) = store.file_store.scan_changes_sync();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].name, "external");
        assert!(removed.is_empty());

        // Apply via upsert
        for jf in added {
            let existing = store
                .list()
                .unwrap()
                .into_iter()
                .find(|j| j.name == jf.name);
            store.upsert(jf.into_job(existing.as_ref()));
        }
        assert_eq!(store.list().unwrap().len(), 1);

        // Second scan should see no changes
        let (added2, removed2) = store.file_store.scan_changes_sync();
        assert!(added2.is_empty());
        assert!(removed2.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_file_store_hot_reload_detect_deleted() {
        let dir = std::env::temp_dir().join(format!("jia_cron_hotdel_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        // Add a job → persist, then scan to set initial mtime
        store
            .add("todel", "* * * * *", "p", None, TriggerMode::Schedule)
            .unwrap();
        // Reset mtimes to simulate clean state after loading
        {
            let mut mtimes = store.file_store.mtimes.lock().unwrap();
            mtimes.clear();
        }
        let _ = store.file_store.scan_changes_sync(); // seed mtimes

        // Delete the file
        std::fs::remove_file(dir.join("todel.json")).unwrap();

        let (_added, removed) = store.file_store.scan_changes_sync();
        assert!(removed.contains(&"todel".to_string()));

        store.remove_by_names(&removed);
        assert!(store.list().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_store_update() {
        let dir = std::env::temp_dir().join(format!("jia_cron_upd_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let store = CronStore::new(dir.clone());

        store
            .add(
                "upd",
                "0 9 * * *",
                "orig",
                Some(3600),
                TriggerMode::Schedule,
            )
            .unwrap();

        // Update schedule and prompt
        store
            .update("upd", Some("0 12 * * *"), Some("changed"), None)
            .unwrap();
        let j = store
            .list()
            .unwrap()
            .into_iter()
            .find(|j| j.name == "upd")
            .unwrap();
        assert_eq!(j.schedule, "0 12 * * *");
        assert_eq!(j.prompt, "changed");
        assert_eq!(
            j.cooldown_secs,
            Some(3600),
            "cooldown unchanged when None passed"
        );

        // Update only cooldown
        store.update("upd", None, None, Some(7200)).unwrap();
        let j = store
            .list()
            .unwrap()
            .into_iter()
            .find(|j| j.name == "upd")
            .unwrap();
        assert_eq!(j.schedule, "0 12 * * *", "schedule unchanged");
        assert_eq!(j.prompt, "changed", "prompt unchanged");
        assert_eq!(j.cooldown_secs, Some(7200));

        // Update nonexistent
        assert!(store.update("nope", Some("* * * * *"), None, None).is_err());

        // Persistence
        let store2 = CronStore::new(dir.clone());
        let j = store2
            .list()
            .unwrap()
            .into_iter()
            .find(|j| j.name == "upd")
            .unwrap();
        assert_eq!(j.schedule, "0 12 * * *");
        assert_eq!(j.cooldown_secs, Some(7200));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

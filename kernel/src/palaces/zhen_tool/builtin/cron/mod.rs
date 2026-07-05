use std::sync::Arc;
// ── Cron Tool — Schedule recurring tasks ─────────────────────

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
    fn persist_one(&self, job: &CronJobFile) -> Result<(), String> {
        let path = self.dir.join(format!("{}.json", job.name));
        let json = serde_json::to_string_pretty(job)
            .map_err(|e| format!("serialize cron job '{}': {e}", job.name))?;
        std::fs::write(&path, &json).map_err(|e| format!("write cron job '{}': {e}", job.name))?;
        let meta =
            std::fs::metadata(&path).map_err(|e| format!("stat cron job '{}': {e}", job.name))?;
        let modified = meta
            .modified()
            .map_err(|e| format!("mtime cron job '{}': {e}", job.name))?;
        let dur = modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("duration cron job '{}': {e}", job.name))?;
        self.mtimes
            .lock()
            .unwrap()
            .insert(job.name.clone(), dur.as_secs());
        Ok(())
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
        if let Err(e) = self.file_store.persist_one(&job_file) {
            tracing::warn!("Failed to persist cron job: {e}");
        }
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
        if let Err(e) = self.file_store.persist_one(&job_file) {
            tracing::warn!("Failed to persist cron job: {e}");
        }
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
        if let Err(e) = self.file_store.persist_one(&job_file) {
            tracing::warn!("Failed to persist cron job: {e}");
        }
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
        if let Some(jf) = job_file
            && let Err(e) = self.file_store.persist_one(&jf)
        {
            tracing::warn!("Failed to persist cron job: {e}");
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

mod cron_tool;
pub use cron_tool::CronTool;

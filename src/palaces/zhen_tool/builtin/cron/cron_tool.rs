// ── CronTool ────────────────────────────────────────────────

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::ExecAction;
use crate::stems::CeremoniesIntent;

use super::{CronStore, TriggerMode};

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

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
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
            ).into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
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

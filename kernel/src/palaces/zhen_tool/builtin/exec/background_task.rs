// ── Background Task Store ─────────────────────────────────────────
//
// Inspired by Claude Code's Task.ts + utils/task/framework.ts.
// Manages fire-and-forget background tasks (shell, agent, workflow).
//
// Key design decisions borrowed from Claude Code:
//   - Type-prefixed task IDs (b=shell, a=agent, w=workflow)
//   - `notified` flag with atomic check-and-set to prevent duplicate notification
//   - `output_offset` for incremental output reads
//   - Terminal statuses: Completed | Failed | Killed

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Type-prefixed task IDs. 36^8 ≈ 2.8T combinations — sufficient to resist
/// brute-force symlink attacks (mirrors Claude Code's design rationale).
const TASK_ID_ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn random_base36(len: usize) -> String {
    // Use UUID v4 bytes as entropy source; map each byte to base36.
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();
    (0..len)
        .map(|i| {
            let idx = (bytes[i % 16] as usize) % TASK_ID_ALPHABET.len();
            TASK_ID_ALPHABET[idx] as char
        })
        .collect()
}

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    Shell,
    Agent,
    Workflow,
}

impl TaskType {
    pub fn prefix(&self) -> &str {
        match self {
            TaskType::Shell => "b",
            TaskType::Agent => "a",
            TaskType::Workflow => "w",
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            TaskType::Shell => "shell",
            TaskType::Agent => "agent",
            TaskType::Workflow => "workflow",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "shell" => Some(TaskType::Shell),
            "agent" => Some(TaskType::Agent),
            "workflow" => Some(TaskType::Workflow),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Killed,
    /// Task was running when the daemon crashed. State is unknown — the
    /// OS process may still be running as an orphan, or may have exited.
    /// Terminal: cannot transition without explicit user action.
    Lost,
}

impl TaskStatus {
    pub fn as_str(&self) -> &str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Killed => "killed",
            TaskStatus::Lost => "lost",
        }
    }

    /// True when a task is in a terminal state and will not transition further.
    /// Mirrors Claude Code's `isTerminalTaskStatus()`.
    /// Lost is terminal: we cannot know the real state after a crash.
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Killed | TaskStatus::Lost)
    }
}

/// State for one background task.
/// Mirrors Claude Code's `TaskStateBase`.
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: String,
    pub task_type: TaskType,
    pub status: TaskStatus,
    pub description: String,
    pub output_file: PathBuf,
    pub output_offset: u64,
    pub notified: bool,
    pub started_at: Instant,
    pub ended_at: Option<Instant>,
    pub tool_use_id: Option<String>,
    pub agent_id: Option<String>,
    /// Exit code for shell tasks.
    pub exit_code: Option<i32>,
}

// ── Persisted snapshot for crash recovery ─────────────────────────

/// Minimal serializable snapshot for crash recovery.
/// Only stores running tasks so they can be marked Lost on restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedTask {
    id: String,
    task_type: String,
    description: String,
    output_file: String,
    tool_use_id: Option<String>,
    agent_id: Option<String>,
}

impl From<&BackgroundTask> for PersistedTask {
    fn from(t: &BackgroundTask) -> Self {
        Self {
            id: t.id.clone(),
            task_type: t.task_type.as_str().to_string(),
            description: t.description.clone(),
            output_file: t.output_file.to_string_lossy().to_string(),
            tool_use_id: t.tool_use_id.clone(),
            agent_id: t.agent_id.clone(),
        }
    }
}

fn persist_path() -> PathBuf {
    crate::palaces::kun_config::default_data_dir().join("background_tasks.json")
}

// ── BackgroundTaskStore ────────────────────────────────────────────

/// Thread-safe store for all background tasks.
/// Held in EarthPlate; cloned Arcs are shared with ShellTool, TUI, cron runner.
pub struct BackgroundTaskStore {
    tasks: Mutex<HashMap<String, BackgroundTask>>,
}

impl BackgroundTaskStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tasks: Mutex::new(HashMap::new()),
        })
    }

    /// Persist all currently-running tasks to a JSON snapshot file.
    /// Called after every state change (register/update_status/kill).
    pub fn persist_snapshot(&self) {
        let guard = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let running: Vec<PersistedTask> = guard
            .values()
            .filter(|t| t.status == TaskStatus::Running || t.status == TaskStatus::Pending)
            .map(PersistedTask::from)
            .collect();

        let path = persist_path();
        if running.is_empty() {
            let _ = std::fs::remove_file(&path);
            return;
        }

        // Serde JSON write — infrequent (once per tool call completion),
        // small data (few KB max), acceptable to use synchronous I/O
        if let Ok(json) = serde_json::to_string_pretty(&running) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, json);
        }
    }

    /// On daemon startup, load the persisted snapshot and mark any
    /// previously-running tasks as Lost. Returns the count of lost tasks.
    pub fn hydrate_and_mark_lost(&self) -> usize {
        let path = persist_path();
        let json = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return 0, // no snapshot = clean start
        };
        let persisted: Vec<PersistedTask> = match serde_json::from_str(&json) {
            Ok(tasks) => tasks,
            Err(_) => return 0,
        };
        if persisted.is_empty() {
            return 0;
        }

        let mut guard = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        let mut lost_count = 0;
        for pt in &persisted {
            let task = BackgroundTask {
                id: pt.id.clone(),
                task_type: TaskType::from_str(&pt.task_type).unwrap_or(TaskType::Shell),
                status: TaskStatus::Lost,
                description: pt.description.clone(),
                output_file: PathBuf::from(&pt.output_file),
                output_offset: 0,
                notified: false, // will be picked up by agent loop notification
                started_at: Instant::now(),
                ended_at: Some(Instant::now()),
                tool_use_id: pt.tool_use_id.clone(),
                agent_id: pt.agent_id.clone(),
                exit_code: None,
            };
            guard.insert(pt.id.clone(), task);
            lost_count += 1;
        }

        // Clean up the snapshot so we don't re-hydrate on next restart
        let _ = std::fs::remove_file(&path);
        lost_count
    }

    /// Generate a typed task ID: prefix + 8-char base36.
    pub fn generate_id(task_type: TaskType) -> String {
        format!("{}{}", task_type.prefix(), random_base36(8))
    }

    /// Register a new background task. Returns the task ID.
    pub fn register(&self, mut task: BackgroundTask) -> String {
        let id = task.id.clone();
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        task.status = TaskStatus::Running;
        task.started_at = Instant::now();
        guard.insert(id.clone(), task);
        drop(guard);
        self.persist_snapshot();
        id
    }

    /// Update a task's status. Returns the updated task if found.
    ///
    /// Rejects transitions on already-terminal tasks (Completed/Failed/Killed).
    /// A terminal task cannot be resurrected back to Running or Pending.
    pub fn update_status(&self, task_id: &str, status: TaskStatus, exit_code: Option<i32>) -> Option<BackgroundTask> {
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(task) = guard.get_mut(task_id) {
            // Guard: reject transitions on already-terminal tasks.
            // This prevents the race where kill() sets Killed and the completion
            // handler then overwrites it with Completed.
            if task.status.is_terminal() {
                return Some(task.clone());
            }
            task.status = status;
            if status.is_terminal() {
                task.ended_at = Some(Instant::now());
            } else {
                task.ended_at = None;
            }
            if let Some(code) = exit_code {
                task.exit_code = Some(code);
            }
            let result = Some(task.clone());
            drop(guard);
            if status.is_terminal() {
                self.persist_snapshot();
            }
            result
        } else {
            None
        }
    }

    /// Atomically check and set the `notified` flag.
    /// Returns true if the notification should be enqueued (flag was false → set to true).
    /// This CAS pattern mirrors Claude Code's `updateTaskState` with `notified` check.
    pub fn mark_notified(&self, task_id: &str) -> bool {
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(task) = guard.get_mut(task_id) {
            if task.notified {
                false
            } else {
                task.notified = true;
                true
            }
        } else {
            false
        }
    }

    /// Get a task by ID.
    pub fn get(&self, task_id: &str) -> Option<BackgroundTask> {
        let guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.get(task_id).cloned()
    }

    /// List all tasks, optionally filtered by status.
    pub fn list(&self, status_filter: Option<TaskStatus>) -> Vec<BackgroundTask> {
        let guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard
            .values()
            .filter(|t| status_filter.map_or(true, |s| t.status == s))
            .cloned()
            .collect()
    }

    /// Get all completed/failed/killed tasks that haven't been notified.
    pub fn unnotified_terminal_tasks(&self) -> Vec<BackgroundTask> {
        let guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard
            .values()
            .filter(|t| t.status.is_terminal() && !t.notified)
            .cloned()
            .collect()
    }

    /// Get count of running tasks (for TUI pill display).
    pub fn running_count(&self) -> usize {
        let guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard
            .values()
            .filter(|t| t.status == TaskStatus::Running)
            .count()
    }

    /// Update output_offset for incremental reads.
    pub fn update_offset(&self, task_id: &str, new_offset: u64) {
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(task) = guard.get_mut(task_id) {
            task.output_offset = new_offset;
        }
    }

    /// Evict a terminal+notified task from memory.
    /// Mirrors Claude Code's `evictTerminalTask()`.
    pub fn evict(&self, task_id: &str) -> bool {
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(task) = guard.get(task_id) {
            if task.status.is_terminal() && task.notified {
                guard.remove(task_id);
                return true;
            }
        }
        false
    }

    /// Kill a running task (marks it as Killed with notified=true).
    /// Returns the killed task, or None if the task wasn't running.
    ///
    /// The notified flag is set atomically with the status transition,
    /// so a subsequent call to mark_notified will return false.
    pub fn kill(&self, task_id: &str) -> Option<BackgroundTask> {
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(task) = guard.get_mut(task_id) {
            // Guard: only kill running tasks. Terminal tasks are rejected.
            if task.status != TaskStatus::Running {
                return None;
            }
            task.status = TaskStatus::Killed;
            task.ended_at = Some(Instant::now());
            task.notified = true;
            let result = Some(task.clone());
            drop(guard);
            self.persist_snapshot();
            result
        } else {
            None
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_id_has_prefix() {
        let id = BackgroundTaskStore::generate_id(TaskType::Shell);
        assert!(id.starts_with('b'));
        assert_eq!(id.len(), 9); // prefix + 8
    }

    #[test]
    fn agent_id_has_prefix() {
        let id = BackgroundTaskStore::generate_id(TaskType::Agent);
        assert!(id.starts_with('a'));
    }

    #[test]
    fn register_and_get() {
        let store = BackgroundTaskStore::new();
        let task = BackgroundTask {
            id: "b_test_01".into(),
            task_type: TaskType::Shell,
            status: TaskStatus::Pending,
            description: "test cmd".into(),
            output_file: PathBuf::from("/tmp/test.output"),
            output_offset: 0,
            notified: false,
            started_at: Instant::now(),
            ended_at: None,
            tool_use_id: None,
            agent_id: None,
            exit_code: None,
        };
        let id = store.register(task);
        assert_eq!(id, "b_test_01");

        let got = store.get("b_test_01").unwrap();
        assert_eq!(got.status, TaskStatus::Running);
    }

    #[test]
    fn update_status_terminal() {
        let store = BackgroundTaskStore::new();
        let task = BackgroundTask {
            id: "b_test_02".into(),
            task_type: TaskType::Shell,
            status: TaskStatus::Pending,
            description: "test".into(),
            output_file: PathBuf::from("/tmp/test.output"),
            output_offset: 0,
            notified: false,
            started_at: Instant::now(),
            ended_at: None,
            tool_use_id: None,
            agent_id: None,
            exit_code: None,
        };
        store.register(task);
        let updated = store.update_status("b_test_02", TaskStatus::Completed, Some(0));
        assert!(updated.is_some());
        let task = store.get("b_test_02").unwrap();
        assert!(task.status.is_terminal());
        assert_eq!(task.exit_code, Some(0));
    }

    #[test]
    fn mark_notified_cas() {
        let store = BackgroundTaskStore::new();
        let task = BackgroundTask {
            id: "b_test_03".into(),
            task_type: TaskType::Shell,
            status: TaskStatus::Pending,
            description: "test".into(),
            output_file: PathBuf::from("/tmp/test.output"),
            output_offset: 0,
            notified: false,
            started_at: Instant::now(),
            ended_at: None,
            tool_use_id: None,
            agent_id: None,
            exit_code: None,
        };
        store.register(task);
        store.update_status("b_test_03", TaskStatus::Completed, Some(0));

        // First call: should return true (was false, now set to true)
        assert!(store.mark_notified("b_test_03"));

        // Second call: should return false (already true)
        assert!(!store.mark_notified("b_test_03"));
    }

    #[test]
    fn unnotified_terminal_tasks() {
        let store = BackgroundTaskStore::new();
        for i in 0..3 {
            let task = BackgroundTask {
                id: format!("b_test_0{i}"),
                task_type: TaskType::Shell,
                status: TaskStatus::Pending,
                description: format!("cmd{i}"),
                output_file: PathBuf::from("/tmp/test.output"),
                output_offset: 0,
                notified: false,
                started_at: Instant::now(),
                ended_at: None,
                tool_use_id: None,
                agent_id: None,
                exit_code: None,
            };
            store.register(task);
        }
        store.update_status("b_test_00", TaskStatus::Completed, Some(0));
        store.update_status("b_test_01", TaskStatus::Running, None);
        // b_test_02 still running

        let unnotified = store.unnotified_terminal_tasks();
        assert_eq!(unnotified.len(), 1);
        assert_eq!(unnotified[0].id, "b_test_00");
    }

    #[test]
    fn running_count() {
        let store = BackgroundTaskStore::new();
        assert_eq!(store.running_count(), 0);

        let task = BackgroundTask {
            id: "b_rc_01".into(),
            task_type: TaskType::Shell,
            status: TaskStatus::Pending,
            description: "t".into(),
            output_file: PathBuf::from("/tmp/t.out"),
            output_offset: 0,
            notified: false,
            started_at: Instant::now(),
            ended_at: None,
            tool_use_id: None,
            agent_id: None,
            exit_code: None,
        };
        store.register(task);
        assert_eq!(store.running_count(), 1);

        store.update_status("b_rc_01", TaskStatus::Completed, Some(0));
        assert_eq!(store.running_count(), 0);
    }

    #[test]
    fn evict_terminal() {
        let store = BackgroundTaskStore::new();
        let task = BackgroundTask {
            id: "b_evict_01".into(),
            task_type: TaskType::Shell,
            status: TaskStatus::Pending,
            description: "t".into(),
            output_file: PathBuf::from("/tmp/t.out"),
            output_offset: 0,
            notified: false,
            started_at: Instant::now(),
            ended_at: None,
            tool_use_id: None,
            agent_id: None,
            exit_code: None,
        };
        store.register(task);
        store.update_status("b_evict_01", TaskStatus::Completed, Some(0));

        // Can't evict before notified
        assert!(!store.evict("b_evict_01"));
        assert!(store.get("b_evict_01").is_some());

        // Mark notified, then evict
        store.mark_notified("b_evict_01");
        assert!(store.evict("b_evict_01"));
        assert!(store.get("b_evict_01").is_none());
    }

    #[test]
    fn kill_running_task() {
        let store = BackgroundTaskStore::new();
        let task = BackgroundTask {
            id: "b_kill_01".into(),
            task_type: TaskType::Shell,
            status: TaskStatus::Pending,
            description: "long cmd".into(),
            output_file: PathBuf::from("/tmp/t.out"),
            output_offset: 0,
            notified: false,
            started_at: Instant::now(),
            ended_at: None,
            tool_use_id: None,
            agent_id: None,
            exit_code: None,
        };
        store.register(task);

        let killed = store.kill("b_kill_01").unwrap();
        assert_eq!(killed.status, TaskStatus::Killed);
        assert!(killed.notified);

        // Can't kill a terminal task
        assert!(store.kill("b_kill_01").is_none());
    }
}

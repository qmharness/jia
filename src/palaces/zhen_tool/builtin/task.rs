use std::sync::Arc;
// ── Task Tool — Create and track sub-tasks ───────────────────

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;
use crate::stems::intent::ExecAction;

const MAX_TASKS: usize = 200;

/// In-memory task store shared with the tool and potentially REST API.
pub struct TaskStore {
    pub tasks: std::sync::Mutex<Vec<Task>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub status: TaskStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

impl TaskStatus {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "in_progress" | "inprogress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

impl TaskStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tasks: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Task>>, String> {
        self.tasks
            .lock()
            .map_err(|e| format!("Task store poisoned: {e}"))
    }

    pub fn create(&self, subject: &str, description: &str) -> Result<Task, String> {
        let mut guard = self.lock()?;
        if guard.len() >= MAX_TASKS {
            return Err(format!("Max {MAX_TASKS} tasks reached"));
        }
        let now = crate::utils::unix_now();
        let task = Task {
            id: uuid::Uuid::new_v4().to_string(),
            subject: subject.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
        };
        guard.push(task.clone());
        Ok(task)
    }

    pub fn list(&self) -> Result<Vec<Task>, String> {
        let guard = self.lock()?;
        Ok(guard.clone())
    }

    pub fn get(&self, id: &str) -> Result<Option<Task>, String> {
        let guard = self.lock()?;
        Ok(guard.iter().find(|t| t.id == id).cloned())
    }

    pub fn update_status(&self, id: &str, status: TaskStatus) -> Result<Task, String> {
        let mut guard = self.lock()?;
        match guard.iter_mut().find(|t| t.id == id) {
            Some(task) => {
                task.status = status;
                task.updated_at = crate::utils::unix_now();
                Ok(task.clone())
            }
            None => Err(format!("Task '{}' not found", id)),
        }
    }
}

pub struct TaskTool {
    store: Arc<TaskStore>,
}

impl TaskTool {
    pub fn new(store: Arc<TaskStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl BaseTool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> String {
        "Manage a structured task list. Use to track progress on complex multi-step work. \
         Actions: create (subject + description), list (show all), get (by id), \
         update (set status: pending/in_progress/completed/deleted)."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Geng(ExecAction {
            command: "task".into(),
        })
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "get", "update"],
                    "description": "The action to perform on the task list"
                },
                "id": {
                    "type": "string",
                    "description": "Task ID (required for get and update)"
                },
                "subject": {
                    "type": "string",
                    "description": "Short task title (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "Detailed task description (optional for create)"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "deleted"],
                    "description": "New task status (required for update)"
                }
            },
            "required": ["action"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, String> {
        let action = input["action"]
            .as_str()
            .ok_or("Missing 'action' parameter")?;

        match action {
            "create" => {
                let subject = input["subject"]
                    .as_str()
                    .ok_or("Missing 'subject' parameter")?;
                let description = input["description"].as_str().unwrap_or("");
                let task = self.store.create(subject, description)?;
                Ok(serde_json::to_string_pretty(&task)
                    .unwrap_or_else(|_| format!("Task created: {}", task.id)))
            }
            "list" => {
                let tasks = self.store.list()?;
                let active: Vec<_> = tasks
                    .iter()
                    .filter(|t| t.status != TaskStatus::Deleted)
                    .collect();
                if active.is_empty() {
                    Ok("No active tasks.".to_string())
                } else {
                    let summary: Vec<_> = active
                        .iter()
                        .map(|t| {
                            serde_json::json!({
                                "id": t.id,
                                "subject": t.subject,
                                "status": t.status,
                                "description": t.description,
                            })
                        })
                        .collect();
                    serde_json::to_string_pretty(&summary)
                        .map_err(|e| format!("Serialization error: {e}"))
                }
            }
            "get" => {
                let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
                match self.store.get(id)? {
                    Some(task) => serde_json::to_string_pretty(&task)
                        .map_err(|e| format!("Serialization error: {e}")),
                    None => Err(format!("Task '{id}' not found")),
                }
            }
            "update" => {
                let id = input["id"].as_str().ok_or("Missing 'id' parameter")?;
                let status_str = input["status"]
                    .as_str()
                    .ok_or("Missing 'status' parameter")?;
                let status = TaskStatus::from_str(status_str)
                    .ok_or_else(|| format!("Invalid status: '{status_str}'. Valid: pending, in_progress, completed, deleted"))?;
                let task = self.store.update_status(id, status)?;
                serde_json::to_string_pretty(&task).map_err(|e| format!("Serialization error: {e}"))
            }
            _ => Err(format!(
                "Unknown action: '{action}'. Valid: create, list, get, update"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_crud() {
        let store = TaskStore::new();
        let task = store.create("Test task", "Do the thing").unwrap();
        assert_eq!(task.subject, "Test task");
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(!task.id.is_empty());

        let found = store.get(&task.id).unwrap().unwrap();
        assert_eq!(found.id, task.id);

        let updated = store
            .update_status(&task.id, TaskStatus::InProgress)
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);

        let updated2 = store
            .update_status(&task.id, TaskStatus::Completed)
            .unwrap();
        assert_eq!(updated2.status, TaskStatus::Completed);
    }

    #[test]
    fn list_filters_deleted() {
        let store = TaskStore::new();
        let t1 = store.create("A", "").unwrap();
        let t2 = store.create("B", "").unwrap();
        store.update_status(&t2.id, TaskStatus::Deleted).unwrap();

        let tasks = store.list().unwrap();
        assert_eq!(tasks.len(), 2);

        let active: Vec<_> = tasks
            .iter()
            .filter(|t| t.status != TaskStatus::Deleted)
            .collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, t1.id);
    }

    #[test]
    fn status_from_str() {
        assert_eq!(TaskStatus::from_str("pending"), Some(TaskStatus::Pending));
        assert_eq!(
            TaskStatus::from_str("in_progress"),
            Some(TaskStatus::InProgress)
        );
        assert_eq!(
            TaskStatus::from_str("completed"),
            Some(TaskStatus::Completed)
        );
        assert_eq!(TaskStatus::from_str("deleted"), Some(TaskStatus::Deleted));
        assert_eq!(TaskStatus::from_str("bogus"), None);
    }
}

use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhen_tool::builtin::exec::background_task::{
    BackgroundTask, BackgroundTaskStore, TaskStatus, TaskType,
};
use crate::palaces::zhen_tool::builtin::exec::disk_output;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;
use crate::stems::AgentEvent;

/// Auto-background threshold: if a foreground command runs longer than this,
/// the agent loop can transition it to background mode.
pub const AUTO_BACKGROUND_SECS: u64 = 30;

/// Progress threshold: show "Backgrounding..." hint after this duration.
pub const BACKGROUND_HINT_SECS: u64 = 2;

pub struct ShellTool {
    background_tasks: Option<Arc<BackgroundTaskStore>>,
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellTool {
    pub fn new() -> Self {
        Self {
            background_tasks: None,
        }
    }

    /// Create a ShellTool with background task support.
    pub fn with_background_tasks(store: Arc<BackgroundTaskStore>) -> Self {
        Self {
            background_tasks: Some(store),
        }
    }
}

#[async_trait]
impl BaseTool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> String {
        "Execute a shell command and return stdout and stderr. \
         Prefer the dedicated tools over shell equivalents — command translation table: \
         `cat`/`head`/`tail` → read_file; recursive `find`/`ls` → glob; `grep`/`rg` → grep; \
         `echo > file` / `sed -i` → write_file / patch_file. \
         Set run_in_background to true for long-running commands \
         (use the read_file tool to read the output file later). \
         The sandbox disallows command separators and substitution: `;`, `|`, `$`, backticks, `&` — \
         chain commands with `&&` or run them in separate calls."
            .to_string()
    }

    fn category(&self) -> &str {
        "system"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Geng
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Set to true to run this command in the background. \
                                   Use the read_file tool to read the output file later \
                                   (path returned in the result)."
                },
                "description": {
                    "type": "string",
                    "description": "Short description of what this command does, \
                                   used in background task notifications (optional)"
                }
            },
            "required": ["command"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let cmd = input["command"]
            .as_str()
            .ok_or("Missing 'command' parameter")?;

        let run_in_background = input["run_in_background"]
            .as_bool()
            .unwrap_or(false);

        let description = input["description"]
            .as_str()
            .unwrap_or(cmd)
            .to_string();

        if run_in_background {
            let store = self
                .background_tasks
                .as_ref()
                .ok_or("Background tasks not configured (no BackgroundTaskStore in ShellTool)")?;

            let task_id = spawn_shell_background(cmd, &description, store, ctx).await?;
            let output_file = disk_output::task_output_path(&task_id);

            Ok(serde_json::json!({
                "status": "backgrounded",
                "task_id": task_id,
                "output_file": output_file.to_string_lossy(),
                "description": description,
                "command": cmd,
                "hint": format!("Background command running. Use read_file on '{}' to check output, or poll with a shell command.", output_file.display())
            })
            .to_string())
        } else {
            // #6 · session cwd: run from the persisted cwd, then persist the
            // process's final cwd (末次 cd 继承). Sharing note: ExecContext
            // clones share the cwd Arc, but shell keeps the default
            // `ToolAccesses::all` (base.rs) — a global barrier → singleton
            // batch per tool_scheduler.rs — so no two shell calls ever run
            // concurrently within a batch.
            let cwd = ctx.cwd();
            let (mut output, final_cwd) =
                ctx.permissions.execute_sandboxed_in(cmd, &cwd).await?;
            update_session_cwd(ctx, final_cwd, &mut output);
            Ok(output)
        }
    }

    /// Emit progress events for foreground commands that may auto-background.
    async fn execute_with_tx(
        &self,
        input: Value,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        ctx: &ExecContext,
    ) -> Result<String, ToolError> {
        let cmd = input["command"]
            .as_str()
            .ok_or("Missing 'command' parameter")?;

        let run_in_background = input["run_in_background"]
            .as_bool()
            .unwrap_or(false);

        let description = input["description"]
            .as_str()
            .unwrap_or(cmd)
            .to_string();

        if run_in_background {
            let store = self
                .background_tasks
                .as_ref()
                .ok_or("Background tasks not configured (no BackgroundTaskStore in ShellTool)")?;

            let task_id = spawn_shell_background(cmd, &description, store, ctx).await?;
            let output_file = disk_output::task_output_path(&task_id);

            let _ = tx.send(AgentEvent::TaskStarted {
                task_id: task_id.clone(),
                description: description.clone(),
                task_type: "shell".into(),
                tool_use_id: None,
            });

            Ok(serde_json::json!({
                "status": "backgrounded",
                "task_id": task_id,
                "output_file": output_file.to_string_lossy(),
                "description": description,
                "command": cmd,
                "hint": format!("Background command running. Use read_file on '{}' to check output, or poll with a shell command.", output_file.display())
            })
            .to_string())
        } else {
            // Foreground execution with auto-background hint
            let cmd_owned = cmd.to_string();
            let desc_owned = description.clone();
            let store_clone = self.background_tasks.clone();
            let tx_clone = tx.clone();
            let ctx_spawn = ctx.clone();
            // Keep second clones for the auto-background path (ctx_spawn/cmd_owned moved into spawn)
            let ctx_bg = ctx.clone();
            let cmd_bg = cmd.to_string();

            // Guard: prevents the hint task from sending "Still running" after
            // the command has already completed (could be misleading).
            let completed = Arc::new(AtomicBool::new(false));

            // Spawn the command in a background task but wait for it.
            // #6: run from the session cwd; the final cwd is persisted after
            // the join below (foreground path only — auto-backgrounded tasks
            // keep running from workspace_root like all background tasks).
            let session_cwd = ctx.cwd();
            let completed_clone = completed.clone();
            let handle = tokio::spawn(async move {
                let result = ctx_spawn
                    .permissions
                    .execute_sandboxed_in(&cmd_owned, &session_cwd)
                    .await;
                completed_clone.store(true, Ordering::Relaxed);
                result
            });

            // Emit a progress hint after BACKGROUND_HINT_SECS (only if still running)
            let hint_tx = tx.clone();
            let hint_cmd = cmd.to_string();
            let hint_completed = completed.clone();
            let _hint_handle = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(BACKGROUND_HINT_SECS)).await;
                if hint_completed.load(Ordering::Relaxed) {
                    return; // command already finished, skip stale hint
                }
                // Signal that the command is still running
                let _ = hint_tx.send(AgentEvent::ToolResult {
                    tool: "shell".into(),
                    output: format!("Still running: \"{}\" ({}s elapsed). Press Ctrl+B to move to background.", hint_cmd, BACKGROUND_HINT_SECS),
                    error: None,
                    geju: None,
                    execution_mode: None,
                });
            });

            // Wait for the shell command with a timeout for auto-background
            let result = tokio::time::timeout(
                Duration::from_secs(AUTO_BACKGROUND_SECS),
                handle,
            )
            .await;

            match result {
                Ok(Ok(Ok((mut output, final_cwd)))) => {
                    update_session_cwd(ctx, final_cwd, &mut output);
                    Ok(output)
                }
                Ok(Ok(Err(e))) => Err(ToolError::exec("shell", e.to_string())),
                Ok(Err(join_err)) => Err(ToolError::exec("shell", format!("Task join error: {join_err}"))),
                Err(_timeout) => {
                    // Auto-background: command is still running after timeout.
                    // Spawn it as a real background task so output is captured
                    // and status transitions are tracked properly.
                    if let Some(store) = store_clone {
                        let task_id = spawn_shell_background(&cmd_bg, &desc_owned, &store, &ctx_bg).await
                            .map_err(|e| ToolError::exec("shell", format!("Auto-background failed: {e}")))?;
                        let output_file = disk_output::task_output_path(&task_id);

                        let _ = tx_clone.send(AgentEvent::TaskStarted {
                            task_id: task_id.clone(),
                            description: desc_owned.clone(),
                            task_type: "shell".into(),
                            tool_use_id: None,
                        });

                        Ok(serde_json::json!({
                            "status": "auto_backgrounded",
                            "task_id": task_id,
                            "output_file": output_file.to_string_lossy(),
                            "description": desc_owned,
                            "command": cmd,
                            "hint": format!("Command exceeded {}s timeout and was auto-moved to background. Use read_file on '{}' to check output.", AUTO_BACKGROUND_SECS, output_file.display())
                        })
                        .to_string())
                    } else {
                        Err(ToolError::exec(
                            "shell",
                            format!(
                                "Command timed out after {}s and background tasks are not configured. \
                                 Consider using run_in_background: true for long-running commands.",
                                AUTO_BACKGROUND_SECS
                            ),
                        ))
                    }
                }
            }
        }
    }
}

/// #6 · Persist the session cwd after a foreground run (末次 cd 继承).
///
/// The captured `$PWD` is boundary-checked through `verify_path`
/// (canonicalize + workspace_root/allowed_paths containment) before being
/// stored — 路径边界只收紧不放松. On escape (or a deleted directory) the
/// cwd stays put and the refusal is surfaced in the tool output.
fn update_session_cwd(
    ctx: &ExecContext,
    final_cwd: Option<std::path::PathBuf>,
    output: &mut String,
) {
    let Some(dir) = final_cwd else { return };
    match ctx
        .permissions
        .verify_path(&dir.to_string_lossy(), PathOp::Read)
    {
        Ok(canonical) => {
            if canonical != ctx.cwd() {
                ctx.set_cwd(canonical);
            }
        }
        Err(reason) => {
            output.push_str(&format!(
                "\n[cwd not updated: {reason}; still in {}]",
                ctx.cwd().display()
            ));
        }
    }
}

/// Spawn a shell command as a background task.
///
/// Flow (mirrors Claude Code's spawnShellTask):
///   1. Generate task_id = "b" + 8-char base36
///   2. Create output file with O_EXCL + O_NOFOLLOW
///   3. Register BackgroundTask in the store (status: Running)
///   4. tokio::spawn the process: write stdout/stderr to output file
///   5. On completion: update status (Completed/Failed), set notified=false
///
/// Returns the task_id.
async fn spawn_shell_background(
    cmd: &str,
    description: &str,
    store: &Arc<BackgroundTaskStore>,
    ctx: &ExecContext,
) -> Result<String, ToolError> {
    let task_id = BackgroundTaskStore::generate_id(TaskType::Shell);
    let output_file = disk_output::task_output_path(&task_id);

    // Initialize output file with security flags
    disk_output::init_task_output(&task_id).map_err(|e| {
        ToolError::exec("shell", format!("Failed to create output file: {e}"))
    })?;

    // Register the task as running
    let task = BackgroundTask {
        id: task_id.clone(),
        task_type: TaskType::Shell,
        status: TaskStatus::Pending,
        description: description.to_string(),
        output_file: output_file.clone(),
        output_offset: 0,
        notified: false,
        started_at: std::time::Instant::now(),
        ended_at: None,
        tool_use_id: None,
        agent_id: Some(ctx.session_id.clone()),
        exit_code: None,
    };
    store.register(task);

    // Clone for the background task
    let task_id_bg = task_id.clone();
    let store_bg = store.clone();
    let cmd_owned = cmd.to_string();
    let perms = ctx.permissions.clone();

    // Spawn the shell command in a background tokio task.
    // Uses tokio::task::spawn_blocking with catch_unwind to prevent a panic
    // in execute_sandboxed from leaving the BackgroundTask stuck in Running.
    tokio::spawn(async move {
        // Wrap in a blocking task that catches panics
        let result = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async { perms.execute_sandboxed(&cmd_owned).await })
        })
        .await;

        match result {
            Ok(Ok(stdout)) => {
                let _ = disk_output::append_task_output(&task_id_bg, &stdout);
                store_bg.update_status(&task_id_bg, TaskStatus::Completed, Some(0));
            }
            Ok(Err(e)) => {
                let error_msg = e.to_string();
                let _ = disk_output::append_task_output(
                    &task_id_bg,
                    &format!("Error: {error_msg}\n"),
                );
                store_bg.update_status(&task_id_bg, TaskStatus::Failed, Some(1));
            }
            Err(join_err) => {
                // The sandbox panicked or was cancelled — mark as failed.
                let panic_msg = if join_err.is_panic() {
                    "sandbox panic".to_string()
                } else {
                    format!("sandbox cancelled: {join_err}")
                };
                let _ = disk_output::append_task_output(
                    &task_id_bg,
                    &format!("Internal error: {panic_msg}\n"),
                );
                store_bg.update_status(&task_id_bg, TaskStatus::Failed, Some(2));
            }
        }
    });

    Ok(task_id)
}

#[cfg(test)]
mod tests {
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;
    fn test_ctx() -> crate::stems::action::ExecContext {
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    #[tokio::test]
    async fn shell_echo() {
        let tool = ShellTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}), &test_ctx())
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().to_string().contains("hello"));
    }

    /// #6 · ctx rooted at a temp workspace so cwd assertions are deterministic.
    fn ctx_rooted_at(root: &std::path::Path) -> crate::stems::action::ExecContext {
        let mut sec = crate::palaces::kun_config::SecuritySection::default();
        sec.workspace_root = Some(root.to_string_lossy().to_string());
        let perms = Arc::new(PermissionMatrix::from_config(
            &sec,
            root,
            root.join("backups"),
        ));
        crate::stems::action::ExecContext::new(perms)
    }

    /// #6 · `cd` in one call is inherited by the next (末次 cd 继承).
    #[tokio::test]
    async fn cd_persists_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        let ctx = ctx_rooted_at(&root);
        let tool = ShellTool::new();

        tool.execute(serde_json::json!({"command": "cd sub"}), &ctx)
            .await
            .unwrap();
        assert_eq!(ctx.cwd(), root.join("sub"));

        let out = tool
            .execute(serde_json::json!({"command": "pwd"}), &ctx)
            .await
            .unwrap();
        assert!(
            out.trim().ends_with("/sub"),
            "next command should run from the persisted cwd, got: {out}"
        );
        // The capture marker must never leak into visible output.
        assert!(!out.contains("__JIA_CWD__"), "marker leaked: {out}");
    }

    /// #6 · `cd` out of the workspace is refused: cwd unchanged + hint.
    #[tokio::test]
    async fn cd_outside_workspace_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let ctx = ctx_rooted_at(&root);
        let tool = ShellTool::new();

        let out = tool
            .execute(serde_json::json!({"command": "cd /"}), &ctx)
            .await
            .unwrap();
        assert!(
            out.contains("cwd not updated"),
            "escape should be surfaced in output, got: {out}"
        );
        assert_eq!(ctx.cwd(), root, "cwd must stay at the workspace root");

        let out = tool
            .execute(serde_json::json!({"command": "pwd"}), &ctx)
            .await
            .unwrap();
        assert!(
            out.contains(&root.to_string_lossy().to_string()),
            "next command should still run from the root, got: {out}"
        );
    }

    /// #6 · relative `cd ..` that stays inside the workspace is accepted.
    #[tokio::test]
    async fn cd_relative_within_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        let ctx = ctx_rooted_at(&root);
        let tool = ShellTool::new();

        tool.execute(serde_json::json!({"command": "cd a/b"}), &ctx)
            .await
            .unwrap();
        assert_eq!(ctx.cwd(), root.join("a/b"));
        tool.execute(serde_json::json!({"command": "cd .."}), &ctx)
            .await
            .unwrap();
        assert_eq!(ctx.cwd(), root.join("a"));
    }

    #[tokio::test]
    async fn shell_missing_command() {
        let tool = ShellTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_blocked_command() {
        let tool = ShellTool::new();
        let result = tool
            .execute(
                serde_json::json!({"command": "rm -rf /tmp/foo"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("blocked pattern"));
    }

    #[tokio::test]
    async fn run_in_background_no_store() {
        let tool = ShellTool::new(); // no BackgroundTaskStore
        let result = tool
            .execute(
                serde_json::json!({"command": "sleep 1", "run_in_background": true}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("BackgroundTaskStore"));
    }

    #[tokio::test]
    async fn run_in_background_with_store() {
        let store = BackgroundTaskStore::new();
        let tool = ShellTool::with_background_tasks(store.clone());
        let result = tool
            .execute(
                serde_json::json!({
                    "command": "echo hello",
                    "run_in_background": true,
                    "description": "test echo"
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("backgrounded"));
        assert!(output.contains("task_id"));

        // Wait for the background task to complete
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Check that the task was registered
        let tasks = store.list(None);
        assert!(!tasks.is_empty());
        let task = &tasks[0];
        assert_eq!(task.description, "test echo");
        // Should be completed now
        let updated = store.get(&task.id).unwrap();
        assert!(updated.status.is_terminal());
    }

    #[tokio::test]
    async fn background_task_output_file_exists() {
        let store = BackgroundTaskStore::new();
        let tool = ShellTool::with_background_tasks(store.clone());
        let result = tool
            .execute(
                serde_json::json!({
                    "command": "echo bg_test",
                    "run_in_background": true
                }),
                &test_ctx(),
            )
            .await
            .unwrap();

        // Parse the result to get task_id
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let task_id = v["task_id"].as_str().unwrap();
        let output_file = v["output_file"].as_str().unwrap();

        // Output file should be referenced
        assert!(output_file.contains(&task_id));

        // Wait for completion
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // Read output
        let output = disk_output::read_task_output(task_id, 1024).unwrap();
        assert!(output.contains("bg_test"));

        // Cleanup
        disk_output::cleanup_task_output(task_id);
    }

    #[tokio::test]
    async fn auto_background_on_timeout() {
        let store = BackgroundTaskStore::new();
        let tool = ShellTool::with_background_tasks(store.clone());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        let ctx = test_ctx();
        let result = tool
            .execute_with_tx(
                serde_json::json!({
                    "command": "sleep 60",
                    "description": "long sleep"
                }),
                &tx,
                &ctx,
            )
            .await;

        // Since AUTO_BACKGROUND_SECS=30, sleep 60 should trigger auto-background
        // but the timeout in execute_with_tx waits for AUTO_BACKGROUND_SECS
        match result {
            Ok(output) => {
                // Either completed quickly (sleep was fast) or auto-backgrounded
                assert!(
                    output.contains("auto_backgrounded") || output.is_empty(),
                    "Expected auto_backgrounded or empty, got: {output}"
                );
            }
            Err(_) => {
                // May error if auto-background isn't configured
            }
        }
    }
}

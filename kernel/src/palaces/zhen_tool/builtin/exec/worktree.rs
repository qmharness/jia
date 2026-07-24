use crate::error::ToolError;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// P6 · Worktree isolation tools (艮八 · 山镇隔离).
///
/// `enter_worktree` runs `git worktree add` to create an isolated working tree
/// on a new branch, then the agent loop swaps `active_tools` to a rebuilt
/// registry scoped to the worktree root (file/shell/git tools execute against
/// the worktree). `exit_worktree` restores the main registry and optionally
/// removes the worktree.
/// Both tools are 戊仪 (Wu, read-only ceremony) so they remain available in
/// plan mode — they are workspace-management actions, not code mutations.
///
/// Compute the worktree path for a given base root and name.
pub fn worktree_path(base_root: &Path, name: &str) -> PathBuf {
    base_root.join(".jia").join("worktrees").join(name)
}

/// Run `git -C root worktree add <path> -b <branch>` (blocking, offloaded).
fn git_worktree_add(root: &Path, path: &Path, branch: &str) -> Result<(), String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("worktree")
        .arg("add")
        .arg(path)
        .arg("-b")
        .arg(branch)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree add failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Run `git -C root worktree remove <path>` (blocking, offloaded).
fn git_worktree_remove(root: &Path, path: &Path, force: bool) -> Result<(), String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .arg("worktree")
        .arg("remove")
        .arg(path);
    if force {
        cmd.arg("--force");
    }
    let output = cmd
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub struct EnterWorktreeTool {}

impl Default for EnterWorktreeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl EnterWorktreeTool {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl BaseTool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "enter_worktree"
    }

    fn description(&self) -> String {
        "Create an isolated git worktree on a new branch and switch the agent's \
         file/shell/git tools to operate within it. Useful for parallel or \
         exploratory work without touching the main checkout. Use exit_worktree \
         to return to the main project. Nested worktrees are not supported."
            .to_string()
    }

    fn category(&self) -> &str {
        "control"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Worktree name (also the new branch name)"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let name = input["name"]
            .as_str()
            .ok_or("Missing 'name' parameter")?
            .to_string();
        if name.is_empty() || name.contains('/') {
            return Err("worktree name must be non-empty and contain no '/'".into());
        }

        // Sandbox-check the base root (read access to current project root).
        let base_root = ctx
            .permissions
            .verify_path(".", PathOp::Read)
            .map_err(|e| format!("cannot resolve project root: {e}"))?;

        // Refuse nesting: if the current root is itself under a .jia/worktrees path.
        if base_root.components().any(|c| c.as_os_str() == ".jia") {
            return Err("already inside a worktree; nested worktrees are not supported".into());
        }

        let path = worktree_path(&base_root, &name);
        let root_clone = base_root.clone();
        let path_clone = path.clone();
        let name_clone = name.clone();
        tokio::task::spawn_blocking(move || {
            git_worktree_add(&root_clone, &path_clone, &name_clone)
        })
        .await
        .map_err(|e| format!("git join error: {e}"))??;

        Ok(format!(
            "Created worktree '{}' at {}. File/shell/git tools now operate within it. \
             Use exit_worktree to return to the main project.",
            name,
            path.display()
        ))
    }
}

pub struct ExitWorktreeTool;

#[async_trait]
impl BaseTool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "exit_worktree"
    }

    fn description(&self) -> String {
        "Exit the current worktree and restore the agent's tools to the main \
         project root. With action='remove' the worktree and its branch are \
         deleted (after verifying no uncommitted changes); with 'keep' they are \
         left for later use. This tool is a marker — the agent loop performs \
         the actual restore and optional removal."
            .to_string()
    }

    fn category(&self) -> &str {
        "control"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["keep", "remove"],
                    "description": "keep = leave the worktree on disk; remove = delete it (default: keep)"
                }
            }
        })
    }

    async fn execute(&self, _input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        // Marker tool — the agent loop handles the actual restore + removal so
        // it can swap active_tools (which the stateless tool cannot reach).
        Ok("Exiting worktree; tools restored to the main project root.".to_string())
    }
}

/// P6 · loop-facing helper: remove a worktree from the main root.
pub async fn remove_worktree(main_root: &Path, worktree: &Path, force: bool) -> Result<(), String> {
    let root = main_root.to_path_buf();
    let wt = worktree.to_path_buf();
    tokio::task::spawn_blocking(move || git_worktree_remove(&root, &wt, force))
        .await
        .map_err(|e| format!("git join error: {e}"))?
}

#[cfg(test)]
mod tests {
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    #[test]
    fn worktree_path_layout() {
        let p = worktree_path(Path::new("/repo"), "feat");
        assert_eq!(p, PathBuf::from("/repo/.jia/worktrees/feat"));
    }

    /// End-to-end: enter_worktree creates a real git worktree. Ignored by
    /// default (needs git + filesystem). Run: `cargo test --lib worktree -- --ignored`.
    #[tokio::test]
    #[ignore = "requires git repository with worktree support"]
    async fn enter_worktree_creates_worktree() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let root = dir.path().to_path_buf();
        // init a git repo with an initial commit (worktree add needs a commit)
        let _ = std::process::Command::new("git")
            .arg("init")
            .current_dir(&root)
            .output();
        let _ = std::process::Command::new("git")
            .arg("config")
            .arg("user.email")
            .arg("t@t")
            .current_dir(&root)
            .output();
        let _ = std::process::Command::new("git")
            .arg("config")
            .arg("user.name")
            .arg("t")
            .current_dir(&root)
            .output();
        std::fs::write(root.join("README"), "init").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&root)
            .output();

        // PermissionMatrix rooted at the temp repo
        let mut sec = crate::palaces::kun_config::SecuritySection::default();
        sec.workspace_root = Some(root.to_string_lossy().to_string());
        let perms = Arc::new(
            crate::palaces::qian_permission::PermissionMatrix::from_config(
                &sec,
                &root,
                root.join("backups"),
            ),
        );
        let tool = EnterWorktreeTool::new();
        let res = tool
            .execute(serde_json::json!({ "name": "feat-x" }), &test_ctx())
            .await;
        assert!(res.is_ok(), "enter_worktree failed: {:?}", res.err());
        let wt = worktree_path(&root, "feat-x");
        assert!(wt.is_dir(), "worktree dir should exist: {}", wt.display());
        assert!(wt.join(".git").exists(), "worktree should have .git");

        // cleanup: remove the worktree
        let _ = remove_worktree(&root, &wt, true).await;
    }
}

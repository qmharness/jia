use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent, ReadAction, WriteAction};

/// P8 · cross-worker scratchpad (跨 agent 共享知识).
///
/// A simple key→content file store under `<project_root>/.jia/scratchpad/`,
/// shared by the main agent and all sub-agents (any worker can read/write the
/// same keys). Lets a delegate sub-agent leave findings for the coordinator or
/// for a sibling sub-agent. Keys are restricted to `[A-Za-z0-9_-]+` (no path
/// traversal). PermissionMatrix confines the directory to project_root.
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn scratchpad_path(exec_ctx: &ExecContext, key: &str) -> std::path::PathBuf {
    exec_ctx
        .permissions
        .sandbox
        .project_root
        .join(".jia")
        .join("scratchpad")
        .join(key)
}

pub struct ScratchpadWriteTool;

impl Default for ScratchpadWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScratchpadWriteTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BaseTool for ScratchpadWriteTool {
    fn name(&self) -> &str {
        "scratchpad_write"
    }

    fn description(&self) -> String {
        "Write a note to a shared cross-agent scratchpad under a key. The main \
         agent and all sub-agents share this scratchpad, so a delegate can leave \
         findings for the coordinator or siblings. Keys: [A-Za-z0-9_-]+."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        // 己仪 (Ji, write) → is_destructive=true. Blocked in plan mode (correct:
        // writing shared state is a mutation).
        CeremoniesIntent::Ji(WriteAction {
            target: String::new(),
            content: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Scratchpad key ([A-Za-z0-9_-]+)"},
                "content": {"type": "string", "description": "Content to store"}
            },
            "required": ["key", "content"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
        if !valid_key(key) {
            return Err(format!("invalid key '{key}': must be non-empty [A-Za-z0-9_-]+").into());
        }
        let content = input["content"]
            .as_str()
            .ok_or("Missing 'content' parameter")?;

        let path = scratchpad_path(ctx, key);
        // Re-validate via PermissionMatrix (defense in depth; parent must exist for Write canonicalize)
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create scratchpad dir: {e}"))?;
        }
        let canonical = ctx
            .permissions
            .verify_path(&path.to_string_lossy(), PathOp::Write)?;
        std::fs::write(&canonical, content)
            .map_err(|e| format!("failed to write scratchpad: {e}"))?;
        Ok(format!(
            "Wrote {} bytes to scratchpad key '{key}'.",
            content.len()
        ))
    }
}

pub struct ScratchpadReadTool;

impl Default for ScratchpadReadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScratchpadReadTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BaseTool for ScratchpadReadTool {
    fn name(&self) -> &str {
        "scratchpad_read"
    }

    fn description(&self) -> String {
        "Read a note from the shared cross-agent scratchpad by key. Returns the \
         stored content, or an error if the key does not exist."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu(ReadAction {
            target: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Scratchpad key to read"}
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
        if !valid_key(key) {
            return Err(format!("invalid key '{key}': must be non-empty [A-Za-z0-9_-]+").into());
        }
        let path = scratchpad_path(ctx, key);
        let canonical = ctx
            .permissions
            .verify_path(&path.to_string_lossy(), PathOp::Read)?;
        Ok(std::fs::read_to_string(&canonical).map_err(|e| {
            ToolError::exec(
                self.name(),
                format!("scratchpad key '{key}' not readable: {e}"),
            )
        })?)
    }
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

    fn test_perms_at(root: &std::path::Path) -> Arc<PermissionMatrix> {
        let mut sec = crate::palaces::kun_config::SecuritySection::default();
        sec.project_root = Some(root.to_string_lossy().to_string());
        Arc::new(PermissionMatrix::from_config(
            &sec,
            root,
            root.join("backups"),
        ))
    }

    #[test]
    fn key_validation() {
        assert!(valid_key("findings"));
        assert!(valid_key("feat-1_notes"));
        assert!(!valid_key(""));
        assert!(!valid_key("a/b"));
        assert!(!valid_key(".."));
        assert!(!valid_key("a b"));
    }

    #[tokio::test]
    async fn scratchpad_write_read_roundtrip() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let perms = test_perms_at(dir.path());
        let ctx = ExecContext::new(perms.clone());
        let w = ScratchpadWriteTool::new();
        let r = ScratchpadReadTool::new();

        let res = w
            .execute(
                serde_json::json!({ "key": "findings", "content": "hello world" }),
                &ctx,
            )
            .await;
        assert!(res.is_ok(), "write failed: {:?}", res.err());

        let res = r
            .execute(serde_json::json!({ "key": "findings" }), &ctx)
            .await;
        assert!(res.is_ok(), "read failed: {:?}", res.err());
        assert_eq!(res.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn scratchpad_rejects_bad_key() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let perms = test_perms_at(dir.path());
        let ctx = ExecContext::new(perms);
        let w = ScratchpadWriteTool::new();
        let res = w
            .execute(
                serde_json::json!({ "key": "../escape", "content": "x" }),
                &ctx,
            )
            .await;
        assert!(res.is_err());
    }
}

use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

pub struct EditTool {}

impl Default for EditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl EditTool {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl BaseTool for EditTool {
    fn name(&self) -> &str {
        "patch_file"
    }

    fn description(&self) -> String {
        "Perform exact string replacements in an existing file. \
         The old_string must match exactly one location in the file — \
         if it is absent or occurs multiple times, the edit is rejected; \
         include more surrounding context to make it unique. \
         Freshness gate: you MUST read_file this file earlier in the session \
         before patching it; if the file was modified externally since your \
         last read (mtime changed), the edit is rejected and you must \
         read_file it again — your old_string was matched against bytes that \
         no longer exist. A successful patch refreshes the gate, so \
         consecutive edits by you do not require a re-read."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ji
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace (must be unique in the file)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn accesses(&self, input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // U1: declares exactly the target path; patches to disjoint files may
        // run in parallel (freshness gate is per-path and read_state is
        // Arc<Mutex>, so concurrent checks are safe).
        match input["path"].as_str() {
            Some(p) if !p.is_empty() => {
                crate::palaces::zhen_tool::base::ToolAccesses::write_only(vec![
                    std::path::PathBuf::from(p),
                ])
            }
            _ => crate::palaces::zhen_tool::base::ToolAccesses::all(),
        }
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
        let old_string = input["old_string"]
            .as_str()
            .ok_or("Missing 'old_string' parameter")?;
        let new_string = input["new_string"]
            .as_str()
            .ok_or("Missing 'new_string' parameter")?;

        let canonical = ctx.permissions.verify_path(path, PathOp::Write)?;

        // Freshness gate (#4): verify agent has read the file recently
        let meta = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| format!("Cannot stat file: {e}"))?;
        if let Ok(mtime) = meta.modified() {
            ctx.check_freshness(&canonical, mtime)
                .map_err(|e| ToolError::PermissionDenied(e))?;
        }

        let content = tokio::fs::read_to_string(&canonical)
            .await
            .map_err(|e| format!("read error: {e}"))?;

        let matches: Vec<_> = content.match_indices(old_string).take(2).collect();

        if matches.is_empty() {
            return Err(format!("old_string not found in file '{}'", canonical.display()).into());
        }

        if matches.len() > 1 {
            let line_num = content[..matches[1].0].lines().count();
            let line_start = content[..matches[1].0]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(0);
            let line_end = content[matches[1].0..]
                .find('\n')
                .map(|i| matches[1].0 + i)
                .unwrap_or(content.len());
            return Err(format!(
                "old_string matches multiple locations in '{}'. Must be unique. \
                 Second occurrence at line {}: {}",
                canonical.display(),
                line_num + 1,
                &content[line_start..line_end].trim(),
            )
            .into());
        }

        let pos = matches[0].0;
        let new_content = format!(
            "{}{}{}",
            &content[..pos],
            new_string,
            &content[pos + old_string.len()..],
        );
        // Backup original content before mutation
        {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let backup_dir = ctx.permissions.backup_dir.join(ts.to_string());
            if tokio::fs::create_dir_all(&backup_dir).await.is_ok()
                && let Some(fname) = canonical.file_name()
            {
                // Save original content (already in `content` from the read above)
                let _ = tokio::fs::write(backup_dir.join(fname), &content).await;
            }
        }

        tokio::fs::write(&canonical, &new_content)
            .await
            .map_err(|e| format!("write error: {e}"))?;

        // Update read_state after successful patch (#4 write-then-read rule)
        if let Ok(meta) = tokio::fs::metadata(&canonical).await {
            if let Ok(mtime) = meta.modified() {
                ctx.record_read(canonical.clone(), mtime);
            }
        }

        Ok(format!(
            "Successfully edited {} (1 replacement)",
            canonical.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    fn test_dir() -> tempfile::TempDir {
        tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap()
    }

    fn with_temp_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = test_dir();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[tokio::test]
    async fn edit_single_replacement() {
        let (_dir, path) = with_temp_file("Hello, world!\nThis is a test.\n");
        let path_str = path.to_string_lossy().to_string();

        // Pre-populate read_state for freshness gate (#4)
        let ctx = test_ctx();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ctx.record_read(path.clone(), mtime);

        let tool = EditTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "old_string": "world",
                    "new_string": "Jia"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok(), "edit failed: {:?}", result.err());

        let new_content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(new_content, "Hello, Jia!\nThis is a test.\n");
    }

    #[tokio::test]
    async fn edit_not_unique() {
        let (_dir, path) = with_temp_file("foo\nbar\nfoo\n");
        let path_str = path.to_string_lossy().to_string();

        // Pre-populate read_state for freshness gate (#4)
        let ctx = test_ctx();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ctx.record_read(path.clone(), mtime);

        let tool = EditTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "old_string": "foo",
                    "new_string": "baz"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("matches multiple locations")
        );
    }

    #[tokio::test]
    async fn edit_not_found() {
        let (_dir, path) = with_temp_file("hello\n");
        let path_str = path.to_string_lossy().to_string();

        // Pre-populate read_state for freshness gate (#4)
        let ctx = test_ctx();
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ctx.record_read(path.clone(), mtime);

        let tool = EditTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "old_string": "nonexistent",
                    "new_string": "x"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn edit_missing_params() {
        let tool = EditTool::new();
        assert!(
            tool.execute(serde_json::json!({}), &test_ctx())
                .await
                .is_err()
        );
    }
}

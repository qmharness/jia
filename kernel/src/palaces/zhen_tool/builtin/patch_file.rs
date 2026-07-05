use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent, WriteAction};

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
         The old_string must match exactly one location in the file. \
         If the string is not unique, the edit is rejected."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ji(WriteAction {
            target: String::new(),
            content: String::new(),
        })
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

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
        let old_string = input["old_string"]
            .as_str()
            .ok_or("Missing 'old_string' parameter")?;
        let new_string = input["new_string"]
            .as_str()
            .ok_or("Missing 'new_string' parameter")?;

        let canonical = ctx.permissions.verify_path(path, PathOp::Write)?;

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

        Ok(format!(
            "Successfully edited {} (1 replacement)",
            canonical.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext {
            permissions: Arc::new(PermissionMatrix::default()),
        }
    }

    use super::*;

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

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

        let tool = EditTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "old_string": "world",
                    "new_string": "Jia"
                }),
                &test_ctx(),
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

        let tool = EditTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "old_string": "foo",
                    "new_string": "baz"
                }),
                &test_ctx(),
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

        let tool = EditTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": path_str,
                    "old_string": "nonexistent",
                    "new_string": "x"
                }),
                &test_ctx(),
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

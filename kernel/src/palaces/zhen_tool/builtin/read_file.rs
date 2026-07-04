use async_trait::async_trait;
use crate::error::ToolError;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent, ReadAction};

pub struct ReadFileTool {}

impl Default for ReadFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadFileTool {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl BaseTool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> String {
        "Read the contents of a file at the given path. Returns the file content as a string. Use max_lines to limit output for large files (default 500).".to_string()
    }

    fn category(&self) -> &str {
        "file"
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
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "max_lines": {
                    "type": "integer",
                    "description": "Maximum lines to read (default 500)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
        let canonical = ctx.permissions.verify_path(path, PathOp::Read)?;

        let max_lines = input["max_lines"]
            .as_u64()
            .map(|v| v as usize)
            .unwrap_or(500)
            .max(1);

        use tokio::io::AsyncBufReadExt;
        let file = tokio::fs::File::open(&canonical)
            .await
            .map_err(|e| format!("read_file error: {e}"))?;
        let reader = tokio::io::BufReader::new(file);
        let mut lines = reader.lines();
        let mut content = String::new();
        let mut count = 0usize;
        let mut truncated = false;

        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| format!("read_file error: {e}"))?
        {
            if count >= max_lines {
                truncated = true;
                break;
            }
            content.push_str(&line);
            content.push('\n');
            count += 1;
        }

        if truncated {
            content.push_str(&format!(
                "\n... [truncated at line {max_lines}, more lines follow]\n"
            ));
        }

        Ok(content)
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

    #[tokio::test]
    async fn read_file_happy_path() {
        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({"path": "Cargo.toml"}), &test_ctx())
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().to_string().contains("[package]"));
    }

    #[tokio::test]
    async fn read_file_missing_path() {
        let tool = ReadFileTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_file_nonexistent() {
        let tool = ReadFileTool::new();
        let result = tool
            .execute(
                serde_json::json!({"path": "/nonexistent/file.txt"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn max_lines_truncation() {
        let tool = ReadFileTool::new();
        let result = tool
            .execute(
                serde_json::json!({"path": "Cargo.toml", "max_lines": 2}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(
            content.to_string().contains("[truncated at line 2"),
            "should have truncation marker, got: {content}"
        );
        // Must contain the first line of Cargo.toml
        assert!(content.to_string().contains("[package]"), "should contain file content");
    }

    #[tokio::test]
    async fn read_file_outside_root_blocked() {
        let tool = ReadFileTool::new();
        let result = tool
            .execute(serde_json::json!({"path": "/etc/passwd"}), &test_ctx())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("outside project root"));
    }
}

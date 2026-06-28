use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::{PathOp, PermissionMatrix};
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::{CeremoniesIntent, ReadAction};

pub struct ReadFileTool {
    permissions: Arc<PermissionMatrix>,
}

impl ReadFileTool {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self { permissions }
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

    async fn execute(&self, input: Value) -> Result<String, String> {
        let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
        let canonical = self.permissions.verify_path(path, PathOp::Read)?;

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
    use super::*;

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

    #[tokio::test]
    async fn read_file_happy_path() {
        let tool = ReadFileTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({"path": "Cargo.toml"}))
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("[package]"));
    }

    #[tokio::test]
    async fn read_file_missing_path() {
        let tool = ReadFileTool::new(test_perms());
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_file_nonexistent() {
        let tool = ReadFileTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({"path": "/nonexistent/file.txt"}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn max_lines_truncation() {
        let tool = ReadFileTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({"path": "Cargo.toml", "max_lines": 2}))
            .await;
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(
            content.contains("[truncated at line 2"),
            "should have truncation marker, got: {content}"
        );
        // Must contain the first line of Cargo.toml
        assert!(content.contains("[package]"), "should contain file content");
    }

    #[tokio::test]
    async fn read_file_outside_root_blocked() {
        let tool = ReadFileTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({"path": "/etc/passwd"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside project root"));
    }
}

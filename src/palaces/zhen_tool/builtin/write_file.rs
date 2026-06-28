use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::{PathOp, PermissionMatrix};
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::{CeremoniesIntent, WriteAction};

pub struct WriteFileTool {
    permissions: Arc<PermissionMatrix>,
}

impl WriteFileTool {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self { permissions }
    }
}

#[async_trait]
impl BaseTool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> String {
        "Write content to a file at the given path. Creates or overwrites the file.".to_string()
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
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> Result<String, String> {
        let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
        let content = input["content"]
            .as_str()
            .ok_or("Missing 'content' parameter")?;
        let canonical = self.permissions.verify_path(path, PathOp::Write)?;

        // Backup existing file before overwriting
        if tokio::fs::try_exists(&canonical).await.unwrap_or(false) {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let backup_dir = self.permissions.backup_dir.join(ts.to_string());
            if let Ok(()) = tokio::fs::create_dir_all(&backup_dir).await
                && let Some(fname) = canonical.file_name()
            {
                let _ = tokio::fs::copy(&canonical, backup_dir.join(fname)).await;
            }
        }

        tokio::fs::write(&canonical, content)
            .await
            .map_err(|e| format!("write_file error: {e}"))?;
        Ok(format!(
            "Wrote {} bytes to {}",
            content.len(),
            canonical.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

    #[tokio::test]
    async fn write_and_read_file() {
        let tool = WriteFileTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({
                "path": "jia-test-write.txt",
                "content": "hello jia"
            }))
            .await;
        assert!(result.is_ok());

        let content = tokio::fs::read_to_string("jia-test-write.txt")
            .await
            .unwrap();
        assert_eq!(content, "hello jia");
        let _ = tokio::fs::remove_file("jia-test-write.txt").await;
    }

    #[tokio::test]
    async fn write_file_missing_params() {
        let tool = WriteFileTool::new(test_perms());
        assert!(tool.execute(serde_json::json!({})).await.is_err());
        assert!(
            tool.execute(serde_json::json!({"path": "/tmp/test.txt"}))
                .await
                .is_err()
        );
    }
}

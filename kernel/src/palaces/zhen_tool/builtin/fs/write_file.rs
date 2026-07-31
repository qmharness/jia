use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

pub struct WriteFileTool {}

impl Default for WriteFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WriteFileTool {
    pub fn new() -> Self {
        Self {}
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
        CeremoniesIntent::Ji
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

    fn accesses(&self, input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // U1: declares exactly the target path; the conflict matrix serializes
        // intersecting reads/writes, so disjoint writes may run in parallel.
        // (Backups go to a timestamped per-second dir — disjoint targets keep
        // disjoint backup file names.)
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
        let content = input["content"]
            .as_str()
            .ok_or("Missing 'content' parameter")?;
        let canonical = ctx.permissions.verify_path(path, PathOp::Write)?;

        // Freshness gate (#4): if file exists, verify agent has read it recently
        if let Ok(meta) = tokio::fs::metadata(&canonical).await {
            if let Ok(mtime) = meta.modified() {
                ctx.check_freshness(&canonical, mtime)
                    .map_err(|e| ToolError::PermissionDenied(e))?;
            }
        }

        // Backup existing file before overwriting
        if tokio::fs::try_exists(&canonical).await.unwrap_or(false) {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let backup_dir = ctx.permissions.backup_dir.join(ts.to_string());
            if let Ok(()) = tokio::fs::create_dir_all(&backup_dir).await
                && let Some(fname) = canonical.file_name()
            {
                let _ = tokio::fs::copy(&canonical, backup_dir.join(fname)).await;
            }
        }

        tokio::fs::write(&canonical, content)
            .await
            .map_err(|e| format!("write_file error: {e}"))?;

        // Update read_state after successful write (#4 write-then-read rule)
        if let Ok(meta) = tokio::fs::metadata(&canonical).await {
            if let Ok(mtime) = meta.modified() {
                ctx.record_read(canonical.clone(), mtime);
            }
        }

        Ok(format!(
            "Wrote {} bytes to {}",
            content.len(),
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

    #[tokio::test]
    async fn write_and_read_file() {
        let tool = WriteFileTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "path": "jia-test-write.txt",
                    "content": "hello jia"
                }),
                &test_ctx(),
            )
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
        let tool = WriteFileTool::new();
        assert!(
            tool.execute(serde_json::json!({}), &test_ctx())
                .await
                .is_err()
        );
        assert!(
            tool.execute(serde_json::json!({"path": "/tmp/test.txt"}), &test_ctx())
                .await
                .is_err()
        );
    }
}

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::{CeremoniesIntent, ExecAction};

pub struct ShellTool {
    permissions: Arc<PermissionMatrix>,
}

impl ShellTool {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self { permissions }
    }
}

#[async_trait]
impl BaseTool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> String {
        "Execute a shell command and return stdout and stderr.".to_string()
    }

    fn category(&self) -> &str {
        "system"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Geng(ExecAction {
            command: String::new(),
        })
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value) -> Result<String, String> {
        let cmd = input["command"]
            .as_str()
            .ok_or("Missing 'command' parameter")?;
        self.permissions.execute_sandboxed(cmd).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

    #[tokio::test]
    async fn shell_echo() {
        let tool = ShellTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}))
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn shell_missing_command() {
        let tool = ShellTool::new(test_perms());
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_blocked_command() {
        let tool = ShellTool::new(test_perms());
        let result = tool
            .execute(serde_json::json!({"command": "rm -rf /tmp/foo"}))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("blocked pattern"));
    }
}

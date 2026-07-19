use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::CeremoniesIntent;

pub struct ShellTool {}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ShellTool {
    pub fn new() -> Self {
        Self {}
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
        CeremoniesIntent::Geng
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

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let cmd = input["command"]
            .as_str()
            .ok_or("Missing 'command' parameter")?;
        Ok(ctx.permissions.execute_sandboxed(cmd).await?)
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

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

    #[tokio::test]
    async fn shell_echo() {
        let tool = ShellTool::new();
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}), &test_ctx())
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().to_string().contains("hello"));
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
}

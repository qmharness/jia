use std::sync::Arc;
use crate::error::ToolError;
// ── McpTool — BaseTool wrapper for a single MCP tool ─────────

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;
use crate::stems::intent::{ExecAction, ReadAction};

use super::connection::McpConnection;
use super::protocol::McpToolDef;

/// A single MCP tool exposed as a framework BaseTool.
///
/// Owns an `Arc<McpConnection>` so it can call `tools/call`
/// when the agent invokes it. Permissions are injected via ExecContext
/// at execution time for sandboxing declared params.
pub struct McpTool {
    def: McpToolDef,
    connection: Arc<McpConnection>,
    sandbox_params: Vec<String>,
    read_only: bool,
}

impl McpTool {
    pub fn new(
        def: McpToolDef,
        connection: Arc<McpConnection>,
        sandbox_params: Vec<String>,
        read_only_tools: &[String],
    ) -> Self {
        let read_only = read_only_tools.contains(&def.name);
        Self {
            def,
            connection,
            sandbox_params,
            read_only,
        }
    }
}

#[async_trait]
impl BaseTool for McpTool {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn description(&self) -> String {
        self.def.description.clone()
    }

    fn ceremony(&self) -> CeremoniesIntent {
        if self.read_only {
            CeremoniesIntent::Wu(ReadAction {
                target: format!("MCP:{}", self.def.name),
            })
        } else {
            CeremoniesIntent::Geng(ExecAction {
                command: self.def.name.clone(),
            })
        }
    }

    fn parameters_schema(&self) -> Value {
        self.def.input_schema.clone()
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let sandboxed = ctx
            .permissions
            .sandbox_known_params(&input, &self.sandbox_params)?;
        let args = match &sandboxed {
            Value::Null => None,
            Value::Object(o) if o.is_empty() => None,
            _ => Some(sandboxed),
        };
        Ok(self.connection.call_tool(&self.def.name, args).await?)
    }
}

use crate::error::ToolError;
use std::sync::Arc;
use std::sync::Weak;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhen_tool::registry::ToolRegistry;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// P9 · ToolSearch — discover external (MCP/WASM) tools on demand.
///
/// jia does not use native function-calling; tool descriptions live in the
/// system prompt's stable segment (which P2 caches). To keep that segment
/// bounded when many MCP/WASM tools are registered, external tools are NOT
/// described in the prompt — the agent calls `toolsearch` to find them by
/// query, receiving their schema as a tool result (in the conversation, not
/// the system prompt). The tool is then callable normally (it is already in
/// the registry). This keeps the stable segment stable → P2 cache holds (D4).
///
/// Holds a `Weak<ToolRegistry>` to avoid a self-referential Arc at
/// registration time (the tool is registered into the registry it searches).
pub struct ToolSearchTool {
    registry: Weak<ToolRegistry>,
}

impl ToolSearchTool {
    pub fn new(registry: Weak<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl BaseTool for ToolSearchTool {
    fn name(&self) -> &str {
        "toolsearch"
    }

    fn description(&self) -> String {
        "Search for available external (MCP/plugin) tools by keyword. Returns \
         the name, description, and parameters schema of matching tools so you \
         can call them. Use this when you need a tool that isn't listed in your \
         system prompt (e.g., an MCP server tool)."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Keyword to search for in tool names/descriptions"},
                "limit": {"type": "integer", "description": "Max tools to return (default 8)"}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
        let limit = input["limit"].as_u64().unwrap_or(8) as usize;
        let query_lower = query.to_lowercase();

        let registry = self
            .registry
            .upgrade()
            .ok_or_else(|| "tool registry unavailable".to_string())?;

        let external = registry.list_external();
        if external.is_empty() {
            return Ok("No external (MCP/plugin) tools are registered.".to_string());
        }

        // Score by substring match in name / description / category (simple, no vectors).
        let mut scored: Vec<(usize, &Arc<dyn BaseTool>)> = external
            .into_iter()
            .filter_map(|t| {
                let name = t.name().to_lowercase();
                let desc = t.description().to_lowercase();
                let cat = t.category().to_lowercase();
                let score = (name.contains(&query_lower) as usize) * 3
                    + (desc.contains(&query_lower) as usize) * 2
                    + (cat.contains(&query_lower) as usize);
                if score == 0 { None } else { Some((score, t)) }
            })
            .collect();

        if scored.is_empty() {
            return Ok(format!(
                "No external tools matched '{}'. Available external tools: {}",
                query,
                registry
                    .list_external()
                    .iter()
                    .map(|t| t.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.truncate(limit);

        let mut lines = vec![format!(
            "Found {} external tool(s) for '{}':",
            scored.len(),
            query
        )];
        for (_, t) in scored {
            lines.push(format!(
                "### {}\n{}\nParameters: {}\n",
                t.name(),
                t.description(),
                serde_json::to_string_pretty(&t.parameters_schema()).unwrap_or_default()
            ));
        }
        Ok(lines.join("\n"))
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

    struct DummyExternal {
        n: &'static str,
        d: &'static str,
    }
    #[async_trait]
    impl BaseTool for DummyExternal {
        fn name(&self) -> &str {
            self.n
        }
        fn description(&self) -> String {
            self.d.to_string()
        }
        fn ceremony(&self) -> CeremoniesIntent {
            CeremoniesIntent::Wu
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({"type":"object","properties":{"x":{"type":"string"}}})
        }
        fn is_concurrency_safe(&self) -> bool {
            true
        }
        async fn execute(&self, _input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }

    #[tokio::test]
    async fn toolsearch_finds_external_by_name() {
        let mut reg = ToolRegistry::new();
        reg.register_external(Arc::new(DummyExternal {
            n: "mcp_github",
            d: "GitHub PR operations",
        }));
        reg.register_external(Arc::new(DummyExternal {
            n: "mcp_slack",
            d: "Slack messaging",
        }));
        let reg_arc = Arc::new(reg);
        let weak = Arc::downgrade(&reg_arc);
        let tool = ToolSearchTool::new(weak);

        let res = tool
            .execute(serde_json::json!({ "query": "github" }), &test_ctx())
            .await
            .unwrap();
        assert!(res.contains("mcp_github"));
        assert!(!res.contains("mcp_slack"));
        drop(reg_arc);
    }

    #[tokio::test]
    async fn toolsearch_no_external() {
        let reg_arc = Arc::new(ToolRegistry::new());
        let weak = Arc::downgrade(&reg_arc);
        let tool = ToolSearchTool::new(weak);
        let res = tool
            .execute(serde_json::json!({ "query": "anything" }), &test_ctx())
            .await
            .unwrap();
        assert!(res.contains("No external"));
        drop(reg_arc);
    }

    #[tokio::test]
    async fn toolsearch_no_match_lists_available() {
        let mut reg = ToolRegistry::new();
        reg.register_external(Arc::new(DummyExternal {
            n: "mcp_x",
            d: "does things",
        }));
        let reg_arc = Arc::new(reg);
        let weak = Arc::downgrade(&reg_arc);
        let tool = ToolSearchTool::new(weak);
        let res = tool
            .execute(serde_json::json!({ "query": "zzznomatch" }), &test_ctx())
            .await
            .unwrap();
        assert!(res.contains("No external tools matched"));
        assert!(res.contains("mcp_x"));
        drop(reg_arc);
    }
}

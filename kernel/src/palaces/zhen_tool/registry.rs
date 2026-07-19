use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::base::BaseTool;

/// 震三宫 — Tool Registry
///
/// Registers and looks up tools by name. Tools are either "core" (built-in,
/// always described in the system prompt) or "external" (MCP/WASM, surfaced
/// on demand via the `toolsearch` tool — P9).
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn BaseTool>>,
    /// Names of external (MCP/WASM) tools — not described in the system prompt;
    /// discovered via `toolsearch`. Keeps the P2 cacheable stable segment
    /// bounded (D4).
    external: HashSet<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            external: HashSet::new(),
        }
    }

    /// Register a core (built-in) tool.
    pub fn register(&mut self, tool: Arc<dyn BaseTool>) {
        let name = tool.name().to_string();
        self.tools.insert(name, tool);
    }

    /// Register an external (MCP/WASM) tool. External tools are NOT described in
    /// the system prompt; they are discovered via `toolsearch` (P9).
    pub fn register_external(&mut self, tool: Arc<dyn BaseTool>) {
        let name = tool.name().to_string();
        self.external.insert(name.clone());
        self.tools.insert(name, tool);
    }

    /// Is a tool name external (MCP/WASM)?
    pub fn is_external(&self, name: &str) -> bool {
        self.external.contains(name)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn BaseTool>> {
        // Fast path: exact match
        if let Some(tool) = self.tools.get(name) {
            return Some(tool);
        }
        // Fallback: case-insensitive match (LLMs may vary casing)
        let lower = name.to_lowercase();
        self.tools.iter().find_map(|(k, v)| {
            if k.to_lowercase() == lower {
                Some(v)
            } else {
                None
            }
        })
    }

    pub fn list_names(&self) -> Vec<&String> {
        let mut names: Vec<_> = self.tools.keys().collect();
        names.sort();
        names
    }

    pub fn list_all(&self) -> Vec<&Arc<dyn BaseTool>> {
        self.tools.values().collect()
    }

    /// Core (built-in, non-external) tools — described in the system prompt.
    pub fn list_core(&self) -> Vec<&Arc<dyn BaseTool>> {
        self.tools
            .iter()
            .filter(|(k, _)| !self.external.contains(*k))
            .map(|(_, v)| v)
            .collect()
    }

    /// External (MCP/WASM) tools — surfaced via `toolsearch` (P9).
    pub fn list_external(&self) -> Vec<&Arc<dyn BaseTool>> {
        self.tools
            .iter()
            .filter(|(k, _)| self.external.contains(*k))
            .map(|(_, v)| v)
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ToolError;
    use crate::stems::action::ExecContext;

    struct DummyTool;
    #[async_trait::async_trait]
    impl BaseTool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> String {
            "a dummy tool".to_string()
        }
        fn ceremony(&self) -> crate::stems::CeremoniesIntent {
            crate::stems::CeremoniesIntent::Wu
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn is_concurrency_safe(&self) -> bool {
            false
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ExecContext,
        ) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }

    /// Configurable dummy for core/external split tests.
    struct NamedTool(&'static str);
    #[async_trait::async_trait]
    impl BaseTool for NamedTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> String {
            "named".to_string()
        }
        fn ceremony(&self) -> crate::stems::CeremoniesIntent {
            crate::stems::CeremoniesIntent::Wu
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        fn is_concurrency_safe(&self) -> bool {
            false
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ExecContext,
        ) -> Result<String, ToolError> {
            Ok("ok".into())
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(DummyTool));
        assert!(reg.get("dummy").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert_eq!(reg.list_names(), vec!["dummy"]);
        assert_eq!(reg.list_all().len(), 1);
    }

    #[test]
    fn core_vs_external_split() {
        // P9 D4 mechanism: core tools go in the stable prompt, external don't.
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(NamedTool("core_a")));
        reg.register(Arc::new(NamedTool("core_b")));
        reg.register_external(Arc::new(NamedTool("mcp_x")));
        reg.register_external(Arc::new(NamedTool("wasm_y")));

        assert_eq!(reg.list_core().len(), 2, "core = 2 builtins");
        assert_eq!(reg.list_external().len(), 2, "external = 2");
        assert_eq!(reg.list_all().len(), 4, "all = 4");
        assert!(!reg.is_external("core_a"));
        assert!(reg.is_external("mcp_x"));
        // list_core must NOT contain external tools
        let core_names: Vec<&str> = reg.list_core().iter().map(|t| t.name()).collect();
        assert!(!core_names.contains(&"mcp_x"));
        assert!(!core_names.contains(&"wasm_y"));
    }
}

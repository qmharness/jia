// ── MCP Manager — Connect servers, discover tools, register ───

use std::collections::HashMap;
use std::sync::Arc;

use crate::palaces::kun_config::McpServerConfig;
use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::ToolRegistry;

use super::connection::McpConnection;
use super::tool::McpTool;

/// Manages MCP server connections and tool registration.
pub struct McpManager;

impl McpManager {
    /// Connect to all configured MCP servers concurrently, discover their tools,
    /// and register them into the given `ToolRegistry`.
    pub fn connect_all(
        servers: &[McpServerConfig],
        registry: &mut ToolRegistry,
        permissions: Arc<PermissionMatrix>,
    ) {
        if servers.is_empty() {
            return;
        }

        // Index per-server config for McpTool construction
        let mut config_lookup: HashMap<String, (Vec<String>, Vec<String>)> = servers
            .iter()
            .map(|cfg| {
                (
                    cfg.name.clone(),
                    (cfg.sandbox_params.clone(), cfg.read_only_tools.clone()),
                )
            })
            .collect();

        let rt = tokio::runtime::Handle::current();

        let futures: Vec<_> = servers
            .iter()
            .map(|cfg| {
                let cfg = cfg.clone();
                tokio::spawn(async move {
                    let conn = McpConnection::connect(&cfg).await?;
                    let tools = conn.list_tools().await?;
                    Ok::<_, String>((Arc::new(conn), cfg.name.clone(), tools))
                })
            })
            .collect();

        let results = rt.block_on(async {
            let mut results = Vec::new();
            for f in futures {
                results.push(f.await.unwrap_or_else(|e| Err(format!("Join error: {e}"))));
            }
            results
        });

        for result in results {
            match result {
                Ok((conn, name, tools)) => {
                    let (sandbox_params, read_only_tools) =
                        config_lookup.remove(&name).unwrap_or_default();
                    let count = tools.len();
                    for def in tools {
                        let tool = McpTool::new(
                            def,
                            conn.clone(),
                            permissions.clone(),
                            sandbox_params.clone(),
                            &read_only_tools,
                        );
                        registry.register_external(Arc::new(tool));
                    }
                    tracing::info!(server = %name, tools = count, "MCP server connected");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to connect MCP server");
                }
            }
        }
    }
}

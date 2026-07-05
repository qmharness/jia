use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;

use super::AppState;

pub async fn handle_providers(State(state): State<Arc<AppState>>) -> Json<Vec<serde_json::Value>> {
    let list: Vec<_> = state
        .providers
        .iter()
        .map(|(name, p)| {
            serde_json::json!({
                "name": name,
                "kind": p.kind,
                "models": p.models,
                "default_model": p.default_main_model(),
            })
        })
        .collect();
    Json(list)
}

#[derive(Debug, Deserialize)]
pub struct FilesQuery {
    path: Option<String>,
    #[serde(default)]
    root: Option<String>,
}

pub async fn handle_files(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<FilesQuery>,
) -> Json<serde_json::Value> {
    let relative = query.path.as_deref().unwrap_or(".");
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };

    // Use explicit root as sandbox boundary, or fall back to global project_root
    let sandbox_root = match &query.root {
        Some(r) => {
            let p = std::path::PathBuf::from(r);
            let p2 = p.clone();
            tokio::task::spawn_blocking(move || std::fs::canonicalize(&p).unwrap_or(p))
                .await
                .unwrap_or(p2)
        }
        None => earth.permissions.sandbox.project_root.clone(),
    };

    // Resolve path relative to sandbox root
    let candidate = if relative.starts_with('/') {
        std::path::PathBuf::from(relative)
    } else {
        sandbox_root.join(relative)
    };
    let resolved =
        match tokio::task::spawn_blocking(move || std::fs::canonicalize(&candidate)).await {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                return Json(serde_json::json!({"error": format!("Cannot resolve path: {e}")}));
            }
            Err(_) => return Json(serde_json::json!({"error": "Internal error resolving path"})),
        };

    // Verify within the sandbox root
    if !resolved.starts_with(&sandbox_root) {
        return Json(serde_json::json!({
            "error": format!("path '{}' is outside project root '{}'", relative, sandbox_root.display())
        }));
    }

    if resolved.is_dir() {
        match tokio::fs::read_dir(&resolved).await {
            Ok(mut entries) => {
                let mut items: Vec<serde_json::Value> = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                    let entry_path = resolved.join(&name);
                    let rel = entry_path
                        .strip_prefix(&sandbox_root)
                        .unwrap_or(&entry_path)
                        .to_string_lossy()
                        .into_owned();
                    items.push(serde_json::json!({
                        "name": name,
                        "path": rel,
                        "isDir": is_dir,
                    }));
                }
                items.sort_by(|a, b| {
                    let a_dir = a["isDir"].as_bool().unwrap_or(false);
                    let b_dir = b["isDir"].as_bool().unwrap_or(false);
                    match (a_dir, b_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a["name"]
                            .as_str()
                            .unwrap_or("")
                            .cmp(b["name"].as_str().unwrap_or("")),
                    }
                });
                Json(serde_json::json!({"type": "directory", "entries": items}))
            }
            Err(e) => Json(serde_json::json!({"error": format!("Cannot read directory: {e}")})),
        }
    } else if resolved.is_file() {
        match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => Json(serde_json::json!({
                "type": "file",
                "content": content,
                "name": resolved.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                "path": resolved.strip_prefix(&sandbox_root)
                    .unwrap_or(&resolved)
                    .to_string_lossy()
                    .to_string(),
            })),
            Err(e) => Json(serde_json::json!({"error": format!("Cannot read file: {e}")})),
        }
    } else {
        Json(serde_json::json!({"error": "Path is neither a file nor directory"}))
    }
}

pub async fn handle_config(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut config = serde_json::json!({
        "server": {
            "host": "127.0.0.1",
            "port": 3000,
        },
        "mcp_servers": [],
        "bots": {},
        "security": {},
    });

    if let Some(earth) = &state.earth {
        let app = &earth.config.app_config;

        config["server"] = serde_json::json!({
            "host": app.host,
            "port": app.port,
        });
        config["mcp_servers"] = serde_json::json!(
            app.mcp_servers
                .iter()
                .map(|m| serde_json::json!({
                    "name": m.name,
                    "command": m.command,
                    "args": m.args,
                }))
                .collect::<Vec<_>>()
        );

        config["bots"] = serde_json::json!({
            "telegram": app.bots.telegram.as_ref().map(|_| true).unwrap_or(false),
            "wechat": app.bots.wechat.as_ref().map(|_| true).unwrap_or(false),
        });

        config["security"] = serde_json::json!({
            "project_root": earth.permissions.sandbox.project_root.to_string_lossy(),
            "sandbox_disabled": earth.permissions.sandbox_mode.clone(),
            "confirmation_timeout_secs": earth.permissions.confirmation_timeout.as_secs(),
        });
    }

    Json(config)
}

pub async fn handle_tools(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let mut groups: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
        std::collections::BTreeMap::new();

    if let Some(earth) = &state.earth {
        for tool in earth.tools.list_all() {
            let entry = serde_json::json!({
                "name": tool.name(),
                "description": tool.description(),
                "parameters": tool.parameters_schema(),
            });
            groups
                .entry(tool.category().to_string())
                .or_default()
                .push(entry);
        }
    }

    let groups: Vec<_> = groups
        .into_iter()
        .map(|(category, tools)| serde_json::json!({ "category": category, "tools": tools }))
        .collect();

    Json(serde_json::json!({ "groups": groups }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_query_deserializes() {
        let q: FilesQuery = serde_json::from_str(r#"{"path": "/tmp"}"#).unwrap();
        assert_eq!(q.path, Some("/tmp".into()));
    }

    #[test]
    fn files_query_defaults_root() {
        let q: FilesQuery = serde_json::from_str(r#"{}"#).unwrap();
        assert!(q.path.is_none());
        assert!(q.root.is_none());
    }
}

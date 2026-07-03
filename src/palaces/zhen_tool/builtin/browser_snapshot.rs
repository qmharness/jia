// ── browser_snapshot — Get structured page content via Accessibility.getFullAXTree ──

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::stems::action::ExecContext;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserSnapshotTool ───────────────────────────────────────

pub struct BrowserSnapshotTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl BrowserSnapshotTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }
}

#[async_trait]
impl BaseTool for BrowserSnapshotTool {
    fn name(&self) -> &str {
        "browser_snapshot"
    }

    fn description(&self) -> String {
        "Get a structured accessibility snapshot of the current page. \
         Returns a tree with ref IDs for interactive elements that can be \
         used with browser_click. Each ref is a backend DOM node ID. \
         More compact than the DOM snapshot from web_execute_js — \
         use this to identify buttons, inputs, and links."
            .to_string()
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren(CommunicateAction {
            endpoint: String::new(),
            payload: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, String> {
        let tab_id = input["tab_id"].as_str();

        let tabs = browser_cdp::get_tabs(&self.client).await?;
        if tabs.is_empty() {
            return Err(
                "No browser tabs found. Start Chrome with: chrome --remote-debugging-port=9222"
                    .into(),
            );
        }

        let target = if let Some(tid) = tab_id {
            tabs.iter().find(|t| t.id == tid).ok_or_else(|| {
                format!(
                    "Tab '{tid}' not found. Available: {}",
                    browser_cdp::tab_list(&tabs)
                )
            })?
        } else {
            &tabs[0]
        };

        match get_snapshot(&target.web_socket_debugger_url).await {
            Ok(snapshot) => Ok(serde_json::json!({
                "snapshot": snapshot,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "snapshot": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

// ── AX tree flattening ────────────────────────────────────────

async fn get_snapshot(ws_url: &str) -> Result<String, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable Accessibility domain
    browser_cdp::cdp_send(&mut ws, 0, "Accessibility.enable", serde_json::json!({})).await?;

    // Get full AX tree with computed properties
    let result = browser_cdp::cdp_send(
        &mut ws,
        1,
        "Accessibility.getFullAXTree",
        serde_json::json!({ "depth": 8 }),
    )
    .await?;

    let nodes: Vec<Value> = result
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();

    if nodes.is_empty() {
        return Ok("(empty page — no accessibility nodes)".into());
    }

    // Build parent lookup
    let mut nodes_by_id: HashMap<String, &Value> = HashMap::new();
    for node in &nodes {
        if let Some(id) = node.get("nodeId").and_then(|v| v.as_str()) {
            nodes_by_id.insert(id.to_string(), node);
        }
    }

    // Find root and render
    let root_ids: Vec<String> = nodes
        .iter()
        .filter(|n| {
            !n.get("role")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
                .map(|r| r == "RootWebArea")
                .unwrap_or(false)
                || n.get("parentId").is_none()
        })
        .filter_map(|n| n.get("nodeId").and_then(|v| v.as_str()).map(String::from))
        .collect();

    let mut out = String::new();
    for root_id in &root_ids {
        render_node(root_id, &nodes_by_id, "", &mut out);
    }

    if out.is_empty() {
        return Ok("(no accessible content)".into());
    }

    Ok(out)
}

fn render_node(node_id: &str, nodes: &HashMap<String, &Value>, indent: &str, out: &mut String) {
    let node = match nodes.get(node_id) {
        Some(n) => n,
        None => return,
    };

    let role = node
        .get("role")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let name = node
        .get("name")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let value = node
        .get("value")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str());

    let backend_id = node.get("backendDOMNodeId").and_then(|v| v.as_u64());

    let ignored = node
        .get("ignored")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Skip pure layout/ignored nodes
    if ignored && role != "RootWebArea" {
        // Still render children of ignored nodes
        if let Some(children) = node.get("childIds").and_then(|c| c.as_array()) {
            for child in children {
                if let Some(cid) = child.as_str() {
                    render_node(cid, nodes, indent, out);
                }
            }
        }
        return;
    }

    // Build line
    let ref_str = if let Some(bid) = backend_id {
        format!("[ref=e{}]", bid)
    } else {
        String::new()
    };

    let name_str = if name.is_empty() {
        String::new()
    } else if name.len() > 80 {
        format!(" \"{}...\"", &name[..80])
    } else {
        format!(" \"{}\"", name)
    };

    let value_str = if let Some(v) = value {
        if v.len() > 40 {
            format!(" = \"{}...\"", &v[..40])
        } else {
            format!(" = \"{}\"", v)
        }
    } else {
        String::new()
    };

    let line = format!("{indent}{role}{name_str}{value_str} {ref_str}\n");
    out.push_str(&line);

    // Render children
    if let Some(children) = node.get("childIds").and_then(|c| c.as_array()) {
        let next_indent = format!("{indent}  ");
        for child in children {
            if let Some(cid) = child.as_str() {
                render_node(cid, nodes, &next_indent, out);
            }
        }
    }
}

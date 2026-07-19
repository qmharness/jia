// ── browser_type — Type text into an input element ──
//
// Resolves the element by ref ID, focuses it, sets value, and dispatches
// input/change events so JS frameworks (React, Vue) detect the change.

use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::CeremoniesIntent;

use crate::palaces::zhen_tool::browser_cdp;

const OPT_HTML_JS: &str = include_str!("opt_html.js");

// ── BrowserTypeTool ───────────────────────────────────────────

pub struct BrowserTypeTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserTypeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserTypeTool {
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
impl BaseTool for BrowserTypeTool {
    fn name(&self) -> &str {
        "browser_type"
    }

    fn description(&self) -> String {
        "Type text into an input or textarea element. \
         Get the element ref from browser_snapshot. \
         Dispatches focus + input + change events so JS frameworks detect the change. \
         Returns a DOM diff showing what changed."
            .to_string()
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "Element ref ID from browser_snapshot (e.g. 'e42'). Must be an input, textarea, or select."
                },
                "text": {
                    "type": "string",
                    "description": "The text to type into the element."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": ["ref", "text"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let ref_str = input["ref"].as_str().ok_or("missing 'ref' parameter")?;
        let text = input["text"].as_str().ok_or("missing 'text' parameter")?;
        let tab_id = input["tab_id"].as_str();

        let backend_id: u64 = ref_str
            .strip_prefix('e')
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("Invalid ref '{ref_str}'. Expected format: eNNN (e.g. e42)"))?;

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

        match type_text(&target.web_socket_debugger_url, backend_id, text).await {
            Ok(r) => Ok(serde_json::json!({
                "ref": ref_str,
                "typed": text,
                "tagName": r.tag_name,
                "domDiff": r.dom_diff,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "ref": ref_str,
                "typed": text,
                "error": e,
                "tabs": browser_cdp::tabs_json(&tabs),
            })
            .to_string()),
        }
    }
}

struct TypeResult {
    tag_name: String,
    dom_diff: String,
}

async fn type_text(ws_url: &str, backend_id: u64, text: &str) -> Result<TypeResult, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    browser_cdp::cdp_send(&mut ws, 0, "Runtime.enable", serde_json::json!({})).await?;
    browser_cdp::cdp_send(&mut ws, 1, "DOM.enable", serde_json::json!({})).await?;

    // Pre-snapshot
    let before_html = browser_cdp::cdp_evaluate(&mut ws, &format!("({OPT_HTML_JS})()")).await?;

    // Resolve node
    let resolve = browser_cdp::cdp_send(
        &mut ws,
        2,
        "DOM.resolveNode",
        serde_json::json!({ "backendNodeId": backend_id }),
    )
    .await?;
    let object_id = resolve
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Element with backendNodeId {backend_id} not found"))?;

    // Get tag name
    let info = browser_cdp::cdp_send(&mut ws, 3, "Runtime.callFunctionOn",
        serde_json::json!({
            "objectId": object_id,
            "functionDeclaration": "function() { return this.tagName?.toLowerCase() || 'unknown'; }",
            "returnByValue": true,
        })).await?;
    let tag_name = info
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // Scroll into view, focus, set value, dispatch events
    let text_escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
    let script = format!(
        "function() {{ this.scrollIntoView({{block:'center',behavior:'instant'}}); this.focus(); \
         this.value = '{text_escaped}'; \
         this.dispatchEvent(new Event('input',{{bubbles:true}})); \
         this.dispatchEvent(new Event('change',{{bubbles:true}})); \
         return 'ok'; }}"
    );
    browser_cdp::cdp_send(
        &mut ws,
        4,
        "Runtime.callFunctionOn",
        serde_json::json!({
            "objectId": object_id,
            "functionDeclaration": script,
            "returnByValue": true,
        }),
    )
    .await?;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Post-snapshot + diff
    let after_html = browser_cdp::cdp_evaluate(&mut ws, &format!("({OPT_HTML_JS})()")).await?;
    let dom_diff = if before_html == after_html {
        "页面无变化".into()
    } else {
        let delta = after_html.len() as i64 - before_html.len() as i64;
        format!(
            "DOM变化: {} → {} 字节 ({}{})",
            before_html.len(),
            after_html.len(),
            if delta >= 0 { "+" } else { "" },
            delta
        )
    };

    Ok(TypeResult { tag_name, dom_diff })
}

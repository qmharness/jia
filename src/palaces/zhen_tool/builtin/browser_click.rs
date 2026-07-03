// ── browser_click — Click an element by accessibility ref ID ──
//
// Uses the backendDOMNodeId from browser_snapshot's [ref=eNNN] markers.
// Resolves the node via DOM.resolveNode, then scrolls into view and clicks.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserClickTool ──────────────────────────────────────────

pub struct BrowserClickTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserClickTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserClickTool {
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
impl BaseTool for BrowserClickTool {
    fn name(&self) -> &str {
        "browser_click"
    }

    fn description(&self) -> String {
        "Click an element in the live browser by its ref ID. \
         Get ref IDs from browser_snapshot's [ref=eNNN] markers. \
         The browser must be running with --remote-debugging-port=9222. \
         Returns a DOM diff showing what changed after the click."
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
        false
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "Element ref ID from browser_snapshot (e.g. 'e42')."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": ["ref"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, String> {
        let ref_str = input["ref"].as_str().ok_or("missing 'ref' parameter")?;
        let tab_id = input["tab_id"].as_str();

        // Parse backend DOM node ID from ref string: "e123" → 123
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

        match click_element(&target.web_socket_debugger_url, backend_id).await {
            Ok(r) => Ok(serde_json::json!({
                "clicked": ref_str,
                "tagName": r.tag_name,
                "visibleText": r.text,
                "domDiff": r.dom_diff,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "clicked": ref_str,
                "tagName": null,
                "visibleText": null,
                "domDiff": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

struct ClickResult {
    tag_name: String,
    text: String,
    dom_diff: String,
}

async fn click_element(ws_url: &str, backend_id: u64) -> Result<ClickResult, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable required domains
    browser_cdp::cdp_send(&mut ws, 0, "Runtime.enable", serde_json::json!({})).await?;
    browser_cdp::cdp_send(&mut ws, 1, "DOM.enable", serde_json::json!({})).await?;

    // Snapshot before click
    let before_html =
        browser_cdp::cdp_evaluate(&mut ws, &format!("({})()", include_str!("opt_html.js"))).await?;

    // Resolve backend node to an object
    let resolve_result = browser_cdp::cdp_send(
        &mut ws,
        2,
        "DOM.resolveNode",
        serde_json::json!({ "backendNodeId": backend_id }),
    )
    .await?;

    let object_id = resolve_result
        .get("object")
        .and_then(|o| o.get("objectId"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Element with backendNodeId {backend_id} not found. It may have been removed from the DOM."))?;

    // Get element info: tag name and visible text
    let info = browser_cdp::cdp_send(
        &mut ws,
        3,
        "Runtime.callFunctionOn",
        serde_json::json!({
            "objectId": object_id,
            "functionDeclaration": "function() { return { tagName: this.tagName?.toLowerCase() || 'unknown', text: (this.textContent || '').trim().substring(0, 200) }; }",
            "returnByValue": true,
        }),
    )
    .await?;

    let tag_name = info
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.get("tagName"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let text = info
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Scroll into view and click
    browser_cdp::cdp_send(
        &mut ws,
        4,
        "Runtime.callFunctionOn",
        serde_json::json!({
            "objectId": object_id,
            "functionDeclaration": "function() { this.scrollIntoView({block:'center',inline:'center',behavior:'instant'}); this.click(); return 'clicked'; }",
            "returnByValue": true,
        }),
    )
    .await?;

    // Wait for DOM updates
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Snapshot after click
    let after_html =
        browser_cdp::cdp_evaluate(&mut ws, &format!("({})()", include_str!("opt_html.js"))).await?;

    // Diff
    let dom_diff = if before_html == after_html {
        "页面无变化".into()
    } else {
        let before_len = before_html.len();
        let after_len = after_html.len();
        let delta = after_len as i64 - before_len as i64;
        format!(
            "DOM变化: {} → {} 字节 ({}{})",
            before_len,
            after_len,
            if delta >= 0 { "+" } else { "" },
            delta
        )
    };

    Ok(ClickResult {
        tag_name,
        text,
        dom_diff,
    })
}

// ── web_execute_js — Execute JavaScript in a live browser via CDP ──
//
// Connects to a Chromium-based browser running with --remote-debugging-port.
// Each call: connect → snapshot optHTML → eval user script → snapshot optHTML → diff → return.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::browser_cdp;

const MAX_RESULT_CHARS: usize = 3_000;

// ── Embedded JS from GenericAgent's simphtml.py ──────────────
const OPT_HTML_JS: &str = include_str!("opt_html.js");

// ── WebExecuteJsTool ─────────────────────────────────────────

pub struct WebExecuteJsTool {
    #[allow(dead_code)]
    permissions: Arc<PermissionMatrix>,
    client: reqwest::Client,
}

impl WebExecuteJsTool {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self {
            permissions,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        }
    }
}

#[async_trait]
impl BaseTool for WebExecuteJsTool {
    fn name(&self) -> &str {
        "web_execute_js"
    }

    fn description(&self) -> String {
        "Execute JavaScript in a live Chromium-based browser via CDP. \
         The browser must be running with --remote-debugging-port=9222. \
         Before and after your script runs, a simplified DOM snapshot is \
         taken (optHTML), and the diff is returned as domChanges. \
         Use an empty script to just read the current page snapshot. \
         The return value of your script is in jsReturn."
            .to_string()
    }

    fn category(&self) -> &str {
        "web"
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
                "script": {
                    "type": "string",
                    "description": "JavaScript code to execute in the browser. \
                                   Use return to send values back. Empty string = read page snapshot only."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID (from the tabs list in previous results). \
                                   Uses the first available tab if omitted."
                }
            },
            "required": ["script"]
        })
    }

    async fn execute(&self, input: Value) -> Result<String, String> {
        let script = input["script"].as_str().unwrap_or("");
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

        let result = execute_cdp_flow(&target.web_socket_debugger_url, script).await;

        match result {
            Ok(r) => Ok(serde_json::json!({
                "jsReturn": r.js_return,
                "tabs": browser_cdp::tabs_json(&tabs),
                "domChanges": r.dom_changes,
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "jsReturn": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "domChanges": null,
                "error": e,
            })
            .to_string()),
        }
    }
}

struct ExecuteResult {
    js_return: String,
    dom_changes: String,
}

async fn execute_cdp_flow(ws_url: &str, script: &str) -> Result<ExecuteResult, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable Runtime domain
    browser_cdp::cdp_send(&mut ws, 0, "Runtime.enable", serde_json::json!({})).await?;

    // Step 1: pre-snapshot via optHTML
    let before_html = browser_cdp::cdp_evaluate(&mut ws, &format!("({OPT_HTML_JS})()")).await?;

    // Step 2: execute user script
    let js_return = browser_cdp::cdp_evaluate(&mut ws, script).await?;

    // Step 3: wait 1s for potential DOM updates
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Step 4: post-snapshot
    let after_html = browser_cdp::cdp_evaluate(&mut ws, &format!("({OPT_HTML_JS})()")).await?;

    // Step 5: diff
    let dom_changes = diff_html(&before_html, &after_html);

    Ok(ExecuteResult {
        js_return: truncate(&js_return, MAX_RESULT_CHARS),
        dom_changes,
    })
}

// ── Simple DOM diff (Rust-side, string-based) ────────────────

fn diff_html(before: &str, after: &str) -> String {
    if before == after {
        return "页面无变化".into();
    }

    let before_len = before.len();
    let after_len = after.len();
    let size_delta = after_len as i64 - before_len as i64;

    let mut diff_pos = 0usize;
    let min_len = before_len.min(after_len);
    let before_chars: Vec<char> = before.chars().collect();
    let after_chars: Vec<char> = after.chars().collect();
    while diff_pos < min_len && before_chars[diff_pos] == after_chars[diff_pos] {
        diff_pos += 1;
    }

    let mut summary = format!(
        "发现变化 (DOM大小: {} → {}, 差异: {}{}字节)",
        before_len,
        after_len,
        if size_delta >= 0 { "+" } else { "" },
        size_delta
    );

    if diff_pos < after_chars.len() {
        let start = diff_pos.saturating_sub(50);
        let end = (diff_pos + 200).min(after_chars.len());
        let snippet: String = after_chars[start..end].iter().collect();
        let clean = snippet
            .replace('\n', " ")
            .replace("  ", " ")
            .trim()
            .to_string();
        if clean.len() > 20 {
            summary.push_str(&format!("\n变化区域: ...{}", clean));
        }
    }

    summary
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars].iter().collect();
        format!("{truncated}... [截断，完整长度: {}]", s.len())
    }
}

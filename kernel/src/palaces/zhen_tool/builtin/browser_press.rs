// ── browser_press — Press a keyboard key via Input.dispatchKeyEvent ──
//
// Sends keyDown + keyUp events for the given key. Useful for Enter (submit forms),
// Escape (close dialogs), Tab (navigate fields), arrow keys, and shortcuts.

use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::CeremoniesIntent;

use crate::palaces::zhen_tool::browser_cdp;

const OPT_HTML_JS: &str = include_str!("opt_html.js");

// ── BrowserPressKeyTool ───────────────────────────────────────

pub struct BrowserPressKeyTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserPressKeyTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserPressKeyTool {
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
impl BaseTool for BrowserPressKeyTool {
    fn name(&self) -> &str {
        "browser_press"
    }

    fn description(&self) -> String {
        "Press a keyboard key in the browser. \
         Common keys: Enter, Escape, Tab, ArrowDown, ArrowUp, ArrowLeft, ArrowRight, Backspace, Delete. \
         Use to submit forms, close dialogs, or navigate. \
         Returns a DOM diff showing what changed.".to_string()
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
                "key": {
                    "type": "string",
                    "description": "Key name: Enter, Escape, Tab, ArrowDown/Up/Left/Right, Backspace, Delete, PageDown, PageUp, Home, End."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": ["key"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let key = input["key"].as_str().ok_or("missing 'key' parameter")?;
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

        match press_key(&target.web_socket_debugger_url, key).await {
            Ok(dom_diff) => Ok(serde_json::json!({
                "key": key,
                "domDiff": dom_diff,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "key": key,
                "domDiff": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

fn key_to_cdp(key: &str) -> Result<(&str, Option<&str>, Option<u32>), String> {
    match key {
        "Enter" => Ok(("Enter", Some("\r"), Some(13))),
        "Escape" => Ok(("Escape", Some("\u{001b}"), Some(27))),
        "Tab" => Ok(("Tab", Some("\t"), Some(9))),
        "Backspace" => Ok(("Backspace", Some("\u{0008}"), Some(8))),
        "Delete" => Ok(("Delete", Some("\u{007f}"), Some(46))),
        "ArrowDown" => Ok(("ArrowDown", None, Some(40))),
        "ArrowUp" => Ok(("ArrowUp", None, Some(38))),
        "ArrowLeft" => Ok(("ArrowLeft", None, Some(37))),
        "ArrowRight" => Ok(("ArrowRight", None, Some(39))),
        "PageDown" => Ok(("PageDown", None, Some(34))),
        "PageUp" => Ok(("PageUp", None, Some(33))),
        "Home" => Ok(("Home", None, Some(36))),
        "End" => Ok(("End", None, Some(35))),
        _ => Err(format!(
            "Unknown key '{key}'. Supported: Enter, Escape, Tab, Backspace, Delete, ArrowDown/Up/Left/Right, PageDown/Up, Home, End"
        )),
    }
}

async fn press_key(ws_url: &str, key: &str) -> Result<String, String> {
    let (key_name, text, code) = key_to_cdp(key)?;

    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    browser_cdp::cdp_send(&mut ws, 0, "Runtime.enable", serde_json::json!({})).await?;
    browser_cdp::cdp_send(&mut ws, 1, "Input.enable", serde_json::json!({})).await?;

    // Pre-snapshot
    let before_html = browser_cdp::cdp_evaluate(&mut ws, &format!("({OPT_HTML_JS})()")).await?;

    // Build dispatch params
    let mut params = serde_json::json!({ "type": "keyDown", "key": key_name });
    if let Some(t) = text {
        params["text"] = Value::String(t.to_string());
    }
    if let Some(c) = code {
        params["windowsVirtualKeyCode"] = Value::Number(c.into());
    }
    params["code"] = Value::String(key_name.to_string());

    // keyDown
    browser_cdp::cdp_send(&mut ws, 2, "Input.dispatchKeyEvent", params.clone()).await?;

    // keyUp
    params["type"] = Value::String("keyUp".to_string());
    browser_cdp::cdp_send(&mut ws, 3, "Input.dispatchKeyEvent", params).await?;

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

    Ok(dom_diff)
}

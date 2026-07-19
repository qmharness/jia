// ── browser_scroll — Scroll the page without heavy DOM snapshots ──

use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::CeremoniesIntent;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserScrollTool ─────────────────────────────────────────

pub struct BrowserScrollTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserScrollTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserScrollTool {
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
impl BaseTool for BrowserScrollTool {
    fn name(&self) -> &str {
        "browser_scroll"
    }

    fn description(&self) -> String {
        "Scroll the page up or down. Lightweight — no DOM snapshot overhead. \
         'down' scrolls toward page bottom, 'up' scrolls toward top."
            .to_string()
    }

    fn category(&self) -> &str {
        "browser"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "direction": {
                    "type": "string",
                    "description": "Scroll direction: 'down' or 'up'."
                },
                "amount": {
                    "type": "number",
                    "description": "Pixels to scroll. Default: one viewport height (window.innerHeight)."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": ["direction"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let direction = input["direction"]
            .as_str()
            .ok_or("missing 'direction' parameter")?;
        let amount = input["amount"].as_f64();

        let sign: f64 = match direction {
            "down" => 1.0,
            "up" => -1.0,
            _ => return Err("direction must be 'up' or 'down'".into()),
        };

        let pixels = amount.unwrap_or(0.0);
        let scroll_expr = if pixels > 0.0 {
            format!(
                "window.scrollBy({{top: {}, behavior:'smooth'}}); return window.scrollY;",
                sign * pixels
            )
        } else {
            format!(
                "window.scrollBy({{top: window.innerHeight * {}, behavior:'smooth'}}); return window.scrollY;",
                sign
            )
        };

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

        match do_scroll(&target.web_socket_debugger_url, &scroll_expr).await {
            Ok(scroll_y) => Ok(serde_json::json!({
                "direction": direction,
                "scrollY": scroll_y,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "direction": direction,
                "scrollY": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

async fn do_scroll(ws_url: &str, expr: &str) -> Result<f64, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;
    browser_cdp::cdp_send(&mut ws, 0, "Runtime.enable", serde_json::json!({})).await?;

    let result = browser_cdp::cdp_evaluate(&mut ws, expr).await?;
    result
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("Unexpected scrollY value: {result}"))
}

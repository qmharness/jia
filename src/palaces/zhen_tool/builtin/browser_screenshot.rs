// ── browser_screenshot — Capture a page screenshot via Page.captureScreenshot ──
//
// Returns a base64-encoded PNG image. Useful for vision-based verification.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::stems::action::ExecContext;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserScreenshotTool ─────────────────────────────────────

pub struct BrowserScreenshotTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl BrowserScreenshotTool {
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
impl BaseTool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "browser_screenshot"
    }

    fn description(&self) -> String {
        "Take a screenshot of the current browser page. \
         Returns a base64-encoded PNG image that the LLM can analyze with vision. \
         Use this to visually verify page state after interactions."
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

        match capture(&target.web_socket_debugger_url).await {
            Ok(data) => Ok(serde_json::json!({
                "screenshot": data,
                "format": "png",
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "screenshot": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

async fn capture(ws_url: &str) -> Result<String, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable Page domain
    browser_cdp::cdp_send(&mut ws, 0, "Page.enable", serde_json::json!({})).await?;

    // Capture screenshot with moderate quality
    let result = browser_cdp::cdp_send(
        &mut ws,
        1,
        "Page.captureScreenshot",
        serde_json::json!({
            "format": "png",
            "captureBeyondViewport": false,
        }),
    )
    .await?;

    let data = result
        .get("data")
        .and_then(|v| v.as_str())
        .ok_or("No screenshot data returned")?
        .to_string();

    // Prefix for vision-model consumption
    Ok(format!("data:image/png;base64,{data}"))
}

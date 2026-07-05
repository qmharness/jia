// ── browser_navigate — Navigate browser to a URL via CDP Page.navigate ──

use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserNavigateTool ───────────────────────────────────────

pub struct BrowserNavigateTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserNavigateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserNavigateTool {
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
impl BaseTool for BrowserNavigateTool {
    fn name(&self) -> &str {
        "browser_navigate"
    }

    fn description(&self) -> String {
        "Navigate the live browser to a URL. Returns the page title, \
         final URL (after redirects), and a simplified DOM snapshot. \
         The browser must be running with --remote-debugging-port=9222."
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
                "url": {
                    "type": "string",
                    "description": "The URL to navigate to (e.g. https://example.com)."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let url = input["url"].as_str().ok_or("missing 'url' parameter")?;
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

        match navigate_and_snapshot(&target.web_socket_debugger_url, url).await {
            Ok(r) => Ok(serde_json::json!({
                "url": r.url,
                "title": r.title,
                "domSnapshot": r.dom_snapshot,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "url": null,
                "title": null,
                "domSnapshot": null,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

struct NavigateResult {
    url: String,
    title: String,
    dom_snapshot: String,
}

async fn navigate_and_snapshot(ws_url: &str, url: &str) -> Result<NavigateResult, String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable Page domain
    browser_cdp::cdp_send(&mut ws, 0, "Page.enable", serde_json::json!({})).await?;

    // Navigate
    browser_cdp::cdp_send(
        &mut ws,
        1,
        "Page.navigate",
        serde_json::json!({ "url": url }),
    )
    .await?;

    // Wait for page to settle
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Enable Runtime and get final URL + title + snapshot
    browser_cdp::cdp_send(&mut ws, 2, "Runtime.enable", serde_json::json!({})).await?;

    let final_url = browser_cdp::cdp_evaluate(&mut ws, "window.location.href")
        .await
        .unwrap_or_else(|_| url.to_string());
    let title = browser_cdp::cdp_evaluate(&mut ws, "document.title").await?;
    let dom_snapshot =
        browser_cdp::cdp_evaluate(&mut ws, &format!("({})()", include_str!("opt_html.js"))).await?;

    Ok(NavigateResult {
        url: final_url,
        title,
        dom_snapshot,
    })
}

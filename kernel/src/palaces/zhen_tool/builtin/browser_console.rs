// ── browser_console — Read browser console logs and JS errors ──
//
// Enables Runtime domain, briefly collects console and exception events,
// then returns them. Useful for debugging JS errors after interactions.

use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::CeremoniesIntent;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserConsoleTool ────────────────────────────────────────

pub struct BrowserConsoleTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserConsoleTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserConsoleTool {
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
impl BaseTool for BrowserConsoleTool {
    fn name(&self) -> &str {
        "browser_console"
    }

    fn description(&self) -> String {
        "Read recent browser console logs and JavaScript errors. \
         Useful for debugging after interactions. \
         Returns up to 50 most recent console entries and exceptions."
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
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
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

        match collect_console(&target.web_socket_debugger_url).await {
            Ok((entries, errors)) => Ok(serde_json::json!({
                "consoleEntries": entries,
                "jsErrors": errors,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "consoleEntries": [],
                "jsErrors": [],
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

async fn collect_console(ws_url: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable Runtime to receive console events + exceptions
    browser_cdp::cdp_send(&mut ws, 0, "Runtime.enable", serde_json::json!({})).await?;

    // Also enable Log domain
    browser_cdp::cdp_send(&mut ws, 1, "Log.enable", serde_json::json!({})).await?;

    // Collect events for a short window
    let mut entries: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(800);

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let v: Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                // consoleAPICalled event
                if v.get("method").and_then(|m| m.as_str()) == Some("Runtime.consoleAPICalled")
                    && let Some(args) = v
                        .get("params")
                        .and_then(|p| p.get("args"))
                        .and_then(|a| a.as_array())
                {
                    let msg: Vec<String> = args
                        .iter()
                        .filter_map(|a| {
                            a.get("value").map(|val| {
                                if val.is_string() {
                                    val.as_str().unwrap().to_string()
                                } else {
                                    val.to_string()
                                }
                            })
                        })
                        .collect();
                    if !msg.is_empty() {
                        let level = v
                            .get("params")
                            .and_then(|p| p.get("type"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("log");
                        entries.push(format!("[{level}] {}", msg.join(" ")));
                    }
                }

                // exceptionThrown event
                if v.get("method").and_then(|m| m.as_str()) == Some("Runtime.exceptionThrown") {
                    let exc = &v["params"]["exceptionDetails"];
                    let text = exc
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    let url = exc.get("url").and_then(|u| u.as_str()).unwrap_or("");
                    let line = exc.get("lineNumber").and_then(|l| l.as_u64()).unwrap_or(0);
                    errors.push(format!("{text} ({url}:{line})"));
                }

                // Log.entryAdded event
                if v.get("method").and_then(|m| m.as_str()) == Some("Log.entryAdded")
                    && let Some(entry) = v.get("params").and_then(|p| p.get("entry"))
                {
                    let source = entry.get("source").and_then(|s| s.as_str()).unwrap_or("");
                    let level = entry
                        .get("level")
                        .and_then(|l| l.as_str())
                        .unwrap_or("info");
                    let text = entry.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    if !text.is_empty() {
                        entries.push(format!("[{level}][{source}] {text}"));
                    }
                }
            }
            Ok(Some(Ok(_))) => continue, // Binary/Ping/Pong/Frame/Close — ignore
            Ok(Some(Err(_))) => break,
            Ok(None) => break,
            Err(_) => break,
        }
    }

    // Trim to most recent
    entries.truncate(50);
    errors.truncate(50);

    Ok((entries, errors))
}

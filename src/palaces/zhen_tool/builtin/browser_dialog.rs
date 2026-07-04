// ── browser_dialog — Handle native JavaScript dialogs ──
//
// Accepts or dismisses alert(), confirm(), prompt() dialogs via CDP.
// If prompt_text is provided, fills the prompt input before accepting.

use std::time::Duration;
use crate::error::ToolError;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::browser_cdp;

// ── BrowserDialogTool ─────────────────────────────────────────

pub struct BrowserDialogTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for BrowserDialogTool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserDialogTool {
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
impl BaseTool for BrowserDialogTool {
    fn name(&self) -> &str {
        "browser_dialog"
    }

    fn description(&self) -> String {
        "Accept or dismiss a native JavaScript dialog (alert, confirm, prompt). \
         Use action='accept' to click OK, action='dismiss' to click Cancel. \
         For prompt() dialogs, provide prompt_text to fill in the input."
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
                "action": {
                    "type": "string",
                    "description": "'accept' to click OK, 'dismiss' to click Cancel."
                },
                "prompt_text": {
                    "type": "string",
                    "description": "Text to enter into a prompt() dialog before accepting."
                },
                "tab_id": {
                    "type": "string",
                    "description": "Target tab ID. Uses the first available tab if omitted."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let action = input["action"]
            .as_str()
            .ok_or("missing 'action' parameter")?;
        let prompt_text = input["prompt_text"].as_str().unwrap_or("");

        let accept = match action {
            "accept" => true,
            "dismiss" => false,
            _ => return Err("action must be 'accept' or 'dismiss'".into()),
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

        match handle_dialog(&target.web_socket_debugger_url, accept, prompt_text).await {
            Ok(()) => Ok(serde_json::json!({
                "action": if accept { "accepted" } else { "dismissed" },
                "promptText": prompt_text,
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": null,
            })
            .to_string()),
            Err(e) => Ok(serde_json::json!({
                "action": if accept { "accepted" } else { "dismissed" },
                "tabs": browser_cdp::tabs_json(&tabs),
                "error": e,
            })
            .to_string()),
        }
    }
}

async fn handle_dialog(ws_url: &str, accept: bool, prompt_text: &str) -> Result<(), String> {
    let mut ws = browser_cdp::connect_cdp(ws_url).await?;

    // Enable Page domain (needed to handle dialogs)
    browser_cdp::cdp_send(&mut ws, 0, "Page.enable", serde_json::json!({})).await?;

    let mut params = serde_json::json!({ "accept": accept });
    if accept && !prompt_text.is_empty() {
        params["promptText"] = Value::String(prompt_text.to_string());
    }

    browser_cdp::cdp_send(&mut ws, 1, "Page.handleJavaScriptDialog", params).await?;

    Ok(())
}

// ── computer_use — Desktop control via native OS APIs ──
//
// Single tool with action discriminator for macOS desktop automation.
// Uses CGEventPostToPid for input events, AX API for accessibility tree,
// and CGWindowListCreateImage for screenshots.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::intent::CeremoniesIntent;
use crate::stems::intent::CommunicateAction;

use crate::palaces::zhen_tool::computer_driver::check_security;
use crate::palaces::zhen_tool::computer_driver::schema::{self, ComputerAction, ComputerUseInput};

pub struct ComputerUseTool {
    #[allow(dead_code)]
    permissions: Arc<PermissionMatrix>,
}

impl ComputerUseTool {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self { permissions }
    }
}

#[async_trait]
impl BaseTool for ComputerUseTool {
    fn name(&self) -> &str {
        "computer_use"
    }

    fn description(&self) -> String {
        "Control macOS desktop apps. Actions: capture (screenshot+AX tree with SOM), \
         click (by element or coordinate), type (keyboard text input), key (key combo), \
         scroll, wait, list_apps. Requires Accessibility permission in System Settings."
            .to_string()
    }

    fn category(&self) -> &str {
        "desktop"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren(CommunicateAction {
            endpoint: String::new(),
            payload: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false // Desktop operations must be sequential
    }

    fn parameters_schema(&self) -> Value {
        schema::parameters_schema()
    }

    async fn execute(&self, input: Value) -> Result<String, String> {
        let parsed: ComputerUseInput =
            serde_json::from_value(input).map_err(|e| format!("Invalid input: {e}"))?;

        parsed.validate()?;
        check_security(&parsed)?;

        #[cfg(target_os = "macos")]
        {
            execute_macos(parsed).await
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("computer_use is only supported on macOS".into())
        }
    }
}

#[cfg(target_os = "macos")]
async fn execute_macos(input: ComputerUseInput) -> Result<String, String> {
    use crate::palaces::zhen_tool::computer_driver::MacOsBackend;

    let backend = MacOsBackend;

    let result = match input.action {
        ComputerAction::Capture => {
            let mode = input.mode.clone().unwrap_or_else(|| "som".into());
            let app = input.app.clone();
            let mode_for_json = mode.clone();
            let capture =
                tokio::task::spawn_blocking(move || backend.capture(&mode, app.as_deref()))
                    .await
                    .map_err(|e| format!("join: {e}"))??;

            serde_json::json!({
                "action": "capture",
                "mode": mode_for_json,
                "screenshotBase64": capture.screenshot_b64,
                "axTree": capture.ax_tree,
                "elements": capture.elements,
                "app": capture.app,
                "screen": capture.screen,
                "error": null,
            })
            .to_string()
        }

        ComputerAction::Click => {
            let input_clone = input.clone();
            let result = tokio::task::spawn_blocking(move || backend.click_action(&input_clone))
                .await
                .map_err(|e| format!("join: {e}"))??;

            serde_json::json!({
                "action": "click",
                "success": result.success,
                "message": result.message,
                "data": result.data,
                "error": null,
            })
            .to_string()
        }

        ComputerAction::Type => {
            let input_clone = input.clone();
            let result = tokio::task::spawn_blocking(move || backend.type_action(&input_clone))
                .await
                .map_err(|e| format!("join: {e}"))??;

            serde_json::json!({
                "action": "type",
                "success": result.success,
                "message": result.message,
                "error": null,
            })
            .to_string()
        }

        ComputerAction::Key => {
            let input_clone = input.clone();
            let result = tokio::task::spawn_blocking(move || backend.key_action(&input_clone))
                .await
                .map_err(|e| format!("join: {e}"))??;

            serde_json::json!({
                "action": "key",
                "keys": input.keys,
                "success": result.success,
                "message": result.message,
                "error": null,
            })
            .to_string()
        }

        ComputerAction::Scroll => {
            let input_clone = input.clone();
            let result = tokio::task::spawn_blocking(move || backend.scroll_action(&input_clone))
                .await
                .map_err(|e| format!("join: {e}"))??;

            serde_json::json!({
                "action": "scroll",
                "success": result.success,
                "message": result.message,
                "error": null,
            })
            .to_string()
        }

        ComputerAction::Wait => {
            let seconds = input.seconds.unwrap_or(1.0).clamp(0.0, 30.0);
            tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
            serde_json::json!({
                "action": "wait",
                "seconds": seconds,
                "message": format!("Waited {seconds:.1}s"),
                "error": null,
            })
            .to_string()
        }

        ComputerAction::ListApps => {
            let result = tokio::task::spawn_blocking(move || backend.list_apps_action())
                .await
                .map_err(|e| format!("join: {e}"))??;

            serde_json::json!({
                "action": "list_apps",
                "success": result.success,
                "message": result.message,
                "apps": result.data,
                "error": null,
            })
            .to_string()
        }
    };

    Ok(result)
}

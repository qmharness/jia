// ── computer/schema.rs — Action enum and JSON Schema for computer_use ──

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerAction {
    /// Screenshot + AX tree. mode: "som" (numbered overlay), "ax" (text tree), "vision" (raw screenshot).
    Capture,
    /// Click on element ref or screen coordinate.
    Click,
    /// Type text character by character.
    Type,
    /// Key combo (e.g. "cmd+s", "cmd+tab").
    Key,
    /// Scroll in direction by amount.
    Scroll,
    /// Wait for N seconds (max 30).
    Wait,
    /// List running applications.
    ListApps,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComputerUseInput {
    pub action: ComputerAction,
    /// App name or bundle ID to target. Uses frontmost app if omitted.
    pub app: Option<String>,
    /// Capture mode: "som", "ax", or "vision". Default "som".
    pub mode: Option<String>,
    /// Element ref number from SOM overlay.
    pub element: Option<u32>,
    /// Click coordinate (normalized 0..1 or absolute pixels).
    pub coordinate: Option<(f64, f64)>,
    /// If true, coordinate is in 0..1 normalized range. Default false.
    pub normalized: Option<bool>,
    /// Mouse button: "left", "right", "center". Default "left".
    pub button: Option<String>,
    /// Text to type.
    pub text: Option<String>,
    /// Key combo string (e.g. "cmd+s", "cmd+shift+tab").
    pub keys: Option<String>,
    /// Scroll direction: "up", "down", "left", "right". Default "down".
    pub direction: Option<String>,
    /// Scroll amount in lines/pixels. Default 3.
    pub amount: Option<u32>,
    /// Wait duration in seconds (max 30).
    pub seconds: Option<f64>,
}

impl ComputerUseInput {
    pub fn validate(&self) -> Result<(), String> {
        match self.action {
            ComputerAction::Capture => {
                if let Some(ref mode) = self.mode
                    && !["som", "ax", "vision"].contains(&mode.as_str())
                {
                    return Err(format!("Invalid mode '{}'. Use som, ax, or vision.", mode));
                }
                Ok(())
            }
            ComputerAction::Click => {
                if self.element.is_none() && self.coordinate.is_none() {
                    return Err("click requires 'element' or 'coordinate'".into());
                }
                if let Some((x, y)) = self.coordinate {
                    if !self.normalized.unwrap_or(false) && (x < 0.0 || y < 0.0) {
                        return Err(format!(
                            "coordinate ({x}, {y}) out of bounds; use positive pixel values or set normalized=true for 0..1 range"
                        ));
                    }
                    if self.normalized.unwrap_or(false)
                        && (!(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y))
                    {
                        return Err(format!(
                            "normalized coordinate ({x}, {y}) out of 0..1 range"
                        ));
                    }
                }
                if let Some(ref btn) = self.button
                    && !["left", "right", "center"].contains(&btn.as_str())
                {
                    return Err(format!(
                        "Invalid button '{}'. Use left, right, or center.",
                        btn
                    ));
                }
                Ok(())
            }
            ComputerAction::Type => {
                match self.text.as_deref() {
                    None => return Err("type requires 'text'".into()),
                    Some("") => return Err("type requires non-empty 'text'".into()),
                    Some(_) => {}
                }
                Ok(())
            }
            ComputerAction::Key => {
                if self.keys.is_none() {
                    return Err("key requires 'keys'".into());
                }
                Ok(())
            }
            ComputerAction::Scroll => Ok(()),
            ComputerAction::Wait => {
                if let Some(s) = self.seconds
                    && (s <= 0.0 || s > 30.0)
                {
                    return Err("wait seconds must be > 0 and <= 30".into());
                }
                Ok(())
            }
            ComputerAction::ListApps => Ok(()),
        }
    }
}

pub fn parameters_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "description": "Action: capture, click, type, key, scroll, wait, list_apps."
            },
            "app": {
                "type": "string",
                "description": "Target app name or bundle ID. Uses frontmost app if omitted."
            },
            "mode": {
                "type": "string",
                "description": "Capture mode: som (numbered overlay), ax (text tree), vision (raw screenshot). Default: som."
            },
            "element": {
                "type": "integer",
                "description": "Element number from SOM overlay to click."
            },
            "coordinate": {
                "type": "array",
                "description": "Click position as [x, y]: absolute pixels, or [0..1, 0..1] when normalized=true.",
                "items": {"type": "number"}
            },
            "normalized": {
                "type": "boolean",
                "description": "If true, coordinate is normalized (0..1 range). Default false."
            },
            "button": {
                "type": "string",
                "description": "Mouse button: left, right, center. Default: left."
            },
            "text": {
                "type": "string",
                "description": "Text to type character by character."
            },
            "keys": {
                "type": "string",
                "description": "Key combo: cmd+s, cmd+shift+tab, ctrl+c, etc."
            },
            "direction": {
                "type": "string",
                "description": "Scroll direction: up, down, left, right. Default: down."
            },
            "amount": {
                "type": "integer",
                "description": "Scroll amount in lines. Default: 3."
            },
            "seconds": {
                "type": "number",
                "description": "Wait duration in seconds (max 30)."
            }
        },
        "required": ["action"]
    })
}

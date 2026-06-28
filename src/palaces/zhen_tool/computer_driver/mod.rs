// ── computer/mod.rs — Platform abstraction for desktop control ──
//
// Dispatches to macOS backend; Linux/Windows backends can be added here.
// All CGEvent / AX API usage is behind #[cfg(target_os = "macos")].

pub mod schema;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::MacOsBackend;

use schema::ComputerUseInput;

// ── Security: hard-blocked key combos ──

/// Canonical blocked combos: modifiers sorted alphabetically, key last.
const BLOCKED_KEY_COMBOS: &[&[&str]] = &[
    &["cmd", "q"],
    &["cmd", "shift", "q"],
    &["cmd", "opt", "q"],
    &["cmd", "ctrl", "q"],
    &["cmd", "opt", "esc"],
];

const BLOCKED_TYPE_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -r ",
    "rm --recursive",
    "sudo ",
    "mkfs.",
    "mkfs ",
    "dd if=",
    "dd of=",
    "> /dev/",
    "> /dev",
    ":() { :|:& };:", // fork bomb (space before `{` required by bash)
];

/// Normalize a key combo string into canonical form for security comparison.
/// Handles modifier aliases (command→cmd, option/alt→opt, control→ctrl),
/// removes whitespace, lowercases, sorts modifiers, keeps key last.
fn canonicalize_keys(keys: &str) -> String {
    let parts: Vec<String> = keys.split('+').map(|s| s.trim().to_lowercase()).collect();
    let mut normalized: Vec<&str> = Vec::with_capacity(parts.len());
    for p in &parts {
        let canonical = match p.as_str() {
            "command" | "cmd" => "cmd",
            "shift" => "shift",
            "option" | "alt" | "opt" => "opt",
            "control" | "ctrl" => "ctrl",
            other => other,
        };
        normalized.push(canonical);
    }
    if normalized.len() > 1 {
        let key = normalized.pop().unwrap();
        normalized.sort_unstable();
        normalized.push(key);
    }
    normalized.join("+")
}

pub fn check_security(input: &ComputerUseInput) -> Result<(), String> {
    match input.action {
        schema::ComputerAction::Key => {
            if let Some(ref keys) = input.keys {
                let canonical = canonicalize_keys(keys);
                for blocked in BLOCKED_KEY_COMBOS {
                    if canonical == blocked.join("+") {
                        return Err(format!("Blocked key combo: '{}'", keys));
                    }
                }
            }
        }
        schema::ComputerAction::Type => {
            if let Some(ref text) = input.text {
                // Collapse all whitespace (tabs, multiple spaces) to single spaces
                // before checking patterns, preventing whitespace-based bypass.
                let collapsed = text
                    .to_lowercase()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                for pattern in BLOCKED_TYPE_PATTERNS {
                    if collapsed.contains(pattern) {
                        return Err(format!("Blocked type pattern: '{}'", pattern));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

// ── Common types ──

#[derive(Debug, Clone, serde::Serialize)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub is_frontmost: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureResult {
    pub screenshot_b64: Option<String>,
    pub ax_tree: Option<String>,
    pub elements: Vec<SomElement>,
    pub app: AppInfo,
    pub screen: ScreenInfo,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SomElement {
    pub id: u32,
    pub role: String,
    pub label: Option<String>,
    pub value: Option<String>,
    pub bounds: (f64, f64, f64, f64), // (x, y, width, height)
    pub enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScreenInfo {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActionResult {
    pub success: bool,
    pub action: String,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

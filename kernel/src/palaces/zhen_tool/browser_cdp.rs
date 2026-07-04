// ── browser_cdp — Shared CDP client for browser tools ──
//
// Provides tab discovery, WebSocket connection, and CDP command dispatch.
// Used by web_execute_js, browser_navigate, and browser_snapshot.

use std::time::Duration;

use futures::SinkExt;
use futures::StreamExt;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub const CDP_PORT: u16 = 9222;
pub const TIMEOUT_SECS: u64 = 15;

// ── Types ─────────────────────────────────────────────────────

pub type CdpWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, serde::Deserialize)]
pub struct CdpTab {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: String,
}

#[derive(Debug, Clone)]
pub struct TabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
    pub web_socket_debugger_url: String,
}

impl TabInfo {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "url": self.url,
            "title": self.title
        })
    }
}

pub fn tabs_json(tabs: &[TabInfo]) -> Vec<Value> {
    tabs.iter().map(|t| t.to_json()).collect()
}

pub fn tab_list(tabs: &[TabInfo]) -> String {
    tabs.iter()
        .map(|t| format!("{} ({})", t.id, t.url))
        .collect::<Vec<_>>()
        .join(", ")
}

// ── HTTP: tab discovery ───────────────────────────────────────

pub async fn get_tabs(client: &reqwest::Client) -> Result<Vec<TabInfo>, String> {
    let url = format!("http://localhost:{CDP_PORT}/json");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("CDP endpoint unreachable (http://localhost:{CDP_PORT}): {e}"))?;

    let tabs: Vec<CdpTab> = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse CDP tab list: {e}"))?;

    Ok(tabs
        .into_iter()
        .filter(|t| t.web_socket_debugger_url.starts_with("ws://") && !t.url.is_empty())
        .map(|t| TabInfo {
            id: t.id,
            url: t.url,
            title: t.title,
            web_socket_debugger_url: t.web_socket_debugger_url,
        })
        .collect())
}

// ── CDP WebSocket ─────────────────────────────────────────────

pub async fn connect_cdp(ws_url: &str) -> Result<CdpWs, String> {
    let (ws, _resp) = connect_async(ws_url)
        .await
        .map_err(|e| format!("CDP WebSocket connect failed: {e}"))?;
    Ok(ws)
}

/// Send a CDP command and wait for the matching response.
/// Ignores events (messages without "id") while waiting.
pub async fn cdp_send(
    ws: &mut CdpWs,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let msg = serde_json::json!({
        "id": id,
        "method": method,
        "params": params,
    });
    ws.send(Message::Text(msg.to_string()))
        .await
        .map_err(|e| format!("CDP send error: {e}"))?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(TIMEOUT_SECS);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!("CDP timeout waiting for {method}"));
        }

        let msg = tokio::time::timeout(remaining, ws.next())
            .await
            .map_err(|_| format!("CDP recv timeout waiting for {method}"))?
            .ok_or_else(|| format!("CDP connection closed waiting for {method}"))?
            .map_err(|e| format!("CDP recv error: {e}"))?;

        if let Message::Text(text) = msg {
            let v: Value =
                serde_json::from_str(&text).map_err(|e| format!("CDP parse error: {e}"))?;
            if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                if let Some(err) = v.get("error") {
                    return Err(format!("CDP error for {method}: {err}"));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

/// Convenience: evaluate a JS expression via Runtime.evaluate.
pub async fn cdp_evaluate(ws: &mut CdpWs, expression: &str) -> Result<String, String> {
    let result = cdp_send(
        ws,
        1,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
            "timeout": 10_000,
        }),
    )
    .await?;

    if let Some(val) = result.get("result").and_then(|r| r.get("value")) {
        if val.is_string() {
            Ok(val.as_str().unwrap_or("").to_string())
        } else {
            Ok(val.to_string())
        }
    } else if let Some(exc) = result.get("exceptionDetails") {
        let text = exc
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown exception");
        Err(format!("JS exception: {text}"))
    } else {
        Ok("undefined".into())
    }
}

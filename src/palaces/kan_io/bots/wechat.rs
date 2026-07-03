use std::sync::Arc;
// ── WeChat Bot (iLink long-polling) ──────────────────────────
//
// Connects Jia to personal WeChat accounts via Tencent's iLink Bot API.
// Follows the same pattern as telegram.rs: background tokio task,
// long-poll getupdates, push ChannelInput into ChannelManager.
//
// Phase 1: text messages only. Media (AES-128-ECB CDN) is Phase 2.

use std::collections::HashMap;

use futures::FutureExt;
use serde::Deserialize;
use serde_json::json;

use crate::config::WeChatBotConfig;
use crate::palaces::kan_io::{ChannelInput, ChannelSource};
use crate::types::{Message, Role};

// ── Constants ─────────────────────────────────────────────────

const ILINK_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const ILINK_APP_ID: &str = "bot";
const CHANNEL_VERSION: &str = "2.2.0";
/// iLink-App-ClientVersion = (2 << 16) | (2 << 8) | 0
const ILINK_APP_CLIENT_VERSION: &str = "131328";

const LONG_POLL_TIMEOUT_SECS: u64 = 35;
const API_TIMEOUT_SECS: u64 = 15;
const QR_POLL_SECS: u64 = 1;
const QR_TOTAL_TIMEOUT_SECS: u64 = 480; // 8 minutes

// iLink error codes
const ERRCODE_SESSION_EXPIRED: i64 = -14;
const ERRCODE_RATE_LIMIT: i64 = -2;

// Message item types
const ITEM_TEXT: i64 = 1;

// ── API types (deserialization-only, minimal) ─────────────────

#[derive(Debug, Deserialize)]
struct QrCodeResponse {
    qrcode: Option<String>,
    qrcode_img_content: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct QrStatusResponse {
    status: Option<String>,
    ilink_bot_id: Option<String>,
    bot_token: Option<String>,
    baseurl: Option<String>,
    ilink_user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GetUpdatesResponse {
    #[serde(default)]
    ret: i64,
    #[serde(default)]
    errcode: Option<i64>,
    #[serde(default)]
    errmsg: Option<String>,
    msgs: Option<Vec<WeChatMessage>>,
    get_updates_buf: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WeChatMessage {
    msg_id: Option<String>,
    from_user_id: Option<String>,
    to_user_id: Option<String>,
    room_id: Option<String>,
    msg_type: Option<i64>,
    item_list: Option<Vec<MessageItem>>,
    context_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageItem {
    #[serde(rename = "type")]
    item_type: i64,
    text_item: Option<TextItem>,
}

#[derive(Debug, Deserialize)]
struct TextItem {
    text: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────

fn random_uin() -> String {
    let uid = uuid::Uuid::new_v4();
    let val = u64::from_be_bytes(uid.as_bytes()[..8].try_into().unwrap());
    val.to_string()
}

fn base_info() -> serde_json::Value {
    json!({"channel_version": CHANNEL_VERSION})
}

fn build_headers(token: &str) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderMap, HeaderValue};
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("application/json"));
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|e| format!("invalid Bearer token: {e}"))?,
    );
    headers.insert(
        "AuthorizationType",
        HeaderValue::from_static("ilink_bot_token"),
    );
    headers.insert(
        "X-WECHAT-UIN",
        HeaderValue::from_str(&random_uin()).map_err(|e| format!("invalid UIN header: {e}"))?,
    );
    headers.insert("iLink-App-Id", HeaderValue::from_static(ILINK_APP_ID));
    headers.insert(
        "iLink-App-ClientVersion",
        HeaderValue::from_static(ILINK_APP_CLIENT_VERSION),
    );
    Ok(headers)
}

// ── QR login ──────────────────────────────────────────────────

/// Run the iLink QR login flow.
///
/// Prints a scan prompt to stdout, polls until the user confirms in WeChat
/// (or the 8-minute timeout expires), then persists credentials.
///
/// Returns `(account_id, token, base_url)` on success.
pub async fn qr_login() -> Result<(String, String, String), String> {
    let client = reqwest::Client::new();

    // 1. Fetch QR code
    let qr_url = format!("{ILINK_BASE_URL}/ilink/bot/get_bot_qrcode?bot_type=3");
    let qr_resp: QrCodeResponse = client
        .get(&qr_url)
        .headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("iLink-App-Id", "bot".parse().unwrap());
            h.insert(
                "iLink-App-ClientVersion",
                ILINK_APP_CLIENT_VERSION.parse().unwrap(),
            );
            h
        })
        .send()
        .await
        .map_err(|e| format!("Failed to fetch QR code: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse QR response: {e}"))?;

    let qrcode = qr_resp
        .qrcode
        .ok_or_else(|| "QR response missing qrcode field".to_string())?;
    let qrcode_url = qr_resp.qrcode_img_content.unwrap_or_default();

    if qrcode.is_empty() {
        return Err("QR code value is empty".to_string());
    }

    // 2. Print QR code in terminal
    let scan_data = if !qrcode_url.is_empty() {
        &qrcode_url
    } else {
        &qrcode
    };
    render_terminal_qr(scan_data);
    println!();
    println!("等待扫码中...");

    // 3. Poll scan status
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(QR_TOTAL_TIMEOUT_SECS);
    let mut refresh_count = 0u32;
    let mut current_qrcode = qrcode;

    loop {
        if std::time::Instant::now() > deadline {
            return Err("QR login timed out".to_string());
        }

        let status_url =
            format!("{ILINK_BASE_URL}/ilink/bot/get_qrcode_status?qrcode={current_qrcode}");
        let status_resp: QrStatusResponse = match client
            .get(&status_url)
            .headers({
                let mut h = reqwest::header::HeaderMap::new();
                h.insert("iLink-App-Id", "bot".parse().unwrap());
                h.insert(
                    "iLink-App-ClientVersion",
                    ILINK_APP_CLIENT_VERSION.parse().unwrap(),
                );
                h
            })
            .send()
            .await
        {
            Ok(r) => r.json().await.unwrap_or_else(|_| QrStatusResponse {
                status: Some("wait".into()),
                ilink_bot_id: None,
                bot_token: None,
                baseurl: None,
                ilink_user_id: None,
            }),
            Err(_) => {
                tokio::time::sleep(tokio::time::Duration::from_secs(QR_POLL_SECS)).await;
                continue;
            }
        };

        match status_resp.status.as_deref().unwrap_or("wait") {
            "wait" => {
                print!(".");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            "scaned" => {
                println!();
                println!("已扫码，请在微信中确认...");
            }
            "scaned_but_redirect" => {
                // Server redirected; base URL already changes via the new
                // endpoint returned in the next poll.
            }
            "expired" => {
                refresh_count += 1;
                if refresh_count > 3 {
                    println!();
                    return Err("QR code expired too many times".to_string());
                }
                println!();
                println!("二维码已过期，正在刷新... ({refresh_count}/3)");
                // Re-fetch QR
                let new_qr: QrCodeResponse = client
                    .get(&qr_url)
                    .headers({
                        let mut h = reqwest::header::HeaderMap::new();
                        h.insert("iLink-App-Id", "bot".parse().unwrap());
                        h.insert(
                            "iLink-App-ClientVersion",
                            ILINK_APP_CLIENT_VERSION.parse().unwrap(),
                        );
                        h
                    })
                    .send()
                    .await
                    .map_err(|e| format!("QR refresh failed: {e}"))?
                    .json()
                    .await
                    .map_err(|e| format!("QR refresh parse: {e}"))?;
                current_qrcode = new_qr
                    .qrcode
                    .ok_or_else(|| "QR refresh missing qrcode".to_string())?;
                let new_url = new_qr.qrcode_img_content.unwrap_or_default();
                let new_scan = if !new_url.is_empty() {
                    &new_url
                } else {
                    &current_qrcode
                };
                render_terminal_qr(new_scan);
                println!();
                println!("等待扫码中...");
            }
            "confirmed" => {
                let account_id = status_resp
                    .ilink_bot_id
                    .ok_or_else(|| "Missing ilink_bot_id in confirmed response".to_string())?;
                let token = status_resp
                    .bot_token
                    .ok_or_else(|| "Missing bot_token in confirmed response".to_string())?;
                let base_url = status_resp
                    .baseurl
                    .unwrap_or_else(|| ILINK_BASE_URL.to_string());

                // Persist credentials
                let _ = save_credentials(&account_id, &token, &base_url);

                println!();
                println!("微信连接成功！account_id={account_id}");
                return Ok((account_id, token, base_url));
            }
            _ => {
                // unknown status, keep waiting
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(QR_POLL_SECS)).await;
    }
}

/// Persist WeChat credentials to ~/.jia/wechat/{account_id}.json
fn save_credentials(account_id: &str, token: &str, base_url: &str) -> Result<(), String> {
    let home = dirs_next().unwrap_or_else(|| {
        let fallback = std::path::PathBuf::from(".");
        eprintln!("Could not determine home directory; saving to current dir");
        fallback
    });
    let dir = home.join(".jia").join("wechat");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir: {e}"))?;

    let payload = serde_json::json!({
        "account_id": account_id,
        "token": token,
        "base_url": base_url,
        "saved_at": chrono_now(),
    });

    let path = dir.join(format!("{account_id}.json"));
    std::fs::write(&path, payload.to_string()).map_err(|e| format!("write: {e}"))?;

    // Restrict permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }

    tracing::info!("WeChat credentials saved to {}", path.display());
    Ok(())
}

/// Persist sync_buf to credentials file so the bot can resume after restart.
fn save_sync_buf(account_id: &str, sync_buf: &str) {
    let home = match dirs_next() {
        Some(h) => h,
        None => return,
    };
    let path = home
        .join(".jia")
        .join("wechat")
        .join(format!("{account_id}.json"));
    // Read existing, update sync_buf, write back
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return,
    };
    parsed["sync_buf"] = serde_json::json!(sync_buf);
    let _ = std::fs::write(&path, parsed.to_string());
}

/// Try to load persisted credentials (including sync_buf).
pub fn load_credentials(account_id: &str) -> Option<(String, String, String)> {
    let home = dirs_next()?;
    let path = home
        .join(".jia")
        .join("wechat")
        .join(format!("{account_id}.json"));

    let raw = std::fs::read_to_string(&path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let token = parsed.get("token")?.as_str()?.to_string();
    let base_url = parsed
        .get("base_url")
        .and_then(|v| v.as_str())
        .unwrap_or(ILINK_BASE_URL)
        .to_string();
    let sync_buf = parsed
        .get("sync_buf")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some((token, base_url, sync_buf))
}

fn chrono_now() -> String {
    // Avoid pulling in chrono — use std only.
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    // RFC 3339-ish
    let days_since_epoch = secs / 86400;
    let remaining = secs % 86400;
    let h = remaining / 3600;
    let m = (remaining % 3600) / 60;
    let s = remaining % 60;
    // Simple format; precise date calculation requires more code.
    format!("unix={secs} ({days_since_epoch}d {h:02}:{m:02}:{s:02})")
}

fn dirs_next() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .ok()
}

/// Render a QR code to the terminal using Unicode block characters.
fn render_terminal_qr(data: &str) {
    use qrcode::{Color, QrCode};
    let code = match QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => {
            println!("(二维码生成失败，请打开链接: {data})");
            return;
        }
    };

    let width = code.width();
    let dark = "██";
    let bright = "  ";

    // Top border
    print!("  ");
    for _ in 0..(width + 2) {
        print!("{dark}");
    }
    println!();

    for y in 0..width {
        print!("  {dark}"); // left border
        for x in 0..width {
            match code[(x, y)] {
                Color::Dark => print!("{dark}"),
                Color::Light => print!("{bright}"),
            }
        }
        println!("{dark}"); // right border
    }

    // Bottom border
    print!("  ");
    for _ in 0..(width + 2) {
        print!("{dark}");
    }
    println!();
}

// ── Adapter ───────────────────────────────────────────────────

struct WeChatAdapter {
    config: WeChatBotConfig,
    client: reqwest::Client,
    sync_buf: String,
    context_tokens: HashMap<String, String>,
    cm: Arc<crate::palaces::kan_io::ChannelManager>,
    consecutive_errors: u32,
    seen_msg_ids: HashMap<String, std::time::Instant>,
}

impl WeChatAdapter {
    fn new(config: WeChatBotConfig, cm: Arc<crate::palaces::kan_io::ChannelManager>) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
            sync_buf: String::new(),
            context_tokens: HashMap::new(),
            cm,
            consecutive_errors: 0,
            seen_msg_ids: HashMap::new(),
        }
    }

    async fn run(mut self) {
        // Try to restore sync_buf from saved credentials (crash recovery)
        if let Some((_, _, saved_buf)) = load_credentials(&self.config.account_id)
            && !saved_buf.is_empty()
        {
            tracing::info!(
                sync_buf_len = saved_buf.len(),
                "WeChat bot restored sync buffer"
            );
            self.sync_buf = saved_buf;
        }
        tracing::info!(
            account_id = %self.config.account_id,
            base_url = %self.config.base_url,
            "WeChat bot starting"
        );

        loop {
            match self.poll_once().await {
                Ok(()) => {
                    if self.consecutive_errors > 0 {
                        tracing::info!(errs = self.consecutive_errors, "WeChat poll recovered");
                        self.consecutive_errors = 0;
                    }
                }
                Err(PollError::Timeout) => {
                    // Normal — long-poll expired, retry immediately.
                    self.consecutive_errors = 0;
                    continue;
                }
                Err(PollError::SessionExpired) => {
                    tracing::warn!(
                        "WeChat session expired, clearing context tokens. Cooling down 10 min."
                    );
                    self.context_tokens.clear();
                    self.sync_buf.clear();
                    tokio::time::sleep(tokio::time::Duration::from_secs(600)).await;
                }
                Err(kind) => {
                    self.consecutive_errors += 1;
                    let delay = backoff_delay(self.consecutive_errors);
                    let label = match &kind {
                        PollError::RateLimited => "rate-limited",
                        PollError::Http(_) => "http",
                        PollError::Parse(_) => "parse",
                        _ => "unknown",
                    };
                    tracing::warn!(
                        errs = self.consecutive_errors,
                        delay_ms = delay.as_millis(),
                        error = %kind,
                        "WeChat poll {label}, backing off"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    async fn poll_once(&mut self) -> Result<(), PollError> {
        let body = json!({
            "base_info": base_info(),
            "get_updates_buf": self.sync_buf,
        });
        let body_str = body.to_string();
        let url = format!("{}/ilink/bot/getupdates", self.config.base_url);

        let resp = self
            .client
            .post(&url)
            .headers(build_headers(&self.config.token).map_err(PollError::Http)?)
            .body(body_str)
            .timeout(std::time::Duration::from_secs(LONG_POLL_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    PollError::Timeout
                } else {
                    PollError::Http(e.to_string())
                }
            })?;

        let data: GetUpdatesResponse = resp
            .json()
            .await
            .map_err(|e| PollError::Parse(format!("getupdates json: {e}")))?;

        // Check for error codes
        if let Some(ec) = data.errcode {
            if ec == ERRCODE_SESSION_EXPIRED || data.ret == -1 {
                return Err(PollError::SessionExpired);
            }
            if ec == ERRCODE_RATE_LIMIT {
                return Err(PollError::RateLimited);
            }
        }

        // Update sync buffer and persist for crash recovery
        if let Some(buf) = data.get_updates_buf {
            self.sync_buf = buf;
            save_sync_buf(&self.config.account_id, &self.sync_buf);
        }

        // Process messages
        if let Some(msgs) = data.msgs {
            for msg in msgs {
                self.handle_message(msg).await;
            }
        }

        Ok(())
    }

    async fn handle_message(&mut self, msg: WeChatMessage) {
        // Deduplicate by msg_id — iLink delivers at-least-once.
        // Entries older than 300 s are pruned on each incoming message.
        if let Some(ref mid) = msg.msg_id
            && !mid.is_empty()
        {
            let now = std::time::Instant::now();
            self.seen_msg_ids
                .retain(|_, ts| now.duration_since(*ts).as_secs() < 300);
            if self.seen_msg_ids.contains_key(mid) {
                tracing::debug!(msg_id = %mid, "WeChat duplicate message skipped");
                return;
            }
            self.seen_msg_ids.insert(mid.clone(), now);
        }

        let from_user = match &msg.from_user_id {
            Some(u) if !u.is_empty() => u.clone(),
            _ => return,
        };

        // Determine chat type (dm vs group)
        let is_group = msg.room_id.as_ref().is_some_and(|r| !r.is_empty());
        let chat_type = if is_group { "group" } else { "dm" };

        // Policy check
        if is_group {
            if self.config.group_policy.as_str() == "disabled" {
                return;
            }
        } else {
            match self.config.dm_policy.as_str() {
                "disabled" => return,
                "allowlist" => {
                    if !self.is_allowed(&from_user) {
                        tracing::debug!(user = %from_user, "WeChat DM blocked by allowlist");
                        return;
                    }
                }
                _ => {}
            }
        }

        // Store context token
        if let Some(ref ct) = msg.context_token
            && !ct.is_empty()
        {
            self.context_tokens.insert(from_user.clone(), ct.clone());
        }

        // Extract text
        let text = match msg.item_list {
            Some(items) => {
                let parts: Vec<String> = items
                    .iter()
                    .filter(|i| i.item_type == ITEM_TEXT)
                    .filter_map(|i| i.text_item.as_ref()?.text.as_deref())
                    .map(|s| s.to_string())
                    .collect();
                if parts.is_empty() {
                    return;
                }
                parts.join("")
            }
            None => return,
        };

        if text.trim().is_empty() {
            return;
        }

        tracing::info!(
            user = %from_user,
            chat_type = %chat_type,
            text = %text,
            "WeChat message received"
        );

        // Create reply channel — the IO consumer will send the Agent's
        // response back through this, and the dispatcher posts it to iLink.
        let (reply_tx, mut reply_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::palaces::kan_io::OutboundReply>();
        let client = self.client.clone();
        let base_url = self.config.base_url.clone();
        let token = self.config.token.clone();
        let to_user = from_user.clone();
        let context_token = self.context_tokens.get(&to_user).cloned();

        let typing_client = self.client.clone();
        let typing_base = self.config.base_url.clone();
        let typing_token = self.config.token.clone();
        let typing_user = from_user.clone();
        let to_user_for_log = to_user.clone();
        tokio::spawn(async move {
            let result: Result<(), Box<dyn std::any::Any + Send>> =
                std::panic::AssertUnwindSafe(async {
                    // Fire typing indicator so the user sees "typing..." in chat
                    send_wechat_typing(&typing_client, &typing_base, &typing_token, &typing_user)
                        .await;
                    while let Some(reply) = reply_rx.recv().await {
                        match send_wechat_message(
                            &client,
                            &base_url,
                            &token,
                            &to_user,
                            &reply.text,
                            context_token.as_deref(),
                        )
                        .await
                        {
                            Ok(()) => tracing::info!(user = %to_user, "WeChat reply sent"),
                            Err(e) => {
                                tracing::warn!(user = %to_user, error = %e, "WeChat reply failed")
                            }
                        }
                    }
                })
                .catch_unwind()
                .await;

            if let Err(panic_err) = result {
                let payload = panic_err
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic_err.downcast_ref::<String>().map(|s| s.as_str()))
                    .unwrap_or("<unknown panic payload>");
                tracing::error!(
                    user = %to_user_for_log,
                    panic.payload = %payload,
                    "WeChat reply dispatcher panicked"
                );
            }
        });

        let input = ChannelInput {
            messages: vec![Message::text(Role::User, text)],
            source: ChannelSource::Webhook {
                endpoint: format!("wechat:{from_user}"),
            },
            reply_tx: Some(reply_tx),
        };
        self.cm.push(input);
    }

    fn is_allowed(&self, user_id: &str) -> bool {
        if self.config.allowed_users.is_empty() {
            return false;
        }
        self.config
            .allowed_users
            .split(',')
            .any(|u| u.trim().eq_ignore_ascii_case(user_id))
    }
}

// ── send_typing (free function) ──────────────────────────────────

/// Fetch a typing ticket from iLink and fire a "typing" indicator to a user.
async fn send_wechat_typing(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    to_user_id: &str,
) {
    // 1. Get typing ticket
    let cfg_url = format!("{base_url}/ilink/bot/getconfig");
    let config_resp: Result<serde_json::Value, _> = async {
        let resp = client
            .post(&cfg_url)
            .headers(build_headers(token)?)
            .body("{}")
            .timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| format!("getconfig HTTP: {e}"))?;
        resp.json()
            .await
            .map_err(|e| format!("getconfig json: {e}"))
    }
    .await;

    let ticket = match config_resp {
        Ok(ref v) => v
            .get("typing_ticket")
            .and_then(|t| t.as_str())
            .unwrap_or(""),
        Err(e) => {
            tracing::debug!("WeChat getconfig failed: {e}");
            return;
        }
    };

    if ticket.is_empty() {
        return;
    }

    // 2. Send typing indicator
    let body = json!({
        "base_info": base_info(),
        "touser": to_user_id,
        "typing_ticket": ticket,
    });
    let body_str = body.to_string();
    let typing_url = format!("{base_url}/ilink/bot/sendtyping");

    let result: Result<(), String> = async {
        let resp = client
            .post(&typing_url)
            .headers(build_headers(token)?)
            .body(body_str)
            .timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
            .send()
            .await
            .map_err(|e| format!("sendtyping HTTP: {e}"))?;
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("sendtyping json: {e}"))?;
        if let Some(ec) = data.get("errcode").and_then(|v| v.as_i64())
            && ec != 0
        {
            return Err(format!("sendtyping errcode {ec}"));
        }
        Ok(())
    }
    .await;

    if let Err(e) = result {
        tracing::debug!(user = %to_user_id, error = %e, "WeChat typing failed");
    }
}

// ── send_message (free function, called by reply dispatcher) ────

async fn send_wechat_message(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    to_user_id: &str,
    text: &str,
    context_token: Option<&str>,
) -> Result<(), String> {
    let mut msg = json!({
        "from_user_id": "",
        "to_user_id": to_user_id,
        "client_id": uuid::Uuid::new_v4().to_string(),
        "message_type": 2,
        "message_state": 2,
        "item_list": [{"type": 1, "text_item": {"text": text}}],
    });
    if let Some(ct) = context_token {
        msg["context_token"] = json!(ct);
    }

    let body = json!({
        "base_info": base_info(),
        "msg": msg,
    });
    let body_str = body.to_string();
    let url = format!("{base_url}/ilink/bot/sendmessage");

    let resp = client
        .post(&url)
        .headers(build_headers(token)?)
        .body(body_str)
        .timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("sendmessage HTTP: {e}"))?;

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("sendmessage json: {e}"))?;

    if let Some(ec) = data.get("errcode").and_then(|v| v.as_i64()) {
        if ec == ERRCODE_SESSION_EXPIRED {
            return Err("session expired".to_string());
        }
        if ec != 0 {
            let err = data
                .get("errmsg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(format!("iLink error {ec}: {err}"));
        }
    }
    Ok(())
}

// ── Spawn ─────────────────────────────────────────────────────

/// Spawn a WeChat bot that long-polls iLink `getupdates` and pushes
/// incoming text messages into the `ChannelManager`.
///
/// Follows the same signature as [`super::telegram::spawn_telegram_bot`].
///
/// If the bot's main loop panics, it is automatically restarted with
/// exponential backoff (up to 10 retries). After 10 consecutive panics,
/// the bot gives up permanently.
pub fn spawn_wechat_bot(
    config: WeChatBotConfig,
    cm: Arc<crate::palaces::kan_io::ChannelManager>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut restart_count = 0u32;
        const MAX_RESTARTS: u32 = 10;

        loop {
            let adapter = WeChatAdapter::new(config.clone(), cm.clone());
            let result: Result<(), Box<dyn std::any::Any + Send>> =
                std::panic::AssertUnwindSafe(adapter.run())
                    .catch_unwind()
                    .await;

            match result {
                Ok(()) => {
                    // run() loops forever under normal conditions;
                    // a return means something unexpected happened.
                    tracing::warn!("WeChat bot run() returned unexpectedly, restarting");
                }
                Err(panic_err) => {
                    let payload = panic_err
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic_err.downcast_ref::<String>().map(|s| s.as_str()))
                        .unwrap_or("<unknown panic payload>");
                    tracing::error!(
                        panic.payload = %payload,
                        restart_count,
                        "WeChat bot panicked"
                    );
                }
            }

            restart_count += 1;
            if restart_count > MAX_RESTARTS {
                tracing::error!(
                    restart_count,
                    max_restarts = MAX_RESTARTS,
                    "WeChat bot exceeded max restarts, giving up permanently"
                );
                break;
            }

            let delay = backoff_delay(restart_count);
            tracing::info!(
                restart_count,
                delay_ms = delay.as_millis(),
                "WeChat bot restarting"
            );
            tokio::time::sleep(delay).await;
        }
    })
}

// ── Poll errors ───────────────────────────────────────────────

enum PollError {
    Timeout,
    SessionExpired,
    RateLimited,
    Http(String),
    Parse(String),
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "timeout"),
            Self::SessionExpired => write!(f, "session-expired"),
            Self::RateLimited => write!(f, "rate-limited"),
            Self::Http(m) => write!(f, "http: {m}"),
            Self::Parse(m) => write!(f, "parse: {m}"),
        }
    }
}

/// Exponential backoff: 2^n seconds, capped at 5 minutes.
fn backoff_delay(consecutive_errors: u32) -> std::time::Duration {
    let secs = 1u64 << consecutive_errors.min(10); // max ~17 min, clamped below
    let capped = secs.min(300); // 5 min cap
    std::time::Duration::from_secs(capped)
}

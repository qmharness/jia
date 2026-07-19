use std::sync::Arc;
// ── Telegram Bot (long-polling mode) ────────────────────────

use serde::Deserialize;

use kernel::palaces::kun_config::TelegramBotConfig;
use kernel::palaces::kan_io::{ChannelInput, ChannelSource};
use kernel::types::{Message, Role};

/// Raw Telegram API types (minimal, only what we need)
#[derive(Debug, Deserialize)]
struct TgResponse {
    ok: bool,
    result: Vec<TgUpdate>,
}

#[derive(Debug, Deserialize)]
struct TgUpdate {
    update_id: u64,
    message: Option<TgMessage>,
}

#[derive(Debug, Deserialize)]
struct TgMessage {
    #[allow(dead_code)]
    message_id: u64,
    chat: TgChat,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
}

/// Spawn a Telegram bot that polls `getUpdates` and pushes
/// incoming text messages into the `ChannelManager`.
///
/// Panic policy: the workspace is built with `panic = "abort"` in release
/// mode, so any panic in the bot task aborts the whole process. There is no
/// in-process catch/restart; recovery is the responsibility of the external
/// supervisor (launchd/systemd/etc.).
pub fn spawn_telegram_bot(
    config: TelegramBotConfig,
    cm: Arc<kernel::palaces::kan_io::ChannelManager>,
) -> tokio::task::JoinHandle<()> {
    let client = reqwest::Client::new();
    let token = config.token.clone();
    let base = format!("https://api.telegram.org/bot{token}");
    let allowed_chat_ids = config.allowed_chat_ids;

    tokio::spawn(async move {
        run_telegram_loop(client, token, base, cm, allowed_chat_ids).await;
    })
}

/// Main Telegram long-poll loop. Runs until the process aborts on panic.
async fn run_telegram_loop(
    client: reqwest::Client,
    token: String,
    base: String,
    cm: Arc<kernel::palaces::kan_io::ChannelManager>,
    allowed_chat_ids: Vec<String>,
) {
    let mut last_update_id: u64 = load_last_update_id(&token).unwrap_or(0);
    if last_update_id > 0 {
        tracing::info!(
            last_update_id,
            "Telegram bot restored persisted update offset"
        );
    }
    // update_id 去重窗口(300 s,与 wechat seen_msg_ids 同模式):
    // 挡同一进程内 offset 确认前的重投;窗口在内存中,进程重启后为空,
    // 重启重放的批次不挡(at-least-once,与 wechat 侧同一取舍)。
    let mut seen_update_ids = crate::dedup::DedupWindow::new(std::time::Duration::from_secs(300));
    let mut consecutive_errors: u32 = 0;

    loop {
        let url = format!("{base}/getUpdates?timeout=30&offset={}", last_update_id + 1);
        let resp = match client
            .get(&url)
            .timeout(std::time::Duration::from_secs(45))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                consecutive_errors += 1;
                let delay =
                    std::time::Duration::from_secs((1u64 << consecutive_errors.min(6)).min(120));
                tracing::warn!(
                    errs = consecutive_errors,
                    delay_ms = delay.as_millis(),
                    "Telegram poll error: {e}"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
        };
        let updates: TgResponse = match resp.json().await {
            Ok(r) => r,
            Err(e) => {
                consecutive_errors += 1;
                tracing::warn!(errs = consecutive_errors, "Telegram parse error: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        consecutive_errors = 0;
        if !updates.ok {
            continue;
        }
        for update in &updates.result {
            last_update_id = last_update_id.max(update.update_id);
            // 去重:offset 确认前(同一进程内)的重投在这里挡掉。注意去重表
            // 在内存中——进程重启后为空,重启重放的批次不会被挡住
            // (at-least-once,与 wechat 侧同一取舍)。
            if seen_update_ids.is_duplicate(update.update_id, std::time::Instant::now()) {
                tracing::debug!(
                    update_id = update.update_id,
                    "Telegram duplicate update skipped"
                );
                continue;
            }
            if let Some(msg) = &update.message
                && let Some(text) = &msg.text
            {
                if text.is_empty() {
                    continue;
                }
                // Trust gate: if allowed_chat_ids is configured, only respond
                // to those chats. Empty = no one can interact (fail-closed).
                let chat_id_str = msg.chat.id.to_string();
                if allowed_chat_ids.is_empty()
                    || !allowed_chat_ids.iter().any(|id| id == &chat_id_str)
                {
                    tracing::warn!(
                        chat_id = msg.chat.id,
                        "Telegram message rejected: chat_id not in allowlist"
                    );
                    continue;
                }
                tracing::info!(
                    chat_id = msg.chat.id,
                    text = %text,
                    "Telegram message received"
                );

                // Create reply channel so Agent responses flow back to Telegram
                let (reply_tx, mut reply_rx) = tokio::sync::mpsc::unbounded_channel::<
                    kernel::palaces::kan_io::OutboundReply,
                >();
                let reply_client = client.clone();
                let reply_token = token.clone();
                let reply_base = base.clone();
                let chat_id = msg.chat.id;
                tokio::spawn(async move {
                    while let Some(reply) = reply_rx.recv().await {
                        match send_telegram_message(
                            &reply_client,
                            &reply_token,
                            &reply_base,
                            chat_id,
                            &reply.text,
                        )
                        .await
                        {
                            Ok(()) => tracing::info!(chat_id, "Telegram reply sent"),
                            Err(e) => {
                                tracing::warn!(chat_id, error = %e, "Telegram reply failed")
                            }
                        }
                    }
                });

                let input = ChannelInput {
                    messages: vec![Message::text(Role::User, text.clone())],
                    source: ChannelSource::Webhook {
                        endpoint: format!("telegram:{}", msg.chat.id),
                    },
                    reply_tx: Some(reply_tx),
                };
                cm.push(input);
            }
        }

        // 整批处理完毕后持久化 offset(P1-6,审计 W2)——与 wechat 同一取舍:
        // 崩溃时磁盘保留旧 offset,重启后重投该批(at-least-once),
        // 重投由 seen_update_ids 去重窗口兜底;宁重复不丢失。
        if !updates.result.is_empty() {
            save_last_update_id(&token, last_update_id);
        }
    }
}

// ── Offset 持久化(~/.jia/telegram/{bot_id}.json) ────────────────

/// 状态文件名取 bot token 冒号前的数字 bot_id,绝不把 secret 写进路径。
fn bot_state_id(token: &str) -> String {
    // 无冒号的畸形 token 整串即 secret——不得净化后用作文件名,回退 default。
    let Some((id, _)) = token.split_once(':') else {
        return "default".to_string();
    };
    let sanitized: String = id.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

fn state_path(token: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .ok()?;
    Some(
        home.join(".jia")
            .join("telegram")
            .join(format!("{}.json", bot_state_id(token))),
    )
}

fn save_offset_to(path: &std::path::Path, last_update_id: u64) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let payload = serde_json::json!({ "last_update_id": last_update_id });
    if std::fs::write(path, payload.to_string()).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

fn load_offset_from(path: &std::path::Path) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed.get("last_update_id")?.as_u64()
}

fn save_last_update_id(token: &str, last_update_id: u64) {
    if let Some(path) = state_path(token) {
        save_offset_to(&path, last_update_id);
    }
}

fn load_last_update_id(token: &str) -> Option<u64> {
    state_path(token).and_then(|p| load_offset_from(&p))
}

// ── send_message (free function, called by reply dispatcher) ────

async fn send_telegram_message(
    client: &reqwest::Client,
    token: &str,
    _base: &str,
    chat_id: i64,
    text: &str,
) -> Result<(), String> {
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Telegram sendMessage HTTP: {e}"))?;
    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Telegram sendMessage json: {e}"))?;
    if data.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let desc = data
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        return Err(format!("Telegram API error: {desc}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("jia-tg-test-{}-{tag}.json", uuid::Uuid::new_v4()))
    }

    #[test]
    fn bot_state_id_uses_numeric_prefix() {
        assert_eq!(bot_state_id("123456:ABC-secret"), "123456");
        assert_eq!(bot_state_id("no-colon"), "default"); // 无冒号=畸形,不落 secret
        assert_eq!(bot_state_id(":::"), "default");
        assert_eq!(bot_state_id(""), "default");
    }

    #[test]
    fn offset_roundtrip() {
        let path = temp_state_path("roundtrip");
        save_offset_to(&path, 987_654);
        assert_eq!(load_offset_from(&path), Some(987_654));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let path = temp_state_path("missing");
        assert_eq!(load_offset_from(&path), None);
    }

    #[test]
    fn load_garbage_returns_none() {
        let path = temp_state_path("garbage");
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load_offset_from(&path), None);
        std::fs::write(&path, r#"{"other": 1}"#).unwrap();
        assert_eq!(load_offset_from(&path), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn offset_overwrite_advances() {
        let path = temp_state_path("advance");
        save_offset_to(&path, 10);
        save_offset_to(&path, 20);
        assert_eq!(load_offset_from(&path), Some(20));
        let _ = std::fs::remove_file(&path);
    }
}

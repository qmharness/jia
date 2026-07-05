use std::sync::Arc;
// ── Telegram Bot (long-polling mode) ────────────────────────

use futures::FutureExt;
use serde::Deserialize;

use kernel::config::TelegramBotConfig;
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
/// If the bot's main loop panics, it is automatically restarted with
/// exponential backoff (up to 10 retries). After 10 consecutive panics,
/// the bot gives up permanently.
pub fn spawn_telegram_bot(
    config: TelegramBotConfig,
    cm: Arc<kernel::palaces::kan_io::ChannelManager>,
) -> tokio::task::JoinHandle<()> {
    let client = reqwest::Client::new();
    let token = config.token.clone();
    let base = format!("https://api.telegram.org/bot{token}");

    tokio::spawn(async move {
        let allowed_chat_ids = config.allowed_chat_ids;
        let mut restart_count = 0u32;
        const MAX_RESTARTS: u32 = 10;

        loop {
            let token = token.clone();
            let base = base.clone();
            let client = client.clone();
            let cm = cm.clone();
            let allowed = allowed_chat_ids.clone();

            let result: Result<(), Box<dyn std::any::Any + Send>> =
                std::panic::AssertUnwindSafe(run_telegram_loop(client, token, base, cm, allowed))
                    .catch_unwind()
                    .await;

            match result {
                Ok(()) => {
                    tracing::warn!("Telegram bot loop returned unexpectedly, restarting");
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
                        "Telegram bot panicked"
                    );
                }
            }

            restart_count += 1;
            if restart_count > MAX_RESTARTS {
                tracing::error!(
                    restart_count,
                    max_restarts = MAX_RESTARTS,
                    "Telegram bot exceeded max restarts, giving up permanently"
                );
                break;
            }

            let delay = std::time::Duration::from_secs((1u64 << restart_count.min(10)).min(300));
            tracing::info!(
                restart_count,
                delay_ms = delay.as_millis(),
                "Telegram bot restarting"
            );
            tokio::time::sleep(delay).await;
        }
    })
}

/// Main Telegram long-poll loop. Extracted so we can catch panics
/// and reconstruct state on restart.
async fn run_telegram_loop(
    client: reqwest::Client,
    token: String,
    base: String,
    cm: Arc<kernel::palaces::kan_io::ChannelManager>,
    allowed_chat_ids: Vec<String>,
) {
    let mut last_update_id: u64 = 0;
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
            if let Some(msg) = &update.message
                && let Some(text) = &msg.text
            {
                if text.is_empty() {
                    continue;
                }
                // Trust gate: if allowed_chat_ids is configured, only respond
                // to those chats. Empty = no one can interact (fail-closed).
                let chat_id_str = msg.chat.id.to_string();
                if !allowed_chat_ids.is_empty()
                    && !allowed_chat_ids.iter().any(|id| id == &chat_id_str)
                {
                    tracing::warn!(chat_id = msg.chat.id, "Telegram message rejected: chat_id not in allowlist");
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
                    let result: Result<(), Box<dyn std::any::Any + Send>> =
                        std::panic::AssertUnwindSafe(async {
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
                            chat_id,
                            panic.payload = %payload,
                            "Telegram reply dispatcher panicked"
                        );
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
    }
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

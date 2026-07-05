//! WeChat bot adapter and spawn entry point.

use std::collections::HashMap;
use std::sync::Arc;

use futures::FutureExt;
use serde_json::json;

use kernel::config::WeChatBotConfig;
use kernel::palaces::kan_io::{ChannelInput, ChannelSource};
use kernel::types::{Message, Role};

use super::types::{
    ERRCODE_RATE_LIMIT, ERRCODE_SESSION_EXPIRED, GetUpdatesResponse, ITEM_TEXT,
    LONG_POLL_TIMEOUT_SECS, PollError, WeChatMessage, base_info, build_headers, load_credentials,
    save_sync_buf, send_wechat_message, send_wechat_typing,
};

// ── Adapter ───────────────────────────────────────────────────

struct WeChatAdapter {
    config: WeChatBotConfig,
    client: reqwest::Client,
    sync_buf: String,
    context_tokens: HashMap<String, String>,
    cm: Arc<kernel::palaces::kan_io::ChannelManager>,
    consecutive_errors: u32,
    seen_msg_ids: HashMap<String, std::time::Instant>,
}

impl WeChatAdapter {
    fn new(config: WeChatBotConfig, cm: Arc<kernel::palaces::kan_io::ChannelManager>) -> Self {
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
            tokio::sync::mpsc::unbounded_channel::<kernel::palaces::kan_io::OutboundReply>();
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
    cm: Arc<kernel::palaces::kan_io::ChannelManager>,
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

fn backoff_delay(consecutive_errors: u32) -> std::time::Duration {
    let secs = 1u64 << consecutive_errors.min(10); // max ~17 min, clamped below
    let capped = secs.min(300); // 5 min cap
    std::time::Duration::from_secs(capped)
}

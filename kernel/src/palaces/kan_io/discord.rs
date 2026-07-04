use std::sync::Arc;
// ── Discord Bot (interaction webhook reply) ──────────────

use tokio::sync::mpsc;

use crate::palaces::kan_io::{ChannelInput, ChannelSource, OutboundReply};
use crate::types::{Message, Role};

/// Metadata extracted from a Discord interaction, needed to send followup messages.
#[derive(Debug, Clone)]
pub struct DiscordInteractionMeta {
    pub application_id: String,
    pub interaction_token: String,
}

/// Extract the fields needed for webhook followup from a Discord interaction payload.
pub fn extract_meta(interaction: &serde_json::Value) -> Option<DiscordInteractionMeta> {
    let app_id = interaction["application_id"].as_str()?;
    let token = interaction["token"].as_str()?;
    Some(DiscordInteractionMeta {
        application_id: app_id.to_string(),
        interaction_token: token.to_string(),
    })
}

/// Extract user-facing text from an APPLICATION_COMMAND interaction.
pub fn extract_command_text(interaction: &serde_json::Value) -> String {
    interaction["data"]["options"]
        .as_array()
        .and_then(|opts| opts.first())
        .and_then(|opt| opt["value"].as_str())
        .or_else(|| interaction["data"]["name"].as_str())
        .unwrap_or("")
        .to_string()
}

/// Build a `ChannelInput` with a reply channel that sends the agent response
/// back to Discord via the interaction webhook followup endpoint.
pub fn enqueue_agent_task(
    meta: DiscordInteractionMeta,
    text: String,
    cm: Arc<crate::palaces::kan_io::ChannelManager>,
) {
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<OutboundReply>();

    tokio::spawn(async move {
        while let Some(reply) = reply_rx.recv().await {
            match send_discord_followup(&meta.application_id, &meta.interaction_token, &reply.text)
                .await
            {
                Ok(()) => tracing::info!("Discord reply sent"),
                Err(e) => tracing::warn!(error = %e, "Discord reply failed"),
            }
        }
    });

    let input = ChannelInput {
        messages: vec![Message::text(Role::User, text)],
        source: ChannelSource::Webhook {
            endpoint: "discord".into(),
        },
        reply_tx: Some(reply_tx),
    };
    cm.push(input);
}

/// Send a followup message to a Discord interaction.
///
/// For deferred responses (type 5 ACK), this edits the original "thinking..."
/// message in-place. For large responses (>2000 chars), it splits into
/// multiple messages using Discord's `send followup` endpoint.
async fn send_discord_followup(
    application_id: &str,
    interaction_token: &str,
    text: &str,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let base = format!("https://discord.com/api/v10/webhooks/{application_id}/{interaction_token}");

    // Edit the original deferred response message first
    let first_chunk = text.chars().take(2000).collect::<String>();
    let patch_url = format!("{base}/messages/@original");
    let resp = client
        .patch(&patch_url)
        .json(&serde_json::json!({"content": first_chunk}))
        .send()
        .await
        .map_err(|e| format!("Discord PATCH @original HTTP: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Discord PATCH {status}: {body}"));
    }

    // If text exceeds 2000 chars, send remaining chunks as followups
    let total_chars = text.chars().count();
    if total_chars > 2000 {
        let tail_chars: Vec<char> = text.chars().skip(2000).collect();
        for chunk in tail_chars.chunks(1999) {
            let chunk_str: String = chunk.iter().collect();
            if chunk_str.trim().is_empty() {
                continue;
            }
            let resp = client
                .post(&base)
                .json(&serde_json::json!({"content": chunk_str}))
                .send()
                .await
                .map_err(|e| format!("Discord POST followup HTTP: {e}"))?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                tracing::warn!("Discord followup chunk {status}: {body}");
            }
        }
    }

    Ok(())
}

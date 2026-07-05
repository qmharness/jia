use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde::Deserialize;

use crate::types::{Message, Role};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct WebhookBody {
    message: String,
    #[serde(default)]
    source: Option<String>,
}

pub async fn handle_webhook(
    State(state): State<Arc<AppState>>,
    Json(body): Json<WebhookBody>,
) -> Json<serde_json::Value> {
    if let Some(earth) = &state.earth {
        let input = crate::palaces::kan_io::ChannelInput {
            messages: vec![Message::text(Role::User, body.message)],
            source: crate::palaces::kan_io::ChannelSource::Webhook {
                endpoint: body.source.unwrap_or_else(|| "webhook".into()),
            },
            reply_tx: None,
        };
        earth.io.push(input);
        tracing::info!("Webhook: message queued");
        Json(serde_json::json!({"status": "ok", "queued": true}))
    } else {
        Json(serde_json::json!({"status": "error", "message": "Agent not initialized"}))
    }
}

// Discord webhook handler removed — JIA focuses on WebSocket and long-poll channels.
// See ROADMAP.md for planned channels (Slack Socket Mode, QQ WebSocket, Feishu).

pub async fn handle_discord_webhook() -> impl IntoResponse {
    (
        axum::http::StatusCode::GONE,
        axum::response::Json(
            serde_json::json!({"error": "Discord support removed. See ROADMAP.md for planned channels."}),
        ),
    )
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_body_deserializes() {
        let body: WebhookBody =
            serde_json::from_str(r#"{"message": "hello", "source": "webhook"}"#).unwrap();
        assert_eq!(body.message, "hello");
        assert_eq!(body.source, Some("webhook".into()));
    }

    #[test]
    fn webhook_body_defaults_source_to_none() {
        let body: WebhookBody = serde_json::from_str(r#"{"message": "test"}"#).unwrap();
        assert_eq!(body.message, "test");
        assert!(body.source.is_none());
    }
}

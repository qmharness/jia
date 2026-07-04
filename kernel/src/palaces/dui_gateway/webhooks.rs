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

pub async fn handle_discord_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    use axum::http::StatusCode;

    let public_key = match &state.discord_public_key {
        Some(k) => k.clone(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Discord not configured"})),
            );
        }
    };

    // Verify signature
    let signature = match headers
        .get("X-Signature-Ed25519")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Missing signature"})),
            );
        }
    };
    let timestamp = match headers
        .get("X-Signature-Timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(s) => s,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Missing timestamp"})),
            );
        }
    };

    let pk_bytes = match hex::decode(&public_key) {
        Ok(b) if b.len() == 32 => b,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Invalid public key"})),
            );
        }
    };
    let sig_bytes = match hex::decode(signature) {
        Ok(b) if b.len() == 64 => b,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Invalid signature format"})),
            );
        }
    };

    let message = format!("{timestamp}{body}");
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let vk = match VerifyingKey::from_bytes(
        &pk_bytes[..32]
            .try_into()
            .expect("pk len already verified 32"),
    ) {
        Ok(k) => k,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Bad public key"})),
            );
        }
    };
    let sig = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Bad signature"})),
            );
        }
    };
    if vk.verify(message.as_bytes(), &sig).is_err() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Invalid signature"})),
        );
    }

    // Parse interaction
    let interaction: serde_json::Value = match serde_json::from_str(&body) {
        Ok(i) => i,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid body: {e}")})),
            );
        }
    };

    let itype = interaction["type"].as_u64().unwrap_or(0);
    match itype {
        1 => {
            // PING → PONG
            (StatusCode::OK, Json(serde_json::json!({"type": 1})))
        }
        2 => {
            // APPLICATION_COMMAND → push to agent with reply channel
            let text = crate::palaces::kan_io::discord::extract_command_text(&interaction);
            if text.is_empty() {
                tracing::debug!("Discord interaction with empty command text, skipping");
            } else if let Some(meta) =
                crate::palaces::kan_io::discord::extract_meta(&interaction)
            {
                if let Some(earth) = &state.earth {
                    crate::palaces::kan_io::discord::enqueue_agent_task(
                        meta,
                        text,
                        earth.io.clone(),
                    );
                }
            } else {
                tracing::debug!("Discord interaction missing application_id or token, skipping");
            }

            // ACK (type 5 = DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE)
            (StatusCode::OK, Json(serde_json::json!({"type": 5})))
        }
        _ => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Unknown interaction type"})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_body_deserializes() {
        let body: WebhookBody =
            serde_json::from_str(r#"{"message": "hello", "source": "discord"}"#).unwrap();
        assert_eq!(body.message, "hello");
        assert_eq!(body.source, Some("discord".into()));
    }

    #[test]
    fn webhook_body_defaults_source_to_none() {
        let body: WebhookBody = serde_json::from_str(r#"{"message": "test"}"#).unwrap();
        assert_eq!(body.message, "test");
        assert!(body.source.is_none());
    }
}

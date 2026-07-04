use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use futures::stream::Stream;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

use crate::plates::shen_spirit::RuntimeEvent;

use super::AppState;
use super::auth::constant_time_eq;

/// GET /events — persistent SSE stream for server-to-client notifications.
///
/// Subscribes to the event bus and forwards CronCompleted events to the
/// frontend so cron job results appear in the chat in real time.
/// When no API key is configured, the prompt field is redacted to prevent
/// eavesdropping via unauthenticated SSE.
pub async fn handle_events(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Validate token via query param (EventSource API doesn't support custom headers)
    let authenticated = if let Some(ref expected) = state.api_key {
        let token = params.get("token").map(|s| s.as_str()).unwrap_or("");
        if !constant_time_eq(token.as_bytes(), expected.as_bytes()) {
            return Sse::new(UnboundedReceiverStream::new(
                mpsc::unbounded_channel::<Result<Event, Infallible>>().1,
            ))
            .keep_alive(
                axum::response::sse::KeepAlive::new()
                    .interval(std::time::Duration::from_secs(15))
                    .text("keep-alive"),
            );
        }
        true
    } else {
        false
    };

    // P2 · Session-scoped filtering: only forward cron events for the
    // requesting session (if provided). No session filter → all events.
    let session_filter: Option<String> = params.get("session_id").cloned();

    let (tx, rx) = mpsc::unbounded_channel();

    if let Some(ref earth) = state.earth {
        let mut broadcast_rx = earth.spirit.event_bus.subscribe();
        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(RuntimeEvent::CronCompleted {
                        job_name,
                        prompt,
                        response,
                        timestamp,
                        session_id: ev_session_id,
                        ..
                    }) => {
                        // P2 · Session-scoped filter
                        if let Some(ref filter) = session_filter
                            && filter != &ev_session_id
                        {
                            continue;
                        }
                        tracing::info!(
                            job = %job_name,
                            "GET /events: forwarding CronCompleted to client"
                        );
                        let safe_prompt = if authenticated {
                            prompt
                        } else {
                            "[authenticate with API key to see prompt]".into()
                        };
                        let data = serde_json::json!({
                            "type": "cron_notification",
                            "job_name": job_name,
                            "prompt": safe_prompt,
                            "response": response,
                            "timestamp": timestamp,
                        });
                        let event = Event::default().data(data.to_string());
                        if tx.send(Ok(event)).is_err() {
                            break; // client disconnected
                        }
                    }
                    Ok(_) => {} // ignore other event types
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "GET /events listener lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    let stream = UnboundedReceiverStream::new(rx);
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[derive(Debug, Deserialize)]
pub struct CancelBody {
    session_id: String,
}

pub async fn handle_cancel(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CancelBody>,
) -> StatusCode {
    state.session_tokens.cancel(&body.session_id);
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_body_deserializes() {
        let b: CancelBody = serde_json::from_str(r#"{"session_id": "abc"}"#).unwrap();
        assert_eq!(b.session_id, "abc");
    }
}

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::telemetry::metrics::{metrics_handler, metrics_json};

use super::{AppState, SessionInfo};

pub async fn handle_health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "version": env!("CARGO_PKG_VERSION")})),
    )
}

pub async fn handle_ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if state.earth.is_some() {
        (StatusCode::OK, Json(serde_json::json!({"status": "ready"})))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"status": "not_ready"})),
        )
    }
}

pub async fn handle_metrics() -> String {
    metrics_handler()
}

pub async fn handle_monitor(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let ctx_window = state
        .providers
        .get(&state.default_main_provider_name)
        .and_then(|p| p.context_window)
        .unwrap_or(8192);

    Json(serde_json::json!({
        "context_window": { "max_tokens": ctx_window },
        "metrics": metrics_json(),
        "active_sessions": state.session_tokens.active_count(),
    }))
}

pub async fn handle_active_sessions(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let sessions: Vec<SessionInfo> = state.session_tokens.list_active();
    Json(serde_json::json!({ "sessions": sessions }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_returns_ok() {
        let response = handle_health().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn metrics_returns_text() {
        let output = handle_metrics().await;
        // Prometheus metrics handler returns empty string if no metrics yet collected
        assert!(output.is_empty() || output.contains("jia_") || output.contains("# HELP"));
    }
}

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;

use super::AppState;

pub async fn handle_list_sessions(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;

    let filter = params.get("filter").map(|s| s.as_str()).unwrap_or("active");

    let sessions = earth
        .store
        .list_sessions_filtered(filter)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Collect active (currently streaming) session ids
    let active_ids: std::collections::HashSet<String> = state
        .session_tokens
        .list_active()
        .into_iter()
        .map(|info| info.id)
        .collect();

    // Merge active status + error status into a single "status" field
    let sessions: Vec<serde_json::Value> = sessions
        .into_iter()
        .map(|mut s| {
            let has_error = s.get("hasError").and_then(|v| v.as_bool()).unwrap_or(false);
            let id = s
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // Remove internal hasError field, add computed status
            if let Some(obj) = s.as_object_mut() {
                obj.remove("hasError");
                let status = if active_ids.contains(&id) {
                    "active"
                } else if has_error {
                    "error"
                } else {
                    "idle"
                };
                obj.insert("status".to_string(), serde_json::json!(status));
            }
            s
        })
        .collect();

    Ok(Json(serde_json::json!({ "sessions": sessions })))
}

pub async fn handle_archive_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;
    earth
        .store
        .archive_session(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn handle_unarchive_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;
    earth
        .store
        .unarchive_session(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct BulkDeleteBody {
    ids: Vec<String>,
}

pub async fn handle_bulk_delete_sessions(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BulkDeleteBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;

    earth
        .store
        .delete_sessions(&body.ids)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "deleted": body.ids.len() })))
}

pub async fn handle_delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;

    earth
        .store
        .delete_session(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "deleted": id })))
}

#[derive(Deserialize)]
pub struct RenameBody {
    title: String,
}

pub async fn handle_rename_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<RenameBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;

    earth
        .store
        .rename_session(&id, &body.title)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "id": id, "title": body.title })))
}

pub async fn handle_get_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state.earth.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent not initialized".into(),
        )
    })?;

    let json_str = earth
        .store
        .load_session(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Session {id} not found")))?;

    // History is now a unified array (messages + tool cards), deserialize directly.
    let entries: Vec<serde_json::Value> = serde_json::from_str(&json_str)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(
        serde_json::json!({ "session_id": id, "entries": entries }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_delete_body_deserializes() {
        let body: BulkDeleteBody = serde_json::from_str(r#"{"ids": ["a", "b", "c"]}"#).unwrap();
        assert_eq!(body.ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn bulk_delete_body_accepts_empty() {
        let body: BulkDeleteBody = serde_json::from_str(r#"{"ids": []}"#).unwrap();
        assert!(body.ids.is_empty());
    }

    #[test]
    fn rename_body_deserializes() {
        let body: RenameBody = serde_json::from_str(r#"{"title": "new title"}"#).unwrap();
        assert_eq!(body.title, "new title");
    }
}

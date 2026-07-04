//! SystemPrinciple REST handlers — list, archive, unarchive.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use serde_json::{json, Value};

use super::AppState;

/// GET /principles — list all principles (active + archived).
pub async fn handle_list_principles(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, StatusCode> {
    let store = state
        .earth
        .as_ref()
        .and_then(|e| Some(e.store.clone()))
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let jsons = store
        .load_all_principles()
        .map_err(|e| {
            tracing::error!("Failed to load principles: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let principles: Vec<Value> = jsons
        .iter()
        .filter_map(|j| serde_json::from_str(j).ok())
        .collect();

    Ok(Json(json!({ "principles": principles })))
}

/// POST /principles/:id/archive — archive a single principle.
pub async fn handle_archive_principle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let store = state
        .earth
        .as_ref()
        .map(|e| e.store.clone())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    store.archive_principle(&id).map_err(|e| {
        tracing::error!(%id, error = %e, "Failed to archive principle");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(json!({ "id": id, "archived": true })))
}

/// POST /principles/:id/unarchive — restore an archived principle.
pub async fn handle_unarchive_principle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let store = state
        .earth
        .as_ref()
        .map(|e| e.store.clone())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    store.unarchive_principle(&id).map_err(|e| {
        tracing::error!(%id, error = %e, "Failed to unarchive principle");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(json!({ "id": id, "archived": false })))
}

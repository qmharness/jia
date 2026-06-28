use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use super::AppState;

// ── Project handlers ───────────────────────────────────────

pub async fn handle_list_projects(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let include_archived = params.get("filter").map(|f| f == "all").unwrap_or(false);
    match earth.store.list_projects(include_archived) {
        Ok(projects) => Ok(Json(serde_json::json!({ "projects": projects }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[derive(serde::Deserialize)]
pub struct CreateProjectBody {
    name: String,
    #[serde(default)]
    cwd: Option<String>,
}

pub async fn handle_create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let base = body.cwd.unwrap_or_else(|| {
        state
            .earth
            .as_ref()
            .map(|e| e.config.app_config.workspace_path.display().to_string())
            .unwrap_or_default()
    });
    let cwd = format!("{}/{}", base, body.name);
    // Create directories
    std::fs::create_dir_all(&cwd).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create directory: {}", e),
        )
    })?;
    std::fs::create_dir_all(format!("{}/.jia", &cwd)).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create .jia: {}", e),
        )
    })?;
    // Generate project
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        format!("{}/.jia/config.toml", &cwd),
        format!("[project]\nid = \"{}\"\nname = \"{}\"\n", id, body.name),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write config: {}", e),
        )
    })?;
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    earth
        .store
        .ensure_project(&id, &cwd, &body.name, "", "[]")
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "id": id, "cwd": cwd, "name": body.name }),
    ))
}

pub async fn handle_get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let proj = earth
        .store
        .get_project(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".into()))?;
    Ok(Json(proj))
}

pub async fn handle_archive_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    earth
        .store
        .archive_project(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn handle_unarchive_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    earth
        .store
        .unarchive_project(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(serde::Deserialize)]
pub struct PatchProjectBody {
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
}

pub async fn handle_patch_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchProjectBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let proj = earth
        .store
        .get_project(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".into()))?;
    let cwd = proj["cwd"].as_str().unwrap_or("");
    let name = body
        .name
        .as_deref()
        .unwrap_or(proj["name"].as_str().unwrap_or(""));
    let desc = body
        .description
        .as_deref()
        .unwrap_or(proj["description"].as_str().unwrap_or(""));
    let tags_json = if let Some(ref tags) = body.tags {
        serde_json::to_string(tags).unwrap_or_default()
    } else {
        proj["tags"].to_string()
    };
    earth
        .store
        .ensure_project(&id, cwd, name, desc, &tags_json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_project_body_deserializes() {
        let b: CreateProjectBody =
            serde_json::from_str(r#"{"name": "test", "cwd": "/tmp"}"#).unwrap();
        assert_eq!(b.name, "test");
        assert_eq!(b.cwd, Some("/tmp".into()));
    }

    #[test]
    fn patch_project_body_deserializes() {
        let b: PatchProjectBody = serde_json::from_str(r#"{"name": "renamed"}"#).unwrap();
        assert_eq!(b.name, Some("renamed".into()));
    }
}

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
    pub name: String,
    pub cwd: String,
}

pub async fn handle_create_project(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProjectBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Validate cwd is an absolute path
    if !std::path::Path::new(&body.cwd).is_absolute() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'cwd' must be an absolute path, got: {}", body.cwd),
        ));
    }
    let cwd = &body.cwd;
    // Create directories
    std::fs::create_dir_all(cwd).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create directory: {}", e),
        )
    })?;
    std::fs::create_dir_all(format!("{cwd}/.jia")).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to create .jia: {}", e),
        )
    })?;
    // Generate project
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        format!("{cwd}/.jia/config.toml"),
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
        .ensure_project(&id, cwd, &body.name, "", "[]")
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
        assert_eq!(b.cwd, "/tmp");
    }

    #[test]
    fn create_project_body_requires_cwd() {
        let b: Result<CreateProjectBody, _> = serde_json::from_str(r#"{"name": "test"}"#);
        assert!(b.is_err());
    }

    #[test]
    fn patch_project_body_deserializes() {
        let b: PatchProjectBody = serde_json::from_str(r#"{"name": "renamed"}"#).unwrap();
        assert_eq!(b.name, Some("renamed".into()));
    }
}

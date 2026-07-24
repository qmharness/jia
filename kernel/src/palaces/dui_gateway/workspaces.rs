use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;

use super::AppState;

// ── Workspace handlers ─────────────────────────────────────

pub async fn handle_list_workspaces(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let include_archived = params.get("filter").map(|f| f == "all").unwrap_or(false);
    match earth.store.list_workspaces(include_archived) {
        Ok(workspaces) => Ok(Json(serde_json::json!({ "workspaces": workspaces }))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[derive(serde::Deserialize)]
pub struct CreateWorkspaceBody {
    pub name: String,
    pub cwd: String,
}

pub async fn handle_create_workspace(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateWorkspaceBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Validate cwd is an absolute path
    if !std::path::Path::new(&body.cwd).is_absolute() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'cwd' must be an absolute path, got: {}", body.cwd),
        ));
    }
    let cwd = &body.cwd;
    // 拒绝覆盖已有工作区:已有 .jia/config.toml 说明该目录已归属某个
    // workspace id——重复创建会静默换 id 并级联旧会话/种子(审计 F6)。
    let config_path = format!("{cwd}/.jia/config.toml");
    if std::path::Path::new(&config_path).exists() {
        return Err((
            StatusCode::CONFLICT,
            format!("workspace already exists at {cwd} (.jia/config.toml present)"),
        ));
    }
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
    // Generate workspace
    let id = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        &config_path,
        format!("[workspace]\nid = \"{}\"\nname = \"{}\"\n", id, body.name),
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
        .ensure_workspace(&id, cwd, &body.name)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(
        serde_json::json!({ "id": id, "cwd": cwd, "name": body.name }),
    ))
}

pub async fn handle_get_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let proj = earth
        .store
        .get_workspace(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Workspace not found".into()))?;
    Ok(Json(proj))
}

pub async fn handle_archive_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let n = earth
        .store
        .archive_workspace(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if n == 0 {
        return Err((StatusCode::NOT_FOUND, "Workspace not found".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn handle_unarchive_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let n = earth
        .store
        .unarchive_workspace(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if n == 0 {
        return Err((StatusCode::NOT_FOUND, "Workspace not found".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(serde::Deserialize)]
pub struct PatchWorkspaceBody {
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
}

pub async fn handle_patch_workspace(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PatchWorkspaceBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let earth = state
        .earth
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Not ready".into()))?;
    let proj = earth
        .store
        .get_workspace(&id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Workspace not found".into()))?;
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
    // 元数据走专用更新;ensure_workspace 只维护身份(id/cwd/name),
    // 不能在 PATCH 之外的路径改 description/tags(审计 F1)。
    let n = earth
        .store
        .update_workspace_metadata(&id, name, desc, &tags_json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if n == 0 {
        // get_workspace 预检后行被并发删除/换 id——不得假报成功。
        return Err((StatusCode::NOT_FOUND, "Workspace not found".into()));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_workspace_body_deserializes() {
        let b: CreateWorkspaceBody =
            serde_json::from_str(r#"{"name": "test", "cwd": "/tmp"}"#).unwrap();
        assert_eq!(b.name, "test");
        assert_eq!(b.cwd, "/tmp");
    }

    #[test]
    fn create_workspace_body_requires_cwd() {
        let b: Result<CreateWorkspaceBody, _> = serde_json::from_str(r#"{"name": "test"}"#);
        assert!(b.is_err());
    }

    #[test]
    fn patch_workspace_body_deserializes() {
        let b: PatchWorkspaceBody = serde_json::from_str(r#"{"name": "renamed"}"#).unwrap();
        assert_eq!(b.name, Some("renamed".into()));
    }
}

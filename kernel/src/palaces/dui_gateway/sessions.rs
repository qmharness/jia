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

    sweep_destroyed_sessions(&earth.session_bus, &body.ids);

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

    sweep_destroyed_sessions(&earth.session_bus, std::slice::from_ref(&id));

    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// N1/#15 · HTTP 侧会话终点清扫:DELETE /sessions/{id} 与 bulk-delete 是
/// HTTP 网关仅有的会话销毁点 —— 会话行从 store 删除后 session_id(UUID v4)
/// 不再复用,SessionBus 上的批准记忆/验收标准即成不可达残留,按 rin 断连
/// 清扫(rin.rs sweep_pending_for_sessions)同点清除。
///
/// SSE 断连【不是】会话终点:HTTP 会话长驻可续聊,每个 turn 都是同一
/// session_id 的新 SSE 连接(agent.rs),在断连处清扫会把"每轮新连接"
/// 误判为会话结束、逼用户每轮重新批准,故只挂销毁点。会话无超时回收
/// (store 持久化保留);archive 可逆,不在此列。
fn sweep_destroyed_sessions(
    session_bus: &crate::plates::ren_human::SessionBus,
    ids: &[String],
) {
    for id in ids {
        session_bus.clear_session_approvals(id);
        session_bus.clear_criteria(id);
    }
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

    /// N1/#15 · 会话销毁清扫后须重新询问:批准记忆与验收标准随会话删除
    /// 而清空(同 sid 不会再命中旧批准),且不影响其他会话。
    #[test]
    fn sweep_destroyed_sessions_clears_approvals_and_criteria() {
        let bus = crate::plates::ren_human::SessionBus::new();
        // 模拟:用户曾批准过 "exec:ls"(命中即免问),并设过验收标准。
        bus.session_approvals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert("s1".to_string(), ["exec:ls".to_string()].into_iter().collect());
        bus.set_criteria("s1", vec!["tests pass".into()]);
        bus.set_criteria("s2", vec!["other session".into()]);

        sweep_destroyed_sessions(&bus, &["s1".to_string()]);

        // 批准记忆已清:同 sid 再次出现时必须重新询问(首次必询问语义)。
        assert!(
            bus.session_approvals
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get("s1")
                .is_none(),
            "swept session must not retain approval memory"
        );
        // 验收标准已清(桶被移除,而非仅勾完)。
        assert!(
            bus.completion_criteria
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get("s1")
                .is_none()
        );
        // 其他会话不受影响。
        assert_eq!(bus.unchecked_criteria("s2"), ["other session"]);
    }
}

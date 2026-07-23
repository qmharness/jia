use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct ConfirmBody {
    id: String,
    token: String,
    approved: bool,
}

pub async fn handle_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ConfirmBody>,
) -> Json<serde_json::Value> {
    // P2-1 收口:pending 表唯一来源是人盘 SessionBus(经 earth),AppState
    // 不再持有重复句柄。
    let Some(earth) = &state.earth else {
        return Json(serde_json::json!({"error": "Agent not initialized"}));
    };
    let pending = {
        let mut map = earth
            .session_bus
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.remove(&body.id)
    };
    match pending {
        Some(p) if p.token == body.token => {
            let _ = p.sender.send(body.approved);
            tracing::info!(
                "Confirmation {} resolved: approved={}",
                body.id,
                body.approved,
            );
            Json(serde_json::json!({"status": "ok", "resolved": true}))
        }
        Some(_) => {
            tracing::warn!(
                "Confirmation {} token mismatch — possible replay or forgery",
                body.id,
            );
            Json(serde_json::json!({"status": "token_mismatch", "resolved": false}))
        }
        None => {
            tracing::warn!(
                "Confirmation {} not found (expired or already resolved)",
                body.id,
            );
            Json(serde_json::json!({"status": "not_found", "resolved": false}))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AnswerBody {
    id: String,
    token: String,
    answer: String,
}

pub async fn handle_answer(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AnswerBody>,
) -> Json<serde_json::Value> {
    let Some(earth) = &state.earth else {
        return Json(serde_json::json!({"error": "Agent not initialized"}));
    };
    let pending = {
        let mut map = earth
            .session_bus
            .pending_questions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.remove(&body.id)
    };
    match pending {
        Some(p) if p.token == body.token => {
            let answer_len = body.answer.len();
            let _ = p.sender.send(body.answer);
            tracing::info!("Question {} answered: {} chars", body.id, answer_len,);
            Json(serde_json::json!({"status": "ok", "resolved": true}))
        }
        Some(_) => {
            tracing::warn!(
                "Question {} token mismatch — possible replay or forgery",
                body.id,
            );
            Json(serde_json::json!({"status": "token_mismatch", "resolved": false}))
        }
        None => {
            tracing::warn!(
                "Question {} not found (expired or already resolved)",
                body.id,
            );
            Json(serde_json::json!({"status": "not_found", "resolved": false}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_body_deserializes() {
        let b: ConfirmBody =
            serde_json::from_str(r#"{"id": "x", "token": "t", "approved": true}"#).unwrap();
        assert!(b.approved);
    }

    #[test]
    fn answer_body_deserializes() {
        let b: AnswerBody =
            serde_json::from_str(r#"{"id": "x", "token": "t", "answer": "yes"}"#).unwrap();
        assert_eq!(b.answer, "yes");
    }
}

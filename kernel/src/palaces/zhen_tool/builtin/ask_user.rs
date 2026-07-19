use crate::error::ToolError;
use std::sync::Arc;
// ── Ask User Question Tool — Interactive user query ─────────

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::palaces::zhen_tool::base::BaseTool;
use crate::plates::ren_human::PendingQuestion;
use crate::plates::tian_heaven::r#loop::AgentEvent;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;
use crate::stems::intent::CommunicateAction;

pub struct AskUserQuestionTool {
    pending_questions: Arc<Mutex<HashMap<String, PendingQuestion>>>,
}

impl AskUserQuestionTool {
    pub fn new(pending_questions: Arc<Mutex<HashMap<String, PendingQuestion>>>) -> Self {
        Self { pending_questions }
    }
}

#[async_trait]
impl BaseTool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "ask_user"
    }

    fn description(&self) -> String {
        "Ask the user a question when you need clarification or a decision. \
         Use this when you are uncertain about the user's intent, need to choose \
         between multiple valid approaches, or require confirmation before \
         proceeding with a potentially risky action. \
         The user's answer will be returned as the tool output."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren(CommunicateAction {
            endpoint: "user".into(),
            payload: String::new(),
        })
    }

    fn is_destructive(&self) -> bool {
        false // asking the user is read-only
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user. Be clear and specific."
                },
                "options": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 9,
                    "description": "Optional list of choices (max 9). If provided, the user can select one with arrow keys. The last option should be \"Other (free-text)\" to let the user type a custom answer."
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(&self, _input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        // Never called directly — execute_with_tx is used instead.
        Err("ask_user requires event channel access".into())
    }

    async fn execute_with_tx(
        &self,
        input: Value,
        tx: &mpsc::UnboundedSender<AgentEvent>,
        ctx: &ExecContext,
    ) -> Result<String, ToolError> {
        let question = input["question"]
            .as_str()
            .ok_or("Missing 'question' parameter")?
            .to_string();

        let options: Option<Vec<String>> = input["options"].as_array().map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });

        let id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let timeout_secs = ctx.permissions.confirmation_timeout.as_secs();

        let (otx, orx) = oneshot::channel::<String>();

        {
            let mut guard = self
                .pending_questions
                .lock()
                .map_err(|e| format!("Question store poisoned: {e}"))?;
            guard.insert(
                id.clone(),
                PendingQuestion {
                    sender: otx,
                    token: token.clone(),
                    created_at: crate::utils::unix_now(),
                    session_id: ctx.session_id.clone(),
                },
            );
        }

        let _ = tx.send(AgentEvent::UserQuestion {
            id: id.clone(),
            question: question.clone(),
            timeout_secs,
            token: token.clone(),
            options: options.clone(),
        });

        // Wait for the user's answer, or wake on cancellation (HTTP 取消 /
        // SSE 断连 CancelOnDropStream / rin cancel)。无超时裸等会让 agent 任务
        // 在 session_lock 内永卡 → 同 sid 后续 run 永久阻塞(审计 F2+L5)。
        // 断连清扫(rin 连接结束按 session_id remove pending 条目)使 sender
        // drop,orx 醒为 Err,走 "(user disconnected)" 分支。
        tracing::info!(%id, "ask_user: waiting for answer");
        let answered = tokio::select! {
            r = orx => r,
            _ = ctx.cancel_token.cancelled() => {
                // 取消语义:清理 pending 条目,返回错误让 loop 感知取消。
                // 锁中毒时取回内部值继续清理,不留残留(与 sweep/ren_human 一致)。
                self.pending_questions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                tracing::info!(%id, "ask_user: cancelled while waiting");
                return Err("ask_user cancelled".into());
            }
        };
        match answered {
            Ok(answer) => {
                tracing::info!(%id, answer_len = answer.len(), "ask_user: received answer");
                if answer.is_empty() {
                    Ok("(user cancelled)".into())
                } else {
                    Ok(answer)
                }
            }
            Err(_) => {
                // Sender dropped — cleanup and return default
                tracing::warn!(%id, "ask_user: sender dropped (user disconnected)");
                let _ = self.pending_questions.lock().map(|mut g| g.remove(&id));
                Ok("(user disconnected)".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::qian_permission::PermissionMatrix;

    fn ctx_with_token(cancel_token: tokio_util::sync::CancellationToken) -> ExecContext {
        ExecContext {
            permissions: Arc::new(PermissionMatrix::default()),
            session_id: "sess-1".into(),
            cancel_token,
        }
    }

    fn wait_until_inserted(pending: &Arc<Mutex<HashMap<String, PendingQuestion>>>) -> String {
        for _ in 0..100 {
            if let Ok(g) = pending.lock()
                && let Some(id) = g.keys().next()
            {
                return id.clone();
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("question was never inserted into pending_questions");
    }

    /// P0-4 · 等待中收到取消 → 工具返回取消错误,pending_questions 无残留。
    #[tokio::test]
    async fn ask_user_cancelled_while_waiting() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let tool = AskUserQuestionTool::new(pending.clone());
        let (tx, _rx) = mpsc::unbounded_channel();
        let token = tokio_util::sync::CancellationToken::new();
        let ctx = ctx_with_token(token.clone());

        let handle = tokio::spawn(async move {
            tool.execute_with_tx(serde_json::json!({"question": "q?"}), &tx, &ctx)
                .await
        });

        let id = tokio::task::spawn_blocking({
            let pending = pending.clone();
            move || wait_until_inserted(&pending)
        })
        .await
        .unwrap();
        assert!(!id.is_empty());

        token.cancel();

        let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("ask_user must wake on cancel (deadlock!)")
            .unwrap();
        assert!(res.is_err(), "cancelled ask_user must return an error");
        assert!(
            res.unwrap_err().to_string().contains("cancel"),
            "error should mention cancellation"
        );
        assert!(
            pending.lock().unwrap().is_empty(),
            "pending_questions must have no residue after cancel"
        );
    }

    /// P0-4 · ESC 路径不回归:空 answer 仍返回 "(user cancelled)"。
    #[tokio::test]
    async fn ask_user_empty_answer_still_user_cancelled() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let tool = AskUserQuestionTool::new(pending.clone());
        let (tx, _rx) = mpsc::unbounded_channel();
        let ctx = ctx_with_token(tokio_util::sync::CancellationToken::new());

        let handle = tokio::spawn(async move {
            tool.execute_with_tx(serde_json::json!({"question": "q?"}), &tx, &ctx)
                .await
        });

        let id = tokio::task::spawn_blocking({
            let pending = pending.clone();
            move || wait_until_inserted(&pending)
        })
        .await
        .unwrap();

        // Simulate ESC: rin "answer" handler resolves with empty string.
        let sender = pending.lock().unwrap().remove(&id).unwrap().sender;
        let _ = sender.send(String::new());

        let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("ask_user must wake on answer")
            .unwrap();
        assert_eq!(res.unwrap(), "(user cancelled)");
    }

    /// P0-4 · 断连清扫语义:remove pending 条目使 sender drop → "(user disconnected)"。
    #[tokio::test]
    async fn ask_user_sender_dropped_returns_disconnected() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let tool = AskUserQuestionTool::new(pending.clone());
        let (tx, _rx) = mpsc::unbounded_channel();
        let ctx = ctx_with_token(tokio_util::sync::CancellationToken::new());

        let handle = tokio::spawn(async move {
            tool.execute_with_tx(serde_json::json!({"question": "q?"}), &tx, &ctx)
                .await
        });

        let id = tokio::task::spawn_blocking({
            let pending = pending.clone();
            move || wait_until_inserted(&pending)
        })
        .await
        .unwrap();

        // Simulate rin disconnect sweep: remove entry → sender dropped.
        let _ = pending.lock().unwrap().remove(&id);

        let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("ask_user must wake when sender dropped")
            .unwrap();
        assert_eq!(res.unwrap(), "(user disconnected)");
        assert!(pending.lock().unwrap().is_empty());
    }
}

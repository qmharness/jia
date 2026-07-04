use std::sync::Arc;
use crate::error::ToolError;
// ── Ask User Question Tool — Interactive user query ─────────

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::palaces::zhen_tool::base::BaseTool;
use crate::plates::tian_heaven::r#loop::AgentEvent;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;
use crate::stems::intent::CommunicateAction;

/// A pending question awaiting user answer.
pub struct PendingQuestion {
    pub sender: oneshot::Sender<String>,
    pub token: String,
    pub created_at: i64,
}

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

        // Wait indefinitely — user must answer or cancel (ESC).
        tracing::info!(%id, "ask_user: waiting for answer");
        match orx.await {
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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Optional images for multimodal (vision) input.
    /// When present, the provider sends content as a multimodal array
    /// with both text and image blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageContent>,
}

impl Message {
    /// Create a text-only message.
    pub fn text(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            images: vec![],
        }
    }

    /// Create a multimodal message with text and images.
    pub fn with_images(role: Role, content: impl Into<String>, images: Vec<ImageContent>) -> Self {
        Self {
            role,
            content: content.into(),
            images,
        }
    }
}

/// An image for multimodal LLM input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContent {
    /// Base64-encoded image data (without data URI prefix).
    pub data: String,
    /// MIME type, e.g. "image/png", "image/jpeg".
    pub media_type: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    pub messages: Vec<Message>,
}

fn default_provider() -> String {
    String::new()
}

/// Agent request — includes provider selection for the agent loop
#[derive(Debug, Clone, Deserialize)]
pub struct AgentRequest {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    pub messages: Vec<Message>,
    /// Session ID for cross-request memory persistence
    #[serde(default)]
    pub session_id: Option<String>,
    /// Override aux provider. Falls back to server-configured default_aux_model_provider.
    #[serde(default)]
    pub aux_provider: Option<String>,
    /// Override aux model for consolidation/distillation/reflection.
    /// Falls back to the aux provider's default_main_model if unset.
    #[serde(default)]
    pub aux_model: Option<String>,
    /// Workspace directory for this session (project path)
    #[serde(default)]
    pub cwd: Option<String>,
    /// Project ID (UUID from .jia/config.toml)
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    #[serde(rename = "delta")]
    Delta { content: String },
    #[serde(rename = "stream_end")]
    StreamEnd,
    #[serde(rename = "tool_batch_start")]
    ToolBatchStart,
    #[serde(rename = "tool_call")]
    ToolCall {
        tool: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool: String,
        output: String,
        error: Option<String>,
        geju: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        execution_mode: Option<String>,
    },
    #[serde(rename = "session")]
    Session { session_id: String },
    #[serde(rename = "confirm_request")]
    ConfirmationRequest {
        id: String,
        tool: String,
        reason: String,
        timeout_secs: u64,
        token: String,
    },
    #[serde(rename = "user_question")]
    UserQuestion {
        id: String,
        question: String,
        timeout_secs: u64,
        token: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        options: Option<Vec<String>>,
    },
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "error")]
    Error { message: String },
    /// P3 · interaction mode changed (谋划态 toggle).
    #[serde(rename = "interaction_mode_changed")]
    InteractionModeChanged { planning: bool },
    #[serde(rename = "context_pressure")]
    ContextPressure { tokens: usize, threshold: usize },
    #[serde(rename = "compacting")]
    Compacting,
}

impl Role {
    pub fn to_api_str(&self) -> &str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

// ── Tool Status / Execution Mode ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    #[serde(alias = "Direct")]
    Direct,
    #[serde(alias = "Guarded")]
    Guarded,
    #[serde(alias = "Sandbox")]
    Sandbox,
    #[serde(alias = "Denied")]
    Denied,
}

// ── HistoryEntry ────────────────────────────────────────────

/// Unified history entry for storage and frontend transport.
/// Messages and tool calls share the `role` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum HistoryEntry {
    #[serde(rename = "user")]
    User {
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
    },
    #[serde(rename = "assistant")]
    Assistant { content: String },
    #[serde(rename = "system")]
    System { content: String },
    #[serde(rename = "tool_call")]
    ToolCall {
        id: String,
        tool: String,
        input: serde_json::Value,
        status: ToolStatus,
        output: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        geju: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(rename = "executionMode")]
        execution_mode: Option<ExecutionMode>,
    },
}

impl HistoryEntry {
    pub fn user(content: impl Into<String>) -> Self {
        HistoryEntry::User {
            content: content.into(),
            images: vec![],
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        HistoryEntry::Assistant {
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        HistoryEntry::System {
            content: content.into(),
        }
    }

    /// Whether this is a conversational message (not a tool card).
    pub fn is_message(&self) -> bool {
        !matches!(self, HistoryEntry::ToolCall { .. })
    }

    /// Convert to an LLM-consumable Message. ToolCall becomes a User message
    /// with the tool result/error text, so the LLM sees tool outcomes in context.
    pub fn to_llm_message(&self) -> Option<Message> {
        match self {
            HistoryEntry::User { content, images } => Some(Message {
                role: Role::User,
                content: content.clone(),
                images: images.clone(),
            }),
            HistoryEntry::Assistant { content } => {
                Some(Message::text(Role::Assistant, content.clone()))
            }
            HistoryEntry::System { content } => Some(Message::text(Role::System, content.clone())),
            HistoryEntry::ToolCall {
                tool,
                output,
                error,
                ..
            } => {
                let content = if let Some(err) = error {
                    format!("Tool {tool} error: {err}")
                } else {
                    format!("Tool {tool} result: {output}")
                };
                Some(Message::text(Role::User, content))
            }
        }
    }
}

/// Convert history entries to LLM-consumable messages.
pub fn to_llm_messages(entries: &[HistoryEntry]) -> Vec<Message> {
    entries.iter().filter_map(|e| e.to_llm_message()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_deserializes() {
        let json = r#"{"messages": [{"role": "user", "content": "hello"}]}"#;
        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.messages[0].content, "hello");
    }

    #[test]
    fn chat_request_with_optional_fields() {
        let json = r#"{"messages": [], "provider": "anthropic", "model": "claude"}"#;
        let req: ChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.provider, "anthropic");
        assert_eq!(req.model.as_deref(), Some("claude"));
    }

    #[test]
    fn agent_request_deserializes() {
        let json = r#"{"messages": [{"role": "user", "content": "test"}], "session_id": "s1"}"#;
        let req: AgentRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.messages.len(), 1);
    }

    #[test]
    fn history_entry_user_to_llm() {
        let entry = HistoryEntry::User {
            content: "hi".into(),
            images: vec![],
        };
        let msg = entry.to_llm_message().unwrap();
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "hi");
    }

    #[test]
    fn history_entry_assistant_to_llm() {
        let entry = HistoryEntry::Assistant {
            content: "response".into(),
        };
        let msg = entry.to_llm_message().unwrap();
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "response");
    }
}

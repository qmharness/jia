//! Events emitted by the agent loop.

/// Events emitted by the agent loop to the outside world (SSE, REPL, bots).
#[derive(Debug, Clone)]
pub enum AgentEvent {
    Delta(String),
    StreamEnd,
    ToolBatchStart,
    Done,
    Error(String),
    ToolCall {
        tool: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool: String,
        output: String,
        error: Option<String>,
        geju: Option<String>,
        execution_mode: Option<String>,
    },
    ConfirmRequest {
        id: String,
        tool: String,
        reason: String,
        timeout_secs: u64,
        token: String,
    },
    Session {
        session_id: String,
    },
    UserQuestion {
        id: String,
        question: String,
        timeout_secs: u64,
        token: String,
        options: Option<Vec<String>>,
    },
    /// P3 · interaction mode changed (谋划态 toggle).
    InteractionModeChanged {
        planning: bool,
    },
    /// Context window nearing limit — 天辅.
    ContextPressure {
        tokens: usize,
        threshold: usize,
    },
    /// Context compaction in progress — 天英.
    Compacting,
}

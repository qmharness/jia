//! events — 天干层共享事件与交互语义 (P2-2 自天盘下沉)
//!
//! 哲学依据:天干 = 四盘共享语义层。`AgentEvent` 是天盘 loop 向外界
//! (SSE / REPL / bots) 发出的事件词汇,人盘(ren_human)、震宫工具
//! (ask_user/delegate)、兑宫网关(rin/agent)皆需引用——它是跨盘
//! 共享语义,非天盘私有。`InteractionMode`(谋划态)同理:它是
//! 用户面向的交互状态,会话模式表存于人盘 SessionBus,事件经天盘
//! 发出,消费在兑宫/TUI。
//!
//! 下沉后方向:地/人/宫 → 天干(合法);天 → 地(运行时编排,合法)。

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

/// P3 · Interaction mode — 谋划态 (planning) vs Normal.
///
/// Distinct from `AgentPhase` (九星, loop execution phase, 居天盘): this is a
/// user-facing interaction state. `Planning` forces read-only operation —
/// destructive tools are rejected by a loop-level short-circuit before GeJu
/// evaluation, so GeJu stays a pure 干叠加 evaluator (A2). User-triggered
/// primarily (slash/TUI); the model may also call enter/exit_plan_mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionMode {
    #[default]
    Normal,
    /// 谋划态 — read-only research/planning. Destructive tools blocked.
    Planning,
}

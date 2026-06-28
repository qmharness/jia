use tokio::sync::broadcast;

use crate::telemetry::metrics::JIA_EVENTBUS_DROPS_TOTAL;

pub mod hook;

/// 神盘 (Spirit Plate) — Observability layer.
///
/// Asynchronous hooks, metrics, and tracing. Does not block the main loop.
pub struct SpiritPlate {
    pub event_bus: EventBus,
    pub hook_registry: hook::HookRegistry,
}

impl SpiritPlate {
    pub fn new() -> Self {
        Self {
            event_bus: EventBus::new(),
            hook_registry: hook::HookRegistry::new(),
        }
    }
}

impl Default for SpiritPlate {
    fn default() -> Self {
        Self::new()
    }
}

/// Lightweight event bus for cross-cutting concerns.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<RuntimeEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: RuntimeEvent) {
        if let Err(tokio::sync::broadcast::error::SendError(event)) = self.tx.send(event) {
            JIA_EVENTBUS_DROPS_TOTAL.inc();
            tracing::warn!(
                "EventBus: channel full, dropping event: {:?}",
                std::mem::discriminant(&event)
            );
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Runtime events emitted by the system
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    TurnStart {
        turn: u64,
    },
    TurnEnd {
        turn: u64,
    },
    ToolCall {
        tool: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool: String,
        output: String,
    },
    GeJuResult {
        tool: String,
        pattern: String,
        mode: String,
    },
    Error {
        source: String,
        message: String,
    },
    ConfirmationRequested {
        id: String,
        tool: String,
        reason: String,
    },
    ConfirmationResolved {
        id: String,
        approved: bool,
    },
    LlmUsage {
        input_tokens: u64,
        output_tokens: u64,
    },
    SessionEnd {
        session_id: String,
        turns: u64,
    },
    /// Emitted when a cron job's agent session completes.
    CronCompleted {
        job_name: String,
        prompt: String,
        response: String,
        session_id: String,
        timestamp: u64,
    },
}

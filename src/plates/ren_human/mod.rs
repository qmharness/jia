use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate::geju::{ApprovalGate, ExecutionMode, GeJuResult};
use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::plates::shen_spirit::{EventBus, RuntimeEvent};
use crate::plates::tian_heaven::r#loop::AgentEvent;
use crate::stems::action::ExecContext;
use crate::stems::action::ToolResult;

/// A pending user confirmation, stored until resolved or timed out.
pub struct PendingConfirmation {
    pub sender: tokio::sync::oneshot::Sender<bool>,
    pub token: String,
}

/// 人盘 (Human Plate) — Permission boundary and human interaction gate.
///
/// Implements 八门 (8 Gates) for operational decision-making.
/// GeJu evaluation determines which gates open or close.
#[derive(Clone)]
pub struct HumanPlate {
    pub gates: [GateState; 8],
    pub permissions: Arc<PermissionMatrix>,
    pub pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
}

/// 人盘分发错误
#[derive(Debug, Clone)]
pub enum DispatchError {
    Denied(String),
    ToolError(String),
}

impl std::fmt::Display for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DispatchError::Denied(reason) => write!(f, "Execution denied: {reason}"),
            DispatchError::ToolError(msg) => write!(f, "Tool error: {msg}"),
        }
    }
}

impl HumanPlate {
    pub fn new(permissions: Arc<PermissionMatrix>) -> Self {
        Self {
            gates: [GateState::Open; 8],
            permissions,
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_state(
        permissions: Arc<PermissionMatrix>,
        pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    ) -> Self {
        Self {
            gates: [GateState::Open; 8],
            permissions,
            pending_confirmations,
        }
    }

    fn gate_is_open(&self, gate: HumanGate) -> bool {
        self.gates[gate as usize] == GateState::Open
    }

    /// 分发 (dispatch) — Execute a tool call through the permission boundary.
    ///
    /// The GeJuResult determines execution strategy:
    /// - Direct: immediate execution (requires JingXiangMen open)
    /// - Guarded: check approval chain, enforce permissions + confirmations
    /// - Sandbox: execute with sandboxed input (requires DuMen open)
    /// - Denied: reject with reason (may escalate via ShangMen)
    pub async fn dispatch(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<ToolResult, DispatchError> {
        match geju.execution_mode {
            ExecutionMode::Direct => {
                if !self.gate_is_open(HumanGate::JingXiangMen) {
                    // 景门闭 — downgrade to Guarded
                    tracing::warn!("HumanPlate: JingXiangMen closed, downgrading Direct→Guarded");
                    let guarded = GeJuResult {
                        execution_mode: ExecutionMode::Guarded,
                        ..geju.clone()
                    };
                    return self
                        .dispatch_guarded(&guarded, tool, input, event_bus, tx, exec_ctx)
                        .await;
                }
                let output = tool
                    .execute_with_tx(input.clone(), tx, exec_ctx)
                    .await
                    .map_err(DispatchError::ToolError)?;
                Ok(ToolResult {
                    call_id: String::new(),
                    output,
                    error: None,
                })
            }
            ExecutionMode::Guarded => {
                self.dispatch_guarded(geju, tool, input, event_bus, tx, exec_ctx)
                    .await
            }
            ExecutionMode::Sandbox => {
                self.dispatch_sandbox(geju, tool, input, event_bus, tx, exec_ctx)
                    .await
            }
            ExecutionMode::Denied => {
                // 死门 — reject. Check ShangMen for potential escalation.
                if self.gate_is_open(HumanGate::ShangMen) {
                    tracing::warn!(
                        "HumanPlate: ShangMen open, escalating Denied→Guarded for {}",
                        tool.name()
                    );
                    let guarded = GeJuResult {
                        execution_mode: ExecutionMode::Guarded,
                        approval_chain: vec![ApprovalGate::UserConfirmation(format!(
                            "This operation ({}) was flagged as high-risk (geju: {}). Proceed?",
                            tool.name(),
                            geju.name,
                        ))],
                        ..geju.clone()
                    };
                    return self
                        .dispatch_guarded(&guarded, tool, input, event_bus, tx, exec_ctx)
                        .await;
                }
                event_bus.emit(RuntimeEvent::Error {
                    source: "human_plate".into(),
                    message: format!("Denied: {} (geju: {})", tool.name(), geju.name),
                });
                Err(DispatchError::Denied(geju.name.clone()))
            }
        }
    }

    /// Handle Guarded execution with active approval chain enforcement.
    async fn dispatch_guarded(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<ToolResult, DispatchError> {
        // Check ShangMen gate for destructive actions
        if !self.gate_is_open(HumanGate::ShangMen) && tool.is_destructive() {
            tracing::warn!(
                "HumanPlate: ShangMen closed, blocking destructive tool {}",
                tool.name()
            );
            return Err(DispatchError::Denied(format!(
                "Destructive tool '{}' blocked: ShangMen is closed",
                tool.name()
            )));
        }

        for gate in &geju.approval_chain {
            match gate {
                ApprovalGate::Permission(rule) => {
                    tracing::info!(
                        "HumanPlate: permission check '{}' for {}",
                        rule,
                        tool.name()
                    );
                    // Rule-based permission: currently permissive, extensible
                    // Layer 4 AddGuard principles can inject specific rules
                    if rule.contains("deny") {
                        return Err(DispatchError::Denied(format!(
                            "Permission rule denied: {rule}"
                        )));
                    }
                }
                ApprovalGate::UserConfirmation(reason) => {
                    tracing::info!(
                        "HumanPlate: requesting user confirmation for {}",
                        tool.name()
                    );
                    let approved = self.request_confirmation(tool.name(), reason, tx).await;
                    if !approved {
                        event_bus.emit(RuntimeEvent::Error {
                            source: "human_plate".into(),
                            message: format!("User denied: {} (reason: {})", tool.name(), reason,),
                        });
                        return Err(DispatchError::Denied(format!(
                            "User denied confirmation for {}: {reason}",
                            tool.name(),
                        )));
                    }
                }
                ApprovalGate::SandboxIsolation => {
                    // Escalate to Sandbox mode
                    tracing::info!("HumanPlate: escalating to Sandbox for {}", tool.name());
                    let sandbox_geju = GeJuResult {
                        execution_mode: ExecutionMode::Sandbox,
                        ..geju.clone()
                    };
                    return Box::pin(self.dispatch_sandbox(
                        &sandbox_geju,
                        tool,
                        input,
                        event_bus,
                        tx,
                        exec_ctx,
                    ))
                    .await;
                }
                ApprovalGate::CodeReview => {
                    // Phase 5: log and auto-approve (full code review is Phase 6+)
                    tracing::info!(
                        "HumanPlate: code review required for {} (auto-approving)",
                        tool.name()
                    );
                }
            }
        }

        // All gates passed — execute
        let output = tool
            .execute_with_tx(input.clone(), tx, exec_ctx)
            .await
            .map_err(DispatchError::ToolError)?;
        Ok(ToolResult {
            call_id: String::new(),
            output,
            error: None,
        })
    }

    /// Handle Sandbox execution with path confinement.
    async fn dispatch_sandbox(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<ToolResult, DispatchError> {
        // Check DuMen gate
        if !self.gate_is_open(HumanGate::DuMen) || self.permissions.sandbox_disabled {
            tracing::warn!(
                "HumanPlate: DuMen closed or sandbox disabled, downgrading Sandbox→Guarded for {}",
                tool.name()
            );
            let guarded = GeJuResult {
                execution_mode: ExecutionMode::Guarded,
                approval_chain: vec![ApprovalGate::UserConfirmation(format!(
                    "Sandbox is unavailable for {}. Proceed without isolation?",
                    tool.name(),
                ))],
                ..geju.clone()
            };
            return Box::pin(self.dispatch_guarded(&guarded, tool, input, event_bus, tx, exec_ctx))
                .await;
        }

        // Apply sandbox transformations
        let sandboxed = self
            .permissions
            .sandbox_input(tool.name(), &input)
            .map_err(|e| DispatchError::Denied(format!("Sandbox rejected: {e}")))?;

        tracing::info!(
            "HumanPlate: sandbox execution for {} (geju: {})",
            tool.name(),
            geju.name,
        );

        let output = tool
            .execute_with_tx(sandboxed, tx, exec_ctx)
            .await
            .map_err(DispatchError::ToolError)?;
        Ok(ToolResult {
            call_id: String::new(),
            output,
            error: None,
        })
    }

    /// Request user confirmation via SSE and await response.
    /// Returns true if approved, false if denied or timed out.
    async fn request_confirmation(
        &self,
        tool_name: &str,
        reason: &str,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    ) -> bool {
        let id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let timeout_secs = self.permissions.confirmation_timeout.as_secs();

        let (oneshot_tx, oneshot_rx) = tokio::sync::oneshot::channel();

        // Store the sender so /confirm endpoint can resolve it
        self.pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.clone(),
                PendingConfirmation {
                    sender: oneshot_tx,
                    token: token.clone(),
                },
            );

        // Emit to SSE channel so client shows the prompt
        let _ = tx.send(AgentEvent::ConfirmRequest {
            id: id.clone(),
            tool: tool_name.into(),
            reason: reason.into(),
            timeout_secs,
            token,
        });

        // Await response with timeout
        match tokio::time::timeout(self.permissions.confirmation_timeout, oneshot_rx).await {
            Ok(Ok(true)) => {
                tracing::info!("HumanPlate: user approved {tool_name}");
                true
            }
            Ok(Ok(false)) | Ok(Err(_)) => {
                tracing::warn!("HumanPlate: user denied {tool_name}");
                false
            }
            Err(_elapsed) => {
                // Clean up the stale entry
                self.pending_confirmations
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                tracing::warn!("HumanPlate: confirmation timed out for {tool_name}");
                false
            }
        }
    }
}

impl Default for HumanPlate {
    fn default() -> Self {
        Self::new(Arc::new(PermissionMatrix::default()))
    }
}

/// 八门 — Eight human interaction gates.
/// ShangMen/DuMen/JingXiangMen active in production; remainder reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HumanGate {
    XiuMen,       // 休门 — Rest/idle/listen
    ShengMen,     // 生门 — Skill injection/growth
    ShangMen,     // 伤门 — Destructive action interception
    DuMen,        // 杜门 — Sandbox isolation
    JingXiangMen, // 景门 — UI rendering/result display
    SiMen,        // 死门 — Audit log/immutable record
    JingJueMen,   // 惊门 — Alert notification
    KaiMen,       // 开门 — API open communication
}

/// Gate open/close state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateState {
    Open,
    Closed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geju::ExecutionMode;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::plates::shen_spirit::EventBus;
    use std::sync::Arc;

    struct EchoTool;
    #[async_trait::async_trait]
    impl BaseTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> String {
            "echoes input".to_string()
        }
        fn ceremony(&self) -> crate::stems::CeremoniesIntent {
            crate::stems::CeremoniesIntent::Wu(crate::stems::intent::ReadAction {
                target: String::new(),
            })
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        fn is_concurrency_safe(&self) -> bool {
            true
        }
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ExecContext,
        ) -> Result<String, String> {
            Ok(format!("echo: {}", input))
        }
    }

    struct DestructiveTool;
    #[async_trait::async_trait]
    impl BaseTool for DestructiveTool {
        fn name(&self) -> &str {
            "shell"
        }
        fn description(&self) -> String {
            "executes commands".to_string()
        }
        fn ceremony(&self) -> crate::stems::CeremoniesIntent {
            crate::stems::CeremoniesIntent::Geng(crate::stems::intent::ExecAction {
                command: String::new(),
            })
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"cmd": {"type": "string"}}})
        }
        fn is_concurrency_safe(&self) -> bool {
            false
        }
        async fn execute(
            &self,
            input: serde_json::Value,
            _ctx: &ExecContext,
        ) -> Result<String, String> {
            Ok(format!("exec: {}", input))
        }
    }

    fn make_geju(mode: ExecutionMode) -> GeJuResult {
        GeJuResult {
            name: "test".into(),
            execution_mode: mode,
            requires_audit: false,
            max_retries: 1,
            approval_chain: vec![],
            layer: 3,
        }
    }

    fn make_ctx() -> ExecContext {
        ExecContext {
            permissions: Arc::new(PermissionMatrix::default()),
        }
    }

    fn make_plate() -> (
        HumanPlate,
        EventBus,
        tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    ) {
        let plate = HumanPlate::default();
        let eb = EventBus::new();
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        (plate, eb, tx)
    }

    #[tokio::test]
    async fn dispatch_direct() {
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Direct);
        let result = plate
            .dispatch(
                &geju,
                &tool,
                serde_json::json!({"msg": "hi"}),
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().output.contains("echo"));
    }

    #[tokio::test]
    async fn dispatch_denied() {
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Denied);
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DispatchError::Denied(_)));
    }

    #[tokio::test]
    async fn dispatch_guarded() {
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::Permission("test_perm".into())];
        let result = plate
            .dispatch(
                &geju,
                &tool,
                serde_json::json!({"x": 1}),
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn dispatch_sandbox() {
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Sandbox);
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(result.is_ok());
    }

    // ── 八门 (8 Gates) interaction tests ────────────────────

    #[tokio::test]
    async fn direct_downgrades_when_jingxiangmen_closed() {
        let (mut plate, eb, tx) = make_plate();
        plate.gates[HumanGate::JingXiangMen as usize] = GateState::Closed;
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Direct);
        let result = plate
            .dispatch(
                &geju,
                &tool,
                serde_json::json!({"x": 1}),
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        // Should still work — downgrades to Guarded
        assert!(
            result.is_ok(),
            "JingXiangMen closed should downgrade Direct→Guarded: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn denied_escalates_when_shangmen_open() {
        let (plate, eb, tx) = make_plate();
        // ShangMen is Open by default — Denied should escalate to Guarded+UserConfirmation
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Denied);
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        // Escalation to Guarded with UserConfirmation → waits for confirm → times out → denied
        assert!(
            result.is_err(),
            "Should be denied after confirmation timeout"
        );
    }

    #[tokio::test]
    async fn denied_stays_denied_when_shangmen_closed() {
        let (mut plate, eb, tx) = make_plate();
        plate.gates[HumanGate::ShangMen as usize] = GateState::Closed;
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Denied);
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DispatchError::Denied(_)));
    }

    #[tokio::test]
    async fn guarded_blocks_destructive_when_shangmen_closed() {
        let (mut plate, eb, tx) = make_plate();
        plate.gates[HumanGate::ShangMen as usize] = GateState::Closed;
        let tool: Arc<dyn BaseTool> = Arc::new(DestructiveTool);
        let geju = make_geju(ExecutionMode::Guarded);
        let result = plate
            .dispatch(
                &geju,
                &tool,
                serde_json::json!({"cmd": "rm"}),
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        assert!(
            result.is_err(),
            "Destructive tool should be blocked with ShangMen closed"
        );
    }

    #[tokio::test]
    async fn guarded_read_is_allowed_with_shangmen_closed() {
        let (mut plate, eb, tx) = make_plate();
        plate.gates[HumanGate::ShangMen as usize] = GateState::Closed;
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool); // read_file-like (Harmless read)
        let geju = make_geju(ExecutionMode::Guarded);
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        // read_file is exempt from ShangMen check
        assert!(
            result.is_ok(),
            "Read-like tool should pass with ShangMen closed: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn guarded_deny_permission_rule() {
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::Permission("deny_all".into())];
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(result.is_err(), "Permission rule with 'deny' should block");
    }

    #[tokio::test]
    async fn sandbox_downgrades_when_dumen_closed() {
        let (mut plate, eb, tx) = make_plate();
        plate.gates[HumanGate::DuMen as usize] = GateState::Closed;
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Sandbox);
        // DuMen closed → downgrade Sandbox→Guarded+UserConfirmation → times out → denied
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(
            result.is_err(),
            "Sandbox with DuMen closed should result in denial after timeout: {:?}",
            result.ok()
        );
    }

    #[tokio::test]
    async fn approval_chain_sandbox_isolation_escalates() {
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::SandboxIsolation];
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        // Escalates from Guarded→Sandbox, which then executes directly
        assert!(
            result.is_ok(),
            "SandboxIsolation escalation should work: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn gate_initial_state_all_open() {
        let plate = HumanPlate::default();
        for i in 0..8 {
            assert_eq!(
                plate.gates[i],
                GateState::Open,
                "Gate {} should be Open by default",
                i
            );
        }
    }

    #[tokio::test]
    async fn explicit_deny_no_escalation() {
        // ShangMen is open, but the approval chain has an explicit deny Permission
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::Permission("deny_explicitly".into())];
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(result.is_err());
    }
}

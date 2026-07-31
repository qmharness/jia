//! ren_human — Human Plate / Permission Boundary (人盘)

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::geju::{ApprovalGate, ExecutionMode, GeJuResult};
use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::qian_permission::policy::{ChainVerdict, approval_key};
use crate::palaces::zhen_tool::base::BaseTool;
use crate::plates::shen_spirit::{EventBus, RuntimeEvent};
use crate::stems::AgentEvent;
use crate::stems::action::ExecContext;
use crate::stems::action::ToolResult;

pub mod session_bus;

pub use session_bus::{PendingQuestion, SessionBus, SteerMessage, SteerPriority};

/// A pending user confirmation, stored until resolved or timed out.
pub struct PendingConfirmation {
    pub sender: tokio::sync::oneshot::Sender<bool>,
    pub created_at: i64,
    pub token: String,
    /// 所属会话 — 断连时按会话清扫(rin 连接结束 → remove → sender drop)。
    /// 空串 = 无会话归属(如 resolve_workspace 的建项确认,靠超时兜底)。
    pub session_id: String,
}

/// 人盘 (Human Plate) — Permission boundary and human interaction gate.
///
/// Implements 八门 (8 Gates) for operational decision-making.
/// GeJu evaluation determines which gates open or close.
pub struct HumanPlate {
    pub gates: [GateState; 8],
    /// Session-scoped gate closings by Layer 4 principles (not persisted).
    /// Bit N = gate N is force-closed. Reset on new session.
    pub closed_by_principle: AtomicU8,
    pub permissions: Arc<PermissionMatrix>,
    pub pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    /// N1 · 会话级批准记忆(共享自 SessionBus):session_id → 批准键集。
    /// 只豁免"询问",绝不豁免任何拒绝类策略;GeJu 结果不受其影响。
    pub session_approvals: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    /// Test-only: when set, `request_confirmation` returns this value immediately.
    #[doc(hidden)]
    pub confirmation_override: Option<bool>,
}

impl Clone for HumanPlate {
    fn clone(&self) -> Self {
        Self {
            gates: self.gates,
            closed_by_principle: AtomicU8::new(self.closed_by_principle.load(Ordering::Relaxed)),
            permissions: self.permissions.clone(),
            pending_confirmations: self.pending_confirmations.clone(),
            session_approvals: self.session_approvals.clone(),
            confirmation_override: self.confirmation_override,
        }
    }
}

pub use crate::error::DispatchError;

/// A tool call that has passed ALL HumanPlate gates (policy chain, 八门,
/// approval chain, user confirmations, sandbox transform) and is cleared for
/// execution (U1).
///
/// Produced SERIALLY by [`HumanPlate::prepare`] — one per call, before the
/// call's batch is dispatched. Only [`PreparedCall::execute`] may run
/// concurrently with other prepared calls; it carries no `&HumanPlate` and
/// touches no gate state (公理 3: 门禁逐调用、派发前完成).
pub struct PreparedCall {
    tool: Arc<dyn BaseTool>,
    input: serde_json::Value,
}

impl PreparedCall {
    /// Execute the cleared call. This is the ONLY step the Heaven Plate may
    /// run in parallel across a non-conflicting batch.
    pub async fn execute(
        self,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<ToolResult, DispatchError> {
        let output = self
            .tool
            .execute_with_tx(self.input, tx, exec_ctx)
            .await
            .map_err(|e| DispatchError::ToolError(e.to_string()))?;
        Ok(ToolResult {
            call_id: String::new(),
            output,
            error: None,
        })
    }
}

impl HumanPlate {
    /// 以共享会话总线构造 — 确认表取自 bus,与人盘内聚(P2-1:
    /// pending_confirmations 已迁人盘 SessionBus)。
    pub fn with_state(permissions: Arc<PermissionMatrix>, session_bus: Arc<SessionBus>) -> Self {
        Self {
            gates: [GateState::Open; 8],
            closed_by_principle: AtomicU8::new(0),
            permissions,
            pending_confirmations: session_bus.pending_confirmations.clone(),
            session_approvals: session_bus.session_approvals.clone(),
            confirmation_override: None,
        }
    }

    /// Check if an alert should be escalated to the user.
    /// JingJueMen (惊门) closed → suppress alerts (e.g. during Planning mode).
    pub fn should_escalate_alert(&self) -> bool {
        self.gate_is_open(HumanGate::JingJueMen)
    }

    /// Sync JingJueMen with InteractionMode.
    /// Planning → Closed (suppress noise), Normal → Open (notify user).
    pub fn sync_jingjue_with_mode(&self, planning: bool) {
        let bit = 1u8 << (HumanGate::JingJueMen as u8);
        if planning {
            self.closed_by_principle.fetch_or(bit, Ordering::Relaxed);
        } else {
            self.closed_by_principle.fetch_and(!bit, Ordering::Relaxed);
        }
    }

    /// Close a gate for the remainder of this session (not persisted).
    /// Called by the agent loop when Layer 4 principles detect anomaly patterns.
    pub fn close_gate(&self, gate: HumanGate) {
        let bit = 1u8 << (gate as u8);
        let prev = self.closed_by_principle.fetch_or(bit, Ordering::Relaxed);
        if prev & bit == 0 {
            tracing::warn!(gate = ?gate, "HumanPlate: gate force-closed by principle (session-scoped)");
        }
    }

    /// Check if a gate is open, considering both config state and session-closed state.
    pub fn gate_is_open(&self, gate: HumanGate) -> bool {
        let bit = 1u8 << (gate as u8);
        self.gates[gate as usize] == GateState::Open
            && (self.closed_by_principle.load(Ordering::Relaxed) & bit) == 0
    }

    /// 分发 (dispatch) — Execute a tool call through the permission boundary.
    ///
    /// The GeJuResult determines execution strategy:
    /// - Direct: immediate execution (requires JingXiangMen open)
    /// - Guarded: check approval chain, enforce permissions + confirmations
    /// - Sandbox: execute with sandboxed input (requires DuMen open)
    /// - Denied: reject with reason (may escalate via ShangMen)
    ///
    /// N1 · 策略链(见 qian_permission::policy 模块头顺序表):deny 规则在
    /// 八门分发之前绝对优先;敏感文件强制 ask 为单向收紧(Direct/Sandbox →
    /// Guarded+确认),GeJu 评估本身不受影响。
    ///
    /// U1: this is `prepare` (mode determination + confirmations) followed by
    /// `PreparedCall::execute`. The agent loop calls the phases separately:
    /// `prepare` runs SERIALLY per call before a batch is dispatched; only
    /// `execute` runs concurrently (公理 3).
    pub async fn dispatch(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<ToolResult, DispatchError> {
        let prepared = self
            .prepare(geju, tool, input, event_bus, tx, exec_ctx)
            .await?;
        prepared.execute(tx, exec_ctx).await
    }

    /// 分发模式判定 (U1) — everything EXCEPT tool execution: policy chain,
    /// gate checks, mode downgrades/escalations, sandbox input transform and
    /// user confirmations. Per-call and side-effect-free w.r.t. batch state.
    /// Returns a [`PreparedCall`] cleared for execution, or a denial.
    pub async fn prepare(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<PreparedCall, DispatchError> {
        // N1 · 策略链位次 1/5:deny 规则绝对优先(无任何豁免,含 ShangMen
        // 升级路径);敏感文件强制 ask(收紧,Denied 不再加码)。
        match self.permissions.chain_check(tool.name(), &input) {
            ChainVerdict::Deny { policy, reason } => {
                tracing::warn!(tool = tool.name(), policy, reason = %reason, "HumanPlate: denied by policy chain");
                event_bus.emit(RuntimeEvent::PermissionDecision {
                    tool: tool.name().into(),
                    decision: "deny".into(),
                    policy: policy.into(),
                    reason: reason.clone(),
                });
                return Err(DispatchError::Denied(reason));
            }
            ChainVerdict::Ask { policy, reason }
                if geju.execution_mode != ExecutionMode::Denied =>
            {
                tracing::info!(tool = tool.name(), policy, reason = %reason, "HumanPlate: policy chain forces confirmation");
                event_bus.emit(RuntimeEvent::PermissionDecision {
                    tool: tool.name().into(),
                    decision: "ask".into(),
                    policy: policy.into(),
                    reason: reason.clone(),
                });
                let mut approval_chain = geju.approval_chain.clone();
                if !approval_chain
                    .iter()
                    .any(|g| matches!(g, ApprovalGate::UserConfirmation(_)))
                {
                    approval_chain.push(ApprovalGate::UserConfirmation(reason));
                }
                let guarded = GeJuResult {
                    execution_mode: ExecutionMode::Guarded,
                    approval_chain,
                    ..geju.clone()
                };
                return self
                    .prepare_guarded(&guarded, tool, input, event_bus, tx, exec_ctx)
                    .await;
            }
            _ => {}
        }

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
                        .prepare_guarded(&guarded, tool, input, event_bus, tx, exec_ctx)
                        .await;
                }
                Ok(PreparedCall {
                    tool: tool.clone(),
                    input,
                })
            }
            ExecutionMode::Guarded => {
                self.prepare_guarded(geju, tool, input, event_bus, tx, exec_ctx)
                    .await
            }
            ExecutionMode::Sandbox => {
                self.prepare_sandbox(geju, tool, input, event_bus, tx, exec_ctx)
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
                        .prepare_guarded(&guarded, tool, input, event_bus, tx, exec_ctx)
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

    /// Handle Guarded mode determination with approval chain enforcement.
    async fn prepare_guarded(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<PreparedCall, DispatchError> {
        // Check ShangMen for destructive actions
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
        // Check KaiMen for external communication tools
        if !self.gate_is_open(HumanGate::KaiMen)
            && matches!(tool.ceremony(), crate::stems::CeremoniesIntent::Ren)
        {
            tracing::warn!(
                "HumanPlate: KaiMen closed, blocking communication tool {}",
                tool.name()
            );
            return Err(DispatchError::Denied(format!(
                "Communication tool '{}' blocked: KaiMen is closed",
                tool.name()
            )));
        }
        // Check ShengMen for skill injection
        if !self.gate_is_open(HumanGate::ShengMen) && tool.name() == "skill" {
            tracing::warn!("HumanPlate: ShengMen closed, blocking skill tool");
            return Err(DispatchError::Denied(
                "Skill tool blocked: ShengMen is closed".into(),
            ));
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
                    // N1 · 策略链位次 3:会话级批准记忆。同会话同"工具+入参"
                    // 此前已获用户批准 → 豁免本次询问。红线:这只是"用户主动
                    // 批准的记忆化"——首次仍须询问,绝不自动放行;记忆只跳过
                    // 询问动作,不改变 GeJu 结果,也不豁免任何拒绝类策略。
                    let key = approval_key(tool.name(), &input);
                    let already_approved = {
                        let map = self
                            .session_approvals
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        map.get(&exec_ctx.session_id)
                            .is_some_and(|keys| keys.contains(&key))
                    };
                    if already_approved {
                        tracing::info!(
                            "HumanPlate: session approval memory hit for {} ({key})",
                            tool.name()
                        );
                        event_bus.emit(RuntimeEvent::PermissionDecision {
                            tool: tool.name().into(),
                            decision: "allow".into(),
                            policy: "session_approval".into(),
                            reason: "approved earlier in this session".into(),
                        });
                        continue;
                    }
                    tracing::info!(
                        "HumanPlate: requesting user confirmation for {}",
                        tool.name()
                    );
                    let approved = self
                        .request_confirmation(tool.name(), reason, tx, exec_ctx)
                        .await;
                    if !approved {
                        event_bus.emit(RuntimeEvent::PermissionDecision {
                            tool: tool.name().into(),
                            decision: "deny".into(),
                            policy: "user_confirmation".into(),
                            reason: reason.clone(),
                        });
                        event_bus.emit(RuntimeEvent::Error {
                            source: "human_plate".into(),
                            message: format!("User denied: {} (reason: {})", tool.name(), reason,),
                        });
                        return Err(DispatchError::Denied(format!(
                            "User denied confirmation for {}: {reason}",
                            tool.name(),
                        )));
                    }
                    // 用户批准出口:记录批准记忆(仅本会话、仅该"工具+入参")。
                    {
                        let mut map = self
                            .session_approvals
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        map.entry(exec_ctx.session_id.clone())
                            .or_default()
                            .insert(key);
                    }
                    event_bus.emit(RuntimeEvent::PermissionDecision {
                        tool: tool.name().into(),
                        decision: "allow".into(),
                        policy: "user_confirmation".into(),
                        reason: reason.clone(),
                    });
                }
                ApprovalGate::SandboxIsolation => {
                    // Escalate to Sandbox mode
                    tracing::info!("HumanPlate: escalating to Sandbox for {}", tool.name());
                    let sandbox_geju = GeJuResult {
                        execution_mode: ExecutionMode::Sandbox,
                        ..geju.clone()
                    };
                    return Box::pin(self.prepare_sandbox(
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

        // All gates passed — cleared for execution
        Ok(PreparedCall {
            tool: tool.clone(),
            input,
        })
    }

    /// Handle Sandbox mode determination with path confinement.
    async fn prepare_sandbox(
        &self,
        geju: &GeJuResult,
        tool: &Arc<dyn BaseTool>,
        input: serde_json::Value,
        event_bus: &EventBus,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> Result<PreparedCall, DispatchError> {
        // Check DuMen gate
        if !self.gate_is_open(HumanGate::DuMen)
            || matches!(
                self.permissions.sandbox_mode,
                crate::palaces::kun_config::SandboxMode::Disabled
            )
        {
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
            return Box::pin(self.prepare_guarded(&guarded, tool, input, event_bus, tx, exec_ctx))
                .await;
        }

        // Apply sandbox transformations
        let sandboxed = match self.permissions.sandbox_input(tool.name(), &input) {
            Ok(v) => v,
            Err(e) => {
                // N1 · 策略链位次 2/4:路径沙箱 / 命令策略拒绝 —— 决策可观测。
                event_bus.emit(RuntimeEvent::PermissionDecision {
                    tool: tool.name().into(),
                    decision: "deny".into(),
                    policy: if tool.name() == "shell" {
                        "command_policy"
                    } else {
                        "path_sandbox"
                    }
                    .into(),
                    reason: e.clone(),
                });
                return Err(DispatchError::Denied(format!("Sandbox rejected: {e}")));
            }
        };

        tracing::info!(
            "HumanPlate: sandbox execution for {} (geju: {})",
            tool.name(),
            geju.name,
        );

        Ok(PreparedCall {
            tool: tool.clone(),
            input: sandboxed,
        })
    }

    /// Request user confirmation via SSE and await response.
    /// Returns true if approved, false if denied, cancelled or timed out.
    async fn request_confirmation(
        &self,
        tool_name: &str,
        reason: &str,
        tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        exec_ctx: &ExecContext,
    ) -> bool {
        if let Some(v) = self.confirmation_override {
            return v;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let token = uuid::Uuid::new_v4().to_string();
        let timeout_secs = self.permissions.confirmation_timeout.as_secs();

        let (oneshot_tx, oneshot_rx) = tokio::sync::oneshot::channel();

        // Store the sender so /confirm endpoint can resolve it.
        // Clean up stale entries (>30 min) before inserting.
        let now = crate::utils::unix_now();
        let stale_cutoff = now - 1800;
        {
            let mut map = self
                .pending_confirmations
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            map.retain(|_, v| v.created_at > stale_cutoff);
            map.insert(
                id.clone(),
                PendingConfirmation {
                    sender: oneshot_tx,
                    token: token.clone(),
                    created_at: now,
                    session_id: exec_ctx.session_id.clone(),
                },
            );
        }

        // Emit to SSE channel so client shows the prompt
        let _ = tx.send(AgentEvent::ConfirmRequest {
            id: id.clone(),
            tool: tool_name.into(),
            reason: reason.into(),
            timeout_secs,
            token,
        });

        // Await response with timeout; wake early on run cancellation
        // (HTTP 取消 / SSE 断连 CancelOnDropStream / rin cancel)。
        tokio::select! {
            r = tokio::time::timeout(self.permissions.confirmation_timeout, oneshot_rx) => {
                match r {
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
            _ = exec_ctx.cancel_token.cancelled() => {
                self.pending_confirmations
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                tracing::warn!("HumanPlate: confirmation cancelled for {tool_name}");
                false
            }
        }
    }
}

impl Default for HumanPlate {
    fn default() -> Self {
        Self::with_state(
            Arc::new(PermissionMatrix::default()),
            Arc::new(SessionBus::new()),
        )
    }
}

/// 八门 — Eight human interaction gates.
/// ShangMen/DuMen/JingXiangMen active in production; remainder reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// SiMen + JingJueMen reserved for future wiring
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
    use crate::error::ToolError;
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
            crate::stems::CeremoniesIntent::Wu
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
        ) -> Result<String, ToolError> {
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
            crate::stems::CeremoniesIntent::Geng
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
        ) -> Result<String, ToolError> {
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
        ExecContext::new(Arc::new(PermissionMatrix::default()))
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
        let (mut plate, eb, tx) = make_plate();
        plate.confirmation_override = Some(false);
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

    /// P0-4 · 确认等待中取消 → 立即按拒绝返回(不等 confirmation_timeout),
    /// 且 pending_confirmations 无残留。
    #[tokio::test]
    async fn guarded_confirmation_cancelled_returns_denied() {
        let (plate, eb, tx) = make_plate();
        let pending = plate.pending_confirmations.clone();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::UserConfirmation("sure?".into())];
        let token = tokio_util::sync::CancellationToken::new();
        let mut ctx = make_ctx();
        ctx.cancel_token = token.clone();

        let handle = tokio::spawn(async move {
            plate
                .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &ctx)
                .await
        });

        // Wait until the confirmation is actually pending.
        for _ in 0..100 {
            if !pending.lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            !pending.lock().unwrap_or_else(|e| e.into_inner()).is_empty(),
            "confirmation never became pending"
        );

        token.cancel();

        let res = tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("confirmation wait must wake on cancel (deadlock!)")
            .unwrap();
        assert!(res.is_err(), "cancelled confirmation must deny the tool");
        assert!(
            pending.lock().unwrap_or_else(|e| e.into_inner()).is_empty(),
            "pending_confirmations must have no residue after cancel"
        );
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
        let (mut plate, eb, tx) = make_plate();
        plate.confirmation_override = Some(false);
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
        plate.confirmation_override = Some(false);
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

    // ── Scenario tests: GeJu evaluation through dispatch path ──

    #[tokio::test]
    async fn scenario_sandbox_executes_echo_tool() {
        // EchoTool (Wu/Read, non-destructive) in Sandbox mode should execute
        let (plate, eb, tx) = make_plate();
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Sandbox);
        let result = plate
            .dispatch(
                &geju,
                &tool,
                serde_json::json!({"msg": "hello"}),
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        assert!(result.is_ok(), "Sandbox should execute: {:?}", result.err());
    }

    #[tokio::test]
    async fn scenario_denied_mode_rejects_all_tools() {
        // Denied execution mode blocks even read-only tools
        let (mut plate, eb, tx) = make_plate();
        plate.confirmation_override = Some(false);
        // Close ShangMen so Denied stays Denied (no escalation)
        plate.gates[HumanGate::ShangMen as usize] = GateState::Closed;
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Denied);
        let result = plate
            .dispatch(&geju, &tool, serde_json::json!({}), &eb, &tx, &make_ctx())
            .await;
        assert!(result.is_err(), "Denied mode should reject all tools");
    }

    /// P2-1 结构断言:HumanPlate 与 SessionBus 共享同一份 pending_confirmations
    /// Arc(误改回独立 Arc 时确认会永远超时,编译与普通测试都发现不了)。
    #[test]
    fn with_state_shares_pending_confirmations_arc() {
        let bus = std::sync::Arc::new(crate::plates::ren_human::SessionBus::new());
        let plate = HumanPlate::with_state(
            std::sync::Arc::new(PermissionMatrix::default()),
            bus.clone(),
        );
        assert!(std::sync::Arc::ptr_eq(
            &plate.pending_confirmations,
            &bus.pending_confirmations
        ));
    }

    // ── N1 · 权限策略链 / 会话批准记忆 ─────────────────────

    /// N1 结构断言:HumanPlate 与 SessionBus 共享同一份 session_approvals Arc。
    #[test]
    fn with_state_shares_session_approvals_arc() {
        let bus = std::sync::Arc::new(crate::plates::ren_human::SessionBus::new());
        let plate = HumanPlate::with_state(
            std::sync::Arc::new(PermissionMatrix::default()),
            bus.clone(),
        );
        assert!(std::sync::Arc::ptr_eq(
            &plate.session_approvals,
            &bus.session_approvals
        ));
    }

    fn make_plate_with_rx() -> (
        HumanPlate,
        EventBus,
        tokio::sync::mpsc::UnboundedSender<AgentEvent>,
        tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    ) {
        let plate = HumanPlate::default();
        let eb = EventBus::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (plate, eb, tx, rx)
    }

    fn count_confirm_requests(rx: &mut tokio::sync::mpsc::UnboundedReceiver<AgentEvent>) -> usize {
        let mut n = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, AgentEvent::ConfirmRequest { .. }) {
                n += 1;
            }
        }
        n
    }

    /// 收集自订阅以来发出的 PermissionDecision 事件:(decision, policy)。
    fn drain_decision_events(
        rx: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>,
    ) -> Vec<(String, String)> {
        let mut out = vec![];
        while let Ok(ev) = rx.try_recv() {
            if let RuntimeEvent::PermissionDecision {
                decision, policy, ..
            } = ev
            {
                out.push((decision, policy));
            }
        }
        out
    }

    /// 批准记忆命中:同会话同"工具+入参"第二次不再询问;首次仍须询问。
    /// (confirmation_override 短路时不发 ConfirmRequest,故以 PermissionDecision
    /// 事件为观测面:user_confirmation = 真问了;session_approval = 记忆豁免。)
    #[tokio::test]
    async fn session_approval_memory_skips_repeat_confirmation() {
        let (mut plate, eb, tx, _rx) = make_plate_with_rx();
        let mut events = eb.subscribe();
        plate.confirmation_override = Some(true);
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::UserConfirmation("sure?".into())];
        let mut ctx = make_ctx();
        ctx.session_id = "s1".into();
        let input = serde_json::json!({"msg": "hi"});

        // 首次:必须询问(红线:绝不自动放行)
        let r1 = plate
            .dispatch(&geju, &tool, input.clone(), &eb, &tx, &ctx)
            .await;
        assert!(r1.is_ok(), "first dispatch: {:?}", r1.err());
        assert_eq!(
            drain_decision_events(&mut events),
            vec![("allow".to_string(), "user_confirmation".to_string())],
            "first call must go through a real confirmation"
        );

        // 第二次(同会话同入参):批准记忆命中,不再询问
        let r2 = plate.dispatch(&geju, &tool, input, &eb, &tx, &ctx).await;
        assert!(r2.is_ok(), "second dispatch: {:?}", r2.err());
        assert_eq!(
            drain_decision_events(&mut events),
            vec![("allow".to_string(), "session_approval".to_string())],
            "repeat call must skip confirmation via session approval memory"
        );
    }

    /// 批准记忆不命中:入参不同 → 仍须询问;会话不同 → 仍须询问。
    #[tokio::test]
    async fn session_approval_memory_miss_on_different_input_or_session() {
        let (mut plate, eb, tx, _rx) = make_plate_with_rx();
        let mut events = eb.subscribe();
        plate.confirmation_override = Some(true);
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::UserConfirmation("sure?".into())];
        let mut ctx = make_ctx();
        ctx.session_id = "s1".into();

        let _ = plate
            .dispatch(&geju, &tool, serde_json::json!({"msg": "a"}), &eb, &tx, &ctx)
            .await;
        // 入参不同 → 询问
        let _ = plate
            .dispatch(&geju, &tool, serde_json::json!({"msg": "b"}), &eb, &tx, &ctx)
            .await;
        // 会话不同(同入参)→ 询问
        ctx.session_id = "s2".into();
        let _ = plate
            .dispatch(&geju, &tool, serde_json::json!({"msg": "a"}), &eb, &tx, &ctx)
            .await;
        let decisions = drain_decision_events(&mut events);
        assert_eq!(
            decisions
                .iter()
                .filter(|(d, p)| d == "allow" && p == "user_confirmation")
                .count(),
            3,
            "different input and different session must each ask again: {decisions:?}"
        );
        assert!(
            !decisions.iter().any(|(_, p)| p == "session_approval"),
            "no memory hit expected: {decisions:?}"
        );
    }

    /// 公理 4 红线:批准记忆不改变 GeJu 结果 —— 批准前后同一操作的
    /// ExecutionMode 完全一致(记忆只豁免询问,不可能放松评估)。
    #[tokio::test]
    async fn approval_memory_does_not_change_geju_execution_mode() {
        let (mut plate, eb, tx, _rx) = make_plate_with_rx();
        plate.confirmation_override = Some(true);
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut ctx = make_ctx();
        ctx.session_id = "s1".into();
        let mut g = make_geju(ExecutionMode::Guarded);
        g.approval_chain = vec![ApprovalGate::UserConfirmation("sure?".into())];
        let input = serde_json::json!({"msg": "hi"});

        // 批准前的 GeJu 评估
        let eval_before = crate::geju::GeJu::new(crate::stems::Stem::Geng, crate::stems::Stem::Geng)
            .evaluate()
            .execution_mode;

        // 用户批准 → 记忆写入
        let _ = plate
            .dispatch(&g, &tool, input.clone(), &eb, &tx, &ctx)
            .await;
        let key = approval_key(tool.name(), &input);
        {
            let map = plate
                .session_approvals
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            assert!(map.get("s1").is_some_and(|keys| keys.contains(&key)));
        }

        // 批准后的 GeJu 评估:ExecutionMode 必须一致
        let eval_after = crate::geju::GeJu::new(crate::stems::Stem::Geng, crate::stems::Stem::Geng)
            .evaluate()
            .execution_mode;
        assert_eq!(
            eval_before, eval_after,
            "approval memory must not alter GeJu evaluation"
        );

        // 记忆命中只跳过询问:执行仍走 Guarded 路径(模式未被放松为 Direct)
        assert_eq!(g.execution_mode, ExecutionMode::Guarded);
    }

    fn matrix_with_deny_rules(rules: Vec<String>) -> PermissionMatrix {
        let workspace_root = std::env::current_dir().unwrap();
        PermissionMatrix {
            sandbox: crate::palaces::qian_permission::SandboxConfig {
                workspace_root: workspace_root.canonicalize().unwrap(),
                allowed_paths: vec![],
                blocked_prefixes: vec![".git".into(), ".env".into()],
            },
            shell_policy: crate::palaces::qian_permission::ShellPolicy {
                allowlist: vec![],
                blocklist: vec![],
            },
            deny_rules: rules,
            confirmation_timeout: std::time::Duration::from_secs(30),
            sandbox_mode: crate::palaces::kun_config::SandboxMode::Required,
            backup_dir: std::path::PathBuf::from(".jia/backups"),
            execution_sandbox: None,
        }
    }

    /// deny 规则绝对优先:Direct 模式直接拒;ShangMen 升级的 Denied→Guarded
    /// 路径也不可豁免;全程零询问。
    #[tokio::test]
    async fn deny_rule_is_absolute_no_exemption() {
        let plate = HumanPlate::with_state(
            Arc::new(matrix_with_deny_rules(vec!["Bash(rm *)".into()])),
            Arc::new(crate::plates::ren_human::SessionBus::new()),
        );
        let eb = EventBus::new();
        let mut events = eb.subscribe();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tool: Arc<dyn BaseTool> = Arc::new(DestructiveTool); // name = "shell"
        let input = serde_json::json!({"command": "rm -rf /tmp/x"});

        // Direct 模式也拒
        let r = plate
            .dispatch(
                &make_geju(ExecutionMode::Direct),
                &tool,
                input.clone(),
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        assert!(matches!(r, Err(DispatchError::Denied(_))));

        // Denied + ShangMen 开(默认开)也不得升级为询问
        let r2 = plate
            .dispatch(
                &make_geju(ExecutionMode::Denied),
                &tool,
                input,
                &eb,
                &tx,
                &make_ctx(),
            )
            .await;
        assert!(matches!(r2, Err(DispatchError::Denied(_))));
        assert_eq!(
            count_confirm_requests(&mut rx),
            0,
            "deny rule must never be exempted by a confirmation path"
        );

        // 决策事件已发出(deny + deny_rule)
        let mut saw = false;
        while let Ok(ev) = events.try_recv() {
            if let RuntimeEvent::PermissionDecision {
                decision, policy, ..
            } = ev
                && decision == "deny"
                && policy == "deny_rule"
            {
                saw = true;
            }
        }
        assert!(saw, "PermissionDecision(deny/deny_rule) event must be emitted");
    }

    /// 敏感文件强制 ask:Direct 模式被单向收紧为 Guarded+确认;
    /// 用户批准后执行,且批准记忆生效(第二次不再询问)。
    #[tokio::test]
    async fn sensitive_file_forces_confirmation_then_memory_applies() {
        let (mut plate, eb, tx, _rx) = make_plate_with_rx();
        let mut events = eb.subscribe();
        plate.confirmation_override = Some(true);
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let geju = make_geju(ExecutionMode::Direct);
        let mut ctx = make_ctx();
        ctx.session_id = "s1".into();
        let input = serde_json::json!({"path": "/work/.env"});

        let r1 = plate
            .dispatch(&geju, &tool, input.clone(), &eb, &tx, &ctx)
            .await;
        assert!(r1.is_ok(), "approved sensitive read: {:?}", r1.err());
        assert_eq!(
            drain_decision_events(&mut events),
            vec![
                ("ask".to_string(), "sensitive_file".to_string()),
                ("allow".to_string(), "user_confirmation".to_string()),
            ],
            "sensitive file must force ask even in Direct mode"
        );

        let r2 = plate.dispatch(&geju, &tool, input, &eb, &tx, &ctx).await;
        assert!(r2.is_ok());
        assert_eq!(
            drain_decision_events(&mut events),
            vec![
                ("ask".to_string(), "sensitive_file".to_string()),
                ("allow".to_string(), "session_approval".to_string()),
            ],
            "session approval memory covers the sensitive-file ask too"
        );
    }

    /// 决策可观测:Guarded 确认批准/拒绝均发出 PermissionDecision 事件。
    #[tokio::test]
    async fn user_confirmation_decision_events_emitted() {
        let (mut plate, eb, tx, _rx) = make_plate_with_rx();
        let mut events = eb.subscribe();
        plate.confirmation_override = Some(true);
        let tool: Arc<dyn BaseTool> = Arc::new(EchoTool);
        let mut geju = make_geju(ExecutionMode::Guarded);
        geju.approval_chain = vec![ApprovalGate::UserConfirmation("sure?".into())];
        let mut ctx = make_ctx();
        ctx.session_id = "s1".into();

        let _ = plate
            .dispatch(&geju, &tool, serde_json::json!({"m": 1}), &eb, &tx, &ctx)
            .await;

        plate.confirmation_override = Some(false);
        let _ = plate
            .dispatch(&geju, &tool, serde_json::json!({"m": 2}), &eb, &tx, &ctx)
            .await;

        let mut allow = false;
        let mut deny = false;
        while let Ok(ev) = events.try_recv() {
            if let RuntimeEvent::PermissionDecision {
                decision, policy, ..
            } = ev
                && policy == "user_confirmation"
            {
                if decision == "allow" {
                    allow = true;
                }
                if decision == "deny" {
                    deny = true;
                }
            }
        }
        assert!(allow && deny, "both allow and deny decisions must be observed");
    }
}

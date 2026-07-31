//! Tool dispatch phases (U1).
//!
//! Per call, in order:
//!   1. `gate_one_tool` — SERIAL, before the call enters a batch: failure
//!      streak refusal, unknown tool, 谋划态 short-circuit, GeJu evaluation
//!      (+ Layer 4 principle tightening), pre-tool user hooks, ZhiFu
//!      guarding hooks. GeJu stays a pure 干叠加 evaluator reading no batch
//!      state (公理 3).
//!   2. `HumanPlate::prepare` — SERIAL 分发模式判定 (policy chain, 八门,
//!      confirmations). Lives in ren_human.
//!   3. `PreparedCall::execute` — the ONLY step that may run concurrently
//!      across a non-conflicting batch (JoinSet in loop.rs).
//!   4. `finalize_outcome` — at the batch barrier, in declaration order:
//!      truncation (#10: oversized outputs spill to disk, retrievable via
//!      `retrieve_tool_result`), touched-seed extraction, ToolResult events,
//!      post hook.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::geju::{GeJu, GeJuResult};
use crate::palaces::Palace;
use crate::palaces::xun_context::ToolOutputBudget;
use crate::palaces::zhen_tool::base::{BaseTool, ToolAccesses};
use crate::palaces::zhen_tool::builtin::exec::disk_output::persist_tool_result;
use crate::plates::shen_spirit::hook::{
    HookEvent, HookRegistry, SpiritType, fire_guarding_hooks, fire_void_hooks,
};
use crate::plates::shen_spirit::{EventBus, RuntimeEvent};
use crate::stems::Stem;
use crate::stems::action::{ExecContext, ToolCall};
use crate::stems::{AgentEvent, InteractionMode};
use crate::stems::{CompiledHook, run_pre_tool_hooks};
use crate::telemetry::metrics::JIA_TOOL_DURATION_SECONDS;

/// Final per-call result, produced either by the gate phase (denials) or by
/// `finalize_outcome` at the batch barrier (executed / cancelled calls).
pub struct CallOutcome {
    pub output: String,
    pub error: Option<String>,
    pub geju_name: String,
    pub execution_mode: String,
    pub heaven_stem: Stem,
    pub target_palace: Palace,
    /// U1 sibling abort: the call was cancelled because a sibling in the
    /// same batch failed (or the run was cancelled mid-batch). 合成取消:
    /// NOT written to history and does not touch the failure streak
    /// (B3/U2 — 只保证计数/事件一致).
    pub synthetic_cancel: bool,
}

impl CallOutcome {
    fn gate_denied(
        error: String,
        geju_name: String,
        execution_mode: String,
        heaven_stem: Stem,
        target_palace: Palace,
    ) -> Self {
        Self {
            output: String::new(),
            error: Some(error),
            geju_name,
            execution_mode,
            heaven_stem,
            target_palace,
            synthetic_cancel: false,
        }
    }
}

/// Result of the serial gate phase.
pub enum GatedCall {
    /// Cleared through all pre-dispatch gates; ready for HumanPlate
    /// 分发模式判定 (`prepare`) and then execution.
    Cleared {
        tool: Arc<dyn BaseTool>,
        geju_result: GeJuResult,
        geju_name: String,
        execution_mode: String,
        heaven_stem: Stem,
        target_palace: Palace,
    },
    /// Finalized without execution (failure streak / unknown tool /
    /// 谋划态 / user hook / guarding hook). Events already emitted.
    Finished(CallOutcome),
}

/// A call that cleared BOTH the gate phase and HumanPlate 分发模式判定,
/// ready for (possibly concurrent) execution (U1 step 3 input).
pub struct PreparedExec {
    /// Position of this call within its batch (declaration order).
    pub index: usize,
    pub prepared: crate::plates::ren_human::PreparedCall,
    /// When the gate phase ended (dispatch start, for the duration metric).
    pub start: std::time::Instant,
    pub geju_name: String,
    pub execution_mode: String,
    pub heaven_stem: Stem,
    pub target_palace: Palace,
}

/// Serial gate phase for a single tool call (U1 step 1).
///
/// Runs BEFORE the call is dispatched into a batch — per call, in
/// declaration order. Everything here is unchanged from the legacy
/// sequential dispatch; only the execute step was split out.
#[tracing::instrument(skip(tc, tools, event_bus, hook_registry, user_hooks, tx, tool_failure_count), fields(tool = %tc.name))]
#[allow(clippy::too_many_arguments)]
pub async fn gate_one_tool(
    tc: &ToolCall,
    tools: &crate::palaces::zhen_tool::ToolRegistry,
    event_bus: &EventBus,
    hook_registry: &HookRegistry,
    tx: &mpsc::UnboundedSender<AgentEvent>,
    tool_failure_count: &std::collections::HashMap<String, u32>,
    max_consecutive_failures: u32,
    interaction_mode: InteractionMode,
    user_hooks: &[CompiledHook],
    principles: &[crate::principles::SystemPrinciple],
    atma_graha: f32,
) -> GatedCall {
    // GeJu Layer 3 runtime supplement: refuse tools with consecutive failure streak.
    if let Some(&count) = tool_failure_count.get(&tc.name)
        && count >= max_consecutive_failures
    {
        let err = format!(
            "Tool '{}' has failed {} consecutive times. \
             Consider: (1) check tool prerequisites, (2) use a different tool, \
             (3) simplify the input.",
            tc.name, count
        );
        tracing::warn!(tool = %tc.name, count = count, "gate_one_tool: refused by failure streak");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(err.clone()),
            geju: None,
            execution_mode: None,
        });
        return GatedCall::Finished(CallOutcome::gate_denied(
            err,
            String::new(),
            String::new(),
            Stem::Jia,
            Palace::Zhong,
        ));
    }

    let tool = match tools.get(&tc.name) {
        Some(t) => t.clone(),
        None => {
            let err = format!("Unknown tool: {}", tc.name);
            tracing::warn!(tool = %tc.name, "gate_one_tool: unknown tool");
            let _ = tx.send(AgentEvent::ToolCall {
                tool: tc.name.clone(),
                input: tc.parameters.clone(),
            });
            let _ = tx.send(AgentEvent::ToolResult {
                tool: tc.name.clone(),
                output: String::new(),
                error: Some(err.clone()),
                geju: None,
                execution_mode: None,
            });
            return GatedCall::Finished(CallOutcome::gate_denied(
                err,
                String::new(),
                String::new(),
                Stem::Jia,
                Palace::Zhong,
            ));
        }
    };

    let ceremony = tool.ceremony();
    let target_palace = tool.target_palace(&tc.parameters);
    let earth_stem = target_palace.stem();
    let heaven_stem = super::Agent::intent_stem_from_tool(&ceremony);

    // P3 · 谋划态 short-circuit (B3: loop-level, before GeJu). In Planning mode,
    // reject destructive tools so the agent stays read-only. This runs BEFORE
    // GeJu.evaluate so GeJu remains a pure 干叠加 evaluator (A2) — the planning
    // gate is a 人盘 concern, not a 格局 concern. enter/exit_plan_mode are
    // is_destructive()=false so they pass (D1: no self-deadlock).
    //
    // U4 · 子代理收紧(公理 4,只收紧):delegate/send_message 的 ceremony 为
    // 戊仪(只读型委派),但 Coder 子代理可写。谋划态下按名拦截——
    // delegate 参数请求 Coder(单个或 tasks[] 任一)即拒;send_message 可能
    // 续聊一个类型绑定的 Coder 会话,无法从入参判定,一律拒(退出谋划态后
    // 再续聊)。门禁与主循环同一代码点,杜绝经子代理绕过谋划态。
    let plan_blocked = tool.is_destructive()
        || tc.name == "send_message"
        || (tc.name == "delegate"
            && crate::palaces::zhen_tool::builtin::delegate::requests_coder(&tc.parameters));
    if interaction_mode == InteractionMode::Plan && plan_blocked {
        let err = format!(
            "【谋划态】当前为只读计划模式，变更类工具 '{}' 被拒。完成方案后用 exit_plan_mode 退出谋划态再执行。",
            tc.name
        );
        tracing::debug!(tool = %tc.name, "gate_one_tool: blocked in plan mode");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(err.clone()),
            geju: None,
            execution_mode: Some("planning_denied".to_string()),
        });
        return GatedCall::Finished(CallOutcome::gate_denied(
            err,
            String::new(),
            "planning_denied".to_string(),
            heaven_stem,
            target_palace,
        ));
    }

    let geju = GeJu::new(heaven_stem, earth_stem);
    let mut geju_result = geju.evaluate();

    // Layer 4 · 自进化 — tighten execution mode with system principles
    // (单向收紧: only escalate, never relax). Principles loaded at session start
    // from persisted store; matching is O(n) on the very small principles set.
    let geju_key = geju_result.name.clone();
    for p in principles.iter().filter(|p| p.geju_key == geju_key) {
        p.tighten(&mut geju_result, atma_graha);
    }

    let geju_name = geju_result.name.clone();
    let execution_mode = format!("{:?}", geju_result.execution_mode).to_lowercase();

    // P4 · 人盘门规 pre-tool hooks (B7+C3 gate order: Mou→GeJu→hook→execute).
    // Runs synchronously after GeJu and before dispatch. Only when GeJu did not
    // already deny. v1: Allow/Block only — no Inject (D2: would bypass GeJu).
    if geju_result.execution_mode != crate::geju::ExecutionMode::Denied
        && let Err(block_reason) = run_pre_tool_hooks(user_hooks, &tc.name, &tc.parameters).await
    {
        tracing::info!(tool = %tc.name, reason = %block_reason, "gate_one_tool: blocked by user hook");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(block_reason.clone()),
            geju: Some(geju_name.clone()),
            execution_mode: Some("hook_denied".to_string()),
        });
        return GatedCall::Finished(CallOutcome::gate_denied(
            block_reason,
            geju_name,
            "hook_denied".to_string(),
            heaven_stem,
            target_palace,
        ));
    }

    event_bus.emit(RuntimeEvent::GeJuResult {
        tool: tc.name.clone(),
        pattern: geju_name.clone(),
        mode: execution_mode.clone(),
    });

    let hook_event_d = HookEvent::ToolPreExecute {
        tool_name: tc.name.clone(),
        input: tc.parameters.clone(),
    };
    if let Some(reason) = fire_guarding_hooks(
        hook_registry,
        event_bus,
        SpiritType::ZhiFu,
        earth_stem,
        hook_event_d,
    )
    .await
    {
        let err = format!("Blocked by hook: {reason}");
        tracing::warn!(tool = %tc.name, reason = %reason, "gate_one_tool: blocked by guarding hook");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(err.clone()),
            geju: Some(geju_name.clone()),
            execution_mode: Some(execution_mode.clone()),
        });
        // Fire void hook even on cancel path
        fire_void_hooks(
            hook_registry,
            event_bus,
            SpiritType::ZhiFu,
            earth_stem,
            HookEvent::ToolPostExecute {
                tool_name: tc.name.clone(),
                output: String::new(),
                error: Some(err.clone()),
                duration_ms: 0,
            },
        );
        return GatedCall::Finished(CallOutcome::gate_denied(
            err,
            geju_name,
            execution_mode,
            heaven_stem,
            target_palace,
        ));
    }

    event_bus.emit(RuntimeEvent::ToolCall {
        tool: tc.name.clone(),
        input: tc.parameters.clone(),
    });
    tracing::info!(tool = %tc.name, "AgentEvent::ToolCall sent");
    let _ = tx.send(AgentEvent::ToolCall {
        tool: tc.name.clone(),
        input: tc.parameters.clone(),
    });

    GatedCall::Cleared {
        tool,
        geju_result,
        geju_name,
        execution_mode,
        heaven_stem,
        target_palace,
    }
}

/// Barrier post-processing for an executed (or sibling-cancelled) call
/// (U1 step 4): truncate the raw output, extract touched seed ids from the
/// RAW output (pre-truncation), observe the duration metric, emit ToolResult
/// events and the ZhiFu post-execute hook.
///
/// #10 · 超阈值落盘: 截断实际发生时,完整原始输出落盘到
/// `<workspace>/.jia/tool-results/<session_id>/<tool_call_id>.txt`
/// (复用 disk_output 的 O_EXCL+O_NOFOLLOW 安全写盘;内部路径,不走
/// verify_path,与 backups 同约定),返回内容追加取回指引。O_EXCL 天然
/// 按 tool_call_id 冻结:同一 id 只保留第一次落盘的内容。
///
/// 位识融合红线:落盘内容【不参与】熏习/召回 —— 工具结果 ≠ 记忆种子,
/// 仅由 retrieve_tool_result 按 id 定向取回;下方 touched-seed 提取仍走
/// 内存中的 raw_output,与落盘副本无关(落盘与种子分表)。
///
/// Called at the batch barrier strictly in declaration order, so history,
/// events and touched-seed accumulation are identical to sequential runs.
#[allow(clippy::too_many_arguments)]
pub fn finalize_outcome(
    tc: &ToolCall,
    raw_output: &str,
    outcome: CallOutcome,
    duration_ms: u64,
    touched_acc: &mut Vec<String>,
    output_budget: &ToolOutputBudget,
    exec_ctx: &ExecContext,
    event_bus: &EventBus,
    hook_registry: &HookRegistry,
    tx: &mpsc::UnboundedSender<AgentEvent>,
) -> CallOutcome {
    JIA_TOOL_DURATION_SECONDS
        .with_label_values(&[&tc.name])
        .observe(duration_ms as f64 / 1000.0);

    let mut output = output_budget.truncate_output(raw_output, &tc.name);

    // #10: spill the full raw output to disk when truncation actually
    // happened (truncate_output returns the input unchanged otherwise).
    if !raw_output.is_empty() && output != raw_output {
        match persist_tool_result(
            &exec_ctx.permissions.sandbox.workspace_root,
            &exec_ctx.session_id,
            &tc.id,
            raw_output,
        ) {
            Ok((path, size)) => {
                output.push_str(&format!(
                    "\n[完整输出已保存: {} ({} bytes). 用 retrieve_tool_result 按 tool_call_id \"{}\" 分段取回]",
                    path.display(),
                    size,
                    tc.id
                ));
            }
            Err(e) => {
                tracing::warn!(tool = %tc.name, id = %tc.id, error = %e, "tool result spill to disk failed");
            }
        }
    }

    // Extract touched seed IDs from raw output before truncation (e.g. namarupa query/save)
    if outcome.error.is_none()
        && !raw_output.is_empty()
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(raw_output)
        && let Some(ids) = val.get("touched_ids").and_then(|v| v.as_array())
    {
        touched_acc.extend(ids.iter().filter_map(|v| v.as_str().map(String::from)));
    }

    event_bus.emit(RuntimeEvent::ToolResult {
        tool: tc.name.clone(),
        output: output.clone(),
    });
    let _ = tx.send(AgentEvent::ToolResult {
        tool: tc.name.clone(),
        output: output.clone(),
        error: outcome.error.clone(),
        geju: Some(outcome.geju_name.clone()),
        execution_mode: Some(outcome.execution_mode.clone()),
    });
    tracing::info!(tool = %tc.name, output_len = output.len(), has_error = outcome.error.is_some(), "AgentEvent::ToolResult sent");

    fire_void_hooks(
        hook_registry,
        event_bus,
        SpiritType::ZhiFu,
        outcome.target_palace.stem(),
        HookEvent::ToolPostExecute {
            tool_name: tc.name.clone(),
            output: output.clone(),
            error: outcome.error.clone(),
            duration_ms,
        },
    );

    CallOutcome { output, ..outcome }
}

// ── U7 · 流式早派发 (streaming early dispatch) ─────────────────
//
// native tools 路径上,一个 tool call 在流式期间重组完成即做串行门禁
// (`gate_one_tool`)与 prepare(HumanPlate);资格条件全部满足才立即
// execute。公理 2/3:门禁仍逐调用、在执行前完成 —— 提前的只有"执行",
// 不是"决策"。公理 4:任一条件不满足即降级为流毕批处理(行为与 U1
// 完全一致)。XML `<tool_call>` fallback 不流式(流毕一次性解析),本
// 结构保持为空,批处理路径不受影响。
//
// 与 U1 批处理的一致性:
//   - sibling abort:首个早派发失败即取消在途早派发调用(合成取消,
//     不写 history、不计失败);后续流毕批照常。
//   - 屏障合并:早派发结果在流毕 drain 后按声明序 finalize(#10 截断
//     落盘同一函数),history/失败计数与流毕批在 Phase 4 统一按声明序合并。
//   - ExecContext:早派发任务持自有克隆(U1 的 B1 模式);期间不会发生
//     worktree swap —— enter/exit_worktree 声明 ToolAccesses::all,
//     结构上必然落在流毕批。

/// Per-call streaming-dispatch slot (parallel to the turn's native calls,
/// in declaration order).
pub enum EarlySlot {
    /// Gated during streaming; prepare + execute deferred to the
    /// post-stream batch (needs confirmation / All / write access /
    /// conflict / window full / earlier sibling failure).
    Gated {
        tool: Arc<dyn BaseTool>,
        geju_result: GeJuResult,
        geju_name: String,
        execution_mode: String,
        heaven_stem: Stem,
        target_palace: Palace,
    },
    /// Finalized during streaming (gate or prepare denial); events
    /// already emitted.
    Done(CallOutcome),
    /// Execute spawned during streaming; meta for the barrier finalize.
    Running {
        geju_name: String,
        execution_mode: String,
        heaven_stem: Stem,
        target_palace: Palace,
    },
    /// Consumed by the post-stream batch loop.
    Consumed,
}

/// Early-dispatch state for one LLM stream attempt (native path only).
pub struct EarlyDispatch {
    /// Per native call, in declaration order.
    pub slots: Vec<EarlySlot>,
    join_set: tokio::task::JoinSet<(usize, String, Option<String>, u64)>,
    /// Accesses of in-flight early calls (conflict-matrix input).
    in_flight: Vec<(usize, ToolAccesses)>,
    /// Results collected by `poll` (mid-stream) or `drain` (stream end).
    completed: std::collections::HashMap<usize, (String, Option<String>, u64)>,
    /// First early failure aborts in-flight siblings and disables further
    /// early dispatch for this stream (后续调用一律降级流毕批,照常执行).
    aborted: bool,
}

impl EarlyDispatch {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            join_set: tokio::task::JoinSet::new(),
            in_flight: Vec::new(),
            completed: std::collections::HashMap::new(),
            aborted: false,
        }
    }

    /// U7 资格条件 — 全部满足才允许流式期间立即 execute(公理 4:只收紧,
    /// 任一不满足即降级现行为):
    ///   1. 此前无早派发失败(sibling abort 后本流不再早派发);
    ///   2. `accesses()` 非 All 且【只读】(writes 为空 —— 对齐 kimi-code
    ///      参照系"仅只读类早派发";声明了写路径的调用等流毕批);
    ///   3. 与在途早派发调用无冲突(U1 冲突矩阵);
    ///   4. GeJu 直发(Direct)且景门开、策略链无强制 ask/deny —— 三者
    ///      合起来等价于"prepare 必走 Direct 快径、无用户确认",保证
    ///      需要确认的调用确认时序与现行为一致(一律流毕批);
    ///   5. 在途窗口未满(`JIA_MAX_TOOL_CONCURRENCY`,默认 10)。
    pub fn eligible(
        &self,
        accesses: &ToolAccesses,
        geju_mode: crate::geju::ExecutionMode,
        jingxiang_open: bool,
        chain: &crate::palaces::qian_permission::policy::ChainVerdict,
        max_conc: usize,
    ) -> bool {
        !self.aborted
            && !accesses.all
            && accesses.writes.is_empty()
            && geju_mode == crate::geju::ExecutionMode::Direct
            && jingxiang_open
            && matches!(
                chain,
                crate::palaces::qian_permission::policy::ChainVerdict::Pass
            )
            && self.join_set.len() < max_conc
            && !self
                .in_flight
                .iter()
                .any(|(_, a)| super::tool_scheduler::accesses_conflict(a, accesses))
    }

    /// Spawn an early execute on the JoinSet (U1 spawn_task 同款:任务内
    /// 计时,结果带回声明序索引). B1: `exec_ctx` is the task's own clone.
    pub fn spawn(
        &mut self,
        index: usize,
        prepared: crate::plates::ren_human::PreparedCall,
        exec_ctx: ExecContext,
        tx: mpsc::UnboundedSender<AgentEvent>,
        accesses: ToolAccesses,
    ) {
        self.in_flight.push((index, accesses));
        self.join_set.spawn(async move {
            let start = std::time::Instant::now();
            let res = prepared.execute(&tx, &exec_ctx).await;
            let duration = start.elapsed().as_millis() as u64;
            let (raw, error) = match res {
                Ok(tr) => (tr.output, tr.error),
                Err(crate::error::DispatchError::Denied(r))
                | Err(crate::error::DispatchError::ToolError(r)) => (String::new(), Some(r)),
            };
            (index, raw, error, duration)
        });
    }

    /// Non-blocking harvest of finished early tasks (called before each
    /// eligibility check: frees window slots and propagates sibling abort).
    pub fn poll(&mut self) {
        loop {
            match self.join_set.try_join_next() {
                Some(Ok((index, raw, error, duration))) => {
                    self.in_flight.retain(|(i, _)| *i != index);
                    if error.is_some() && !self.aborted {
                        self.aborted = true;
                        self.join_set.abort_all();
                        self.in_flight.clear();
                    }
                    self.completed.insert(index, (raw, error, duration));
                }
                None => break,
                Some(Err(e)) => {
                    // Aborted (sibling abort) or panicked — the call gets a
                    // synthetic cancel at the barrier (U1 同款).
                    if e.is_panic() {
                        tracing::error!(error = %e, "early tool task panicked (recorded as synthetic cancel)");
                    }
                }
            }
        }
    }

    /// Await all remaining early tasks at stream end (sibling abort on
    /// first error; honors the session cancel token — U1 drain 同款).
    pub async fn drain(&mut self, cancel_token: &CancellationToken) {
        while let Some(joined) = self.join_set.join_next().await {
            match joined {
                Ok((index, raw, error, duration)) => {
                    if error.is_some() && !self.aborted {
                        self.aborted = true;
                        self.join_set.abort_all();
                    }
                    self.completed.insert(index, (raw, error, duration));
                }
                Err(e) => {
                    if e.is_panic() {
                        tracing::error!(error = %e, "early tool task panicked (recorded as synthetic cancel)");
                    }
                }
            }
            if cancel_token.is_cancelled() && !self.aborted {
                self.aborted = true;
                self.join_set.abort_all();
            }
        }
        self.in_flight.clear();
    }

    /// Abort in-flight early calls WITHOUT accounting — the whole stream
    /// attempt is discarded (P0-3 retry path). Resets to empty state.
    pub async fn abort_and_reset(&mut self) {
        self.join_set.abort_all();
        while self.join_set.join_next().await.is_some() {}
        *self = Self::new();
    }

    /// Cancel path: abort in-flight calls; Running slots are turned into
    /// synthetic cancels by the caller (events only, no history).
    pub async fn abort(&mut self) {
        self.join_set.abort_all();
        while self.join_set.join_next().await.is_some() {}
        self.in_flight.clear();
    }

    pub fn take_completed(&mut self, index: usize) -> Option<(String, Option<String>, u64)> {
        self.completed.remove(&index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::kun_config::SecuritySection;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::palaces::xun_context::ToolOutputBudget;
    use std::collections::HashMap;

    fn test_ctx(root: &std::path::Path) -> ExecContext {
        let matrix = PermissionMatrix::from_config(
            &SecuritySection::default(),
            root,
            root.join(".jia/backups"),
        );
        let mut ctx = ExecContext::new(std::sync::Arc::new(matrix));
        ctx.session_id = "s1".to_string();
        ctx
    }

    fn tiny_budget() -> ToolOutputBudget {
        ToolOutputBudget {
            default_budget: 16,
            tool_budgets: HashMap::new(),
            char_fast_path_multiplier: 1,
        }
    }

    fn outcome_ok() -> CallOutcome {
        CallOutcome {
            output: String::new(),
            error: None,
            geju_name: "g".to_string(),
            execution_mode: "auto".to_string(),
            heaven_stem: Stem::Jia,
            target_palace: Palace::Zhong,
            synthetic_cancel: false,
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        ctx: ExecContext,
        bus: EventBus,
        hooks: HookRegistry,
        tx: mpsc::UnboundedSender<AgentEvent>,
        _rx: mpsc::UnboundedReceiver<AgentEvent>,
        budget: ToolOutputBudget,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let ctx = test_ctx(dir.path());
        let (tx, rx) = mpsc::unbounded_channel();
        Fixture {
            _dir: dir,
            ctx,
            bus: EventBus::new(),
            hooks: HookRegistry::new(),
            tx,
            _rx: rx,
            budget: tiny_budget(),
        }
    }

    fn persisted_path(f: &Fixture, id: &str) -> std::path::PathBuf {
        crate::palaces::zhen_tool::builtin::exec::disk_output::tool_result_path(
            &f.ctx.permissions.sandbox.workspace_root,
            "s1",
            id,
        )
    }

    /// U4 · 谋划态拦截(公理 4 只收紧):delegate 请求 Coder(单个或
    /// tasks[] 任一)与 send_message 在计划模式下按变更类拒绝;delegate
    /// 只读类型(Explore/Plan)不受影响。
    #[tokio::test]
    async fn plan_mode_blocks_delegate_coder_and_send_message() {
        use crate::palaces::zhen_tool::builtin::delegate::{DelegateTool, SendMessageTool};

        let mut tools = crate::palaces::zhen_tool::ToolRegistry::new();
        tools.register(Arc::new(DelegateTool::new()));
        tools.register(Arc::new(SendMessageTool::new()));
        tools.register(Arc::new(crate::palaces::zhen_tool::builtin::fs::read_file::ReadFileTool::new()));
        let bus = EventBus::new();
        let hooks = HookRegistry::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let failures: HashMap<String, u32> = HashMap::new();

        let gate = |name: &str, params: serde_json::Value| {
            let tc = ToolCall {
                id: "c1".to_string(),
                name: name.to_string(),
                parameters: params,
            };
            let tools = &tools;
            let bus = &bus;
            let hooks = &hooks;
            let tx = &tx;
            let failures = &failures;
            async move {
                gate_one_tool(
                    &tc,
                    tools,
                    bus,
                    hooks,
                    tx,
                    failures,
                    3,
                    InteractionMode::Plan,
                    &[],
                    &[],
                    0.5,
                )
                .await
            }
        };

        let planning_denied = |g: GatedCall| match g {
            GatedCall::Finished(o) => o.execution_mode == "planning_denied",
            GatedCall::Cleared { .. } => false,
        };

        // delegate + Coder → 拒
        assert!(planning_denied(
            gate("delegate", serde_json::json!({"subagent_type": "Coder", "prompt": "x"})).await
        ));
        // delegate + tasks[] 含 Coder → 拒
        assert!(planning_denied(
            gate("delegate", serde_json::json!({
                "tasks": [{"subagent_type": "Explore", "prompt": "a"},
                          {"subagent_type": "Coder", "prompt": "b"}]
            })).await
        ));
        // send_message(可能续聊 Coder 会话)→ 拒
        assert!(planning_denied(
            gate("send_message", serde_json::json!({"subagent_id": "s", "message": "m"})).await
        ));
        // delegate + 只读类型 → 不拦(过了谋划短路,后续 GeJu 照常)
        assert!(!planning_denied(
            gate("delegate", serde_json::json!({"subagent_type": "Explore", "prompt": "x"})).await
        ));
        // 只读工具 → 不拦
        assert!(!planning_denied(
            gate("read_file", serde_json::json!({"path": "x"})).await
        ));
    }

    #[test]
    fn oversized_output_spills_to_disk_with_retrieve_hint() {
        let f = fixture();
        let tc = ToolCall {
            id: "call_big".to_string(),
            name: "shell".to_string(),
            parameters: serde_json::json!({}),
        };
        let raw = "x".repeat(500);
        let mut touched = Vec::new();

        let out = finalize_outcome(
            &tc,
            &raw,
            outcome_ok(),
            1,
            &mut touched,
            &f.budget,
            &f.ctx,
            &f.bus,
            &f.hooks,
            &f.tx,
        );

        // Preview stays within the truncation budget shape (head+marker+tail).
        assert!(out.output.len() < raw.len(), "preview must be truncated");
        // 路径文案:落盘提示 + tool_call_id 取回指引。
        assert!(
            out.output.contains("完整输出已保存"),
            "notice: {}",
            out.output
        );
        assert!(
            out.output.contains("retrieve_tool_result"),
            "notice: {}",
            out.output
        );
        assert!(out.output.contains("call_big"), "notice: {}", out.output);

        // Full raw output persisted at the conventional path.
        let path = persisted_path(&f, "call_big");
        assert!(
            out.output.contains(&path.display().to_string()),
            "notice must carry the path: {}",
            out.output
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
    }

    #[test]
    fn spill_frozen_by_tool_call_id() {
        let f = fixture();
        let tc = ToolCall {
            id: "call_dup".to_string(),
            name: "shell".to_string(),
            parameters: serde_json::json!({}),
        };
        let mut touched = Vec::new();

        let first = finalize_outcome(
            &tc,
            &"a".repeat(500),
            outcome_ok(),
            1,
            &mut touched,
            &f.budget,
            &f.ctx,
            &f.bus,
            &f.hooks,
            &f.tx,
        );
        // Same id, different content: 冻结 — 不重复落盘,首份内容保留。
        let second = finalize_outcome(
            &tc,
            &"b".repeat(600),
            outcome_ok(),
            1,
            &mut touched,
            &f.budget,
            &f.ctx,
            &f.bus,
            &f.hooks,
            &f.tx,
        );

        let path = persisted_path(&f, "call_dup");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a".repeat(500));
        // The notice reports the frozen (first) byte count in both turns.
        assert!(first.output.contains("(500 bytes)"), "first: {}", first.output);
        assert!(
            second.output.contains("(500 bytes)"),
            "second: {}",
            second.output
        );
    }

    #[test]
    fn small_output_not_spilled() {
        let f = fixture();
        let tc = ToolCall {
            id: "call_small".to_string(),
            name: "shell".to_string(),
            parameters: serde_json::json!({}),
        };
        let mut touched = Vec::new();

        let out = finalize_outcome(
            &tc,
            "tiny",
            outcome_ok(),
            1,
            &mut touched,
            &f.budget,
            &f.ctx,
            &f.bus,
            &f.hooks,
            &f.tx,
        );

        assert_eq!(out.output, "tiny");
        assert!(!persisted_path(&f, "call_small").exists());
    }
}

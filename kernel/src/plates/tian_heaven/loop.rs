use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::certainty::{CertaintyParams, TurnCertainty};
use crate::geju::GeJu;
use crate::palaces::Palace;
use crate::palaces::xun_context::ContextWindow;
use crate::palaces::xun_context::handoff;
use crate::palaces::zhong_core::JiaCore;
use crate::palaces::zhong_core::backoff::RetryBackoff;
use crate::plates::ren_human::{HumanGate, HumanPlate};
use crate::plates::shen_spirit::hook::{HookEvent, HookRegistry, SpiritType, fire_void_hooks};
use crate::plates::shen_spirit::{EventBus, RuntimeEvent};
use crate::stems::Stem;
use crate::stems::action::ExecContext;
use crate::stems::parse_tool_calls;
use crate::stems::{AgentEvent, InteractionMode};
use crate::telemetry::metrics::{JIA_LLM_DURATION_SECONDS, JIA_TOKENS_COMPACTED_TOTAL};
use crate::types::{HistoryEntry, Message, Role, to_llm_messages};
use crate::vijnana::alaya::SeedStore;
use crate::vijnana::mano::TurnSnapshot;
use crate::vijnana::vasana::signal::SignalDetector;

// ── Re-exports from split submodules ────────────────────────────

pub use super::loop_dispatch::{
    CallOutcome, EarlyDispatch, EarlySlot, GatedCall, PreparedExec, finalize_outcome, gate_one_tool,
};

// ── RunContext ──────────────────────────────────────────────────

/// Bundled execution context for [`Agent::run`].
pub struct RunContext<'a> {
    pub core: &'a JiaCore,
    pub human_plate: &'a HumanPlate,
    pub event_bus: &'a EventBus,
    pub hook_registry: &'a HookRegistry,
    pub tx: mpsc::UnboundedSender<AgentEvent>,
    pub cancel_token: &'a CancellationToken,
}

// ── N2 · repeated-tool-call circuit breaker ─────────────────────
//
// Loop-local anti-loop guard: counts CONSECUTIVE identical (tool, params)
// calls and escalates reminders; at the hard cap the turn is terminated
// (HardLimitReached-style exit). Deliberately NOT part of GeJu (a pure
// 干叠加 evaluator) and NOT persisted into memory seeds — it is a
// per-session runtime signal, like `tool_failure_count`.

/// Consecutive identical calls before the first reminder.
const REPEAT_REMIND: u32 = 3;
/// …before the stronger three-choice reminder.
const REPEAT_ESCALATE: u32 = 5;
/// …before the final warning.
const REPEAT_FINAL_WARN: u32 = 8;
/// …at which the turn is force-terminated.
const REPEAT_FORCE_STOP: u32 = 12;

#[derive(Default, Clone)]
struct RepeatGuard {
    last_key: Option<String>,
    streak: u32,
}

impl RepeatGuard {
    /// Track one tool call; returns the current consecutive streak (1 = this
    /// call differs from the previous one and starts a new run).
    fn track(&mut self, name: &str, parameters: &serde_json::Value) -> u32 {
        // serde_json maps are BTreeMaps (the preserve_order feature is off
        // workspace-wide), so to_string is a canonical form: logically-equal
        // parameters always produce the same key regardless of field order.
        let key = format!(
            "{name}:{}",
            serde_json::to_string(parameters).unwrap_or_default()
        );
        if self.last_key.as_deref() == Some(key.as_str()) {
            self.streak += 1;
        } else {
            self.streak = 1;
            self.last_key = Some(key);
        }
        self.streak
    }

    /// Reminder to surface when this streak crosses a threshold exactly
    /// (intermediate counts stay silent — one nudge per threshold).
    fn reminder(&self, tool: &str) -> Option<String> {
        match self.streak {
            REPEAT_REMIND => Some(format!(
                "[Repeat guard] You have called `{tool}` with identical parameters {REPEAT_REMIND} times in a row. \
                 Before calling it again, state explicitly what NEW information you expect this call to return \
                 that the previous identical calls did not."
            )),
            REPEAT_ESCALATE => Some(format!(
                "[Repeat guard] `{tool}` with identical parameters has now run {REPEAT_ESCALATE} times consecutively. \
                 Choose ONE: (1) falsify this direction — use a different tool or different parameters; \
                 (2) ask the user for the missing input; (3) wrap up with what you already have."
            )),
            REPEAT_FINAL_WARN => Some(format!(
                "[Repeat guard] FINAL WARNING: `{tool}` repeated {REPEAT_FINAL_WARN} times with identical parameters. \
                 At {REPEAT_FORCE_STOP} consecutive repetitions the turn is terminated automatically."
            )),
            _ => None,
        }
    }
}

// ── Agent::run ─────────────────────────────────────────────────

impl super::Agent {
    /// F7 · persist the current history verbatim.
    ///
    /// Used by the normal end-of-turn incremental save AND by early exits
    /// (LLM error / cancellation) so content already in history is durable
    /// even if `post_loop` never runs. Deliberately saves history AS-IS:
    /// partial output from failed streams never enters history (P0-3: only
    /// normally-ended streams are recorded), and post_loop's lifecycle work
    /// (consolidation, distillation, …) is not duplicated here.
    async fn save_history_now(&self) {
        if let Ok(json) = serde_json::to_string(&self.history) {
            if let Err(e) = self.earth.store_async.save_session(&self.id, &json).await {
                tracing::warn!(session = %self.id, error = %e, "Failed to save session");
            }
        }
    }

    /// U7 · 流式早派发 — gate (+prepare) one freshly-reassembled native call
    /// DURING streaming; spawn execute immediately iff every eligibility
    /// condition holds (`EarlyDispatch::eligible`). 公理 2/3: 门禁逐调用、
    /// 执行前完成 —— 提前的只有"执行"不是"决策";公理 4: 任一条件不满足
    /// 即降级流毕批(Gated 槽位),确认时序与现行为一致。
    async fn early_gate_dispatch(
        &self,
        index: usize,
        tc: &crate::stems::action::ToolCall,
        ctx: &RunContext<'_>,
        early: &mut EarlyDispatch,
        touched_acc: &mut Vec<String>,
    ) {
        match gate_one_tool(
            tc,
            &self.earth.tools,
            ctx.event_bus,
            ctx.hook_registry,
            &ctx.tx,
            &self.tool_failure_count,
            self.max_consecutive_failures,
            self.interaction_mode,
            &self.earth.user_hooks,
            &self.principles,
            self.manas.atma_graha,
        )
        .await
        {
            GatedCall::Finished(outcome) => early.slots.push(EarlySlot::Done(outcome)),
            GatedCall::Cleared {
                tool,
                geju_result,
                geju_name,
                execution_mode,
                heaven_stem,
                target_palace,
            } => {
                let accesses = tool.accesses(&tc.parameters);
                // 先回收已完成任务(释放窗口、传播 sibling abort),再判资格。
                early.poll();
                let eligible = early.eligible(
                    &accesses,
                    geju_result.execution_mode,
                    ctx.human_plate.gate_is_open(HumanGate::JingXiangMen),
                    &ctx.human_plate
                        .permissions
                        .chain_check(tool.name(), &tc.parameters),
                    crate::plates::tian_heaven::tool_scheduler::max_tool_concurrency(),
                );
                if !eligible {
                    early.slots.push(EarlySlot::Gated {
                        tool,
                        geju_result,
                        geju_name,
                        execution_mode,
                        heaven_stem,
                        target_palace,
                    });
                    return;
                }
                // 资格满足时 prepare 必然走 Direct 快径(GeJu Direct + 景门开
                // + 策略链 Pass,与 HumanPlate::prepare 的判定一一对应)——
                // 无用户确认,不会阻塞流式消费。
                match ctx
                    .human_plate
                    .prepare(
                        &geju_result,
                        &tool,
                        tc.parameters.clone(),
                        ctx.event_bus,
                        &ctx.tx,
                        &self.exec_ctx,
                    )
                    .await
                {
                    Ok(prepared) => {
                        // B1: 早派发任务持自有 ExecContext 克隆;期间不会发生
                        // worktree swap(enter/exit_worktree 声明 All,结构上
                        // 必落流毕批,swap 只发生在 Phase 4)。
                        early.spawn(
                            index,
                            prepared,
                            self.exec_ctx.clone(),
                            ctx.tx.clone(),
                            accesses,
                        );
                        early.slots.push(EarlySlot::Running {
                            geju_name,
                            execution_mode,
                            heaven_stem,
                            target_palace,
                        });
                    }
                    Err(e) => {
                        let err = match e {
                            crate::error::DispatchError::Denied(r)
                            | crate::error::DispatchError::ToolError(r) => r,
                        };
                        let outcome = CallOutcome {
                            output: String::new(),
                            error: Some(err),
                            geju_name,
                            execution_mode,
                            heaven_stem,
                            target_palace,
                            synthetic_cancel: false,
                        };
                        early.slots.push(EarlySlot::Done(finalize_outcome(
                            tc,
                            "",
                            outcome,
                            0,
                            touched_acc,
                            &self.output_budget,
                            &self.exec_ctx,
                            ctx.event_bus,
                            ctx.hook_registry,
                            &ctx.tx,
                        )));
                    }
                }
            }
        }
    }

    /// U7 · 流毕收尾:收干在途早派发(sibling abort + 取消语义同 U1),
    /// Running 槽位按声明序 finalize(#10 截断落盘同一函数),转为 Done
    /// 供 Phase 4 统一按声明序合并(history/失败计数/快照)。
    async fn drain_early_finalize(
        &self,
        early: &mut EarlyDispatch,
        calls: &[crate::stems::action::ToolCall],
        ctx: &RunContext<'_>,
        touched_acc: &mut Vec<String>,
    ) {
        early.drain(ctx.cancel_token).await;
        for index in 0..early.slots.len() {
            let (geju_name, execution_mode, heaven_stem, target_palace) =
                match &early.slots[index] {
                    EarlySlot::Running {
                        geju_name,
                        execution_mode,
                        heaven_stem,
                        target_palace,
                    } => (
                        geju_name.clone(),
                        execution_mode.clone(),
                        *heaven_stem,
                        *target_palace,
                    ),
                    _ => continue,
                };
            let (raw, error, duration, synthetic_cancel) = match early.take_completed(index) {
                Some((raw, error, duration)) => (raw, error, duration, false),
                // 合成取消 (B3/U2): sibling abort / cancel —— 不写 history,
                // 不计失败 streak,事件照发。
                None => (
                    String::new(),
                    Some("cancelled: a sibling tool call in this batch failed".to_string()),
                    0,
                    true,
                ),
            };
            let outcome = CallOutcome {
                output: String::new(),
                error,
                geju_name,
                execution_mode,
                heaven_stem,
                target_palace,
                synthetic_cancel,
            };
            early.slots[index] = EarlySlot::Done(finalize_outcome(
                &calls[index],
                &raw,
                outcome,
                duration,
                touched_acc,
                &self.output_budget,
                &self.exec_ctx,
                ctx.event_bus,
                ctx.hook_registry,
                &ctx.tx,
            ));
        }
    }

    /// U7 · 取消路径(流被截断 / 流毕后取消):中止在途早派发,Running
    /// 调用做合成取消账目(事件,不写 history —— U1/B3 同款)。Gated
    /// (未执行)调用不产生账目,与 U1 取消于派发前的行为一致。
    async fn wind_down_early(
        &self,
        early: &mut EarlyDispatch,
        calls: &[crate::stems::action::ToolCall],
        ctx: &RunContext<'_>,
        touched_acc: &mut Vec<String>,
    ) {
        early.abort().await;
        for index in 0..early.slots.len() {
            let (geju_name, execution_mode, heaven_stem, target_palace) =
                match &early.slots[index] {
                    EarlySlot::Running {
                        geju_name,
                        execution_mode,
                        heaven_stem,
                        target_palace,
                    } => (
                        geju_name.clone(),
                        execution_mode.clone(),
                        *heaven_stem,
                        *target_palace,
                    ),
                    _ => continue,
                };
            let outcome = CallOutcome {
                output: String::new(),
                error: Some(
                    "cancelled: stream interrupted while the call was in flight".to_string(),
                ),
                geju_name,
                execution_mode,
                heaven_stem,
                target_palace,
                synthetic_cancel: true,
            };
            early.slots[index] = EarlySlot::Done(finalize_outcome(
                &calls[index],
                "",
                outcome,
                0,
                touched_acc,
                &self.output_budget,
                &self.exec_ctx,
                ctx.event_bus,
                ctx.hook_registry,
                &ctx.tx,
            ));
        }
    }

    /// Phase 4 per-call barrier bookkeeping (U1 同款, declaration order):
    /// failure streak merge (合成取消不计), worktree transitions (P6),
    /// turn snapshot, history 回填 (合成取消不写)。提取为方法供 U7 的
    /// repeat-stop 早收尾复用。async: worktree remove 内含 await。
    async fn absorb_outcome(
        &mut self,
        tc: &crate::stems::action::ToolCall,
        outcome: CallOutcome,
        turn_tool_count: u32,
        ctx: &RunContext<'_>,
    ) {
        let CallOutcome {
            output,
            error,
            geju_name,
            execution_mode,
            heaven_stem,
            target_palace,
            synthetic_cancel,
        } = outcome;

        // Track consecutive failures per tool (GeJu Layer 3 runtime
        // supplement). 合成取消 neither failed nor succeeded — the
        // streak is left untouched.
        if !synthetic_cancel {
            if error.is_some() {
                *self.tool_failure_count.entry(tc.name.clone()).or_insert(0) += 1;
            } else {
                self.tool_failure_count.remove(&tc.name);
            }
        }

        // P6 · worktree transitions (only on tool success).
        // enter_worktree already ran `git worktree add`; here we swap
        // the ExecContext (O(1)) so subsequent tools in this batch see
        // the worktree-scoped PermissionMatrix. exit_worktree restores
        // the original ExecContext and optionally removes the worktree.
        // B1: both tools are All-barriers → singleton batches → this
        // swap can never race a parallel batch. U7: 它们必然落流毕批
        // (All 声明),故 swap 也不会与在途早派发调用竞争。
        if error.is_none() && !synthetic_cancel {
            if tc.name == "enter_worktree"
                && let Some(name) = tc.parameters.get("name").and_then(|v| v.as_str())
            {
                if self.worktree_root.is_none() {
                    let main_root = self.earth.permissions.sandbox.workspace_root.clone();
                    let path =
                        crate::palaces::zhen_tool::builtin::exec::worktree::worktree_path(
                            &main_root, name,
                        );
                    self.exec_ctx = self.earth.build_worktree_exec_ctx(
                        &path,
                        &self.id,
                        ctx.cancel_token.clone(),
                    );
                    self.worktree_root = Some(path.clone());
                    tracing::info!(
                        session = %self.id,
                        worktree = %path.display(),
                        "entered worktree (ExecContext swapped)"
                    );
                } else {
                    tracing::warn!("enter_worktree ignored: already in a worktree");
                }
            } else if tc.name == "exit_worktree" {
                if let Some(wt) = self.worktree_root.take() {
                    self.exec_ctx = ExecContext {
                        permissions: self.earth.permissions.clone(),
                        session_id: self.id.clone(),
                        cancel_token: ctx.cancel_token.clone(),
                        read_state: self.exec_ctx.read_state.clone(),
                        // #6/B4: exit resets the session cwd to the
                        // main root (same rule as enter: reset, not
                        // relative-path mapping).
                        cwd: ExecContext::default_cwd(&self.earth.permissions),
                    };
                    let action = tc
                        .parameters
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("keep");
                    if action == "remove" {
                        let main_root =
                            self.earth.permissions.sandbox.workspace_root.clone();
                        if let Err(e) =
                            crate::palaces::zhen_tool::builtin::exec::worktree::remove_worktree(
                                &main_root, &wt, false,
                            )
                        .await
                        {
                            tracing::warn!(
                                worktree = %wt.display(),
                                error = %e,
                                "failed to remove worktree (left on disk)"
                            );
                        }
                    }
                    tracing::info!(session = %self.id, "exited worktree (ExecContext restored)");
                } else {
                    tracing::warn!("exit_worktree ignored: not in a worktree");
                }
            }
        }

        // Record turn snapshot for L2 batch consolidation
        self.working_memory.record(TurnSnapshot {
            turn_number: self.turn_count as u64,
            intent_stem: heaven_stem,
            target_palace,
            geju_name: geju_name.clone(),
            execution_mode: execution_mode.clone(),
            tool_name: tc.name.clone(),
            tool_input: tc.parameters.clone(),
            tool_output: crate::utils::truncate_snapshot_output(&output),
            tool_error: error.clone(),
            timestamp: crate::utils::unix_now(),
            certainty: self.certainty_history.last().copied(),
            active_seed_ids: self.touched_seed_ids.clone(),
            tool_count: turn_tool_count,
        });

        // Push structured tool call entry into history — strict
        // declaration order (parallel batches 回填 by batch position).
        // 合成取消 is NOT written (B3/U2: 不写 history).
        if !synthetic_cancel {
            use crate::types::ToolStatus;
            let status = if error.is_some() {
                ToolStatus::Error
            } else {
                ToolStatus::Success
            };
            let exec_mode =
                serde_json::from_value(serde_json::Value::String(execution_mode)).ok();
            self.history.push(HistoryEntry::ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                tool: tc.name.clone(),
                input: tc.parameters.clone(),
                status,
                output: output.clone(),
                error: error.clone(),
                geju: Some(geju_name),
                execution_mode: exec_mode,
            });
        }
    }

    #[tracing::instrument(skip(self, messages, ctx), fields(session = %self.id))]
    pub async fn run(&mut self, messages: Vec<Message>, ctx: &RunContext<'_>) {
        let title = messages
            .iter()
            .find(|m| m.role == Role::User)
            .map(|m| crate::utils::truncate_title(&m.content))
            .unwrap_or_default();
        let _ = ctx.tx.send(AgentEvent::Session {
            session_id: self.id.clone(),
            title,
        });

        // L1 perfuming: detect explicit user signals before appending to history (zero-LLM).
        // Runs in spawn_blocking to avoid SQLite I/O on the tokio worker thread.
        for msg in &messages {
            if matches!(msg.role, Role::User) {
                let store = self.earth.store.clone();
                let session_id = self.id.clone();
                let content = msg.content.clone();
                tokio::task::spawn_blocking(move || {
                    SignalDetector::process(&store, &session_id, &content);
                })
                .await
                .ok();
            }
        }

        // Append incoming user messages to history (sanitized)
        for msg in messages {
            let entry = match msg.role {
                Role::User => HistoryEntry::User {
                    content: crate::utils::sanitize_message(&msg.content),
                    images: msg.images,
                },
                Role::System => HistoryEntry::system(msg.content),
                Role::Assistant => HistoryEntry::assistant(msg.content),
            };
            self.history.push(entry);
        }

        // Persist initial history so user message survives before first turn
        if let Ok(json) = serde_json::to_string(&self.history) {
            if let Err(e) = self.earth.store_async.save_session(&self.id, &json).await {
                tracing::warn!(session = %self.id, error = %e, "Failed to save initial session");
            }
        }

        // N2 · repeat guard — loop-local, spans turns within this run() so a
        // model stuck re-issuing one call across turns is still caught.
        let mut repeat_guard = RepeatGuard::default();
        // Reminder queued by the previous tool batch; injected into the next
        // inference as an ephemeral user message (never persisted to history).
        let mut repeat_reminder: Option<String> = None;

        loop {
            // XiuMen (休门) — agent pause. While the gate is closed the agent
            // idles WITHOUT consuming turn budget (a paused turn is not a
            // turn — F4) and honors cancellation so a paused agent remains
            // stoppable.
            while !ctx.human_plate.gate_is_open(HumanGate::XiuMen) {
                if ctx.cancel_token.is_cancelled() {
                    tracing::info!(
                        session = %self.id,
                        "Agent loop cancelled while paused (XiuMen closed)"
                    );
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            self.turn_count += 1;
            if self.turn_count > self.max_turns {
                tracing::warn!(
                    session = %self.id,
                    turns = self.turn_count,
                    "Agent hit max turn limit"
                );
                let _ = ctx.tx.send(AgentEvent::Error(format!(
                    "Reached maximum turns ({})",
                    self.max_turns
                )));
                break;
            }

            ctx.event_bus.emit(RuntimeEvent::TurnStart {
                turn: self.turn_count as u64,
            });

            self.manas.record_turn();

            // Flush touched seed IDs from previous turn (or previous error exit)
            {
                let ids: Vec<String> = self.touched_seed_ids.drain(..).collect();
                if !ids.is_empty() {
                    let seed_store = SeedStore::new(self.earth.store.clone());
                    seed_store.touch_batch(&ids);
                }
            }

            // Build messages for LLM: system prompt + history.
            // `system_prompt` carries the P2 stable/dynamic split for caching;
            // `system_full` is the concatenated text used for compaction and
            // token counting (llm_messages[0] stays a System message so the
            // existing compaction logic is unchanged).
            let system_prompt = self.build_system_prompt(ctx.core);
            let system_full = if system_prompt.dynamic.is_empty() {
                system_prompt.stable.clone()
            } else {
                format!("{}\n\n{}", system_prompt.stable, system_prompt.dynamic)
            };
            let mut llm_messages = vec![Message::text(Role::System, system_full.clone())];
            llm_messages.extend(to_llm_messages(&self.history));

            // ── Bing (丙奇) — Context compaction ──────────────────────
            let pre_tokens = ContextWindow::count_tokens(&llm_messages);
            let threshold = (self.context_window.max_tokens as f64
                * self.context_window.compaction_threshold) as usize;
            if pre_tokens > threshold {
                let _ = ctx.tx.send(AgentEvent::ContextPressure {
                    tokens: pre_tokens,
                    threshold,
                });
                // U3-2 · 防抖基线 —— cc_tokens_after 即 kimi-code
                // `lastCompactedTokenCount` 基线的等价记录(压缩后 token 数);
                // 判定逻辑提取为纯函数,见 handoff::anti_thrash_skip。
                let skip = handoff::anti_thrash_skip(
                    self.cc_last_turn,
                    self.turn_count,
                    self.cc_tokens_before,
                    self.cc_tokens_after,
                );

                if !skip {
                    // GeJu gate — informational only (no Bing pattern yields Denied)
                    let geju = GeJu::new(Stem::Bing, Palace::Gen.stem());
                    let gr = geju.evaluate();
                    ctx.event_bus.emit(RuntimeEvent::GeJuResult {
                        tool: "compaction".into(),
                        pattern: gr.name.clone(),
                        mode: format!("{:?}", gr.execution_mode).to_lowercase(),
                    });

                    let _ = ctx.tx.send(AgentEvent::Compacting);

                    // Build indexed message list for compaction (tool calls → User messages)
                    let compact_msgs: Vec<Message> = to_llm_messages(&self.history);
                    let msg_indices: Vec<usize> = self
                        .history
                        .iter()
                        .enumerate()
                        .filter_map(|(i, e)| if e.is_message() { Some(i) } else { None })
                        .collect();
                    let (start, count) = self.context_window.victim_range(&compact_msgs);
                    // Clamp to msg_indices: compact_msgs may include entries
                    // (e.g. ToolCall → User) absent from msg_indices.
                    let count = count.min(msg_indices.len().saturating_sub(start));
                    if count > 0 {
                        let messages_before = self.history.len();
                        let rebuild = |h: &Vec<HistoryEntry>| {
                            let mut msgs = vec![Message::text(Role::System, system_full.clone())];
                            msgs.extend(to_llm_messages(h));
                            (ContextWindow::count_tokens(&msgs), msgs)
                        };
                        let hist_start = msg_indices[start];
                        let hist_end = msg_indices[start + count - 1] + 1;

                        // U3-a · 三段式之一 —— 真实用户消息(原话)按 token 预算
                        // 原样保留:头部锚点 + 尾部近况,中间折叠为 elision 标记;
                        // 保留部分【不进】摘要器。TODO 也不抄进笔记(U3-c):
                        // todo 块每轮从 live task store 重建(loop_prompt 动态段)。
                        let victim_entries: Vec<HistoryEntry> = (0..count)
                            .map(|i| self.history[msg_indices[start + i]].clone())
                            .collect();
                        let (keep_mask, elided_users) =
                            handoff::select_preserved_users(&victim_entries);

                        // U3-4 · 竞态指纹 —— 摘要 await 期间历史前缀若被改动
                        // (取消/新消息插入),取消本次压缩,等干净边界重来。
                        let fp_before = handoff::history_fingerprint(&self.history[..hist_end]);

                        let victims_raw = &compact_msgs[start..start + count];
                        // FNV-1a dedup: remove duplicate messages before feeding to
                        // LLM; preserved user messages (U3-a) are excluded too.
                        let mut seen = std::collections::HashSet::new();
                        let victims: Vec<crate::types::Message> = victims_raw.iter()
                            .enumerate()
                            .filter(|(i, _)| !keep_mask[*i])
                            .map(|(_, m)| {
                                let role_tag = match m.role {
                                    crate::types::Role::User => "U",
                                    crate::types::Role::Assistant => "A",
                                    crate::types::Role::System => "S",
                                };
                                let hash_key = crate::vijnana::vasana::distillation::fnv1a_hash(&format!("{role_tag}:{}", m.content));
                                if !seen.insert(hash_key) {
                                    crate::types::Message::text(
                                        m.role,
                                        "[Duplicate — same content as an earlier message in this batch]".to_string(),
                                    )
                                } else {
                                    m.clone()
                                }
                            })
                            .collect();

                        if victims.is_empty() {
                            // 受害者区间全是被保留的真实用户消息 —— 没有可压缩
                            // 材料,本轮跳过压缩,历史原样进入推理。
                            tracing::debug!(
                                session = %self.id,
                                "Compaction skipped: victim range is entirely preserved user messages"
                            );
                        } else {
                            let compaction: Option<(usize, &str)> = {
                                let prev = self.compaction_summary.as_deref();
                                // U3-3 · 降级链:摘要失败按 0.7/0.5/0.35 收缩
                                // 重试(媒体先剥离为文本占位),最终 Err 落到
                                // fit() 滑窗兜底。
                                match handoff::summarize_with_degradation(
                                    &victims,
                                    ctx.core,
                                    Some(ctx.cancel_token.clone()),
                                    prev,
                                )
                                .await
                                {
                                    Ok(summary_msg) => {
                                        if handoff::history_fingerprint(&self.history[..hist_end])
                                            != fp_before
                                        {
                                            // U3-4: 前缀在摘要生成期间被改动 ——
                                            // 取消本次压缩,历史原样收尾,等干净
                                            // 边界重来(同取消路径,不重写历史)。
                                            tracing::warn!(
                                                session = %self.id,
                                                "Compaction aborted: history prefix changed during summarization"
                                            );
                                            None
                                        } else {
                                            // Store for next iterative update
                                            self.compaction_summary = Some(summary_msg.content.clone());
                                            let content = format!(
                                                "[CONTEXT COMPACTION -- REFERENCE ONLY]\n{}",
                                                summary_msg.content
                                            );
                                            // 三段式落位:交接笔记在前,随后是原样
                                            // 保留的用户消息(头部 → elision 标记 →
                                            // 尾部)。位识边界:交接笔记是上下文工程
                                            // 产物,不是记忆种子 —— 不入阿赖耶识、
                                            // 不参与熏习/召回。
                                            let head_len =
                                                keep_mask.iter().take_while(|&&k| k).count();
                                            let mut replacement =
                                                vec![HistoryEntry::system(content)];
                                            replacement.extend(
                                                victim_entries[..head_len].iter().cloned(),
                                            );
                                            if elided_users > 0 {
                                                replacement.push(HistoryEntry::system(
                                                    handoff::elision_marker(elided_users),
                                                ));
                                            }
                                            replacement.extend(
                                                victim_entries[head_len..]
                                                    .iter()
                                                    .zip(keep_mask[head_len..].iter())
                                                    .filter(|(_, k)| **k)
                                                    .map(|(e, _)| e.clone()),
                                            );
                                            self.history.splice(hist_start..hist_end, replacement);
                                            let (tokens, msgs) = rebuild(&self.history);
                                            llm_messages = msgs;
                                            Some((tokens, "summarize"))
                                        }
                                    }
                                    Err(e) => {
                                        // F5: if the session was cancelled, summarize
                                        // already refused the partial checkpoint — do
                                        // NOT rewrite history at all (not even via the
                                        // fit fallback). This arm returns directly
                                        // (winding down with history intact).
                                        if ctx.cancel_token.is_cancelled() {
                                            tracing::info!(
                                                session = %self.id,
                                                error = %e,
                                                "Compaction aborted by cancellation; history left unchanged"
                                            );
                                            None
                                        } else {
                                            tracing::warn!(
                                                error = %e,
                                                "Compaction summarization failed, falling back to fit()"
                                            );
                                            let mut fit_msgs = compact_msgs.clone();
                                            let (_dropped, _) = self.context_window.fit(&mut fit_msgs);
                                            let mut new_history: Vec<HistoryEntry> = Vec::new();
                                            let mut msg_iter = fit_msgs.into_iter();
                                            for entry in std::mem::take(&mut self.history) {
                                                if entry.is_message() {
                                                    if msg_iter.next().is_some() {
                                                        new_history.push(entry);
                                                    }
                                                } else {
                                                    new_history.push(entry);
                                                }
                                            }
                                            self.history = new_history;
                                            let (tokens, msgs) = rebuild(&self.history);
                                            llm_messages = msgs;
                                            Some((tokens, "fit"))
                                        }
                                    }
                                }
                            };

                            let Some((t_after, method)) = compaction else {
                                // Compaction skipped (cancelled / 竞态) — leave
                                // history untouched and wind down immediately; no
                                // LLM call is issued for a session that is going
                                // away. As with the other cancel paths,
                                // SessionEnd/Done are left to the caller's teardown.
                                return;
                            };

                            // Anti-thrashing state (U3-2 基线:cc_tokens_after 即
                            // lastCompactedTokenCount 等价记录)
                            self.cc_last_turn = self.turn_count;
                            self.cc_tokens_before = pre_tokens;
                            self.cc_tokens_after = t_after;

                            fire_void_hooks(
                                ctx.hook_registry,
                                ctx.event_bus,
                                SpiritType::JiuDi,
                                Palace::Gen.stem(),
                                HookEvent::CompactionTriggered {
                                    messages_before,
                                    messages_after: self.history.len(),
                                    tokens_before: pre_tokens,
                                    tokens_after: t_after,
                                    method: method.to_string(),
                                },
                            );

                            JIA_TOKENS_COMPACTED_TOTAL
                                .inc_by(pre_tokens.saturating_sub(t_after) as f64);

                            tracing::info!(
                                tokens_before = pre_tokens,
                                tokens_after = t_after,
                                method,
                                "Context compacted"
                            );
                        }
                    }
                } else {
                    tracing::debug!(
                        tokens = pre_tokens,
                        threshold,
                        last_saved_pct = (self.cc_tokens_before.saturating_sub(self.cc_tokens_after) * 100)
                            / self.cc_tokens_before.max(1),
                        turns_since = self.turn_count.saturating_sub(self.cc_last_turn),
                        "Skipping compaction: anti-thrashing"
                    );
                }
            }

            // LLM inference — P2: pass the system prompt via `infer_with_system`
            // so the Anthropic provider can cache the stable prefix. Strip the
            // leading System message from llm_messages (it was only there for
            // compaction/token-counting); the system travels separately.
            let llm_start = std::time::Instant::now();
            let mut infer_messages = llm_messages;
            if matches!(infer_messages.first().map(|m| m.role), Some(Role::System)) {
                infer_messages.remove(0);
            }

            // ── Background task notifications ─────────────────────
            // Before each LLM turn, check for completed background tasks
            // that haven't been notified yet, and inject their summaries
            // as user messages so the model knows about them.
            // Mirrors Claude Code's task-notification XML injection.
            //
            // CAS protocol: mark_notified atomically sets notified=true and
            // returns whether WE won the race. After winning, re-read the task
            // to get the definitive status (the snapshot from unnotified_terminal
            // may be stale — e.g. kill() may have transitioned Killed since).
            {
                let terminal_tasks = self.earth.background_tasks.unnotified_terminal_tasks();
                for task in &terminal_tasks {
                    // Try to claim this notification. Only one path wins per task.
                    if !self.earth.background_tasks.mark_notified(&task.id) {
                        continue; // another path already notified
                    }

                    // Re-read to get the definitive status post-CAS.
                    let definitive = self.earth.background_tasks.get(&task.id);
                    let status_str = definitive.as_ref()
                        .map(|t| t.status.as_str())
                        .unwrap_or(task.status.as_str());
                    let exit_code = definitive.as_ref().and_then(|t| t.exit_code);
                    let desc = definitive.as_ref()
                        .map(|t| &t.description)
                        .unwrap_or(&task.description);
                    let output_path = definitive.as_ref()
                        .map(|t| t.output_file.clone())
                        .unwrap_or_else(|| task.output_file.clone());

                    let summary = match task.task_type {
                        crate::palaces::zhen_tool::builtin::exec::background_task::TaskType::Shell => {
                            match definitive.as_ref().map(|t| t.status).unwrap_or(task.status) {
                                crate::palaces::zhen_tool::builtin::exec::background_task::TaskStatus::Completed => {
                                    let code_info = exit_code.map_or(String::new(), |c| format!(" (exit code {c})"));
                                    format!("Background command \"{desc}\" completed successfully{}.", code_info)
                                }
                                crate::palaces::zhen_tool::builtin::exec::background_task::TaskStatus::Failed => {
                                    let code_info = exit_code.map_or(String::new(), |c| format!(" (exit code {c})"));
                                    format!("Background command \"{desc}\" failed{}.", code_info)
                                }
                                crate::palaces::zhen_tool::builtin::exec::background_task::TaskStatus::Killed => {
                                    format!("Background command \"{desc}\" was stopped.")
                                }
                                crate::palaces::zhen_tool::builtin::exec::background_task::TaskStatus::Lost => {
                                    format!("Background command \"{desc}\" was lost after a crash (process state unknown).")
                                }
                                _ => continue,
                            }
                        }
                        _ => format!(
                            "Background task \"{desc}\" {status_str}.",
                        ),
                    };
                    let notification = format!(
                        "[Background task {}] {summary}\nOutput file: {}",
                        task.id,
                        output_path.display()
                    );

                    // Inject as a user message so the model sees it in the next turn
                    infer_messages.push(Message::text(Role::User, notification));

                    // Emit event to TUI
                    let _ = ctx.tx.send(AgentEvent::TaskCompleted {
                        task_id: task.id.clone(),
                        status: status_str.to_string(),
                        summary: summary.clone(),
                        output_file: output_path.to_string_lossy().to_string(),
                        tool_use_id: task.tool_use_id.clone(),
                    });
                }

                // Evict old terminal+notified tasks
                if !terminal_tasks.is_empty() {
                    let all = self.earth.background_tasks.list(None);
                    for task in &all {
                        if task.status.is_terminal() && task.notified {
                            self.earth.background_tasks.evict(&task.id);
                        }
                    }
                }
            }

            // N2 · repeat-guard reminder — injected at the same place and in
            // the same shape as background-task notifications: an ephemeral
            // user message visible to this inference only.
            if let Some(reminder) = repeat_reminder.take() {
                infer_messages.push(Message::text(Role::User, reminder));
            }

            // Build tool schemas for native tools API (openai/anthropic/gemini).
            let use_native = crate::palaces::zhong_core::use_native_tools(&ctx.core.provider_kind);
            let tool_schemas: Option<Vec<crate::stems::action::ToolSchema>> = if use_native {
                let schemas: Vec<_> = self
                    .earth
                    .tools
                    .list_core()
                    .iter()
                    .map(|t| crate::stems::action::ToolSchema {
                        name: t.name().to_string(),
                        description: t.description(),
                        parameters: t.parameters_schema(),
                    })
                    .collect();
                Some(schemas)
            } else {
                None
            };
            let tools_ref: Option<&[crate::stems::action::ToolSchema]> = tool_schemas.as_deref();

            // P0-3: LLM retry loop. A retryable mid-stream error with a
            // successful failover RE-ISSUES the request against the new
            // provider (fresh stream) instead of polling the dead stream.
            // Partial output from a failed attempt is discarded before the
            // re-issue, so it never enters history; `record_llm_success`
            // runs only after a stream that ended normally (None).
            const MAX_LLM_RETRIES: u32 = 3;
            // Retry budget is per-turn: reset on turn entry so a previous
            // turn's exhaustion doesn't leave the next turn with zero budget.
            self.retry_count = 0;
            let mut full_response = String::new();
            let mut native_tool_calls: Vec<crate::stems::action::ToolCall> = Vec::new();

            // U7 · 流式早派发状态(native 路径;XML fallback 全程为空,批
            // 处理行为与 U1 完全一致)。repeat_guard 检查点供 P0-3 重试回滚:
            // 失败尝试的门禁计数整体作废,下一段流从检查点重新计数。
            let mut early = EarlyDispatch::new();
            let rg_checkpoint = repeat_guard.clone();
            let mut repeat_stop = false;
            let mut touched_acc: Vec<String> = Vec::new();

            'llm_retry: loop {
                let mut stream = ctx.core.infer_with_system(
                    infer_messages.clone(),
                    system_prompt.clone(),
                    tools_ref,
                    Some(ctx.cancel_token.clone()),
                );
                // Drop partial output from any previous failed attempt.
                full_response.clear();
                native_tool_calls.clear();

                loop {
                    match stream.next().await {
                        Some(Ok(crate::palaces::zhong_core::StreamChunk::NativeToolCall {
                            id,
                            name,
                            arguments,
                        })) => {
                            let params: serde_json::Value =
                                serde_json::from_str(&arguments).unwrap_or_default();
                            let tc = crate::stems::action::ToolCall {
                                id,
                                name,
                                parameters: params,
                            };
                            // U7 · 流式早派发:重组完成一个即串行门禁(repeat
                            // guard → 熔断 → 谋划短路 → GeJu → hooks)与
                            // prepare;资格全满足才立即 execute,否则降级流毕批。
                            if repeat_stop {
                                // 强制停止已触发:本回合即将收尾,剩余调用不再
                                // 门禁/派发(不执行、不入账,同现行为)。
                            } else {
                                // N2 · repeat guard — 门禁时计数(串行,无竞态)。
                                let streak = repeat_guard.track(&tc.name, &tc.parameters);
                                if streak >= REPEAT_FORCE_STOP {
                                    let msg = format!(
                                        "Stopped: tool `{}` was called {REPEAT_FORCE_STOP} times in a row \
                                         with identical parameters (repeat guard, hard limit reached).",
                                        tc.name
                                    );
                                    tracing::warn!(
                                        session = %self.id,
                                        tool = %tc.name,
                                        streak,
                                        "repeat guard terminated the turn"
                                    );
                                    let _ = ctx.tx.send(AgentEvent::Error(msg));
                                    repeat_stop = true;
                                } else {
                                    if let Some(reminder) = repeat_guard.reminder(&tc.name) {
                                        tracing::info!(
                                            session = %self.id,
                                            tool = %tc.name,
                                            streak,
                                            "repeat guard reminder queued"
                                        );
                                        repeat_reminder = Some(reminder);
                                    }
                                    self.early_gate_dispatch(
                                        native_tool_calls.len(),
                                        &tc,
                                        ctx,
                                        &mut early,
                                        &mut touched_acc,
                                    )
                                    .await;
                                }
                            }
                            native_tool_calls.push(tc);
                        }
                        Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) => {
                            full_response.push_str(&delta);
                            let _ = ctx.tx.send(AgentEvent::Delta(delta));
                        }
                        Some(Ok(crate::palaces::zhong_core::StreamChunk::Usage {
                            input_tokens,
                            output_tokens,
                        })) => {
                            ctx.event_bus.emit(RuntimeEvent::LlmUsage {
                                input_tokens,
                                output_tokens,
                            });
                        }
                        Some(Ok(crate::palaces::zhong_core::StreamChunk::CacheHit {
                            cache_read,
                            cache_creation,
                            ..
                        })) => {
                            // P2 prompt-cache telemetry (Anthropic). cache_read > 0
                            // means the stable system prefix was served from cache.
                            tracing::info!(
                                session = %self.id,
                                cache_read,
                                cache_creation,
                                "prompt cache hit"
                            );
                        }
                        Some(Err(crate::error::ProviderError::Cancelled)) => {
                            // S1: truncation sentinel injected by run_or_cancel
                            // — the stream was CUT by cancellation, it did not
                            // end naturally. Unlike the `None` arm (F6), the
                            // partial response is DISCARDED: no history entry,
                            // no StreamEnd, and no record_llm_success (a
                            // cancelled turn must not reset the circuit
                            // breaker). Not retryable — `Cancelled` is excluded
                            // from is_retryable, and cancellation is honored,
                            // not failed over. F7: persist history as-is
                            // before returning.
                            tracing::info!(session = %self.id, "LLM stream truncated by cancellation; partial response discarded");
                            // U7: 中止在途早派发并做合成取消账目(事件,不写
                            // history —— U1/B3 同款)。
                            self.wind_down_early(&mut early, &native_tool_calls, ctx, &mut touched_acc)
                                .await;
                            self.save_history_now().await;
                            return;
                        }
                        Some(Err(e)) => {
                            // P0-3 + #1: retry with exponential backoff.
                            // Always attempt failover (to record failure +
                            // try provider switch), but retry even when
                            // failover fails — single-provider setups also
                            // benefit from backoff + retry within the budget.
                            if e.is_retryable()
                                && self.retry_count < MAX_LLM_RETRIES
                            {
                                if ctx.cancel_token.is_cancelled() {
                                    tracing::info!(session = %self.id, "Agent loop cancelled");
                                    self.save_history_now().await;
                                    return;
                                }
                                let switched = ctx.core.try_llm_failover();
                                tracing::warn!(
                                    session = %self.id,
                                    error = %e,
                                    retry = self.retry_count + 1,
                                    switched_provider = switched,
                                    "LLM error, retrying"
                                );
                                self.retry_count += 1;
                                // U7: 失败尝试的早派发整体作废 —— 中止在途调用
                                // (无账目,整段随 Retrying 回滚),RepeatGuard 回
                                // 到流前检查点;重试流重新门禁/计数。
                                early.abort_and_reset().await;
                                repeat_guard = rg_checkpoint.clone();
                                repeat_reminder = None;
                                // S2: the failed attempt's partial Deltas are
                                // already on the wire — tell frontends to roll
                                // the bubble back before the retried stream
                                // starts appending.
                                let _ = ctx.tx.send(AgentEvent::Retrying {
                                    attempt: self.retry_count,
                                });
                                // #1: a server Retry-After hint beats the
                                // local exponential guess — the provider
                                // knows its own recovery window.
                                let delay = if let Some(retry_after) = e.retry_after() {
                                    retry_after
                                } else {
                                    // #1 exponential backoff before retry
                                    let mut backoff = RetryBackoff::new();
                                    for _ in 0..self.retry_count {
                                        backoff.next_delay();
                                    }
                                    backoff.next_delay()
                                };
                                tracing::debug!(retry = self.retry_count, delay_ms = delay.as_millis(), server_hint = e.retry_after().is_some(), "LLM retry backoff");
                                tokio::time::sleep(delay).await;
                                continue 'llm_retry;
                            }
                            if self.retry_count >= MAX_LLM_RETRIES {
                                tracing::error!(session = %self.id, retries = self.retry_count, "LLM retry limit exhausted");
                            }
                            tracing::error!(session = %self.id, error = %e, "LLM inference error");
                            let _ = ctx.tx.send(AgentEvent::Error(format!("{e}")));
                            self.save_history_now().await;
                            return;
                        }
                        None => {
                            // F6: do NOT discard a normally-ended stream when a
                            // cancel raced its final chunk — break out and run
                            // the normal finalization (StreamEnd + history).
                            // Cancellation is honored right after the response
                            // is safely recorded, before any tool execution.
                            break 'llm_retry;
                        }
                    }
                }
            }

            // Reached only after a stream ended normally: record success on
            // the provider that actually served the completed stream.
            JIA_LLM_DURATION_SECONDS.observe(llm_start.elapsed().as_secs_f64());
            ctx.core.record_llm_success();
            self.retry_count = 0;

            // Notify frontend that LLM stream ended (freeze bubble A)
            let _ = ctx.tx.send(AgentEvent::StreamEnd);

            // Record assistant response in history
            let response_len = full_response.len();

            // Strip trailing JSON fragments + extra blank lines that some
            // models emit before the native tool call.
            let has_native = !native_tool_calls.is_empty();
            if has_native && let Some(pos) = full_response.rfind(['.', '?', '!', '。', '？', '！'])
            {
                let after_sentence = &full_response[pos..];
                let char_len = after_sentence
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
                let after = &full_response[pos + char_len..];
                if after.contains('{') {
                    full_response.truncate(pos + char_len);
                }
            }
            // Trim trailing whitespace so the tool card sits directly after text.
            full_response = full_response.trim_end().to_string();

            // Parse tool calls — prefer native (API-level) over XML text parsing.
            let tool_calls: Vec<crate::stems::action::ToolCall> = if has_native {
                native_tool_calls
            } else {
                let tool_names: Vec<&str> = self
                    .earth
                    .tools
                    .list_names()
                    .iter()
                    .map(|s| s.as_str())
                    .collect();
                let (_clean_text, calls) = parse_tool_calls(&full_response, &tool_names);
                calls
            };

            // Guard (review Important #1): a cancel that arrived during tool
            // execution makes the next infer return None immediately with an
            // EMPTY response — don't record an empty assistant entry
            // (some providers reject empty assistant messages).
            let empty_cancel = ctx.cancel_token.is_cancelled()
                && full_response.is_empty()
                && tool_calls.is_empty();
            if !empty_cancel {
                self.history.push(HistoryEntry::assistant(full_response));
            }

            // F6: cancellation is honored only AFTER a normally-ended stream
            // has been finalized (StreamEnd sent, response in history) — a
            // complete response is never discarded by a late-arriving cancel.
            // Tool calls are NOT executed once cancelled. F7: persist the
            // finalized history before returning.
            if ctx.cancel_token.is_cancelled() {
                tracing::info!(session = %self.id, "Agent loop cancelled");
                // U7: 中止在途早派发并做合成取消账目(事件,不写 history)。
                self.wind_down_early(&mut early, &tool_calls, ctx, &mut touched_acc)
                    .await;
                self.save_history_now().await;
                return;
            }

            tracing::info!(
                session = %self.id,
                response_len,
                tool_call_count = tool_calls.len(),
                "Parsed tool calls from LLM response"
            );

            // ── 确定度评估（在解析工具调用之后、分发之前）──
            let certainty = TurnCertainty::evaluate(
                &self.working_memory.snapshots,
                self.manas.atma_graha,
                self.turn_count,
                self.max_turns,
                &CertaintyParams::default(),
            );
            self.certainty_history.push(certainty.composite);
            // Adjust atma-graha based on certainty trend (feature-gated)
            if self.earth.config.app_config.cognition.certainty_enabled {
                self.manas
                    .adjust_from_certainty_trend(&self.certainty_history);
            }

            fire_void_hooks(
                ctx.hook_registry,
                ctx.event_bus,
                SpiritType::TengShe,
                Stem::Ren,
                HookEvent::LlmResponse {
                    response_len,
                    tool_call_count: tool_calls.len(),
                    certainty: Some(certainty.composite),
                },
            );

            if tool_calls.is_empty() {
                // Certainty signal is informational (logged, observed by TaiYin).
                // Empty tool calls always end the turn — the LLM chose to respond
                // with text only. Certainty enriches the observation but does not
                // gate the break: Continue/EscalateToHuman must not keep looping
                // with the same context (infinite loop).
                tracing::info!(
                    composite = certainty.composite,
                    decision = ?certainty.decision,
                    turn = self.turn_count,
                    "Turn end — no tool calls"
                );
                ctx.event_bus.emit(RuntimeEvent::TurnEnd {
                    turn: self.turn_count as u64,
                });
                break;
            }

            // Notify frontend that tool batch is starting (create bubble B)
            let _ = ctx.tx.send(AgentEvent::ToolBatchStart);

            let mut touched_paths: Vec<&str> = Vec::new();
            for tc in &tool_calls {
                if let Some(path) = tc.parameters.get("path").and_then(|v| v.as_str())
                    && !path.is_empty()
                {
                    touched_paths.push(path);
                }
            }

            // ── Tool dispatch with conflict-matrix batching (U1) ──
            // tool_scheduler::plan_batches groups calls by their ToolAccesses
            // declarations (A2: 工具级声明为唯一并发判据; ceremony 推导已废弃).
            // Per batch, four phases — gates stay SERIAL (公理 3), only the
            // execute step runs concurrently:
            //   1. gate_one_tool (serial, declaration order): repeat guard →
            //      failure streak → 谋划短路 → GeJu → pre-tool hooks
            //   2. HumanPlate::prepare (serial): 分发模式判定 + confirmations
            //   3. execute: JoinSet for multi-call batches (sibling abort on
            //      first error), inline await for singletons
            //   4. barrier (declaration order): finalize events, merge loop
            //      state (failure counts, touched seeds), history 回填
            let mut tool_count: usize = 0;

            // U7 · 流毕收尾:收干在途早派发(sibling abort/取消语义同 U1),
            // Running 槽位按声明序 finalize 转为 Done,与流毕批共享 Phase 4
            // 屏障(history/失败计数/落盘全部按声明序合并)。
            self.drain_early_finalize(&mut early, &tool_calls, ctx, &mut touched_acc)
                .await;
            // 早派发槽位按 tool_call id 索引(native 路径;XML 为空)。
            let early_by_id: std::collections::HashMap<&str, usize> = (0..early.slots.len())
                .map(|k| (tool_calls[k].id.as_str(), k))
                .collect();

            // U7 · repeat guard 在流式门禁期触发强制停止:已执行的早派发调用
            // 按声明序入账(真实副作用必须入账);Gated/未门禁调用不执行、不入账
            // (同现行为:触发点之后的调用一律丢弃)。
            if repeat_stop {
                for k in 0..early.slots.len() {
                    if let EarlySlot::Done(outcome) =
                        std::mem::replace(&mut early.slots[k], EarlySlot::Consumed)
                    {
                        self.absorb_outcome(&tool_calls[k], outcome, tool_calls.len() as u32, ctx)
                            .await;
                    }
                }
            }

            let batches = crate::plates::tian_heaven::tool_scheduler::plan_batches(
                &tool_calls,
                &self.earth.tools,
            );

            let max_fail = self.max_consecutive_failures;
            'batches: for batch in &batches {
                if repeat_stop {
                    // U7: 流式期已触发强制停止 —— 早派发账目已结,不再派发。
                    break;
                }
                // ── Phase 1+2: serial gate + 分发模式判定, declaration order ──
                let mut outcomes: Vec<Option<CallOutcome>> =
                    (0..batch.len()).map(|_| None).collect();
                let mut prepared_calls: Vec<PreparedExec> = Vec::new();
                for (i, tc) in batch.iter().enumerate() {
                    // U7: native 调用已在流式期间门禁 —— 按槽位分流:Done →
                    // 结果就绪(跳 Phase 4);Gated → 跳过门禁直接 prepare;
                    // 其余(XML fallback / 未门禁)走原有串行门禁路径。
                    let slot_gated = match early_by_id.get(tc.id.as_str()).copied() {
                        Some(k) => match std::mem::replace(&mut early.slots[k], EarlySlot::Consumed)
                        {
                            EarlySlot::Done(outcome) => {
                                outcomes[i] = Some(outcome);
                                continue;
                            }
                            EarlySlot::Gated {
                                tool,
                                geju_result,
                                geju_name,
                                execution_mode,
                                heaven_stem,
                                target_palace,
                            } => Some(GatedCall::Cleared {
                                tool,
                                geju_result,
                                geju_name,
                                execution_mode,
                                heaven_stem,
                                target_palace,
                            }),
                            // drain 后只会是 Done/Gated;防御性回退为未门禁处理。
                            other => {
                                early.slots[k] = other;
                                None
                            }
                        },
                        None => None,
                    };

                    let gated = match slot_gated {
                        Some(g) => g,
                        None => {
                            // N2 · repeat guard — count consecutive identical calls before
                            // dispatch. Threshold reminders are queued for the NEXT
                            // inference; at REPEAT_FORCE_STOP the turn is terminated
                            // (HardLimitReached-style, mirroring the max-turns exit).
                            let streak = repeat_guard.track(&tc.name, &tc.parameters);
                            if streak >= REPEAT_FORCE_STOP {
                                let msg = format!(
                                    "Stopped: tool `{}` was called {REPEAT_FORCE_STOP} times in a row \
                                     with identical parameters (repeat guard, hard limit reached).",
                                    tc.name
                                );
                                tracing::warn!(
                                    session = %self.id,
                                    tool = %tc.name,
                                    streak,
                                    "repeat guard terminated the turn"
                                );
                                let _ = ctx.tx.send(AgentEvent::Error(msg));
                                repeat_stop = true;
                                break 'batches;
                            }
                            if let Some(reminder) = repeat_guard.reminder(&tc.name) {
                                tracing::info!(
                                    session = %self.id,
                                    tool = %tc.name,
                                    streak,
                                    "repeat guard reminder queued"
                                );
                                repeat_reminder = Some(reminder);
                            }

                            gate_one_tool(
                                tc,
                                &self.earth.tools,
                                ctx.event_bus,
                                ctx.hook_registry,
                                &ctx.tx,
                                &self.tool_failure_count,
                                max_fail,
                                self.interaction_mode,
                                &self.earth.user_hooks,
                                &self.principles,
                                self.manas.atma_graha,
                            )
                            .await
                        }
                    };

                    match gated {
                        GatedCall::Finished(outcome) => outcomes[i] = Some(outcome),
                        GatedCall::Cleared {
                            tool,
                            geju_result,
                            geju_name,
                            execution_mode,
                            heaven_stem,
                            target_palace,
                        } => {
                            let start = std::time::Instant::now();
                            match ctx
                                .human_plate
                                .prepare(
                                    &geju_result,
                                    &tool,
                                    tc.parameters.clone(),
                                    ctx.event_bus,
                                    &ctx.tx,
                                    &self.exec_ctx,
                                )
                                .await
                            {
                                Ok(prepared) => prepared_calls.push(PreparedExec {
                                    index: i,
                                    prepared,
                                    start,
                                    geju_name,
                                    execution_mode,
                                    heaven_stem,
                                    target_palace,
                                }),
                                Err(e) => {
                                    let err = match e {
                                        crate::error::DispatchError::Denied(r)
                                        | crate::error::DispatchError::ToolError(r) => r,
                                    };
                                    let outcome = CallOutcome {
                                        output: String::new(),
                                        error: Some(err),
                                        geju_name,
                                        execution_mode,
                                        heaven_stem,
                                        target_palace,
                                        synthetic_cancel: false,
                                    };
                                    outcomes[i] = Some(finalize_outcome(
                                        tc,
                                        "",
                                        outcome,
                                        start.elapsed().as_millis() as u64,
                                        &mut touched_acc,
                                        &self.output_budget,
                                        &self.exec_ctx,
                                        ctx.event_bus,
                                        ctx.hook_registry,
                                        &ctx.tx,
                                    ));
                                }
                            }
                        }
                    }
                }

                // ── Phase 3: execute ──
                // B1: parallel tasks run on OWNED ExecContext clones
                // (snapshot); self.exec_ctx is never swapped while a batch is
                // in flight — enter/exit_worktree declare ToolAccesses::all
                // and therefore always land in singleton batches.
                if prepared_calls.len() == 1 {
                    let p = prepared_calls.remove(0);
                    let res = p.prepared.execute(&ctx.tx, &self.exec_ctx).await;
                    let (raw, error) = match res {
                        Ok(tr) => (tr.output, tr.error),
                        Err(crate::error::DispatchError::Denied(r))
                        | Err(crate::error::DispatchError::ToolError(r)) => {
                            (String::new(), Some(r))
                        }
                    };
                    let outcome = CallOutcome {
                        output: String::new(),
                        error,
                        geju_name: p.geju_name,
                        execution_mode: p.execution_mode,
                        heaven_stem: p.heaven_stem,
                        target_palace: p.target_palace,
                        synthetic_cancel: false,
                    };
                    outcomes[p.index] = Some(finalize_outcome(
                        &batch[p.index],
                        &raw,
                        outcome,
                        p.start.elapsed().as_millis() as u64,
                        &mut touched_acc,
                        &self.output_budget,
                        &self.exec_ctx,
                        ctx.event_bus,
                        ctx.hook_registry,
                        &ctx.tx,
                    ));
                } else if !prepared_calls.is_empty() {
                    // Multi-call non-conflicting batch: real concurrent execution.
                    let mut metas: std::collections::HashMap<
                        usize,
                        (String, String, Stem, Palace),
                    > = std::collections::HashMap::new();
                    let mut queue: std::collections::VecDeque<(
                        usize,
                        crate::plates::ren_human::PreparedCall,
                    )> = std::collections::VecDeque::new();
                    for p in prepared_calls {
                        metas.insert(
                            p.index,
                            (p.geju_name, p.execution_mode, p.heaven_stem, p.target_palace),
                        );
                        queue.push_back((p.index, p.prepared));
                    }

                    type TaskOut = (usize, String, Option<String>, u64);
                    fn spawn_task(
                        join_set: &mut tokio::task::JoinSet<TaskOut>,
                        index: usize,
                        prepared: crate::plates::ren_human::PreparedCall,
                        exec_ctx: ExecContext,
                        tx: mpsc::UnboundedSender<AgentEvent>,
                    ) {
                        join_set.spawn(async move {
                            let start = std::time::Instant::now();
                            let res = prepared.execute(&tx, &exec_ctx).await;
                            let duration = start.elapsed().as_millis() as u64;
                            let (raw, error) = match res {
                                Ok(tr) => (tr.output, tr.error),
                                Err(crate::error::DispatchError::Denied(r))
                                | Err(crate::error::DispatchError::ToolError(r)) => {
                                    (String::new(), Some(r))
                                }
                            };
                            (index, raw, error, duration)
                        });
                    }

                    let max_conc =
                        crate::plates::tian_heaven::tool_scheduler::max_tool_concurrency();
                    let exec_snapshot = self.exec_ctx.clone();
                    let mut join_set: tokio::task::JoinSet<TaskOut> = tokio::task::JoinSet::new();
                    let mut completed: std::collections::HashMap<
                        usize,
                        (String, Option<String>, u64),
                    > = std::collections::HashMap::new();
                    let mut aborted = false;

                    for _ in 0..max_conc {
                        let Some((index, prepared)) = queue.pop_front() else {
                            break;
                        };
                        spawn_task(
                            &mut join_set,
                            index,
                            prepared,
                            exec_snapshot.clone(),
                            ctx.tx.clone(),
                        );
                    }

                    while let Some(joined) = join_set.join_next().await {
                        match joined {
                            Ok((index, raw, error, duration)) => {
                                // Sibling abort: the first failing call cancels
                                // all remaining in-flight/queued calls (synthetic
                                // cancel below); later batches run as usual.
                                if error.is_some() && !aborted {
                                    aborted = true;
                                    join_set.abort_all();
                                    queue.clear();
                                }
                                completed.insert(index, (raw, error, duration));
                            }
                            Err(e) => {
                                if e.is_panic() {
                                    tracing::error!(
                                        session = %self.id,
                                        error = %e,
                                        "tool task panicked (recorded as synthetic cancel)"
                                    );
                                }
                                // Aborted (sibling abort) or panicked — the call
                                // gets a synthetic cancel at the barrier.
                            }
                        }
                        if ctx.cancel_token.is_cancelled() && !aborted {
                            aborted = true;
                            join_set.abort_all();
                            queue.clear();
                        }
                        // Refill the concurrency window.
                        while !aborted && join_set.len() < max_conc {
                            let Some((index, prepared)) = queue.pop_front() else {
                                break;
                            };
                            spawn_task(
                                &mut join_set,
                                index,
                                prepared,
                                exec_snapshot.clone(),
                                ctx.tx.clone(),
                            );
                        }
                    }

                    // ── barrier: finalize in declaration order ──
                    for (index, (geju_name, execution_mode, heaven_stem, target_palace)) in metas
                    {
                        let (raw, error, duration, synthetic_cancel) = match completed
                            .remove(&index)
                        {
                            Some((raw, error, duration)) => (raw, error, duration, false),
                            // 合成取消 (B3/U2): not written to history, does not
                            // touch the failure streak — events/counts only.
                            None => (
                                String::new(),
                                Some(
                                    "cancelled: a sibling tool call in this batch failed"
                                        .to_string(),
                                ),
                                0,
                                true,
                            ),
                        };
                        let outcome = CallOutcome {
                            output: String::new(),
                            error,
                            geju_name,
                            execution_mode,
                            heaven_stem,
                            target_palace,
                            synthetic_cancel,
                        };
                        outcomes[index] = Some(finalize_outcome(
                            &batch[index],
                            &raw,
                            outcome,
                            duration,
                            &mut touched_acc,
                            &self.output_budget,
                            &self.exec_ctx,
                            ctx.event_bus,
                            ctx.hook_registry,
                            &ctx.tx,
                        ));
                    }
                }

                // ── Phase 4: barrier bookkeeping, strict declaration order ──
                for (i, tc) in batch.iter().enumerate() {
                    let Some(outcome) = outcomes[i].take() else {
                        continue;
                    };
                    self.absorb_outcome(tc, outcome, tool_calls.len() as u32, ctx)
                        .await;
                    tool_count += 1;
                }
            } // end batch loop
            if repeat_stop {
                // N2 force stop — same收尾 shape as the max-turns exit:
                // persist history as-is and leave the turn loop; SessionEnd
                // and Done are emitted by the shared teardown below.
                self.save_history_now().await;
                break;
            }
            self.touched_seed_ids.extend(touched_acc);
            // Feature-gated: coactivation recording + stability observation
            let cog = &self.earth.config.app_config.cognition;
            if cog.coactivation_enabled {
                self.coactivation.record_coactivation(
                    "",
                    &self.touched_seed_ids,
                    self.turn_count as u64,
                );
            }
            if cog.observation_enabled {
                ctx.event_bus.emit(RuntimeEvent::StabilityTransition {
                    stable: self.manas.is_stable(),
                    atma_graha: self.manas.atma_graha,
                    epochs: self.manas.stable_epochs(),
                });
            }

            // Layer 4 · session-scoped gate closing — detect anomaly patterns
            // and autonomously close gates for the remainder of this session.
            const GATE_CLOSE_THRESHOLD: u32 = 5;
            for (tool_name, &fail_count) in self.tool_failure_count.iter() {
                if fail_count < GATE_CLOSE_THRESHOLD {
                    continue;
                }
                match tool_name.as_str() {
                    "web_fetch" | "web_search" => {
                        ctx.human_plate.close_gate(HumanGate::KaiMen);
                    }
                    "shell" | "write_file" | "patch_file" | "revert_file" => {
                        ctx.human_plate.close_gate(HumanGate::ShangMen);
                    }
                    "skill" => {
                        ctx.human_plate.close_gate(HumanGate::ShengMen);
                    }
                    _ => {}
                }
            }

            // Track skill tool invocations (Phase 2)
            for tc in &tool_calls {
                if tc.name == "skill"
                    && let Some(skill_name) = tc.parameters.get("skill").and_then(|v| v.as_str())
                    && !self.skill_tool_calls.iter().any(|s| s == skill_name)
                {
                    self.skill_tool_calls.push(skill_name.to_string());
                }
            }

            // P3 · plan-mode transitions: detect enter/exit_plan_mode tool calls
            // (tools are stateless; the loop flips the per-session interaction_mode
            // by name, mirroring skill-call tracking). is_destructive()=false so
            // exit_plan_mode passes the Planning short-circuit (D1: no deadlock).
            for tc in &tool_calls {
                match tc.name.as_str() {
                    "enter_plan_mode" => {
                        self.interaction_mode = InteractionMode::Plan;
                        ctx.human_plate.sync_jingjue_with_mode(true); // Planning → suppress alerts
                        tracing::debug!(session = %self.id, "entered plan mode");
                        let _ = ctx
                            .tx
                            .send(AgentEvent::InteractionModeChanged {
                                mode: InteractionMode::Plan,
                            });
                    }
                    "exit_plan_mode" => {
                        self.interaction_mode = InteractionMode::Auto;
                        ctx.human_plate.sync_jingjue_with_mode(false); // Normal → resume alerts
                        tracing::debug!(session = %self.id, "exited plan mode");
                        let _ = ctx
                            .tx
                            .send(AgentEvent::InteractionModeChanged {
                                mode: InteractionMode::Auto,
                            });
                    }
                    _ => {}
                }
            }

            self.activate_skills(&touched_paths);

            // BatchEnded hooks — all four spirits observe different dimensions
            let batch_event = HookEvent::BatchEnded {
                geju_name: None,
                tool_count,
                turn: self.turn_count as u64,
            };
            fire_void_hooks(
                ctx.hook_registry,
                ctx.event_bus,
                SpiritType::LiuHe,
                Stem::Xin,
                batch_event.clone(),
            );
            fire_void_hooks(
                ctx.hook_registry,
                ctx.event_bus,
                SpiritType::TaiYin,
                Stem::Ren,
                batch_event.clone(),
            );
            fire_void_hooks(
                ctx.hook_registry,
                ctx.event_bus,
                SpiritType::XuanWu,
                Stem::Bing,
                batch_event.clone(),
            );
            fire_void_hooks(
                ctx.hook_registry,
                ctx.event_bus,
                SpiritType::JiuTian,
                Stem::Ding,
                batch_event.clone(),
            );

            // Enforce absolute history cap to prevent unbounded growth
            // when compaction anti-thrashing keeps skipping.
            const HISTORY_CAP: usize = 1000;
            if self.history.len() > HISTORY_CAP {
                let excess = self.history.len() - HISTORY_CAP;
                self.history.drain(0..excess);
                tracing::warn!(
                    session = %self.id,
                    excess,
                    "History exceeded cap, truncated oldest entries"
                );
            }

            ctx.event_bus.emit(RuntimeEvent::TurnEnd {
                turn: self.turn_count as u64,
            });

            // Incremental persist: save history after each turn
            self.save_history_now().await;
        }

        ctx.event_bus.emit(RuntimeEvent::SessionEnd {
            session_id: self.id.clone(),
            turns: self.turn_count as u64,
        });

        let _ = ctx.tx.send(AgentEvent::Done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_calls_single() {
        let text = r#"Let me read that file.

<tool_call>
{"tool": "read_file", "parameters": {"file_path": "/tmp/test.txt"}}
</tool_call>

Done."#;
        let (clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(
            calls[0].parameters["file_path"].as_str().unwrap(),
            "/tmp/test.txt"
        );
        assert!(!clean.contains("<tool_call>"));
    }

    #[test]
    fn test_parse_tool_calls_multiple() {
        let text = r#"I'll check.

<tool_call>
{"tool": "read_file", "parameters": {"file_path": "/tmp/a.txt"}}
</tool_call>

<tool_call>
{"tool": "write_file", "parameters": {"file_path": "/tmp/b.txt", "content": "hello"}}
</tool_call>"#;
        let (clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[1].name, "write_file");
        assert!(!clean.contains("<tool_call>"));
    }

    #[test]
    fn test_parse_tool_calls_none() {
        let text = "Just a regular response with no tool calls.";
        let (clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 0);
        assert_eq!(clean, text);
    }

    #[test]
    fn test_parse_tool_calls_unclosed_tag() {
        let text = "Start <tool_call> but never close";
        let (clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 0);
        assert!(clean.contains("<tool_call>"));
    }

    // ── N2: repeat guard ──────────────────────────────────────

    #[test]
    fn repeat_guard_counts_consecutive_identical_calls() {
        let mut g = RepeatGuard::default();
        let params = serde_json::json!({"path": "/tmp/a"});
        assert_eq!(g.track("read_file", &params), 1);
        assert_eq!(g.track("read_file", &params), 2);
        assert_eq!(g.track("read_file", &params), 3);
        // A different call resets the streak…
        assert_eq!(g.track("read_file", &serde_json::json!({"path": "/tmp/b"})), 1);
        // …as does a different tool with the same parameters.
        assert_eq!(g.track("write_file", &serde_json::json!({"path": "/tmp/b"})), 1);
        // And the reset streak can grow again.
        assert_eq!(g.track("write_file", &serde_json::json!({"path": "/tmp/b"})), 2);
    }

    #[test]
    fn repeat_guard_key_is_order_insensitive() {
        // Field order in the JSON must not fragment the streak — the
        // canonical serialization (BTreeMap) sorts keys.
        let mut g = RepeatGuard::default();
        assert_eq!(g.track("shell", &serde_json::json!({"a": 1, "b": 2})), 1);
        assert_eq!(g.track("shell", &serde_json::json!({"b": 2, "a": 1})), 2);
    }

    #[test]
    fn repeat_guard_reminders_fire_at_thresholds_only() {
        let mut g = RepeatGuard::default();
        let params = serde_json::json!({"command": "ls"});
        // Below the first threshold: silent.
        g.track("shell", &params);
        g.track("shell", &params);
        assert!(g.reminder("shell").is_none());
        // 3: ask for the expected NEW information.
        g.track("shell", &params);
        assert!(g.reminder("shell").unwrap().contains("NEW information"));
        // 4: silent between thresholds.
        g.track("shell", &params);
        assert!(g.reminder("shell").is_none());
        // 5: three-choice escalation.
        g.track("shell", &params);
        assert!(g.reminder("shell").unwrap().contains("Choose ONE"));
        // 8: final warning naming the hard cap.
        for _ in 6..=8 {
            g.track("shell", &params);
        }
        let warn = g.reminder("shell").unwrap();
        assert!(warn.contains("FINAL WARNING"));
        assert!(warn.contains(&REPEAT_FORCE_STOP.to_string()));
        // 9..12: no further reminders (the loop force-stops at the cap).
        for _ in 9..=REPEAT_FORCE_STOP {
            g.track("shell", &params);
            assert!(g.reminder("shell").is_none());
        }
    }

    // ── P0-3: LLM retry must re-issue the request ───────────────

    use super::super::tests::temp_earth;
    use crate::error::ProviderError;
    use crate::palaces::zhong_core::{JiaCore, LlmProvider, StreamChunk};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One scripted response for [`ScriptedProvider`].
    enum MockStep {
        /// Stream `partial` as deltas, then fail mid-stream with `err`.
        FailAfter {
            partial: &'static str,
            err: ProviderError,
        },
        /// Stream the text and end the stream normally.
        Complete(&'static str),
        /// S1: stream `partial` as deltas, then end with the truncation
        /// sentinel (what run_or_cancel injects when cancellation cuts the
        /// producer). The consumer must treat this as a cancellation, NOT a
        /// natural end.
        Truncated(&'static str),
    }

    /// A mock provider that plays a per-call script and counts invocations.
    struct ScriptedProvider {
        steps: std::sync::Mutex<std::collections::VecDeque<MockStep>>,
        calls: Arc<AtomicUsize>,
    }

    impl ScriptedProvider {
        fn new(steps: Vec<MockStep>, calls: Arc<AtomicUsize>) -> Self {
            Self {
                steps: std::sync::Mutex::new(steps.into()),
                calls,
            }
        }
    }

    impl LlmProvider for ScriptedProvider {
        fn infer_stream(
            &self,
            _messages: Vec<Message>,
            _tools: Option<&[crate::stems::action::ToolSchema]>,
            _cancel_token: Option<CancellationToken>,
        ) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, ProviderError>> + Send>>
        {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let step = self
                .steps
                .lock()
                .unwrap()
                .pop_front()
                .expect("ScriptedProvider: script exhausted — test bug");
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                let (text, err) = match step {
                    MockStep::FailAfter { partial, err } => (partial, Some(err)),
                    MockStep::Complete(text) => (text, None),
                    MockStep::Truncated(partial) => (partial, Some(ProviderError::Cancelled)),
                };
                for ch in text.chars() {
                    let _ = tx.send(Ok(StreamChunk::Delta(ch.to_string())));
                }
                if let Some(err) = err {
                    let _ = tx.send(Err(err));
                }
            });
            Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
        }
    }

    fn router_core(providers: Vec<Box<dyn LlmProvider>>) -> JiaCore {
        let router = crate::palaces::zhong_core::ProviderRouter::new(
            providers
                .into_iter()
                .enumerate()
                .map(|(i, p)| (i as u32, p))
                .collect(),
        );
        JiaCore::with_router(router, "mock".into(), "mock".into(), 8192)
    }

    /// Run a fresh agent to completion against `core`; collect all events.
    async fn run_agent(
        earth: Arc<crate::plates::di_earth::EarthPlate>,
        core: &JiaCore,
    ) -> (super::super::Agent, Vec<AgentEvent>) {
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("retry-test".into(), earth.clone());
        let ctx = RunContext {
            core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        (agent, events)
    }

    fn assistant_texts(agent: &super::super::Agent) -> Vec<&str> {
        agent
            .history
            .iter()
            .filter_map(|e| match e {
                HistoryEntry::Assistant { content } => Some(content.as_str()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn llm_retry_reissues_request_and_drops_partial_response() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let flaky: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::FailAfter {
                partial: "TRUNCATED_JUNK",
                err: ProviderError::RateLimited {
                body: "429".into(),
                retry_after: None,
            },
            }],
            calls.clone(),
        ));
        let healthy: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete("final answer")],
            calls.clone(),
        ));
        let core = router_core(vec![flaky, healthy]);

        let (agent, events) = run_agent(earth, &core).await;

        // (1) the failed request was actually re-issued (failover → new stream)
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "failed request must be re-issued against the next provider"
        );
        // (2) history carries the retried response, not the truncated partial
        assert_eq!(
            assistant_texts(&agent),
            ["final answer"],
            "partial response from the failed attempt must not enter history"
        );
        // retry succeeded → no Error, exactly one StreamEnd, run completed
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Error(_))),
            "successful retry must not emit Error: {events:?}"
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, AgentEvent::StreamEnd))
                .count(),
            1,
            "StreamEnd exactly once (never for a failed attempt): {events:?}"
        );
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done)));
        assert_eq!(agent.retry_count, 0, "retry_count reset after success");
        // S2: the retry arm must emit exactly one Retrying { attempt: 1 },
        // ordered AFTER the failed attempt's junk Deltas and BEFORE the
        // retried stream's Deltas — frontends truncate the bubble on it.
        let retry_positions: Vec<usize> = events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| matches!(e, AgentEvent::Retrying { attempt: 1 }).then_some(i))
            .collect();
        assert_eq!(
            retry_positions.len(),
            1,
            "exactly one Retrying {{ attempt: 1 }}: {events:?}"
        );
        let rp = retry_positions[0];
        assert!(
            events[..rp]
                .iter()
                .any(|e| matches!(e, AgentEvent::Delta(_))),
            "junk Deltas must precede Retrying: {events:?}"
        );
        assert!(
            events[rp..]
                .iter()
                .any(|e| matches!(e, AgentEvent::Delta(d) if d == "f")),
            "retried stream's Deltas must follow Retrying: {events:?}"
        );
    }

    #[tokio::test]
    async fn llm_retry_exhaustion_emits_error_and_skips_history() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        // Two always-failing providers: failover ping-pongs until retry_count
        // hits MAX_LLM_RETRIES (3) → 1 initial + 3 retries = 4 requests.
        let mk = |calls: &Arc<AtomicUsize>| -> Box<dyn LlmProvider> {
            Box::new(ScriptedProvider::new(
                vec![
                    MockStep::FailAfter {
                        partial: "junk",
                        err: ProviderError::ServerError {
                            status: 500,
                            body: "boom".into(),
                            retry_after: None,
                            },
                    },
                    MockStep::FailAfter {
                        partial: "junk",
                        err: ProviderError::ServerError {
                            status: 500,
                            body: "boom".into(),
                            retry_after: None,
                            },
                    },
                ],
                calls.clone(),
            ))
        };
        let core = router_core(vec![mk(&calls), mk(&calls)]);

        let (agent, events) = run_agent(earth, &core).await;

        // (4) retry_count reaches MAX → exhausted branch (formerly dead code)
        assert_eq!(
            calls.load(Ordering::SeqCst),
            4,
            "1 initial + 3 re-issued retries"
        );
        assert_eq!(
            agent.retry_count, 3,
            "retry_count must reach MAX_LLM_RETRIES"
        );
        // (3) frontend receives an Error event
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Error(_))),
            "exhausted retries must emit Error: {events:?}"
        );
        // no StreamEnd / Done for a failed turn; no半截 assistant entry
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::StreamEnd)),
            "failed turn must not emit StreamEnd: {events:?}"
        );
        assert!(
            assistant_texts(&agent).is_empty(),
            "failed turn must not push assistant history: {:?}",
            assistant_texts(&agent)
        );
    }

    #[tokio::test]
    async fn llm_retry_budget_resets_each_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        // Six failures per provider: #1 same-provider retry may consume extra
        // entries beyond the failover-gated count (when breakers open mid-turn).
        let mk = |calls: &Arc<AtomicUsize>| -> Box<dyn LlmProvider> {
            Box::new(ScriptedProvider::new(
                (0..6)
                    .map(|_| MockStep::FailAfter {
                        partial: "junk",
                        err: ProviderError::ServerError {
                            status: 500,
                            body: "boom".into(),
                            retry_after: None,
                            },
                    })
                    .collect(),
                calls.clone(),
            ))
        };
        let core = router_core(vec![mk(&calls), mk(&calls)]);

        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("retry-test".into(), earth.clone());

        // Turn 1: exhausts the budget (1 initial + 3 retries).
        {
            let (tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
            let ctx = RunContext {
                core: &core,
                human_plate: &human_plate,
                event_bus: &earth.spirit.event_bus,
                hook_registry: &earth.spirit.hook_registry,
                tx,
                cancel_token: &cancel,
            };
            agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;
        }
        assert_eq!(agent.retry_count, 3);
        let turn1_calls = calls.load(Ordering::SeqCst);

        // Turn 2: a stuck per-agent budget (review Issue 1) would allow zero
        // retries (1 request); the per-turn reset must grant a full 1+3 again.
        {
            let (tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
            let ctx = RunContext {
                core: &core,
                human_plate: &human_plate,
                event_bus: &earth.spirit.event_bus,
                hook_registry: &earth.spirit.hook_registry,
                tx,
                cancel_token: &cancel,
            };
            agent
                .run(vec![Message::text(Role::User, "again")], &ctx)
                .await;
        }
        // The per-turn reset is proven by turn 2 making ANY retry at all:
        // a stuck budget (review Issue 1) would allow exactly 1 request and
        // zero retries.
        let turn2_calls = calls.load(Ordering::SeqCst) - turn1_calls;
        assert!(
            turn2_calls >= 2,
            "turn 2 must get a fresh retry budget (stuck budget allows only 1 request, got {turn2_calls})"
        );
        // Exact count is coupled to circuit-breaker internals + #1 same-provider
        // retry behavior; the invariant is that turn 2 retried at all.
        assert!(
            agent.retry_count >= 1,
            "turn 2 must have retried at least once (stuck budget = 0 retries)"
        );
    }

    #[tokio::test]
    async fn llm_non_retryable_error_fails_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let bad: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::FailAfter {
                partial: "junk",
                err: ProviderError::ClientError {
                    status: 400,
                    body: "bad request".into(),
                },
            }],
            calls.clone(),
        ));
        let core = router_core(vec![bad]);

        let (agent, events) = run_agent(earth, &core).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "non-retryable must not retry"
        );
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error(_))));
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::StreamEnd)));
        assert!(assistant_texts(&agent).is_empty());
    }

    // ── P2-3: 取消语义抛光 (F4/F5/F6/F7) ─────────────────────

    /// F4: with XiuMen closed the loop must idle WITHOUT burning turn_count
    /// (old code: +1 per 500ms spin → false "Reached maximum turns" after
    /// ~12.5s) and must exit promptly on cancellation.
    #[tokio::test]
    async fn xiumen_pause_does_not_burn_turns_and_honors_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete("must never be reached")],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        human_plate.close_gate(HumanGate::XiuMen);
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("pause-test".into(), earth.clone());

        let ctx = RunContext {
            core: &core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        let run = agent.run(vec![Message::text(Role::User, "hi")], &ctx);
        let watchdog = async {
            // Several 500ms spin cycles pass, then cancel.
            tokio::time::sleep(std::time::Duration::from_millis(1600)).await;
            cancel.cancel();
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(run, watchdog)
        })
        .await
        .expect("cancel must break the XiuMen pause spin");

        assert_eq!(
            agent.turn_count, 0,
            "paused loop must not consume turn budget"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "LLM must not be called while paused"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Error(_))),
            "pause must not surface a spurious max-turns error: {events:?}"
        );
    }

    /// F5 (loop side): when cancellation cuts the summarize stream short,
    /// compaction must be skipped entirely — no半截 summary inserted, no
    /// fit() fallback rewrite, no messages drained from history.
    #[tokio::test]
    async fn compaction_cancelled_leaves_history_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete(
                "partial checkpoint that must be refused",
            )],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("f5-test".into(), earth.clone());
        // Force the compaction path: tiny context window + removable history.
        agent.context_window = ContextWindow::new(8, 0.75);
        agent.history.push(HistoryEntry::assistant(
            "old answer with enough tokens to exceed the tiny limit",
        ));
        // Model a cancel that lands while summarize is in flight.
        cancel.cancel();

        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let ctx = RunContext {
            core: &core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "only the summarize call ran; the turn wound down before main inference"
        );
        // History = pre-seeded assistant + the new user message — nothing
        // drained, no compaction marker inserted.
        assert_eq!(agent.history.len(), 2, "history must not be rewritten");
        assert!(
            agent
                .history
                .iter()
                .all(|e| !matches!(e, HistoryEntry::System { content } if content.contains("CONTEXT COMPACTION"))),
            "no半截 compaction summary in history: {:?}",
            agent.history
        );
        assert!(
            agent.compaction_summary.is_none(),
            "refused partial must not seed the next iterative update"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Error(_))),
            "cancelled compaction is not an error: {events:?}"
        );
    }

    /// U3 (loop side): 压缩成功路径落位三段式 —— 交接笔记插入受害者区间,
    /// 真实用户消息原样保留(不进摘要器、不被重写),增量 checkpoint 记录
    /// 供下一轮续写。
    #[tokio::test]
    async fn compaction_success_keeps_handoff_note_and_preserved_users() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![
                MockStep::Complete("handoff note from the summarizer"),
                MockStep::Complete("final answer"),
            ],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("u3-test".into(), earth.clone());
        // Force the compaction path: tiny context window + removable history.
        agent.context_window = ContextWindow::new(8, 0.75);
        agent.history.push(HistoryEntry::user(
            "please refactor the parser module carefully",
        ));
        agent.history.push(HistoryEntry::assistant(
            "I will start by reading the parser code thoroughly",
        ));
        agent.history.push(HistoryEntry::assistant(
            "read complete found three call sites to update",
        ));

        let (tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
        let ctx = RunContext {
            core: &core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "summarize + one main inference"
        );
        // 三段式之 b:交接笔记以 REFERENCE ONLY system 条目落位…
        assert!(
            agent.history.iter().any(|e| matches!(
                e,
                HistoryEntry::System { content }
                    if content.contains("CONTEXT COMPACTION")
                        && content.contains("handoff note from the summarizer")
            )),
            "handoff note must be inserted: {:?}",
            agent.history
        );
        // 三段式之 a:真实用户消息原样保留(未被摘要器改写)…
        assert!(
            agent.history.iter().any(|e| matches!(
                e,
                HistoryEntry::User { content, .. }
                    if content == "please refactor the parser module carefully"
            )),
            "original user message must survive verbatim: {:?}",
            agent.history
        );
        // …增量 checkpoint 供下一轮续写。
        assert_eq!(
            agent.compaction_summary.as_deref(),
            Some("handoff note from the summarizer"),
            "checkpoint recorded for the next iterative update"
        );
        assert_eq!(assistant_texts(&agent), ["final answer"]);
    }

    /// F6/F7: a cancel racing the end of a normally-completed stream must NOT
    /// discard the full response — it is finalized (StreamEnd + history) and
    /// persisted before the loop exits.
    #[tokio::test]
    async fn cancel_after_stream_end_keeps_complete_response() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete("full answer")],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("f6-test".into(), earth.clone());
        // The ScriptedProvider ignores the token and completes the stream;
        // the pre-fired token models a cancel arriving as the stream ends.
        cancel.cancel();

        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let ctx = RunContext {
            core: &core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;

        // The complete response survives: finalized into history…
        assert_eq!(
            assistant_texts(&agent),
            ["full answer"],
            "complete response must not be discarded by a racing cancel"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, AgentEvent::StreamEnd))
                .count(),
            1,
            "StreamEnd sent for the completed stream: {events:?}"
        );
        // …and persisted before the early return (F7).
        let saved = earth
            .store_async
            .load_session("f6-test")
            .await
            .unwrap()
            .expect("history must be persisted on the cancel path");
        let saved_hist: Vec<HistoryEntry> = serde_json::from_str(&saved).unwrap();
        assert!(
            saved_hist.iter().any(
                |e| matches!(e, HistoryEntry::Assistant { content } if content == "full answer")
            ),
            "finalized response must reach the store: {saved}"
        );
    }

    /// F6 companion: the finalized response stays in history, but its tool
    /// calls must NOT execute once the session is cancelled.
    #[tokio::test]
    async fn cancel_after_stream_end_does_not_execute_tool_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let target = tmp.path().join("secret.txt");
        std::fs::write(&target, "s3cret").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let text = format!(
            "reading it now\n<tool_call>\n{{\"tool\": \"read_file\", \"parameters\": {{\"file_path\": \"{}\"}}}}\n</tool_call>",
            target.display()
        );
        // MockStep::Complete takes &'static str; leak the test string (test-only).
        let text: &'static str = Box::leak(text.into_boxed_str());
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete(text)],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("f6-tools".into(), earth.clone());
        cancel.cancel();

        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let ctx = RunContext {
            core: &core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;

        assert_eq!(
            assistant_texts(&agent).len(),
            1,
            "complete response still enters history"
        );
        assert!(
            !agent
                .history
                .iter()
                .any(|e| matches!(e, HistoryEntry::ToolCall { .. })),
            "cancelled session must not execute the parsed tool call"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolBatchStart)),
            "no tool batch may start after cancel: {events:?}"
        );
    }

    /// S1: a stream cut by cancellation carries the `Cancelled` sentinel —
    /// the loop must DISCARD the partial response (no history entry, no
    /// StreamEnd, no Error) and must NOT record_llm_success, so the circuit
    /// breaker's failure count survives the cancelled turn.
    #[tokio::test]
    async fn cancelled_stream_truncation_discards_partial_and_skips_success() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let calls = Arc::new(AtomicUsize::new(0));
        // Turn 1: P1 fails retryable → breaker[0]=1, failover to P2 completes.
        // Turn 2: P2 fails retryable → failover back to P1, whose stream is
        // truncated by cancellation (sentinel after partial deltas).
        let p1: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![
                MockStep::FailAfter {
                    partial: "junk",
                    err: ProviderError::RateLimited {
                body: "429".into(),
                retry_after: None,
            },
                },
                MockStep::Truncated("half response that must be dropped"),
            ],
            calls.clone(),
        ));
        let p2: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![
                MockStep::Complete("turn1 answer"),
                MockStep::FailAfter {
                    partial: "junk2",
                    err: ProviderError::RateLimited {
                body: "429".into(),
                retry_after: None,
            },
                },
            ],
            calls.clone(),
        ));
        let core = router_core(vec![p1, p2]);
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("s1-test".into(), earth.clone());

        // Turn 1: establishes a non-zero failure count on P1's breaker.
        {
            let (tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();
            let ctx = RunContext {
                core: &core,
                human_plate: &human_plate,
                event_bus: &earth.spirit.event_bus,
                hook_registry: &earth.spirit.hook_registry,
                tx,
                cancel_token: &cancel,
            };
            agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;
        }
        assert_eq!(
            core.test_breaker_failure_count(0),
            Some(1),
            "turn 1 retryable failure must be recorded on P1's breaker"
        );
        assert_eq!(assistant_texts(&agent), ["turn1 answer"]);

        // Turn 2: ends on P1's truncated (cancelled) stream.
        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        {
            let ctx = RunContext {
                core: &core,
                human_plate: &human_plate,
                event_bus: &earth.spirit.event_bus,
                hook_registry: &earth.spirit.hook_registry,
                tx,
                cancel_token: &cancel,
            };
            agent
                .run(vec![Message::text(Role::User, "again")], &ctx)
                .await;
        }

        // The truncated partial never enters history — only turn 1's answer.
        assert_eq!(
            assistant_texts(&agent),
            ["turn1 answer"],
            "cancelled mid-stream partial must be discarded: {:?}",
            agent.history
        );
        // record_llm_success was NOT called on the cancelled turn: P1 (the
        // active provider when the sentinel arrived) keeps its failure.
        assert_eq!(
            core.test_breaker_failure_count(0),
            Some(1),
            "a cancelled turn must not reset the circuit breaker"
        );
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::StreamEnd)),
            "truncated stream must not emit StreamEnd: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Error(_))),
            "cancellation is not an error: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Done)),
            "cancelled turn exits without Done: {events:?}"
        );
        // F7: history as-is (no half response) reached the store.
        let saved = earth
            .store_async
            .load_session("s1-test")
            .await
            .unwrap()
            .expect("history must be persisted on the truncation path");
        assert!(
            !saved.contains("half response"),
            "truncated partial must not reach the store: {saved}"
        );
    }

    /// N2 loop-level: a model stuck re-issuing the SAME tool call must be
    /// force-stopped at the hard cap — the 12th identical call is refused
    /// pre-dispatch, an Error event mirrors the max-turns exit, and the run
    /// ends (Done) without waiting for max_turns.
    #[tokio::test]
    async fn repeat_guard_force_stops_at_hard_cap() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        // File inside the workspace root so read_file passes path checks.
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let target = ws.join("note.txt");
        std::fs::write(&target, "hello").unwrap();
        let text = format!(
            "reading it\n<tool_call>\n{{\"tool\": \"read_file\", \"parameters\": {{\"path\": \"{}\"}}}}\n</tool_call>",
            target.display()
        );
        // MockStep::Complete takes &'static str; leak the test string (test-only).
        let text: &'static str = Box::leak(text.into_boxed_str());
        let calls = Arc::new(AtomicUsize::new(0));
        // More scripted responses than the cap — the guard must stop the run
        // before the script is exhausted.
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            (0..20).map(|_| MockStep::Complete(text)).collect(),
            calls.clone(),
        ));
        let core = router_core(vec![provider]);

        let (agent, events) = run_agent(earth, &core).await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            REPEAT_FORCE_STOP as usize,
            "one inference per repeated call; the {}th is refused pre-dispatch",
            REPEAT_FORCE_STOP
        );
        assert!(
            events.iter().any(
                |e| matches!(e, AgentEvent::Error(m) if m.contains("repeat guard"))
            ),
            "force stop must surface an Error event: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::Done)),
            "run must end via the shared teardown: {events:?}"
        );
        let executed = agent
            .history
            .iter()
            .filter(|e| matches!(e, HistoryEntry::ToolCall { tool, .. } if tool == "read_file"))
            .count();
        assert_eq!(
            executed,
            (REPEAT_FORCE_STOP - 1) as usize,
            "the capped call must not execute; the previous ones did"
        );
    }

    /// U1 regression: a parallel batch (two non-conflicting read_file calls)
    /// must pass EVERY call through the GeJu gate (公理 3: 评估逐调用、派发前
    /// 完成) and 回填 history in strict declaration order with the matching
    /// per-call outputs.
    #[tokio::test]
    async fn parallel_batch_geju_per_call_and_ordered_writeback() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let fa = ws.join("a.txt");
        std::fs::write(&fa, "content-a").unwrap();
        let fb = ws.join("b.txt");
        std::fs::write(&fb, "content-b").unwrap();
        let text = format!(
            "reading both\n<tool_call>\n{{\"tool\": \"read_file\", \"parameters\": {{\"path\": \"{}\"}}}}\n</tool_call>\n<tool_call>\n{{\"tool\": \"read_file\", \"parameters\": {{\"path\": \"{}\"}}}}\n</tool_call>",
            fa.display(),
            fb.display()
        );
        let text: &'static str = Box::leak(text.into_boxed_str());
        let calls = Arc::new(AtomicUsize::new(0));
        // Turn 1: two parallel reads; turn 2: plain text ends the run.
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete(text), MockStep::Complete("done")],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);

        // Subscribe BEFORE the run so both GeJuResult events are captured.
        let mut geju_rx = earth.spirit.event_bus.subscribe();
        let (agent, _events) = run_agent(earth, &core).await;

        // 公理 3: every call in the parallel batch went through GeJu.
        let mut read_gejus = Vec::new();
        while let Ok(ev) = geju_rx.try_recv() {
            if let RuntimeEvent::GeJuResult { tool, pattern, .. } = ev
                && tool == "read_file"
            {
                read_gejus.push(pattern);
            }
        }
        assert_eq!(
            read_gejus.len(),
            2,
            "each call in the parallel batch must pass GeJu (公理 3)"
        );
        assert!(
            read_gejus.iter().all(|p| !p.is_empty()),
            "GeJu pattern must be evaluated per call: {read_gejus:?}"
        );

        // 保序回填: two ToolCall entries in declaration order, each with its
        // own output and GeJu metadata.
        let entries: Vec<_> = agent
            .history
            .iter()
            .filter_map(|e| match e {
                HistoryEntry::ToolCall {
                    input,
                    output,
                    error,
                    geju,
                    ..
                } => Some((input, output, error, geju)),
                _ => None,
            })
            .collect();
        assert_eq!(entries.len(), 2, "both parallel calls must be recorded");
        assert!(entries[0].0["path"].as_str().unwrap().ends_with("a.txt"));
        assert!(entries[1].0["path"].as_str().unwrap().ends_with("b.txt"));
        assert!(entries[0].1.contains("content-a"));
        assert!(entries[1].1.contains("content-b"));
        assert!(entries.iter().all(|e| e.2.is_none()));
        assert!(
            entries
                .iter()
                .all(|e| e.3.as_deref().is_some_and(|g| !g.is_empty())),
            "each history entry must carry its GeJu evaluation"
        );
    }

    // ── U7: 流式早派发 ─────────────────────────────────────────
    //
    // Mock provider 以任意 StreamChunk 脚本(含中途停车点)喂流,模拟
    // Anthropic/Gemini 的增量重组(NativeToolCall 在流中逐个到达);
    // ProbeTool 提供执行可观测性(执行即给计数器 +1)。

    /// One scripted stream item for [`ChunkScriptProvider`].
    enum ChunkItem {
        /// Emit a stream chunk.
        Chunk(StreamChunk),
        /// Park mid-stream for up to `window_ms`, leaving early once
        /// `until` reaches `target`; `saw` records whether the condition
        /// held at any point during the window.
        WaitCount {
            until: Arc<AtomicUsize>,
            target: usize,
            window_ms: u64,
            saw: Arc<AtomicUsize>,
        },
    }

    /// A mock provider that plays per-call chunk scripts.
    struct ChunkScriptProvider {
        scripts: std::sync::Mutex<std::collections::VecDeque<Vec<ChunkItem>>>,
    }

    impl ChunkScriptProvider {
        fn boxed(scripts: Vec<Vec<ChunkItem>>) -> Box<dyn LlmProvider> {
            Box::new(Self {
                scripts: std::sync::Mutex::new(scripts.into()),
            })
        }
    }

    impl LlmProvider for ChunkScriptProvider {
        fn infer_stream(
            &self,
            _messages: Vec<Message>,
            _tools: Option<&[crate::stems::action::ToolSchema]>,
            _cancel_token: Option<CancellationToken>,
        ) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, ProviderError>> + Send>>
        {
            let script = self
                .scripts
                .lock()
                .unwrap()
                .pop_front()
                .expect("ChunkScriptProvider: script exhausted — test bug");
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                for item in script {
                    match item {
                        ChunkItem::Chunk(c) => {
                            let _ = tx.send(Ok(c));
                        }
                        ChunkItem::WaitCount {
                            until,
                            target,
                            window_ms,
                            saw,
                        } => {
                            for _ in 0..(window_ms / 2 + 1) {
                                if until.load(Ordering::SeqCst) >= target {
                                    saw.store(1, Ordering::SeqCst);
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                            }
                        }
                    }
                }
            });
            Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
        }
    }

    /// Read-only probe tool: execute bumps `fired`. 戊仪 + 只读 path 声明 →
    /// GeJu (Wu, Wu) Direct、accesses 非 All —— 满足早派发资格。
    struct ProbeTool {
        fired: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl crate::palaces::zhen_tool::base::BaseTool for ProbeTool {
        fn name(&self) -> &str {
            "probe_read"
        }
        fn description(&self) -> String {
            "probe".into()
        }
        fn ceremony(&self) -> crate::stems::CeremoniesIntent {
            crate::stems::CeremoniesIntent::Wu
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn is_concurrency_safe(&self) -> bool {
            true
        }
        fn accesses(
            &self,
            input: &serde_json::Value,
        ) -> crate::palaces::zhen_tool::base::ToolAccesses {
            crate::palaces::zhen_tool::base::ToolAccesses::read_only(
                vec![std::path::PathBuf::from(
                    input["path"].as_str().unwrap_or("probe"),
                )],
                false,
            )
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ExecContext,
        ) -> Result<String, crate::error::ToolError> {
            self.fired.fetch_add(1, Ordering::SeqCst);
            Ok("probe-output".to_string())
        }
    }

    /// temp_earth 变体:额外注册 probe 工具(地盘装配后不可变,故重建)。
    fn probe_earth(
        tmp: &std::path::Path,
        extra: Vec<Arc<dyn crate::palaces::zhen_tool::base::BaseTool>>,
    ) -> Arc<crate::plates::di_earth::EarthPlate> {
        use crate::palaces::gen_store::Store;
        use crate::palaces::kan_io::ChannelManager;
        use crate::palaces::kun_config::{AppConfig, CognitionSection, SecuritySection};
        use crate::palaces::li_skill::SkillRegistry;
        use crate::palaces::qian_permission::PermissionMatrix;
        use crate::palaces::zhen_tool::ToolRegistry;
        use crate::palaces::zhen_tool::builtin::exec::shell::ShellTool;
        use crate::palaces::zhen_tool::builtin::fs::read_file::ReadFileTool;
        use crate::palaces::zhen_tool::builtin::fs::write_file::WriteFileTool;
        use crate::plates::shen_spirit::SpiritPlate;
        use crate::plates::shen_spirit::completion_check::CompletionChecklist;

        let security = SecuritySection {
            workspace_root: Some(tmp.to_str().unwrap().to_string()),
            sandbox_mode: crate::palaces::kun_config::SandboxMode::Disabled,
            ..SecuritySection::default()
        };
        let config = AppConfig {
            host: "127.0.0.1".into(),
            port: 8080,
            web_dir: None,
            providers: std::collections::HashMap::new(),
            default_main_model_provider: None,
            default_aux_model_provider: None,
            system_prompt: crate::palaces::kun_config::DEFAULT_SYSTEM_PROMPT.to_string(),
            security: security.clone(),
            mcp_servers: vec![],
            bots: Default::default(),
            hooks: vec![],
            cognition: CognitionSection::default(),
            agent: Default::default(),
        };
        let config_loader = Arc::new(
            crate::palaces::kun_config::ConfigLoader::from_app_config(config),
        );
        let permissions = Arc::new(PermissionMatrix::from_config(
            &security,
            &tmp.join("workspace"),
            tmp.to_path_buf().join("backups"),
        ));
        let mut toollist = ToolRegistry::new();
        toollist.register(Arc::new(ReadFileTool::new()));
        toollist.register(Arc::new(WriteFileTool::new()));
        toollist.register(Arc::new(ShellTool::new()));
        for t in extra {
            toollist.register(t);
        }
        let store = Arc::new(Store::open(tmp.join("store.db").to_str().unwrap()));
        let dummy_profile = crate::palaces::kun_config::ProviderProfile {
            kind: "openai".into(),
            models: vec!["dummy".into()],
            default_aux_model: None,
            default_main_model: None,
            api_key: "sk-dummy".into(),
            base_url: "http://localhost:1/v1".into(),
            max_tokens: Some(256),
            context_window: None,
            priority: None,
            cost_multiplier: None,
        };
        Arc::new(crate::plates::di_earth::EarthPlate {
            io: Arc::new(ChannelManager::default()),
            config: config_loader,
            tools: Arc::new(toollist),
            main_core: Arc::new(JiaCore::new(&dummy_profile, "dummy")),
            aux_core: None,
            permissions,
            skills: Arc::new(std::sync::RwLock::new(SkillRegistry::new())),
            cron: crate::palaces::zhen_tool::builtin::cron::CronStore::new(
                tmp.to_path_buf().join("cron"),
            ),
            task_store: crate::palaces::zhen_tool::builtin::exec::task::TaskStore::new(),
            background_tasks: crate::palaces::zhen_tool::builtin::exec::background_task::BackgroundTaskStore::new(),
            subagent_batch: Arc::new(
                crate::plates::tian_heaven::subagent_batch::SubagentBatch::new(),
            ),
            store_async: crate::palaces::gen_store::async_store::StoreAsync::new(store.clone()),
            store,
            spirit: Arc::new(SpiritPlate::new()),
            completion_checklist: Arc::new(CompletionChecklist::new()),
            user_hooks: Arc::new(Vec::new()),
            session_bus: Arc::new(crate::plates::ren_human::SessionBus::new()),
            data_dir: tmp.to_path_buf(),
            pid_path: tmp.to_path_buf().join("gateway.pid"),
            backup_dir: tmp.to_path_buf().join("backups"),
        })
    }

    /// kind = "anthropic" → native tools 路径(use_native_tools 为 true)。
    fn native_core(provider: Box<dyn LlmProvider>) -> JiaCore {
        let router = crate::palaces::zhong_core::ProviderRouter::new(
            std::iter::once((0u32, provider)).collect(),
        );
        JiaCore::with_router(router, "anthropic".into(), "mock".into(), 8192)
    }

    fn native_tc(id: &str, name: &str, params: serde_json::Value) -> StreamChunk {
        StreamChunk::NativeToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: params.to_string(),
        }
    }

    fn tool_history(agent: &super::super::Agent) -> Vec<(&str, &str)> {
        agent
            .history
            .iter()
            .filter_map(|e| match e {
                HistoryEntry::ToolCall { tool, output, .. } => Some((tool.as_str(), output.as_str())),
                _ => None,
            })
            .collect()
    }

    /// ①②③ 读调用在流毕前已开始执行;写调用等流毕;回填按声明序。
    /// Provider 脚本:probe_read → 停车等待其执行置位(早派发生效才能走
    /// 完)→ write_file → 停车窗口内观测写目标文件(不应出现)→ 流毕。
    #[tokio::test]
    async fn early_dispatch_read_runs_before_stream_end_write_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let target = ws.join("w.txt");
        let probe_fired = Arc::new(AtomicUsize::new(0));
        let read_seen = Arc::new(AtomicUsize::new(0));
        let write_seen = Arc::new(AtomicUsize::new(0));
        let write_flag = Arc::new(AtomicUsize::new(0));
        // 写落盘侦测:文件出现即置位。
        {
            let path = target.clone();
            let flag = write_flag.clone();
            tokio::spawn(async move {
                for _ in 0..3000 {
                    if path.exists() {
                        flag.store(1, Ordering::SeqCst);
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                }
            });
        }

        let probe: Arc<dyn crate::palaces::zhen_tool::base::BaseTool> = Arc::new(ProbeTool {
            fired: probe_fired.clone(),
        });
        let earth = probe_earth(tmp.path(), vec![probe]);
        let provider = ChunkScriptProvider::boxed(vec![
            vec![
                ChunkItem::Chunk(native_tc(
                    "c1",
                    "probe_read",
                    serde_json::json!({"path": "a.txt"}),
                )),
                // ① 若早派发生效,probe 在流中执行,此处 3s 窗口内必观测到。
                ChunkItem::WaitCount {
                    until: probe_fired.clone(),
                    target: 1,
                    window_ms: 3000,
                    saw: read_seen.clone(),
                },
                ChunkItem::Chunk(native_tc(
                    "c2",
                    "write_file",
                    serde_json::json!({"path": target.display().to_string(), "content": "late-write"}),
                )),
                // ② 写调用不得在流中执行:150ms 窗口内文件不应出现。
                ChunkItem::WaitCount {
                    until: write_flag.clone(),
                    target: 1,
                    window_ms: 150,
                    saw: write_seen.clone(),
                },
            ],
            vec![ChunkItem::Chunk(StreamChunk::Delta("done".to_string()))],
        ]);
        let core = native_core(provider);

        let (agent, _events) = run_agent(earth, &core).await;

        assert_eq!(
            read_seen.load(Ordering::SeqCst),
            1,
            "① probe_read must have EXECUTED before the stream ended"
        );
        assert_eq!(
            write_seen.load(Ordering::SeqCst),
            0,
            "② write_file must NOT execute before the stream ended"
        );
        // 流毕后写调用照常执行。
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "late-write",
            "write_file must run in the post-stream batch"
        );
        // ③ 回填按声明序:probe_read 在前,write_file 在后,各带自己的输出。
        let hist = tool_history(&agent);
        assert_eq!(hist.len(), 2, "history: {hist:?}");
        assert_eq!(hist[0].0, "probe_read");
        assert_eq!(hist[0].1, "probe-output");
        assert_eq!(hist[1].0, "write_file");
    }

    /// ④ 需确认调用一律推迟到流毕批:景门关闭使 Direct 降级为 Guarded
    /// (需确认,confirmation_override 自动放行)—— probe 不得在流中执行,
    /// 但流毕后照常执行入账。
    #[tokio::test]
    async fn early_dispatch_confirmation_calls_defer_to_post_stream() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("workspace")).unwrap();
        let probe_fired = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(AtomicUsize::new(0));

        let probe: Arc<dyn crate::palaces::zhen_tool::base::BaseTool> = Arc::new(ProbeTool {
            fired: probe_fired.clone(),
        });
        let earth = probe_earth(tmp.path(), vec![probe]);
        let mut human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        // Direct → Guarded 降级:prepare 将请求确认(override 自动同意)。
        human_plate.close_gate(HumanGate::JingXiangMen);
        human_plate.confirmation_override = Some(true);

        let provider = ChunkScriptProvider::boxed(vec![
            vec![
                ChunkItem::Chunk(native_tc(
                    "c1",
                    "probe_read",
                    serde_json::json!({"path": "a.txt"}),
                )),
                // 若 probe 在流中执行,150ms 窗口内必然观测到 fired>0。
                ChunkItem::WaitCount {
                    until: probe_fired.clone(),
                    target: 1,
                    window_ms: 150,
                    saw: seen.clone(),
                },
            ],
            vec![ChunkItem::Chunk(StreamChunk::Delta("done".to_string()))],
        ]);
        let core = native_core(provider);

        let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
        let cancel = CancellationToken::new();
        let mut agent = super::super::Agent::new("u7-confirm".into(), earth.clone());
        let ctx = RunContext {
            core: &core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent.run(vec![Message::text(Role::User, "hi")], &ctx).await;

        assert_eq!(
            seen.load(Ordering::SeqCst),
            0,
            "④ a confirmation-needing call must NOT execute during streaming"
        );
        assert_eq!(
            probe_fired.load(Ordering::SeqCst),
            1,
            "the deferred call must still execute in the post-stream batch"
        );
        let hist = tool_history(&agent);
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].0, "probe_read");
        assert_eq!(hist[0].1, "probe-output");
        let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done)));
    }

    /// ⑤ XML 路径行为不变:非 native provider(kind=mock)流毕一次性解析,
    /// ToolCall 事件不得先于 StreamEnd 出现。
    #[tokio::test]
    async fn xml_path_has_no_early_dispatch() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = temp_earth(tmp.path());
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let target = ws.join("x.txt");
        std::fs::write(&target, "xml-content").unwrap();
        let text = format!(
            "reading\n<tool_call>\n{{\"tool\": \"read_file\", \"parameters\": {{\"path\": \"{}\"}}}}\n</tool_call>",
            target.display()
        );
        let text: &'static str = Box::leak(text.into_boxed_str());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider: Box<dyn LlmProvider> = Box::new(ScriptedProvider::new(
            vec![MockStep::Complete(text), MockStep::Complete("done")],
            calls.clone(),
        ));
        let core = router_core(vec![provider]);

        let (agent, events) = run_agent(earth, &core).await;

        let first_tool_call = events
            .iter()
            .position(|e| matches!(e, AgentEvent::ToolCall { .. }));
        let stream_end = events
            .iter()
            .position(|e| matches!(e, AgentEvent::StreamEnd));
        assert!(
            matches!((first_tool_call, stream_end), (Some(t), Some(s)) if t > s),
            "⑤ XML path: first ToolCall event must come AFTER StreamEnd: {events:?}"
        );
        let hist = tool_history(&agent);
        assert_eq!(hist.len(), 1);
        assert!(hist[0].1.contains("xml-content"));
    }
}

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::certainty::{CertaintyParams, TurnCertainty};
use crate::geju::GeJu;
use crate::palaces::Palace;
use crate::palaces::xun_context::ContextWindow;
use crate::palaces::zhong_core::JiaCore;
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

pub use super::loop_dispatch::dispatch_one_tool;

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

    #[tracing::instrument(skip(self, messages, ctx), fields(session = %self.id))]
    pub async fn run(&mut self, messages: Vec<Message>, ctx: &RunContext<'_>) {
        let _ = ctx.tx.send(AgentEvent::Session {
            session_id: self.id.clone(),
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
                let skip = {
                    let turns_since = self.turn_count.saturating_sub(self.cc_last_turn);
                    let saved_pct = if self.cc_last_turn > 0 && self.cc_tokens_before > 0 {
                        ((self.cc_tokens_before - self.cc_tokens_after) * 100)
                            / self.cc_tokens_before
                    } else {
                        100
                    };
                    self.cc_last_turn > 0 && turns_since <= 2 && saved_pct < 10
                };

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
                        let compaction: Option<(usize, &str)> = {
                            let victims_raw = &compact_msgs[start..start + count];
                            // FNV-1a dedup: remove duplicate messages before feeding to LLM
                            let mut seen = std::collections::HashSet::new();
                            let victims: Vec<crate::types::Message> = victims_raw.iter()
                                .map(|m| {
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
                            let prev = self.compaction_summary.as_deref();
                            match ContextWindow::summarize(
                                &victims,
                                ctx.core,
                                Some(ctx.cancel_token.clone()),
                                prev,
                            )
                            .await
                            {
                                Ok(summary_msg) => {
                                    // Store for next iterative update
                                    self.compaction_summary = Some(summary_msg.content.clone());
                                    let content = format!(
                                        "[CONTEXT COMPACTION -- REFERENCE ONLY]\n{}",
                                        summary_msg.content
                                    );
                                    let hist_start = msg_indices[start];
                                    let hist_end = msg_indices[start + count - 1] + 1;
                                    self.history.drain(hist_start..hist_end);
                                    self.history
                                        .insert(hist_start, HistoryEntry::system(content));
                                    let (tokens, msgs) = rebuild(&self.history);
                                    llm_messages = msgs;
                                    Some((tokens, "summarize"))
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
                            // Compaction skipped (cancelled) — leave history
                            // untouched and wind down immediately; no LLM call
                            // is issued for a session that is going away. As
                            // with the other cancel paths, SessionEnd/Done are
                            // left to the caller's teardown.
                            return;
                        };

                        // Anti-thrashing state
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
                } else {
                    tracing::debug!(
                        tokens = pre_tokens,
                        threshold,
                        last_saved_pct = ((self.cc_tokens_before - self.cc_tokens_after) * 100)
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
                            native_tool_calls.push(crate::stems::action::ToolCall {
                                id,
                                name,
                                parameters: params,
                            });
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
                            self.save_history_now().await;
                            return;
                        }
                        Some(Err(e)) => {
                            // Check the retry budget BEFORE failover so the
                            // final failure doesn't pointlessly flip the
                            // router's active provider.
                            if e.is_retryable()
                                && self.retry_count < MAX_LLM_RETRIES
                                && ctx.core.try_llm_failover()
                            {
                                if ctx.cancel_token.is_cancelled() {
                                    tracing::info!(session = %self.id, "Agent loop cancelled");
                                    self.save_history_now().await;
                                    return;
                                }
                                tracing::warn!(session = %self.id, error = %e, retry = self.retry_count + 1, "LLM error, re-issuing request with next provider");
                                self.retry_count += 1;
                                // S2: the failed attempt's partial Deltas are
                                // already on the wire — tell frontends to roll
                                // the bubble back before the retried stream
                                // starts appending.
                                let _ = ctx.tx.send(AgentEvent::Retrying {
                                    attempt: self.retry_count,
                                });
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

            // Dispatch tool calls (sequential execution)
            let mut tool_count: usize = 0;
            let mut touched_acc: Vec<String> = Vec::new();

            for tc in &tool_calls {
                // GeJu Layer 3: check failure streak before dispatch
                let max_fail = self.max_consecutive_failures;
                let (output, error, geju_name, execution_mode, heaven_stem, target_palace) =
                    dispatch_one_tool(
                        tc,
                        &self.earth.tools,
                        ctx.human_plate,
                        ctx.event_bus,
                        ctx.hook_registry,
                        &ctx.tx,
                        &mut touched_acc,
                        &self.output_budget,
                        &mut self.tool_failure_count,
                        max_fail,
                        self.interaction_mode,
                        &self.earth.user_hooks,
                        &self.exec_ctx,
                        &self.principles,
                        self.manas.atma_graha,
                    )
                    .await;

                // Track consecutive failures per tool (GeJu Layer 3 runtime supplement)
                if error.is_some() {
                    *self.tool_failure_count.entry(tc.name.clone()).or_insert(0) += 1;
                } else {
                    self.tool_failure_count.remove(&tc.name);
                }

                // P6 · worktree transitions (only on tool success).
                // enter_worktree already ran `git worktree add`; here we swap
                // the ExecContext (O(1)) so subsequent tools in this batch see
                // the worktree-scoped PermissionMatrix. exit_worktree restores
                // the original ExecContext and optionally removes the worktree.
                if error.is_none() {
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
                    tool_count: tool_calls.len() as u32,
                });

                // Push structured tool call entry into history
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

                tool_count += 1;
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
                    "shell" | "write_file" | "patch_file" => {
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
                        self.interaction_mode = InteractionMode::Planning;
                        ctx.human_plate.sync_jingjue_with_mode(true); // Planning → suppress alerts
                        tracing::info!(session = %self.id, "entered planning mode");
                        let _ = ctx
                            .tx
                            .send(AgentEvent::InteractionModeChanged { planning: true });
                    }
                    "exit_plan_mode" => {
                        self.interaction_mode = InteractionMode::Normal;
                        ctx.human_plate.sync_jingjue_with_mode(false); // Normal → resume alerts
                        tracing::info!(session = %self.id, "exited planning mode");
                        let _ = ctx
                            .tx
                            .send(AgentEvent::InteractionModeChanged { planning: false });
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
                err: ProviderError::RateLimited { body: "429".into() },
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
                        },
                    },
                    MockStep::FailAfter {
                        partial: "junk",
                        err: ProviderError::ServerError {
                            status: 500,
                            body: "boom".into(),
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
        // Four failures per provider: enough for two fully-exhausted turns.
        let mk = |calls: &Arc<AtomicUsize>| -> Box<dyn LlmProvider> {
            Box::new(ScriptedProvider::new(
                (0..4)
                    .map(|_| MockStep::FailAfter {
                        partial: "junk",
                        err: ProviderError::ServerError {
                            status: 500,
                            body: "boom".into(),
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
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(agent.retry_count, 3);

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
        // Note: not exactly 4 — the circuit breaker legitimately opens after
        // the providers' repeated consecutive failures across both turns and
        // cuts turn 2 short (failover finds no closed breaker). The per-turn
        // reset is proven by turn 2 making ANY retry at all: a stuck budget
        // (review Issue 1) would allow exactly 1 request and zero retries.
        let turn2_calls = calls.load(Ordering::SeqCst) - 4;
        assert!(
            turn2_calls >= 2,
            "turn 2 must get a fresh retry budget (stuck budget allows only 1 request, got {turn2_calls})"
        );
        // Exact count is coupled to circuit-breaker internals; the invariant
        // is that turn 2 retried at all (retry_count climbed from 0).
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
                    err: ProviderError::RateLimited { body: "429".into() },
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
                    err: ProviderError::RateLimited { body: "429".into() },
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
}

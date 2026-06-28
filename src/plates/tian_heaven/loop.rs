use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::geju::GeJu;
use crate::palaces::Palace;
use crate::palaces::xun_context::ContextWindow;
use crate::palaces::zhong_core::JiaCore;
use crate::plates::ren_human::HumanPlate;
use crate::plates::shen_spirit::hook::{HookEvent, HookRegistry, SpiritType, fire_void_hooks};
use crate::plates::shen_spirit::{EventBus, RuntimeEvent};
use crate::stems::Stem;
use crate::telemetry::metrics::{JIA_LLM_DURATION_SECONDS, JIA_TOKENS_COMPACTED_TOTAL};
use crate::types::{HistoryEntry, Message, Role, to_llm_messages};
use crate::vijnana::alaya::SeedStore;
use crate::vijnana::mano::TurnSnapshot;
use crate::vijnana::xunxi::signal::SignalDetector;

// ── Re-exports from split submodules ────────────────────────────

pub use super::loop_dispatch::dispatch_one_tool;
pub use super::loop_events::AgentEvent;
pub use super::loop_hooks::{CompiledHook, UserHookEvent, run_pre_tool_hooks};
pub use super::loop_parse::parse_tool_calls;

// ── Agent::run ─────────────────────────────────────────────────

impl super::Agent {
    #[tracing::instrument(skip(self, messages, core, human_plate, event_bus, hook_registry, tx, cancel_token), fields(session = %self.id))]
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &mut self,
        messages: Vec<Message>,
        core: &JiaCore,
        human_plate: &HumanPlate,
        event_bus: &EventBus,
        hook_registry: &HookRegistry,
        tx: mpsc::UnboundedSender<AgentEvent>,
        cancel_token: &CancellationToken,
    ) {
        let _ = tx.send(AgentEvent::Session {
            session_id: self.id.clone(),
        });

        // L1 perfuming: detect explicit user signals before appending to history (zero-LLM)
        for msg in &messages {
            if matches!(msg.role, Role::User) {
                SignalDetector::process(&self.earth.store, &self.id, &msg.content);
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
        if let Ok(json) = serde_json::to_string(&self.history)
            && let Err(e) = self.earth.store.save_session(&self.id, &json)
        {
            tracing::warn!(session = %self.id, error = %e, "Failed to save initial session");
        }

        loop {
            self.turn_count += 1;
            if self.turn_count > self.max_turns {
                tracing::warn!(
                    session = %self.id,
                    turns = self.turn_count,
                    "Agent hit max turn limit"
                );
                let _ = tx.send(AgentEvent::Error(format!(
                    "Reached maximum turns ({})",
                    self.max_turns
                )));
                break;
            }

            event_bus.emit(RuntimeEvent::TurnStart {
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
            let system_prompt = self.build_system_prompt(core);
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
                let _ = tx.send(AgentEvent::ContextPressure {
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
                    event_bus.emit(RuntimeEvent::GeJuResult {
                        tool: "compaction".into(),
                        pattern: gr.name.clone(),
                        mode: format!("{:?}", gr.execution_mode).to_lowercase(),
                    });

                    let _ = tx.send(AgentEvent::Compacting);

                    // Build indexed message list for compaction (tool calls → User messages)
                    let compact_msgs: Vec<Message> = to_llm_messages(&self.history);
                    let msg_indices: Vec<usize> = self
                        .history
                        .iter()
                        .enumerate()
                        .filter_map(|(i, e)| if e.is_message() { Some(i) } else { None })
                        .collect();
                    let (start, count) = self.context_window.victim_range(&compact_msgs);
                    if count > 0 {
                        let messages_before = self.history.len();
                        let rebuild = |h: &Vec<HistoryEntry>| {
                            let mut msgs = vec![Message::text(Role::System, system_full.clone())];
                            msgs.extend(to_llm_messages(h));
                            (ContextWindow::count_tokens(&msgs), msgs)
                        };
                        let (t_after, method) = {
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
                                    let hash_key = crate::vijnana::xunxi::distillation::fnv1a_hash(&format!("{role_tag}:{}", m.content));
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
                                core,
                                Some(cancel_token.clone()),
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
                                    (tokens, "summarize")
                                }
                                Err(e) => {
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
                                    (tokens, "fit")
                                }
                            }
                        };

                        // Anti-thrashing state
                        self.cc_last_turn = self.turn_count;
                        self.cc_tokens_before = pre_tokens;
                        self.cc_tokens_after = t_after;

                        fire_void_hooks(
                            hook_registry,
                            event_bus,
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
            let cancel = cancel_token.clone();
            let llm_start = std::time::Instant::now();
            let mut infer_messages = llm_messages;
            if matches!(infer_messages.first().map(|m| m.role), Some(Role::System)) {
                infer_messages.remove(0);
            }

            // Build tool schemas for native tools API (openai/anthropic/gemini).
            let use_native = crate::palaces::zhong_core::use_native_tools(&core.provider_kind);
            let tool_schemas: Option<Vec<crate::stems::action::ToolSchema>> = if use_native {
                let schemas: Vec<_> = self
                    .active_tools
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
            let tools_ref: Option<&[crate::stems::action::ToolSchema]> =
                tool_schemas.as_deref();

            let mut stream =
                core.infer_with_system(infer_messages, system_prompt, tools_ref, Some(cancel));
            let mut full_response = String::new();
            let mut native_tool_calls: Vec<crate::stems::action::ToolCall> = Vec::new();

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
                        let _ = tx.send(AgentEvent::Delta(delta));
                    }
                    Some(Ok(crate::palaces::zhong_core::StreamChunk::Usage {
                        input_tokens,
                        output_tokens,
                    })) => {
                        event_bus.emit(RuntimeEvent::LlmUsage {
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
                    Some(Err(e)) => {
                        tracing::error!(session = %self.id, error = %e, "LLM inference error");
                        let _ = tx.send(AgentEvent::Error(e));
                        return;
                    }
                    None => break,
                }
            }

            JIA_LLM_DURATION_SECONDS.observe(llm_start.elapsed().as_secs_f64());

            // Notify frontend that LLM stream ended (freeze bubble A)
            let _ = tx.send(AgentEvent::StreamEnd);

            // Record assistant response in history
            let response_len = full_response.len();

            // Strip trailing JSON fragments + extra blank lines that some
            // models emit before the native tool call.
            let has_native = !native_tool_calls.is_empty();
            if has_native
                && let Some(pos) = full_response.rfind(|c: char| {
                    matches!(c, '.' | '?' | '!' | '。' | '？' | '！')
                })
            {
                let after_sentence = &full_response[pos..];
                let char_len = after_sentence.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                let after = &full_response[pos + char_len..];
                if after.contains('{') {
                    full_response.truncate(pos + char_len);
                }
            }
            // Trim trailing whitespace so the tool card sits directly after text.
            full_response = full_response.trim_end().to_string();

            // Parse tool calls — prefer native (API-level) over XML text parsing.
            let tool_calls: Vec<crate::stems::action::ToolCall> =
                if has_native {
                    native_tool_calls
                } else {
                    let tool_names: Vec<&str> =
                        self.active_tools.list_names().iter().map(|s| s.as_str()).collect();
                    let (_clean_text, calls) = parse_tool_calls(&full_response, &tool_names);
                    calls
                };

            self.history.push(HistoryEntry::assistant(full_response));

            tracing::info!(
                session = %self.id,
                response_len,
                tool_call_count = tool_calls.len(),
                "Parsed tool calls from LLM response"
            );

            fire_void_hooks(
                hook_registry,
                event_bus,
                SpiritType::TengShe,
                Stem::Ren,
                HookEvent::LlmResponse {
                    response_len,
                    tool_call_count: tool_calls.len(),
                },
            );

            if tool_calls.is_empty() {
                event_bus.emit(RuntimeEvent::TurnEnd {
                    turn: self.turn_count as u64,
                });
                break;
            }

            // Notify frontend that tool batch is starting (create bubble B)
            let _ = tx.send(AgentEvent::ToolBatchStart);

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
                        &self.active_tools,
                        human_plate,
                        event_bus,
                        hook_registry,
                        &tx,
                        &mut touched_acc,
                        &self.output_budget,
                        &mut self.tool_failure_count,
                        max_fail,
                        self.interaction_mode,
                        &self.earth.user_hooks,
                    )
                    .await;

                // Track consecutive failures per tool (GeJu Layer 3 runtime supplement)
                if error.is_some() {
                    *self.tool_failure_count.entry(tc.name.clone()).or_insert(0) += 1;
                } else {
                    self.tool_failure_count.remove(&tc.name);
                }

                // P6 · worktree transitions (only on tool success). enter_worktree
                // already ran `git worktree add`; here we rebuild a sub-matrix
                // registry scoped to the worktree and swap active_tools so
                // subsequent file/shell/git tools in this batch target the
                // worktree. exit_worktree restores earth.tools and optionally
                // removes the worktree.
                if error.is_none() {
                    if tc.name == "enter_worktree"
                        && let Some(name) = tc.parameters.get("name").and_then(|v| v.as_str())
                    {
                        if self.worktree_root.is_none() {
                            let main_root = self.earth.permissions.sandbox.project_root.clone();
                            let path = crate::palaces::zhen_tool::builtin::worktree::worktree_path(
                                &main_root, name,
                            );
                            let sub = self.earth.rebuild_tools_for_root(&path);
                            self.active_tools = sub;
                            self.worktree_root = Some(path.clone());
                            tracing::info!(
                                session = %self.id,
                                worktree = %path.display(),
                                "entered worktree (active_tools swapped)"
                            );
                        } else {
                            tracing::warn!("enter_worktree ignored: already in a worktree");
                        }
                    } else if tc.name == "exit_worktree" {
                        if let Some(wt) = self.worktree_root.take() {
                            self.active_tools = self.earth.tools.clone();
                            let action = tc
                                .parameters
                                .get("action")
                                .and_then(|v| v.as_str())
                                .unwrap_or("keep");
                            if action == "remove" {
                                let main_root = self.earth.permissions.sandbox.project_root.clone();
                                if let Err(e) =
                                    crate::palaces::zhen_tool::builtin::worktree::remove_worktree(
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
                            tracing::info!(session = %self.id, "exited worktree (active_tools restored)");
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
                        self.interaction_mode = super::InteractionMode::Planning;
                        tracing::info!(session = %self.id, "entered planning mode");
                        let _ = tx.send(AgentEvent::InteractionModeChanged { planning: true });
                    }
                    "exit_plan_mode" => {
                        self.interaction_mode = super::InteractionMode::Normal;
                        tracing::info!(session = %self.id, "exited planning mode");
                        let _ = tx.send(AgentEvent::InteractionModeChanged { planning: false });
                    }
                    _ => {}
                }
            }

            self.activate_skills(&touched_paths);

            fire_void_hooks(
                hook_registry,
                event_bus,
                SpiritType::LiuHe,
                Stem::Geng,
                HookEvent::BatchEnded {
                    tool_count,
                    turn: self.turn_count as u64,
                },
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

            event_bus.emit(RuntimeEvent::TurnEnd {
                turn: self.turn_count as u64,
            });

            // Incremental persist: save history after each turn
            if let Ok(json) = serde_json::to_string(&self.history)
                && let Err(e) = self.earth.store.save_session(&self.id, &json)
            {
                tracing::warn!(session = %self.id, error = %e, "Failed to save session incrementally");
            }
        }

        event_bus.emit(RuntimeEvent::SessionEnd {
            session_id: self.id.clone(),
            turns: self.turn_count as u64,
        });

        let _ = tx.send(AgentEvent::Done);
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

    #[test]
    fn compiled_hook_compiles_regex_and_matches() {
        let cfg = crate::palaces::kun_config::HookConfig {
            event: "pre_tool_use".into(),
            tool_pattern: Some("shell|git".into()),
            command: "true".into(),
            block_on_exit: true,
        };
        let h = CompiledHook::compile(&cfg).expect("compile");
        assert_eq!(h.event, UserHookEvent::PreToolUse);
        assert!(h.matches_tool("shell"));
        assert!(h.matches_tool("git"));
        assert!(!h.matches_tool("read_file"));
    }

    #[test]
    fn compiled_hook_no_pattern_matches_all() {
        let cfg = crate::palaces::kun_config::HookConfig {
            event: "post_tool_use".into(),
            tool_pattern: None,
            command: "true".into(),
            block_on_exit: false,
        };
        let h = CompiledHook::compile(&cfg).expect("compile");
        assert_eq!(h.event, UserHookEvent::PostToolUse);
        assert!(h.matches_tool("anything"));
    }

    #[test]
    fn compiled_hook_rejects_bad_regex() {
        let cfg = crate::palaces::kun_config::HookConfig {
            event: "pre_tool_use".into(),
            tool_pattern: Some("(".into()), // invalid regex
            command: "true".into(),
            block_on_exit: false,
        };
        assert!(CompiledHook::compile(&cfg).is_err());
    }

    #[tokio::test]
    async fn pre_tool_hook_blocks_on_nonzero_exit() {
        // A hook that exits 1 with block_on_exit must block.
        let cfg = crate::palaces::kun_config::HookConfig {
            event: "pre_tool_use".into(),
            tool_pattern: Some("shell".into()),
            command: "exit 1".into(),
            block_on_exit: true,
        };
        let hooks = vec![CompiledHook::compile(&cfg).unwrap()];
        let res = run_pre_tool_hooks(&hooks, "shell", &serde_json::json!({})).await;
        assert!(res.is_err(), "expected block");
        assert!(res.unwrap_err().contains("blocked"));
    }

    #[tokio::test]
    async fn pre_tool_hook_allows_on_zero_exit() {
        let cfg = crate::palaces::kun_config::HookConfig {
            event: "pre_tool_use".into(),
            tool_pattern: Some("shell".into()),
            command: "exit 0".into(),
            block_on_exit: true,
        };
        let hooks = vec![CompiledHook::compile(&cfg).unwrap()];
        let res = run_pre_tool_hooks(&hooks, "shell", &serde_json::json!({})).await;
        assert!(res.is_ok(), "expected allow");
    }

    #[tokio::test]
    async fn pre_tool_hook_skips_non_matching_tool() {
        // Hook targets "git"; a "shell" call must not be blocked even if the
        // command would exit non-zero.
        let cfg = crate::palaces::kun_config::HookConfig {
            event: "pre_tool_use".into(),
            tool_pattern: Some("git".into()),
            command: "exit 1".into(),
            block_on_exit: true,
        };
        let hooks = vec![CompiledHook::compile(&cfg).unwrap()];
        let res = run_pre_tool_hooks(&hooks, "shell", &serde_json::json!({})).await;
        assert!(res.is_ok(), "non-matching tool must not be blocked");
    }
}

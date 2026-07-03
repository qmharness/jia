use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::sse::{Event, Sse};
use futures::stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::plates::ren_human::HumanPlate;
use crate::plates::tian_heaven::Agent;
use crate::plates::tian_heaven::r#loop::AgentEvent;
use crate::provider;
use crate::provider::LlmProvider;
use crate::telemetry::metrics::{JIA_REQUEST_DURATION_SECONDS, JIA_REQUESTS_TOTAL};
use crate::types::{AgentRequest, ChatRequest, HistoryEntry, Message, Role, StreamEvent};
use crate::vijnana::manas::Manas;

use super::AppState;
use super::auth::CancelOnDropStream;
use crate::utils::truncate_title;

pub async fn handle_chat(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let profile = state
        .providers
        .get(&req.provider)
        .or_else(|| state.providers.get(&state.default_main_provider_name))
        .cloned();
    let requested_model = req.model.clone();

    let boxed: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> = if let Some(
        profile,
    ) = profile
    {
        let model: &str = requested_model
            .as_deref()
            .unwrap_or_else(|| profile.default_main_model());
        let llm: Box<dyn LlmProvider> = provider::create_provider(&profile, model);

        let mut messages = vec![Message::text(Role::System, state.system_prompt.clone())];
        messages.extend(req.messages.into_iter().map(|mut m| {
            if m.role == Role::User {
                m.content = crate::utils::sanitize_message(&m.content);
            }
            m
        }));

        let cancel = tokio_util::sync::CancellationToken::new();
        let stream = llm.infer_stream(messages, None, Some(cancel.clone()));

        let sse_stream = CancelOnDropStream {
            inner: stream.filter_map(|chunk| {
                let event = match chunk {
                    Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta)) => {
                        Some(StreamEvent::Delta { content: delta })
                    }
                    Ok(crate::palaces::zhong_core::StreamChunk::Usage { .. }) => None,
                    Ok(crate::palaces::zhong_core::StreamChunk::CacheHit { .. }) => None,
                    Ok(crate::palaces::zhong_core::StreamChunk::NativeToolCall { .. }) => None,
                    Err(e) => Some(StreamEvent::Error { message: e }),
                };
                let json = serde_json::to_string(&event?).ok()?;
                Some(Ok(Event::default().data(json)))
            }),
            token: cancel,
        };

        let done = tokio_stream::once({
            let done_json = serde_json::to_string(&StreamEvent::Done).unwrap_or_default();
            Ok(Event::default().data(done_json))
        });

        Box::pin(sse_stream.chain(done))
    } else {
        let json = serde_json::to_string(&StreamEvent::Error {
            message: "No LLM provider configured. Set default_provider in [server] section of config.toml.".into(),
        }).unwrap_or_default();
        Box::pin(tokio_stream::once(Ok(Event::default().data(json))))
    };

    Sse::new(boxed).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Resolve project_id from cwd by reading .jia/config.toml.
/// Returns the UUID if found, otherwise generates a new one and creates the project.
pub async fn handle_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AgentRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let profile = state
        .providers
        .get(&req.provider)
        .or_else(|| state.providers.get(&state.default_main_provider_name))
        .cloned();
    let model = req
        .model
        .as_deref()
        .unwrap_or_else(|| {
            profile
                .as_ref()
                .map(|p| p.default_main_model())
                .unwrap_or("")
        })
        .to_string();
    let effective_aux_provider: Option<String> = req
        .aux_provider
        .clone()
        .or_else(|| state.default_aux_model_provider.clone());
    let aux_model: Option<String> = req.aux_model.clone().or_else(|| {
        effective_aux_provider.as_ref().and_then(|aux_name| {
            state
                .providers
                .get(aux_name)
                .map(|p| p.default_main_model().to_string())
        })
    });
    let has_provider = profile.is_some();
    let earth = state.earth.clone();
    // INVARIANT: tx is moved into the spawned task. The only use outside the
    // spawn is the single Session event above (safe — each SSE connection is
    // independent). All other sends happen inside session_lock, guaranteeing
    // per-stream events (delta, tool_call, tool_result) do not interleave.
    let (tx, rx) = mpsc::unbounded_channel();
    let cancel_token = CancellationToken::new();
    let agent_token = cancel_token.clone();

    match (earth, profile) {
        (Some(earth), Some(profile)) => {
            // Resolve session ID (from request or generate new)
            let is_new = req.session_id.as_ref().is_none_or(|s| s.is_empty());
            let session_id = if is_new {
                uuid::Uuid::new_v4().to_string()
            } else {
                req.session_id.clone().unwrap()
            };

            // Insert placeholder row immediately so session appears in list
            // Resolve effective cwd: validate the request cwd, and for old sessions
            // where cwd="." fall back to project_id reverse-lookup.
            let req_cwd = req.cwd.clone().unwrap_or_default();
            let req_pid = req.project_id.clone().unwrap_or_default();

            // Store in session row for new sessions
            if is_new {
                let title = req
                    .messages
                    .iter()
                    .find(|m| m.role == Role::User)
                    .map(|m| truncate_title(&m.content))
                    .unwrap_or_default();
                let _ = earth
                    .store
                    .create_session(&session_id, &title, &req_cwd, &req_pid);
            }

            // Send session ID immediately so client can persist it
            let _ = tx.send(AgentEvent::Session {
                session_id: session_id.clone(),
            });

            // Core creation (pre-spawn — no session state needed).
            let main_core = crate::palaces::zhong_core::JiaCore::new(&profile, &model);
            let aux_core = effective_aux_provider.as_ref().and_then(|aux_name| {
                match state.providers.get(aux_name) {
                    Some(aux_profile) => {
                        let m = aux_model
                            .as_deref()
                            .unwrap_or_else(|| aux_profile.default_main_model());
                        Some(crate::palaces::zhong_core::JiaCore::new(aux_profile, m))
                    }
                    None => aux_model
                        .as_ref()
                        .map(|am| crate::palaces::zhong_core::JiaCore::new(&profile, am)),
                }
            });
            JIA_REQUESTS_TOTAL
                .with_label_values(&[&req.provider, &model])
                .inc();

            // Register session token for cancellation.
            state.session_tokens.register(
                session_id.clone(),
                cancel_token.clone(),
                req.provider.clone(),
                model.clone(),
            );
            let session_tokens = state.session_tokens.clone();
            let sid = session_id.clone();

            // Clone Arc references for the spawned task.
            let earth_for_spawn = earth.clone();
            let store = earth.store.clone();
            let permissions = earth.permissions.clone();
            let event_bus = earth.spirit.event_bus.clone();
            let pending_confirmations = state.pending_confirmations.clone();

            tokio::spawn(async move {
                // —— session_lock + cancel-aware wait ——
                let session_lock = {
                    let mut map = earth_for_spawn.session_locks.lock().unwrap();
                    map.retain(|_, v| Arc::strong_count(v) > 1);
                    map.entry(sid.clone())
                        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                        .clone()
                };

                tokio::select! {
                    _ = agent_token.cancelled() => {
                        // Client disconnected or cancelled while queued — bail out.
                        session_tokens.remove(&sid);
                    }
                    guard = session_lock.lock() => {
                        let _guard = guard;

                        // —— lock acquired: load session state ——
                        let history: Vec<HistoryEntry> = earth_for_spawn.store.load_session_history(&sid);
                        let manas: Manas = earth_for_spawn.store.load_manas().ok().flatten()
                            .and_then(|json| serde_json::from_str(&json).ok())
                            .unwrap_or_default();
                        let distilled_hashes = earth_for_spawn.store.load_distilled_hashes(&sid);

                        // Resolve effective cwd
                        let effective_cwd = if !req_cwd.is_empty() && req_cwd != "." {
                            req_cwd.clone()
                        } else if !req_pid.is_empty() {
                            // Old session: reverse-lookup cwd from project
                            earth_for_spawn.store.get_project(&req_pid).ok().flatten()
                                .and_then(|p| p.get("cwd").and_then(|v| v.as_str()).map(String::from))
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };

                        // Validate: cwd must be a valid Jia project directory
                        let config_path = std::path::Path::new(&effective_cwd)
                            .join(".jia").join("config.toml");
                        if effective_cwd.is_empty() || !config_path.exists() {
                            let _ = tx.send(AgentEvent::Error(
                                "cwd must be a valid Jia project directory (with .jia/config.toml)".into()
                            ));
                            let _ = tx.send(AgentEvent::Done);
                            session_tokens.remove(&sid);
                            return;
                        }

                        let mut agent = Agent::with_session(sid.clone(), earth_for_spawn.clone(), history, manas, distilled_hashes);
                        agent.exec_ctx = earth_for_spawn.build_worktree_exec_ctx(
                            std::path::Path::new(&effective_cwd)
                        );
                        let human_plate = HumanPlate::with_state(permissions, pending_confirmations);

                        let _start = std::time::Instant::now();
                        agent.run(req.messages, &main_core, &human_plate, &event_bus, &earth_for_spawn.spirit.hook_registry, tx, &agent_token).await;
                        agent.post_loop(store, &main_core, aux_core.as_ref()).await;
                        JIA_REQUEST_DURATION_SECONDS.observe(_start.elapsed().as_secs_f64());
                        session_tokens.remove(&sid);
                    }
                }
            });
        }
        _ => {
            let msg = if has_provider {
                "Agent mode not available: EarthPlate not initialized"
            } else {
                "No LLM provider configured. Set default_provider in [server] section of config.toml."
            };
            let _ = tx.send(AgentEvent::Error(msg.into()));
            drop(tx);
        }
    }

    let sse_stream = CancelOnDropStream {
        inner: UnboundedReceiverStream::new(rx).map(|event| {
            let stream_event = match event {
                AgentEvent::Delta(content) => StreamEvent::Delta { content },
                AgentEvent::StreamEnd => StreamEvent::StreamEnd,
                AgentEvent::ToolBatchStart => StreamEvent::ToolBatchStart,
                AgentEvent::ToolCall { tool, input } => StreamEvent::ToolCall { tool, input },
                AgentEvent::ToolResult {
                    tool,
                    output,
                    error,
                    geju,
                    execution_mode,
                } => StreamEvent::ToolResult {
                    tool,
                    output,
                    error,
                    geju,
                    execution_mode,
                },
                AgentEvent::Session { session_id } => StreamEvent::Session { session_id },
                AgentEvent::ConfirmRequest {
                    id,
                    tool,
                    reason,
                    timeout_secs,
                    token,
                } => StreamEvent::ConfirmationRequest {
                    id,
                    tool,
                    reason,
                    timeout_secs,
                    token,
                },
                AgentEvent::UserQuestion {
                    id,
                    question,
                    timeout_secs,
                    token,
                    options,
                } => StreamEvent::UserQuestion {
                    id,
                    question,
                    timeout_secs,
                    token,
                    options,
                },
                AgentEvent::Done => StreamEvent::Done,
                AgentEvent::Error(message) => StreamEvent::Error { message },
                AgentEvent::InteractionModeChanged { planning } => {
                    StreamEvent::InteractionModeChanged { planning }
                }
                AgentEvent::ContextPressure { tokens, threshold } => {
                    StreamEvent::ContextPressure { tokens, threshold }
                }
                AgentEvent::Compacting => StreamEvent::Compacting,
            };
            let json = serde_json::to_string(&stream_event).unwrap_or_default();
            Ok(Event::default().data(json))
        }),
        token: cancel_token,
    };

    Sse::new(sse_stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use super::super::SessionTokens;
    use super::*;
    use crate::palaces::dui_gateway::auth::RateLimiter;
    use crate::palaces::gen_store::Store;
    use crate::palaces::kan_io::ChannelManager;
    use crate::palaces::kun_config::{AppConfig, BotsSection, ConfigLoader, SecuritySection};
    use crate::palaces::li_skill::SkillRegistry;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::palaces::zhen_tool::builtin::cron::CronStore;
    use crate::palaces::zhen_tool::builtin::task::TaskStore;
    use crate::palaces::zhen_tool::registry::ToolRegistry;
    use crate::palaces::zhong_core::JiaCore;
    use crate::plates::di_earth::EarthPlate;
    use crate::plates::shen_spirit::SpiritPlate;
    use crate::stems::action::ExecContext;
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn make_test_state() -> Arc<AppState> {
        let (io, _rx) = ChannelManager::new();
        let mock_core = Arc::new(JiaCore::with_mock(vec!["Hello from mock LLM".into()]));
        let store = {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("test.db");
            std::mem::forget(dir);
            Arc::new(Store::open(path.to_str().unwrap()))
        };
        let registry = Arc::new(std::sync::RwLock::new(SkillRegistry::new()));
        let dirs = tempfile::tempdir().unwrap();
        let earth = EarthPlate {
            io: Arc::new(io),
            config: Arc::new(ConfigLoader::from_app_config(AppConfig {
                host: "127.0.0.1".into(),
                port: 3000,
                providers: HashMap::new(),
                default_main_model_provider: None,
                default_aux_model_provider: None,
                security: SecuritySection::default(),
                mcp_servers: vec![],
                bots: BotsSection::default(),
                hooks: vec![],
            })),
            tools: Arc::new(ToolRegistry::new()),
            main_core: mock_core,
            aux_core: None,
            permissions: Arc::new(PermissionMatrix::default()),
            skills: registry.clone(),
            cron: CronStore::new(dirs.path().join("cron")),
            task_store: TaskStore::new(),
            store: store.clone(),
            spirit: Arc::new(SpiritPlate::new()),
            user_hooks: Arc::new(Vec::new()),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            subagent_sessions: Arc::new(Mutex::new(HashMap::new())),
            session_locks: Arc::new(Mutex::new(HashMap::new())),
            session_modes: Arc::new(Mutex::new(HashMap::new())),
            data_dir: dirs.path().to_path_buf(),
            pid_path: dirs.path().join("pid"),
            backup_dir: dirs.path().join("backups"),
        };
        Arc::new(AppState {
            providers: HashMap::new(),
            default_main_provider_name: "test".into(),
            default_aux_model_provider: None,
            system_prompt: "test".into(),
            earth: Some(Arc::new(earth)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            discord_public_key: None,
            api_key: None,
            rate_limiter: Arc::new(RateLimiter::new(0)),
            session_tokens: Arc::new(SessionTokens::new()),
        })
    }

    #[tokio::test]
    async fn handle_chat_returns_sse_stream() {
        let state = make_test_state();
        let req = ChatRequest {
            provider: "test".into(),
            model: None,
            messages: vec![Message::text(Role::User, "hi")],
        };
        let sse = handle_chat(State(state), Json(req)).await;
        drop(sse);
    }
}

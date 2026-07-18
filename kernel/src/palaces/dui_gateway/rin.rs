//! Unix socket listener for jia-rin and jia-tui. Protocol: see docs/rin-protocol.md

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::plates::di_earth::EarthPlate;
use crate::plates::ren_human::HumanPlate;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::tian_heaven::Agent;
use crate::plates::tian_heaven::InteractionMode;
use crate::plates::tian_heaven::r#loop::AgentEvent;
use crate::types::{Message, Role, StreamEvent};
use crate::vijnana::manas::Manas;
use std::sync::Arc;

use super::SessionTokens;

/// P0-4 · 断连清扫:连接结束(EOF/读失败)时,移除属于本连接所启动会话的
/// pending_questions / pending_confirmations 条目。remove 使 oneshot sender
/// drop,等待中的 ask_user / 确认立即醒为 Err → "(user disconnected)" / 拒绝,
/// agent 得以继续并释放 session_lock(消除断连死锁,审计 F2+L5)。
/// 无 session 字段可判定时,只能按"插入时打上 session_id"实现按 sid 清扫。
fn sweep_pending_for_sessions(earth: &EarthPlate, sids: &[String]) {
    if sids.is_empty() {
        return;
    }
    let removed_questions = {
        let mut map = earth
            .pending_questions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|_, p| !sids.contains(&p.session_id));
        before - map.len()
    };
    let removed_confirmations = {
        let mut map = earth
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|_, p| !sids.contains(&p.session_id));
        before - map.len()
    };
    if removed_questions + removed_confirmations > 0 {
        tracing::info!(
            removed_questions,
            removed_confirmations,
            sessions = ?sids,
            "rin: swept pending questions/confirmations on disconnect"
        );
    }
}

/// Spawn the Unix Socket listener for jia-rin.
pub fn spawn_rin_listener(
    earth: Arc<EarthPlate>,
    session_tokens: Arc<SessionTokens>,
    rin_sock: std::path::PathBuf,
) {
    let _ = std::fs::remove_file(&rin_sock);

    tokio::spawn(async move {
        let listener = match UnixListener::bind(&rin_sock) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("rin: failed to bind {}: {e}", rin_sock.display());
                return;
            }
        };

        // Set restrictive permissions so only the daemon's UID can connect.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&rin_sock, std::fs::Permissions::from_mode(0o600));
        }

        tracing::info!("rin: listening on {}", rin_sock.display());

        // Spawn CronCompleted forwarder — one per daemon, not per connection.
        let cron_tx = spawn_cron_forwarder(earth.clone());

        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    // Verify peer credentials — only same UID or root may connect.
                    #[cfg(unix)]
                    {
                        if let Ok(cred) = stream.peer_cred() {
                            // SAFETY: getuid() is a POSIX function with no preconditions and no failure mode.
                            let my_uid = unsafe { libc::getuid() };
                            if cred.uid() != 0 && cred.uid() != my_uid {
                                tracing::warn!(
                                    "rin: rejected connection from uid {} (daemon uid {})",
                                    cred.uid(),
                                    my_uid,
                                );
                                continue;
                            }
                        }
                    }

                    let earth = earth.clone();
                    let tokens = session_tokens.clone();
                    let cron_tx = cron_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_rin_connection(stream, earth, tokens, cron_tx).await
                        {
                            tracing::warn!("rin: connection error: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("rin: accept error: {e}");
                }
            }
        }
    });
}

/// Forward CronCompleted events to a broadcast channel so each
/// connected client can subscribe.
fn spawn_cron_forwarder(earth: Arc<EarthPlate>) -> tokio::sync::broadcast::Sender<String> {
    let (tx, _) = tokio::sync::broadcast::channel::<String>(64);
    let tx_ret = tx.clone();
    let mut event_rx = earth.spirit.event_bus.subscribe();

    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(RuntimeEvent::CronCompleted {
                    job_name,
                    prompt,
                    response,
                    timestamp,
                    ..
                }) => {
                    let json = serde_json::json!({
                        "type": "cron_notification",
                        "job_name": job_name,
                        "prompt": prompt,
                        "response": response,
                        "timestamp": timestamp,
                    });
                    let _ = tx.send(json.to_string());
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(skipped = n, "rin: cron forwarder lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    tx_ret
}

/// Resolve project from working directory.
/// Checks for .jia/config.toml; if missing, asks the TUI user whether to create one.
/// Returns (cwd, project_id). Falls back to ("", "") on error or user decline.
async fn resolve_project(
    earth: &EarthPlate,
    cwd: &str,
    tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> (String, String) {
    let config_path = std::path::Path::new(cwd).join(".jia").join("config.toml");
    tracing::info!(cwd = %cwd, "rin: resolve_project called");

    // Already a jia project — read existing ID.
    if config_path.exists()
        && let Ok(content) = std::fs::read_to_string(&config_path)
        && let Ok(parsed) = content.parse::<toml::Table>()
        && let Some(project) = parsed.get("project")
    {
        let id = project.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = project.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !id.is_empty() {
            if let Err(e) = earth.store.ensure_project(id, cwd, name, "", "[]") {
                tracing::warn!(%id, cwd, ?e, "rin: ensure_project failed for existing project");
            }
            return (cwd.to_string(), id.to_string());
        }
    }

    // Not a jia project. Skip prompts for home dir and well-known paths.
    let home = std::env::var("HOME").unwrap_or_default();
    if cwd == home || cwd == "/" || cwd == "/tmp" || cwd.starts_with("/usr") {
        return (cwd.to_string(), String::new());
    }

    // Ask user via TUI confirmation whether to create a project here.
    let dir_name = std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string());
    let confirm_id = uuid::Uuid::new_v4().to_string();
    let confirm_token = uuid::Uuid::new_v4().to_string();

    let (_sender, receiver) = tokio::sync::oneshot::channel::<bool>();
    {
        let mut pending = earth.pending_confirmations.lock().unwrap_or_else(|e| e.into_inner());
        pending.insert(
            confirm_id.clone(),
            crate::plates::ren_human::PendingConfirmation {
                token: confirm_token.clone(),
                sender: _sender,
                created_at: crate::utils::unix_now(),
                // 建项确认尚无会话归属;断连时靠 120s 超时兜底。
                session_id: String::new(),
            },
        );
    }

    // Send confirmation to TUI (no timeout — user must explicitly approve or deny)
    tracing::info!(cwd = %cwd, confirm_id = %confirm_id, "rin: sending project confirmation to TUI");
    let _ = tx.send(AgentEvent::ConfirmRequest {
        id: confirm_id.clone(),
        tool: "jia_init".into(),
        reason: format!("Create Jia project in '{cwd}'?"),
        timeout_secs: 0,
        token: confirm_token,
    });

    // Wait for user response with a generous timeout (project creation approval).
    // Oneshot sender is dropped if TUI disconnects, so receiver completes with
    // Err(Canceled). The timeout is a safety net against stalled confirmations.
    let approved = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(120), receiver).await,
        Ok(Ok(true))
    );

    // Clean up pending confirmation
    {
        let mut pending = earth.pending_confirmations.lock().unwrap_or_else(|e| e.into_inner());
        pending.remove(&confirm_id);
    }

    if approved {
        let project_id = uuid::Uuid::new_v4().to_string();
        let proj_dir = std::path::Path::new(cwd).join(".jia");
        let _ = std::fs::create_dir_all(&proj_dir);
        let config_content = format!(
            "[project]\nid = \"{}\"\nname = \"{}\"\n",
            project_id, dir_name
        );
        let _ = std::fs::write(&config_path, &config_content);
        if let Err(e) = earth
            .store
            .ensure_project(&project_id, cwd, &dir_name, "", "[]")
        {
            tracing::warn!(%project_id, cwd, ?e, "rin: ensure_project failed for new project");
        }
        tracing::info!(cwd = %cwd, project_id = %project_id, "rin: created new project");
        return (cwd.to_string(), project_id);
    }

    (cwd.to_string(), String::new())
}

async fn handle_rin_connection(
    stream: UnixStream,
    earth: Arc<EarthPlate>,
    session_tokens: Arc<SessionTokens>,
    cron_tx: tokio::sync::broadcast::Sender<String>,
) -> std::io::Result<()> {
    let (reader, writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let writer = Arc::new(tokio::sync::Mutex::new(writer));

    // Spawn cron notification forwarder for this connection
    let mut cron_rx = cron_tx.subscribe();
    let cron_writer = writer.clone();
    tokio::spawn(async move {
        loop {
            match cron_rx.recv().await {
                Ok(json) => {
                    let mut w = cron_writer.lock().await;
                    let _ = w.write_all(json.as_bytes()).await;
                    let _ = w.write_all(b"\n").await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let mut line = String::new();
    let earth = &earth; // borrow Arc, don't move — each agent spawn clones

    // P0-4 · 本连接上启动过 agent run 的会话 id(断连清扫的归属依据)。
    let mut conn_sessions: Vec<String> = Vec::new();

    loop {
        line.clear();
        let n = match reader.read_line(&mut line).await {
            Ok(n) => n,
            Err(e) => {
                sweep_pending_for_sessions(earth, &conn_sessions);
                return Err(e);
            }
        };
        if n == 0 {
            break; // EOF
        }

        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = msg["type"].as_str().unwrap_or("");

        match msg_type {
            "hello" => {
                let cwd = msg["cwd"].as_str().unwrap_or(".").to_string();
                let earth_clone = earth.clone();
                let w = writer.clone();
                tokio::spawn(async move {
                    // Create a temporary channel for event forwarding.
                    let (hello_tx, hello_rx) = mpsc::unbounded_channel::<AgentEvent>();
                    // Spawn event forwarder for this hello exchange
                    let w2 = w.clone();
                    tokio::spawn(async move {
                        use tokio_stream::StreamExt;
                        use tokio_stream::wrappers::UnboundedReceiverStream;
                        let mut rx = UnboundedReceiverStream::new(hello_rx);
                        while let Some(event) = rx.next().await {
                            let stream_event = match event {
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
                                _ => continue,
                            };
                            let json = serde_json::to_string(&stream_event).unwrap_or_default();
                            let mut w = w2.lock().await;
                            let _ = w.write_all(json.as_bytes()).await;
                            let _ = w.write_all(b"\n").await;
                        }
                    });
                    let (pcwd, pid) = resolve_project(&earth_clone, &cwd, hello_tx).await;
                    tracing::info!(cwd = %pcwd, project_id = %pid, "rin: hello project resolved");
                    // Send result back to TUI
                    let approved = !pid.is_empty();
                    let resp = serde_json::json!({
                        "type": "project_resolved",
                        "cwd": pcwd,
                        "project_id": pid,
                        "approved": approved,
                    });
                    let mut w = w.lock().await;
                    let _ = w.write_all(resp.to_string().as_bytes()).await;
                    let _ = w.write_all(b"\n").await;
                });
            }
            "agent" => {
                let earth = earth.clone();
                let messages: Vec<Message> = msg["messages"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| {
                                let role: Role = serde_json::from_value(serde_json::Value::String(
                                    m["role"].as_str().unwrap_or("user").to_string(),
                                ))
                                .ok()?;
                                let content = m["content"].as_str().unwrap_or("").to_string();
                                Some(Message::text(role, content))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let session_id = msg["session_id"]
                    .as_str()
                    .map(String::from)
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

                // P0-4 · 登记到本连接会话集,供断连清扫。
                if !conn_sessions.contains(&session_id) {
                    conn_sessions.push(session_id.clone());
                }

                let is_new = msg["session_id"].as_str().is_none_or(|s| s.is_empty());

                // Extract cwd and project_id from the message.
                // Project resolution already happened in the "hello" handler.
                let msg_cwd = msg["cwd"].as_str().unwrap_or(".").to_string();
                let msg_pid = msg["project_id"].as_str().unwrap_or("").to_string();

                // Channel and cancellation setup
                let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();

                if is_new {
                    let title = messages
                        .iter()
                        .find(|m| m.role == Role::User)
                        .map(|m| crate::utils::truncate_title(&m.content))
                        .unwrap_or_default();
                    let init_cwd = if msg_cwd.is_empty() || msg_cwd == "." {
                        ""
                    } else {
                        &msg_cwd
                    };
                    let init_pid = if msg_pid.is_empty() { "" } else { &msg_pid };
                    let _ = earth
                        .store
                        .create_session(&session_id, &title, init_cwd, init_pid);
                }
                let cancel_token = tokio_util::sync::CancellationToken::new();
                let agent_token = cancel_token.clone();
                let sid = session_id.clone();

                // Register for cancellation
                let config = &earth.config.app_config;
                let provider_name = config
                    .default_main_model_provider
                    .clone()
                    .unwrap_or_default();
                let model = config
                    .providers
                    .get(&provider_name)
                    .map(|p: &crate::config::ProviderProfile| p.default_main_model().to_string())
                    .unwrap_or_default();
                session_tokens.register(sid.clone(), agent_token.clone(), provider_name, model);

                // Send session ID
                let session_json = serde_json::json!({
                    "type": "session",
                    "session_id": session_id,
                })
                .to_string();
                {
                    let mut w = writer.lock().await;
                    let _ = w.write_all(session_json.as_bytes()).await;
                    let _ = w.write_all(b"\n").await;
                }

                // Forward agent events to socket (runs independently, outside the lock).
                let w = writer.clone();
                let fwd_sid = sid.clone();
                let fwd_modes = earth.session_modes.clone();
                tokio::spawn(async move {
                    use tokio_stream::StreamExt;
                    use tokio_stream::wrappers::UnboundedReceiverStream;

                    let mut rx = UnboundedReceiverStream::new(rx);
                    while let Some(event) = rx.next().await {
                        let stream_event = match event {
                            AgentEvent::Delta(content) => StreamEvent::Delta { content },
                            AgentEvent::ToolCall { tool, input } => {
                                StreamEvent::ToolCall { tool, input }
                            }
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
                            AgentEvent::ToolBatchStart => StreamEvent::ToolBatchStart,
                            AgentEvent::StreamEnd => StreamEvent::StreamEnd,
                            AgentEvent::ContextPressure { tokens, threshold } => {
                                StreamEvent::ContextPressure { tokens, threshold }
                            }
                            AgentEvent::Compacting => StreamEvent::Compacting,
                            AgentEvent::Done => StreamEvent::Done,
                            AgentEvent::Error(message) => StreamEvent::Error { message },
                            AgentEvent::InteractionModeChanged { planning } => {
                                // Keep session_modes in sync so the next run
                                // continues in the agent's actual mode.
                                let mode = if planning {
                                    InteractionMode::Planning
                                } else {
                                    InteractionMode::Normal
                                };
                                if let Ok(mut m) = fwd_modes.lock() {
                                    m.insert(fwd_sid.clone(), mode);
                                }
                                StreamEvent::InteractionModeChanged { planning }
                            }
                            _ => continue,
                        };

                        let json = serde_json::to_string(&stream_event).unwrap_or_default();
                        if json.is_empty() {
                            continue;
                        }
                        let mut w = w.lock().await;
                        if w.write_all(json.as_bytes()).await.is_err() {
                            break;
                        }
                        if w.write_all(b"\n").await.is_err() {
                            break;
                        }
                    }
                });

                // —— session_lock + cancel-aware wait ——
                // Serialises with HTTP /agent for the same session_id so that
                // session history loads and post_loop writes cannot interleave.
                // IMPORTANT: agent.run() is spawned (not awaited) so the reader
                // loop stays unblocked — otherwise ask_user answers sent through
                // this same socket connection deadlock.
                let session_lock = {
                    let mut map = earth.session_locks.lock().unwrap_or_else(|e| e.into_inner());
                    map.retain(|_, v| Arc::strong_count(v) > 1);
                    map.entry(sid.clone())
                        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                        .clone()
                };

                let store = earth.store.clone();
                let event_bus = earth.spirit.event_bus.clone();
                let main_core = earth.main_core.clone();
                let aux_core = earth.aux_core.clone();
                let session_tokens_clone = session_tokens.clone();
                let permissions = earth.permissions.clone();
                let pending_confirmations = earth.pending_confirmations.clone();
                tokio::spawn(async move {
                    tokio::select! {
                        _ = agent_token.cancelled() => {
                            if is_new { let _ = store.delete_session(&sid); }
                            session_tokens_clone.remove(&sid);
                        }
                        guard = session_lock.lock() => {
                            let _guard = guard;

                            // Re-check cancellation after acquiring lock to prevent race
                            if agent_token.is_cancelled() {
                                session_tokens_clone.remove(&sid);
                                return;
                            }

                            // —— lock acquired: load session state ——
                            let history = store.load_session_history(&sid);
                            let manas: Manas = store.load_manas().ok().flatten()
                                .and_then(|json| serde_json::from_str(&json).ok())
                                .unwrap_or_default();
                            let distilled_hashes = store.load_distilled_hashes(&sid);

                            // Resolve effective cwd from message (project already resolved by "hello")
                            let effective_cwd = if !msg_cwd.is_empty() && msg_cwd != "." {
                                msg_cwd.clone()
                            } else if !msg_pid.is_empty() {
                                // Old session: reverse-lookup cwd from project
                                store.get_project(&msg_pid).ok().flatten()
                                    .and_then(|p| p.get("cwd").and_then(|v| v.as_str()).map(String::from))
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };

                            // Reject if no valid project directory
                            if effective_cwd.is_empty() {
                                let _ = tx.send(AgentEvent::Error(
                                    "No valid project directory. Create a Jia project first with `jia init`.".into()
                                ));
                                let _ = tx.send(AgentEvent::Done);
                                session_tokens_clone.remove(&sid);
                                return;
                            }

                            let mut agent = Agent::with_session(sid.clone(), earth.clone(), history, manas, distilled_hashes);
                            agent.exec_ctx = earth.build_worktree_exec_ctx(
                                std::path::Path::new(&effective_cwd),
                                &sid,
                                agent_token.clone(),
                            );
                            // P3 · apply user-set interaction mode (from /plan slash)
                            if let Some(mode) = earth.session_modes.lock().unwrap_or_else(|e| e.into_inner()).get(&sid).copied() {
                                agent.interaction_mode = mode;
                            }
                            let human_plate = HumanPlate::with_state(permissions, pending_confirmations);

                            let ctx = crate::plates::tian_heaven::r#loop::RunContext {
                                core: &main_core,
                                human_plate: &human_plate,
                                event_bus: &event_bus,
                                hook_registry: &earth.spirit.hook_registry,
                                tx,
                                cancel_token: &agent_token,
                            };
                            agent.run(messages, &ctx).await;
                            agent.post_loop(store, &main_core, aux_core.as_deref(), ctx.human_plate).await;
                            session_tokens_clone.remove(&sid);
                        }
                    }
                });
            }

            "cancel" => {
                if let Some(sid) = msg["session_id"].as_str() {
                    session_tokens.cancel(sid);
                }
            }

            // P3 · /plan slash entry — set per-session interaction mode for the
            // next agent run, and echo the change back to the TUI immediately.
            "set_mode" => {
                let sid = msg["session_id"].as_str().unwrap_or("").to_string();
                let planning = msg["planning"].as_bool().unwrap_or(false);
                if !sid.is_empty() {
                    let mode = if planning {
                        InteractionMode::Planning
                    } else {
                        InteractionMode::Normal
                    };
                    earth.session_modes.lock().unwrap_or_else(|e| e.into_inner()).insert(sid, mode);
                    let resp = serde_json::json!({
                        "type": "interaction_mode_changed",
                        "planning": planning,
                    });
                    let mut w = writer.lock().await;
                    let _ = w.write_all(resp.to_string().as_bytes()).await;
                    let _ = w.write_all(b"\n").await;
                }
            }

            // ── TUI / rin-client protocol extensions ──────────────
            "confirm" => {
                let id = msg["id"].as_str().unwrap_or("");
                let token = msg["token"].as_str().unwrap_or("");
                if id.is_empty() {
                    continue;
                }
                let approved = msg["approved"].as_bool().unwrap_or(false);
                let resolved = {
                    let mut map = earth.pending_confirmations.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(p) = map.remove(id) {
                        if p.token == token {
                            let _ = p.sender.send(approved);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                let resp = serde_json::json!({
                    "type": "confirm_resolved",
                    "id": id,
                    "resolved": resolved,
                });
                let mut w = writer.lock().await;
                let _ = w.write_all(resp.to_string().as_bytes()).await;
                let _ = w.write_all(b"\n").await;
            }

            "answer" => {
                let id = msg["id"].as_str().unwrap_or("");
                let token = msg["token"].as_str().unwrap_or("");
                let answer = msg["answer"].as_str().unwrap_or("");
                tracing::info!(%id, answer_len = answer.len(), "rin: received answer");
                if id.is_empty() {
                    continue;
                }
                let resolved = {
                    let mut map = earth.pending_questions.lock().unwrap_or_else(|e| e.into_inner());
                    let map_size = map.len();
                    if let Some(p) = map.remove(id) {
                        let token_ok = p.token == token;
                        tracing::info!(%id, token_ok, map_size, "rin: answer resolve");
                        if token_ok {
                            let _ = p.sender.send(answer.to_string());
                            true
                        } else {
                            false
                        }
                    } else {
                        tracing::warn!(%id, map_size, "rin: answer for unknown question (already resolved or wrong id)");
                        false
                    }
                };
                let resp = serde_json::json!({
                    "type": "answer_resolved",
                    "id": id,
                    "resolved": resolved,
                });
                let mut w = writer.lock().await;
                let _ = w.write_all(resp.to_string().as_bytes()).await;
                let _ = w.write_all(b"\n").await;
            }

            "model_info" => {
                let config = &earth.config.app_config;
                let provider = config.default_main_provider_name().to_string();
                let model = config
                    .default_main_provider()
                    .ok()
                    .and_then(|p| {
                        p.default_main_model
                            .clone()
                            .or_else(|| p.models.first().cloned())
                    })
                    .unwrap_or_default();
                let resp = serde_json::json!({
                    "type": "model_info",
                    "provider": provider,
                    "model": model,
                });
                let mut w = writer.lock().await;
                let _ = w.write_all(resp.to_string().as_bytes()).await;
                let _ = w.write_all(b"\n").await;
            }

            "sessions" => {
                let sessions = earth
                    .store
                    .list_sessions_filtered("all")
                    .unwrap_or_default();
                let json = serde_json::json!({
                    "type": "sessions",
                    "sessions": sessions,
                })
                .to_string();
                let mut w = writer.lock().await;
                let _ = w.write_all(json.as_bytes()).await;
                let _ = w.write_all(b"\n").await;
            }

            "load_session" => {
                let sid = msg["session_id"].as_str().unwrap_or("");
                if sid.is_empty() {
                    let resp = serde_json::json!({
                        "type": "session_history",
                        "session_id": "",
                        "entries": [],
                    });
                    let mut w = writer.lock().await;
                    let _ = w.write_all(resp.to_string().as_bytes()).await;
                    let _ = w.write_all(b"\n").await;
                } else {
                    let entries = earth.store.load_session_history(sid);
                    let json = serde_json::json!({
                        "type": "session_history",
                        "session_id": sid,
                        "entries": entries,
                    })
                    .to_string();
                    let mut w = writer.lock().await;
                    let _ = w.write_all(json.as_bytes()).await;
                    let _ = w.write_all(b"\n").await;
                }
            }

            _ => {
                tracing::debug!("rin: unknown message type: {msg_type}");
            }
        }
    }

    // P0-4 · 断连清扫(EOF 路径;读失败路径已在循环内清扫)。
    sweep_pending_for_sessions(earth, &conn_sessions);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_event_delta_serializes() {
        let json = serde_json::to_string(&StreamEvent::Delta {
            content: "hello".into(),
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "delta");
        assert_eq!(parsed["content"], "hello");
    }

    #[test]
    fn stream_event_session_serializes() {
        let json = serde_json::to_string(&StreamEvent::Session {
            session_id: "s1".into(),
        })
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "session");
        assert_eq!(parsed["session_id"], "s1");
    }

    #[test]
    fn stream_event_done_serializes() {
        let json = serde_json::to_string(&StreamEvent::Done).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["type"], "done");
    }
}

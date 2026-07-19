//! Unix socket listener for jia-rin and jia-tui. Protocol: see docs/rin-protocol.md

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::plates::di_earth::EarthPlate;
use crate::plates::ren_human::HumanPlate;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::tian_heaven::Agent;
use crate::stems::AgentEvent;
use crate::stems::InteractionMode;
use crate::types::{Message, Role, StreamEvent};
use crate::vijnana::manas::Manas;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::SessionTokens;

/// P0-4 · 断连清扫:连接结束(EOF/读失败)时,移除属于本连接所启动会话的
/// pending_questions / pending_confirmations 条目。remove 使 oneshot sender
/// drop,等待中的 ask_user / 确认立即醒为 Err → "(user disconnected)" / 拒绝,
/// agent 得以继续并释放 session_lock(消除断连死锁,审计 F2+L5)。
/// 无 session 字段可判定时,只能按"插入时打上 session_id"实现按 sid 清扫。
/// (直接收表而非 EarthPlate,便于单元测试。)
///
/// P1-2/L2 · 一并清扫 session_modes:该表此前只有 insert 无任何 remove,
/// 每会话残留一条;断连时按 sid 移除(重连同 sid 会话会由
/// InteractionModeChanged 事件重新同步,丢一条旧 mode 无副作用)。
///
/// 已知限制:断连清扫是一次性的且不 cancel session token(有意保留"TUI
/// 断连后长任务续跑"语义,2026-07-19 裁决)。残留:断连后 agent 若再次调用
/// ask_user,新 pending 条目无人清扫、无超时,会持 session_lock 永久等待;
/// 缓解:确认类等待有 confirmation_timeout 兜底,ask_user 的彻底解法是断连
/// 即 cancel 本连接 token(语义变更,未采纳)。
fn sweep_pending_for_sessions(
    pending_questions: &Arc<Mutex<HashMap<String, crate::plates::ren_human::PendingQuestion>>>,
    pending_confirmations: &Arc<
        Mutex<HashMap<String, crate::plates::ren_human::PendingConfirmation>>,
    >,
    session_modes: &Arc<Mutex<HashMap<String, InteractionMode>>>,
    sids: &[String],
) {
    if sids.is_empty() {
        return;
    }
    let removed_questions = {
        let mut map = pending_questions.lock().unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|_, p| !sids.contains(&p.session_id));
        before - map.len()
    };
    let removed_confirmations = {
        let mut map = pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|_, p| !sids.contains(&p.session_id));
        before - map.len()
    };
    let removed_modes = {
        let mut map = session_modes.lock().unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|sid, _| !sids.contains(sid));
        before - map.len()
    };
    if removed_questions + removed_confirmations + removed_modes > 0 {
        tracing::info!(
            removed_questions,
            removed_confirmations,
            removed_modes,
            sessions = ?sids,
            "rin: swept pending questions/confirmations/modes on disconnect"
        );
    }
}

/// P1-2/L6 · session_tokens 清扫守卫:任何退出路径(正常结束 / cancel /
/// 提前 return / agent.run 在 dev 构建下 panic 展开)都在 Drop 时 remove,
/// 消除幽灵会话(此前 remove 在 agent.run/post_loop 之后,panic 即跳过)。
struct SessionTokenGuard {
    tokens: Arc<SessionTokens>,
    sid: String,
}

impl Drop for SessionTokenGuard {
    fn drop(&mut self) {
        self.tokens.remove(&self.sid);
    }
}

/// P1-7 · UDS 单行帧长上限(审计 D2):与 HTTP 侧 1MB RequestBodyLimit
/// (dui_gateway/mod.rs) 对齐。此前 read_line 无上限,同 UID 客户端发永不
/// 换行的流可致 daemon 内存无界增长。
const MAX_RIN_LINE_BYTES: u64 = 1_048_576;

/// 带界行读取:单行超过 MAX_RIN_LINE_BYTES 时返回 Err(InvalidData)——
/// 调用方据此断连。实现:`take(上限+1)` 限流后 read_line;读到上限+1 字节
/// 且未见换行,即判定超限(恰好 上限+\n 的合法行不受影响)。
async fn read_line_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut String,
) -> std::io::Result<usize> {
    use tokio::io::AsyncReadExt;
    let mut limited = AsyncReadExt::take(reader, MAX_RIN_LINE_BYTES + 1);
    let n = limited.read_line(line).await?;
    if n as u64 > MAX_RIN_LINE_BYTES && !line.ends_with('\n') {
        tracing::warn!(bytes = n, "rin: line exceeds 1MB limit, closing connection");
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "rin: line exceeds 1MB limit",
        ));
    }
    Ok(n)
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
        let mut pending = earth
            .session_bus
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
        let mut pending = earth
            .session_bus
            .pending_confirmations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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

/// P1-2/L1 · 每连接 cron 转发任务。写失败(客户端断连)即 break 退出——
/// 此前 `let _ =` 吞掉写失败,任务只在 daemon 退出(broadcast Closed)时才
/// 结束,客户端每连一次积一个永存任务。模式对齐下方 agent 事件转发器。
fn spawn_conn_cron_forwarder<W>(
    mut cron_rx: tokio::sync::broadcast::Receiver<String>,
    writer: Arc<tokio::sync::Mutex<W>>,
) -> tokio::task::JoinHandle<()>
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            match cron_rx.recv().await {
                Ok(json) => {
                    let mut w = writer.lock().await;
                    if w.write_all(json.as_bytes()).await.is_err() {
                        break;
                    }
                    if w.write_all(b"\n").await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
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
    let cron_rx = cron_tx.subscribe();
    spawn_conn_cron_forwarder(cron_rx, writer.clone());

    let mut line = String::new();
    let earth = &earth; // borrow Arc, don't move — each agent spawn clones

    // P0-4 · 本连接上启动过 agent run 的会话 id(断连清扫的归属依据)。
    let mut conn_sessions: Vec<String> = Vec::new();

    loop {
        line.clear();
        // P1-7 · 带界读取:单行超 1MB 视为恶意/异常客户端,清扫后断连。
        let n = match read_line_bounded(&mut reader, &mut line).await {
            Ok(n) => n,
            Err(e) => {
                sweep_pending_for_sessions(
                    &earth.session_bus.pending_questions,
                    &earth.session_bus.pending_confirmations,
                    &earth.session_bus.session_modes,
                    &conn_sessions,
                );
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
                let fwd_modes = earth.session_bus.session_modes.clone();
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
                    let mut map = earth
                        .session_bus
                        .session_locks
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
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
                let session_bus = earth.session_bus.clone();
                tokio::spawn(async move {
                    // P1-2/L6 · 任何退出路径(cancel / 提前 return / panic
                    // 展开)都 remove session token,消除幽灵会话。
                    let _token_guard = SessionTokenGuard {
                        tokens: session_tokens_clone,
                        sid: sid.clone(),
                    };
                    tokio::select! {
                        _ = agent_token.cancelled() => {
                            if is_new { let _ = store.delete_session(&sid); }
                        }
                        guard = session_lock.lock() => {
                            let _guard = guard;

                            // Re-check cancellation after acquiring lock to prevent race
                            if agent_token.is_cancelled() {
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
                                return;
                            }

                            let mut agent = Agent::with_session(sid.clone(), earth.clone(), history, manas, distilled_hashes);
                            agent.exec_ctx = earth.build_worktree_exec_ctx(
                                std::path::Path::new(&effective_cwd),
                                &sid,
                                agent_token.clone(),
                            );
                            // P3 · apply user-set interaction mode (from /plan slash)
                            if let Some(mode) = earth.session_bus.session_modes.lock().unwrap_or_else(|e| e.into_inner()).get(&sid).copied() {
                                agent.interaction_mode = mode;
                            }
                            let human_plate = HumanPlate::with_state(permissions, session_bus);

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
                    earth
                        .session_bus
                        .session_modes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(sid, mode);
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
                    let mut map = earth
                        .session_bus
                        .pending_confirmations
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
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
                    let mut map = earth
                        .session_bus
                        .pending_questions
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
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
    sweep_pending_for_sessions(
        &earth.session_bus.pending_questions,
        &earth.session_bus.pending_confirmations,
        &earth.session_bus.session_modes,
        &conn_sessions,
    );

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

    /// P0-4 quality: sweep 只移除归属给定 sid 的条目——其他会话与空 sid
    /// (resolve_project 建项确认,靠超时兜底)的条目必须存活。
    /// P1-2/L2: session_modes 一并按 sid 清扫。
    #[test]
    fn sweep_removes_only_target_sessions() {
        use crate::plates::ren_human::{PendingConfirmation, PendingQuestion};

        let questions: Arc<Mutex<HashMap<String, PendingQuestion>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let modes: Arc<Mutex<HashMap<String, InteractionMode>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let mk_q = |sid: &str| {
            let (tx, _rx) = tokio::sync::oneshot::channel::<String>();
            PendingQuestion {
                sender: tx,
                token: "t".into(),
                created_at: 0,
                session_id: sid.into(),
            }
        };
        let mk_c = |sid: &str| {
            let (tx, _rx) = tokio::sync::oneshot::channel::<bool>();
            PendingConfirmation {
                sender: tx,
                created_at: 0,
                token: "t".into(),
                session_id: sid.into(),
            }
        };

        {
            let mut q = questions.lock().unwrap();
            q.insert("q-mine".into(), mk_q("s1"));
            q.insert("q-other".into(), mk_q("s2"));
            q.insert("q-anon".into(), mk_q(""));
            let mut c = confirmations.lock().unwrap();
            c.insert("c-mine".into(), mk_c("s1"));
            c.insert("c-other".into(), mk_c("s2"));
            c.insert("c-anon".into(), mk_c(""));
            let mut m = modes.lock().unwrap();
            m.insert("s1".into(), InteractionMode::Planning);
            m.insert("s2".into(), InteractionMode::Normal);
        }

        sweep_pending_for_sessions(&questions, &confirmations, &modes, &["s1".to_string()]);

        let q = questions.lock().unwrap();
        assert!(!q.contains_key("q-mine"), "own session entry must be swept");
        assert!(q.contains_key("q-other"), "other session must survive");
        assert!(q.contains_key("q-anon"), "anonymous entry must survive");
        let c = confirmations.lock().unwrap();
        assert!(!c.contains_key("c-mine"));
        assert!(c.contains_key("c-other"));
        assert!(c.contains_key("c-anon"));
        let m = modes.lock().unwrap();
        assert!(!m.contains_key("s1"), "own session mode must be swept");
        assert!(m.contains_key("s2"), "other session mode must survive");

        // 空 sids 是 no-op。
        sweep_pending_for_sessions(&questions, &confirmations, &modes, &[]);
        assert_eq!(q.len(), 2);
        assert_eq!(c.len(), 2);
        assert_eq!(m.len(), 1);
    }

    /// P1-2/L1 · 每连接 cron 转发器:对端断连(写失败)后任务必须退出,
    /// 不得残留到 daemon 退出。
    #[tokio::test]
    async fn conn_cron_forwarder_exits_on_write_failure() {
        let (tx, rx) = tokio::sync::broadcast::channel::<String>(4);
        // duplex 一端作为 writer,另一端直接 drop → 写入立即 BrokenPipe。
        let (writer, reader) = tokio::io::duplex(64);
        drop(reader);
        let handle = spawn_conn_cron_forwarder(rx, Arc::new(tokio::sync::Mutex::new(writer)));

        let _ = tx.send("{\"type\":\"cron_notification\"}".to_string());
        tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("forwarder must exit on write failure")
            .expect("forwarder task must not panic");
    }

    /// P1-2/L1 对照:写端存活时,转发器正常转发并在 channel 关闭后退出。
    #[tokio::test]
    async fn conn_cron_forwarder_forwards_and_exits_on_closed() {
        let (tx, rx) = tokio::sync::broadcast::channel::<String>(4);
        let (writer, mut reader) = tokio::io::duplex(64);
        let handle = spawn_conn_cron_forwarder(rx, Arc::new(tokio::sync::Mutex::new(writer)));

        let _ = tx.send("hello".to_string());
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 6];
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            reader.read_exact(&mut buf),
        )
        .await
        .expect("must receive forwarded bytes")
        .expect("read must succeed");
        assert_eq!(&buf, b"hello\n");

        drop(tx);
        tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("forwarder must exit on channel close")
            .expect("forwarder task must not panic");
    }

    /// P1-7 · 限内行(含接近 1MB 的大行)正常读取。
    #[tokio::test]
    async fn read_line_bounded_accepts_within_limit() {
        // 短行
        let data = b"{\"type\":\"hello\"}\nrest".to_vec();
        let mut cursor = std::io::Cursor::new(data);
        let mut line = String::new();
        let n = read_line_bounded(&mut cursor, &mut line).await.unwrap();
        assert_eq!(line, "{\"type\":\"hello\"}\n");
        assert_eq!(n, line.len());

        // 接近 1MB 的大行(正常大消息不得被破坏)
        let big = "x".repeat(MAX_RIN_LINE_BYTES as usize - 1) + "\n";
        let mut cursor = std::io::Cursor::new(big.clone().into_bytes());
        let mut line = String::new();
        let n = read_line_bounded(&mut cursor, &mut line).await.unwrap();
        assert_eq!(n, big.len());
        assert_eq!(line, big);

        // 边界:恰好 上限字节内容 + 换行
        let exact = "y".repeat(MAX_RIN_LINE_BYTES as usize) + "\n";
        let mut cursor = std::io::Cursor::new(exact.into_bytes());
        let mut line = String::new();
        let n = read_line_bounded(&mut cursor, &mut line).await.unwrap();
        assert_eq!(n as u64, MAX_RIN_LINE_BYTES + 1);
    }

    /// P1-7 · 超限行(超过 1MB 无换行)→ Err(InvalidData),调用方断连。
    #[tokio::test]
    async fn read_line_bounded_rejects_over_limit() {
        let oversized = "z".repeat(MAX_RIN_LINE_BYTES as usize + 100);
        let mut cursor = std::io::Cursor::new(oversized.into_bytes());
        let mut line = String::new();
        let err = read_line_bounded(&mut cursor, &mut line)
            .await
            .expect_err("over-limit line must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    /// P1-2/L6 · SessionTokenGuard 在 Drop 时移除 token(覆盖 panic 展开等
    /// 任何退出路径)。
    #[test]
    fn session_token_guard_removes_on_drop() {
        let tokens = Arc::new(SessionTokens::new());
        tokens.register(
            "s1".into(),
            tokio_util::sync::CancellationToken::new(),
            "p".into(),
            "m".into(),
        );
        assert_eq!(tokens.active_count(), 1);
        {
            let _guard = SessionTokenGuard {
                tokens: tokens.clone(),
                sid: "s1".into(),
            };
        }
        assert_eq!(tokens.active_count(), 0, "guard drop must remove token");
    }
}

//! spawn — 天盘运行时编排入口 (P2-2 自地盘迁入)
//!
//! 哲学依据:Heaven Plate is the runtime。构造 Agent/RunContext 并驱动
//! 会话运行,是天盘职责;地盘仅为静态基础设施(一局不变)。原居
//! di_earth 的 spawn_cron_agent / run_io_agent / IO 消费循环皆为此类
//! 编排,迁此。地盘以 `Arc<EarthPlate>` 入参被持有(天→地,合法)。
//!
//! 点火时机:地盘起局(assemble)装配完成后,以全限定路径一次性调用
//! 本模块入口并注入 cron 触发闭包——那是组装根语义的单向点火,
//! 运行期地盘不反向回调天盘。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;

use crate::palaces::kan_io::{ChannelInput, ChannelSource};
use crate::palaces::kun_config::default_workspace_dir;
use crate::plates::di_earth::EarthPlate;
use crate::plates::ren_human::HumanPlate;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::stems::events::AgentEvent;
use crate::types::{HistoryEntry, Message, Role};
use crate::vijnana::manas::Manas;

use super::Agent;
use super::r#loop::RunContext;

/// UUID v5 namespace for Jia IO sessions — deterministically maps a source key
/// (e.g. "webhook:wechat:wxid_xxx") to a session ID.  Generated once, fixed forever.
const JIA_SESSION_NS: uuid::Uuid = uuid::Uuid::from_bytes([
    0xA3, 0xE2, 0x91, 0x7C, 0x8F, 0x4D, 0x42, 0xB1, 0x9E, 0x56, 0xDC, 0x73, 0xFA, 0x10, 0x8B, 0x2F,
]);

/// Spawn the IO consumer — reads from ChannelManager and spawns Agent sessions
/// for bot messages (WeChat, Telegram, Discord, webhooks, etc.).
///
/// CON-M1: Semaphore limits concurrent agent count to prevent resource exhaustion.
/// Same-source dedup: if a session is already active, new messages for that
/// source are dropped (the existing session handles the ongoing conversation).
pub fn spawn_io_consumer(
    earth: Arc<EarthPlate>,
    io_rx: tokio::sync::mpsc::UnboundedReceiver<ChannelInput>,
) {
    let io_permits = Arc::new(tokio::sync::Semaphore::new(8));
    let active_sessions: Arc<Mutex<HashMap<String, ()>>> = Arc::new(Mutex::new(HashMap::new()));
    tokio::spawn(async move {
        let mut rx = UnboundedReceiverStream::new(io_rx);
        while let Some(input) = rx.next().await {
            // Same-source dedup: derive source key and skip if already active
            let source_key = match &input.source {
                ChannelSource::Stdin => "stdin".into(),
                ChannelSource::FileWatch { path } => format!("filewatch:{path}"),
                ChannelSource::Webhook { endpoint } => format!("webhook:{endpoint}"),
                ChannelSource::Api => "api".into(),
            };
            {
                let mut active = active_sessions.lock().unwrap_or_else(|e| e.into_inner());
                if active.contains_key(&source_key) {
                    tracing::debug!(source = %source_key, "Dropping duplicate message: session already active");
                    continue;
                }
                active.insert(source_key.clone(), ());
            }
            let earth = earth.clone();
            let permits = io_permits.clone();
            let sessions = active_sessions.clone();
            tokio::spawn(async move {
                let _permit = permits.acquire().await;
                run_io_agent(earth, input).await;
                sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&source_key);
            });
        }
        tracing::info!("IO consumer stopped");
    });
}

/// Spawn a background agent task for a cron job prompt.
///
/// Runs the full agent loop, logs the response, and stores it on
/// the CronJob so the frontend can retrieve it.
pub fn spawn_cron_agent(earth: Arc<EarthPlate>, job_name: String, prompt: String) {
    let cron = earth.cron.clone();
    tokio::spawn(async move {
        let session_id = uuid::Uuid::new_v4().to_string();
        let human_plate =
            HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
        let distilled_hashes = earth.store.load_distilled_hashes(&session_id);
        let workspace = default_workspace_dir();
        let cancel = CancellationToken::new();
        let mut agent = Agent::with_session(
            session_id.clone(),
            earth.clone(),
            Vec::new(),
            Manas::default(),
            distilled_hashes,
        );
        agent.exec_ctx = earth.build_worktree_exec_ctx(&workspace, &session_id, cancel.clone());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        let messages = vec![Message::text(Role::User, prompt.clone())];
        let event_bus = earth.spirit.event_bus.clone();
        let store = earth.store.clone();

        let collect_handle = tokio::spawn(async move {
            let mut rx = UnboundedReceiverStream::new(rx);
            let mut response = String::new();
            let mut tool_calls: Vec<String> = Vec::new();
            while let Some(event) = rx.next().await {
                match event {
                    AgentEvent::Delta(content) => response.push_str(&content),
                    AgentEvent::ToolCall { tool, input } => {
                        tool_calls.push(format!("{tool}({input})"));
                    }
                    AgentEvent::Done => break,
                    AgentEvent::Error(msg) => {
                        response = format!("Error: {msg}");
                        break;
                    }
                    _ => {}
                }
            }
            (response, tool_calls)
        });

        let ctx = RunContext {
            core: &earth.main_core,
            human_plate: &human_plate,
            event_bus: &event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        tokio::select! {
            _ = agent.run(messages, &ctx) => {
                agent
                    .post_loop(store, &earth.main_core, earth.aux_core.as_deref(), ctx.human_plate)
                    .await;

                match collect_handle.await {
                Ok((mut response, tool_calls)) => {
                    let was_empty = response.is_empty();
                    if was_empty {
                        response = "(cron agent 未产生文本输出)".into();
                    }
                    cron.set_last_response(&job_name, response.clone());

                    // Persist response to disk so the user can review
                    // cron output even when the daemon has no terminal.
                    let now = time::OffsetDateTime::now_local()
                        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
                    let date_dir = format!(
                        "{:04}-{:02}-{:02}",
                        now.year(),
                        u8::from(now.month()),
                        now.day()
                    );
                    let time_file = format!(
                        "{:02}-{:02}-{:02}.md",
                        now.hour(),
                        now.minute(),
                        now.second()
                    );
                    let output_dir = crate::palaces::kun_config::default_data_dir()
                        .join("cron_output")
                        .join(&job_name)
                        .join(&date_dir);
                    if std::fs::create_dir_all(&output_dir).is_ok() {
                        let _ = std::fs::write(output_dir.join(&time_file), &response);
                    }

                    // Emit to event bus so frontend can receive cron
                    // notifications in real time via GET /events SSE.
                    earth.spirit.event_bus.emit(RuntimeEvent::CronCompleted {
                        job_name: job_name.clone(),
                        prompt: prompt.clone(),
                        response: response.clone(),
                        session_id: session_id.clone(),
                        timestamp: crate::utils::unix_now() as u64,
                    });

                    if was_empty {
                        tracing::warn!(
                            session = %session_id,
                            job = %job_name,
                            prompt = %prompt,
                            tools = tool_calls.len(),
                            "Cron agent produced empty response"
                        );
                    }
                    let tool_summary = if tool_calls.is_empty() {
                        String::new()
                    } else {
                        format!(" | tools: {}", tool_calls.join(", "))
                    };
                    tracing::info!(
                        session = %session_id,
                        response_len = response.len(),
                        "Cron agent completed{tool_summary}"
                    );
                    tracing::debug!(
                        session = %session_id,
                        prompt = %prompt,
                        response = %response,
                        "Cron agent completed (details)"
                    );
                }
                Err(e) => {
                    tracing::warn!(session = %session_id, "Cron agent response collector error: {e}");
                    // Still notify frontend so user knows the cron fired but failed.
                    earth.spirit.event_bus.emit(RuntimeEvent::CronCompleted {
                        job_name: job_name.clone(),
                        prompt: prompt.clone(),
                        response: format!("(cron agent 执行失败: {e})"),
                        session_id: session_id.clone(),
                        timestamp: crate::utils::unix_now() as u64,
                    });
                }
            }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(600)) => {
                cancel.cancel();
                tracing::warn!(job = %job_name, "cron agent timed out after 10min");
            }
        }
    });
}

/// Run an Agent session for a single ChannelInput and log the response.
///
/// Shared path for IO-triggered agent invocations
/// (bots, webhooks, file-watch).  The response is logged via tracing.
async fn run_io_agent(earth: Arc<EarthPlate>, input: ChannelInput) {
    let ChannelInput {
        messages,
        source,
        reply_tx,
    } = input;
    let text = messages
        .first()
        .map(|m| m.content.clone())
        .unwrap_or_default();

    if text.trim().is_empty() {
        return;
    }

    // Stable source key — NOT Debug format which can change across compiler versions.
    let source_key = match &source {
        ChannelSource::Stdin => "stdin".into(),
        ChannelSource::FileWatch { path } => format!("filewatch:{path}"),
        ChannelSource::Webhook { endpoint } => format!("webhook:{endpoint}"),
        ChannelSource::Api => "api".into(),
    };

    // Derive deterministic session_id from source_key so the same
    // user/bot/channel always lands in the same session.
    let session_id = uuid::Uuid::new_v5(&JIA_SESSION_NS, source_key.as_bytes()).to_string();

    // Serialize per session — prevent concurrent messages from the same
    // source racing on history read/write in post_loop.
    let session_lock = {
        let mut map = earth
            .session_bus
            .session_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Drop entries with no live holders (strong_count == 1 means only map holds it)
        map.retain(|_, v| Arc::strong_count(v) > 1);
        map.entry(session_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = session_lock.lock().await;

    // Create session with a readable title (idempotent — INSERT OR IGNORE)
    let title = text.chars().take(60).collect::<String>();
    let _ = earth.store.create_session(&session_id, &title, "", "");

    // Load existing history for session continuity
    let history: Vec<HistoryEntry> = earth.store.load_session_history(&session_id);

    let manas: Manas = earth
        .store
        .load_manas()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let human_plate = HumanPlate::with_state(earth.permissions.clone(), earth.session_bus.clone());
    let distilled_hashes = earth.store.load_distilled_hashes(&session_id);
    let workspace = default_workspace_dir();
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut agent = Agent::with_session(
        session_id.clone(),
        earth.clone(),
        history,
        manas,
        distilled_hashes,
    );
    agent.exec_ctx = earth.build_worktree_exec_ctx(&workspace, &session_id, cancel.clone());
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    let messages = vec![Message::text(Role::User, text.clone())];

    let collect_handle = tokio::spawn(async move {
        let mut rx = UnboundedReceiverStream::new(rx);
        let mut response = String::new();
        let mut tool_calls: Vec<String> = Vec::new();
        while let Some(event) = rx.next().await {
            match event {
                AgentEvent::Delta(content) => response.push_str(&content),
                AgentEvent::ToolCall { tool, input } => {
                    tool_calls.push(format!("{tool}({input})"));
                }
                AgentEvent::Done => break,
                AgentEvent::Error(msg) => {
                    response = format!("Error: {msg}");
                    break;
                }
                _ => {}
            }
        }
        (response, tool_calls)
    });

    let ctx = RunContext {
        core: &earth.main_core,
        human_plate: &human_plate,
        event_bus: &earth.spirit.event_bus,
        hook_registry: &earth.spirit.hook_registry,
        tx,
        cancel_token: &cancel,
    };
    // IO session timeout: 600s global deadline prevents permanent hang.
    const IO_SESSION_TIMEOUT_SECS: u64 = 600;
    let run_result = tokio::time::timeout(
        std::time::Duration::from_secs(IO_SESSION_TIMEOUT_SECS),
        agent.run(messages, &ctx),
    )
    .await;
    match run_result {
        Ok(()) => {}
        Err(_elapsed) => {
            tracing::warn!(session = %agent.id, "IO agent timed out after {IO_SESSION_TIMEOUT_SECS}s");
            cancel.cancel();
            let _ = ctx.tx.send(AgentEvent::Error("Session timed out".into()));
            return;
        }
    }
    agent
        .post_loop(
            earth.store.clone(),
            &earth.main_core,
            earth.aux_core.as_deref(),
            &human_plate,
        )
        .await;

    match collect_handle.await {
        Ok((response, tool_calls)) => {
            // Route response back to the bot/platform adapter
            if let Some(tx) = &reply_tx {
                let _ = tx.send(crate::palaces::kan_io::OutboundReply {
                    text: response.clone(),
                });
            }

            let tool_summary = if tool_calls.is_empty() {
                String::new()
            } else {
                format!(" | tools: {}", tool_calls.join(", "))
            };
            tracing::info!(
                source = %source_key,
                session = %session_id,
                response_len = response.len(),
                "IO agent completed{tool_summary}"
            );
            tracing::debug!(
                source = %source_key,
                session = %session_id,
                prompt = %text,
                response = %response,
                "IO agent completed (details)"
            );
        }
        Err(e) => {
            tracing::warn!(source = %source_key, session = %session_id, "IO agent collector error: {e}");
        }
    }
}

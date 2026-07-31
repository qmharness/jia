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
use crate::stems::AgentEvent;
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
            // 重试回滚锚点:失败轮的半截 Delta 已在 response 里,Retrying 到达时
            // 截断回本流起点(与 TUI 的 StreamAnchor 同一语义,审计 W1-1)。
            let mut attempt_start = 0usize;
            let mut tool_calls: Vec<String> = Vec::new();
            while let Some(event) = rx.next().await {
                match event {
                    AgentEvent::Delta(content) => response.push_str(&content),
                    AgentEvent::Retrying { .. } => {
                        response.truncate(attempt_start);
                    }
                    AgentEvent::StreamEnd => {
                        attempt_start = response.len();
                    }
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
        // 重试回滚锚点(同 cron 收集器,审计 W1-1)。
        let mut attempt_start = 0usize;
        let mut tool_calls: Vec<String> = Vec::new();
        while let Some(event) = rx.next().await {
            match event {
                AgentEvent::Delta(content) => response.push_str(&content),
                AgentEvent::Retrying { .. } => {
                    response.truncate(attempt_start);
                }
                AgentEvent::StreamEnd => {
                    attempt_start = response.len();
                }
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

// ── U4 · 子代理运行(delegate 的天盘编排)─────────────────────────
//
// 子代理不再是 XML 旁路循环:一律复用主 Agent 循环(native tools API,
// XML 仅作 provider 不支持时的自动回退),门禁与主循环同一代码路径
// (gate_one_tool → HumanPlate::prepare → execute → finalize_outcome,
// 公理 3)。TOOL-C1(2026-07-05 审计,公理 4 违规)由此结构性修复。
//
// 红线:
//   - 公理 2:LLM 调用经同一 zhong_core/aux_core(model 路由见下),无旁路核心;
//   - 公理 4:子代理无法交互,Guarded(需用户确认)默认即拒,不自动确认;
//     delegate 参数 allow_guarded 可显式授权提升;
//   - 位识边界:ephemeral agent —— 不熏习、不蒸馏、不写种子、不调 post_loop,
//     最终报告经 delegate 结果返回(scratchpad 仍是共享通道);
//   - Coder 强制 worktree 隔离:enter 复用工具层(嵌套拒绝沿用),完成或
//     失败后 worktree 保留在盘上,路径与分支名经报告返回供审阅/合并。

use crate::palaces::zhen_tool::builtin::delegate::{SubagentModel, SubagentType};
use crate::palaces::zhen_tool::builtin::exec::worktree::EnterWorktreeTool;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhong_core::JiaCore;
use crate::stems::InteractionMode;
use crate::stems::action::ExecContext;

/// Coder 隔离方式(强制;只读子代理恒为 None)。
pub enum WorktreeBinding {
    /// 只读子代理:无 worktree,继承父级权限视角。
    None,
    /// 新 Coder:自动 `git worktree add`(嵌套拒绝沿用工具层判定)。
    Enter,
    /// 续聊 Coder:复用既有 worktree(类型/路径绑定自持久化会话)。
    Reattach(std::path::PathBuf),
}

/// 一次子代理运行的全部输入(delegate 新鲜派发与 send_message 续聊共用)。
pub struct SubagentSpec {
    pub session_id: String,
    pub kind: SubagentType,
    pub model: SubagentModel,
    /// delegate 参数显式授权提升:true 时 Guarded 调用的确认门自动放行
    /// (默认 false —— 不自动确认,公理 4 单向收紧)。
    pub allow_guarded: bool,
    pub max_turns: u32,
    /// 续聊的历史(新鲜派发为空)。
    pub history: Vec<HistoryEntry>,
    /// 本轮任务/追问(作为 user 消息进入循环)。
    pub user_message: String,
    pub worktree: WorktreeBinding,
    /// 父级执行上下文:权限视角、worktree enter 的 base root、取消令牌来源。
    pub parent_ctx: ExecContext,
}

/// 子代理运行产物(经 delegate 结果返回给父级,不写主 agent 种子)。
pub struct SubagentReport {
    pub response: String,
    pub history: Vec<HistoryEntry>,
    pub worktree_path: Option<std::path::PathBuf>,
    pub worktree_branch: Option<String>,
}

/// 子代理身份提示词(ren 槽位;精简、无 ren_soul/记忆注入 —— 省 token)。
///
/// Coder 注入精简版编码规范(ren_soul 信之四约的引用版),并声明 worktree
/// 隔离边界;只读子代理不携带任何编码/记忆内容。
fn subagent_identity(
    kind: SubagentType,
    worktree: Option<(&std::path::Path, Option<&str>)>,
) -> String {
    match kind {
        SubagentType::Explore => "\
You are an Explore sub-agent. Research the codebase to answer the task. \
Use the available read-only tools to read files and search the codebase. \
Be thorough: look at multiple files, follow references, and trace logic. \
After researching, provide a detailed analysis with specific file paths and line numbers."
            .to_string(),
        SubagentType::Plan => "\
You are a Plan sub-agent. Design an implementation plan for the task. \
Use the available read-only tools to understand the existing codebase before planning. \
Read relevant files to understand patterns and architecture. \
After researching, provide a step-by-step implementation plan with specific \
file changes, architectural considerations, and dependencies."
            .to_string(),
        SubagentType::Coder => {
            let wt_note = match worktree {
                Some((path, branch)) => format!(
                    "\n\nYou work inside an isolated git worktree at {} (branch: {}). \
                     ALL file/shell/git operations are confined to it — the main checkout \
                     is untouched. Leave the work ready for review; do not merge or push.",
                    path.display(),
                    branch.unwrap_or("(resumed)")
                ),
                None => String::new(),
            };
            format!(
                "You are a Coder sub-agent. Complete the coding task with the available \
                 tools (read/write/patch/shell/git/grep/glob/lsp/revert). \
                 Work autonomously to completion, then report what you changed, \
                 how you verified it, and anything left undone.{wt_note}\n\n\
                 ## 信 (Trustworthiness) — Four Covenants\n\
                 1. 未读不改 — Never edit or patch a file you have not read in this session; read it first.\n\
                 2. 完成前验证 — Before claiming completion, verify: run the tests or commands and check the output.\n\
                 3. 如实报告失败 — Report failures truthfully; never claim success when the output shows failure.\n\
                 4. 不做分外事 — Do only what is asked; mention unrelated issues, but do not fix them."
            )
        }
        // #15 · 对抗性复核:不信"声称完成",只信自己复现的证据。
        SubagentType::Verifier => "\
You are a Verifier sub-agent. Adversarially review a claimed completion: \
independently re-run the tests/checks and confirm the claimed artifacts actually \
exist and hold — do not trust the claim, trust only what you reproduce yourself. \
You have read-only tools plus shell; use shell ONLY for verification commands \
(tests, builds, linters, inspections) and never to modify files or state. \
A hard gate enforces this: only allowlisted verification commands run (cargo \
test/check/clippy/build, pytest, go test, vitest/jest, npm/pnpm/yarn/bun test, \
git status/diff/log/show/blame, ls/cat/grep/rg/find/wc/head/tail); everything \
else, including redirection, is denied. \
End your report with a verdict line: \"Verdict: PASS\" (the claims hold — cite \
the commands you ran and their results) or \"Verdict: FAIL\" (list what does \
not hold, with the relevant command output)."
            .to_string(),
    }
}

/// 运行一个子代理(delegate/send_message 的统一执行路径)。
///
/// 编排居天盘:构造 ephemeral Agent + 子代理 HumanPlate + 模型路由,
/// 驱动主循环;delegate 始终是单甲工具(万甲归宗边界:批量派发的结果
/// 聚合在 delegate 内表述,此处只跑单甲)。
pub async fn run_subagent(
    earth: &Arc<EarthPlate>,
    spec: SubagentSpec,
) -> Result<SubagentReport, String> {
    // Burst-then-throttle: acquire a permit before running (5-minute
    // timeout prevents indefinite blocking; RAII release on drop).
    let _permit = tokio::time::timeout(
        std::time::Duration::from_secs(300),
        earth.subagent_batch.acquire(),
    )
    .await
    .map_err(|_| "Timed out waiting for sub-agent slot".to_string())?;

    // ── Coder 强制 worktree 隔离 ──
    let mut worktree_path: Option<std::path::PathBuf> = None;
    let mut worktree_branch: Option<String> = None;
    match &spec.worktree {
        WorktreeBinding::Enter => {
            let branch = format!(
                "coder-{}",
                uuid::Uuid::new_v4()
                    .to_string()
                    .split('-')
                    .next()
                    .unwrap_or("x")
            );
            // 复用工具层 enter(base root = 父级当前根;嵌套拒绝沿用)。
            EnterWorktreeTool::new()
                .execute(serde_json::json!({ "name": branch }), &spec.parent_ctx)
                .await
                .map_err(|e| {
                    format!(
                        "Coder sub-agent requires worktree isolation, but enter_worktree failed: {e}"
                    )
                })?;
            let base = spec.parent_ctx.permissions.sandbox.workspace_root.clone();
            worktree_path = Some(
                crate::palaces::zhen_tool::builtin::exec::worktree::worktree_path(&base, &branch),
            );
            worktree_branch = Some(branch);
        }
        WorktreeBinding::Reattach(path) => {
            if !path.is_dir() {
                return Err(format!(
                    "Coder worktree {} no longer exists; cannot resume — delegate a fresh Coder sub-agent.",
                    path.display()
                ));
            }
            worktree_path = Some(path.clone());
        }
        WorktreeBinding::None => {}
    }

    // ── ephemeral Agent(共享主循环与门禁;位识边界见 for_subagent)──
    let tools = match spec.kind {
        SubagentType::Coder => earth.subagent_coder_tools.clone(),
        // #15 · Verifier 注册表 = 只读注册表 + shell(跑测试/检查)+
        // retrieve_tool_result。写工具结构性缺席(注册表层面只读);shell
        // 由人盘验证命令白名单硬约束(verifier_shell_only,见下),Guarded
        // 默认即拒(公理 4 不放宽)。
        SubagentType::Verifier => {
            let mut reg = crate::palaces::zhen_tool::ToolRegistry::new();
            for t in earth.subagent_readonly_tools.list_core() {
                reg.register(t.clone());
            }
            reg.register(Arc::new(
                crate::palaces::zhen_tool::builtin::exec::shell::ShellTool::with_background_tasks(
                    earth.background_tasks.clone(),
                ),
            ));
            reg.register(Arc::new(
                crate::palaces::zhen_tool::builtin::exec::retrieve_tool_result::RetrieveToolResultTool::new(),
            ));
            Arc::new(reg)
        }
        _ => earth.subagent_readonly_tools.clone(),
    };
    let identity = subagent_identity(
        spec.kind,
        worktree_path
            .as_deref()
            .map(|p| (p, worktree_branch.as_deref())),
    );
    let mut agent = Agent::for_subagent(
        spec.session_id.clone(),
        earth.clone(),
        spec.history,
        tools,
        identity,
    );
    agent.max_turns = spec.max_turns;
    agent.interaction_mode = match spec.kind {
        SubagentType::Coder => InteractionMode::Auto,
        // Verifier 需要 shell 跑测试(谋划短路会把 shell 按变更类拒掉);
        // 只读约束由注册表结构性承担(写工具缺席),故同 Coder 用 Auto。
        SubagentType::Verifier => InteractionMode::Auto,
        // 只读子代理:谋划短路拒绝变更类工具并提示改用只读替代(同主循环语义)。
        _ => InteractionMode::Plan,
    };
    let cancel = spec.parent_ctx.cancel_token.clone();
    if let Some(path) = &worktree_path {
        agent.exec_ctx = earth.build_worktree_exec_ctx(path, &spec.session_id, cancel.clone());
        agent.worktree_root = Some(path.clone());
    } else {
        // 只读子代理继承父级权限视角(含父级所在 worktree),read_state 独立。
        agent.exec_ctx = ExecContext {
            permissions: spec.parent_ctx.permissions.clone(),
            session_id: spec.session_id.clone(),
            cancel_token: cancel.clone(),
            read_state: ExecContext::default_read_state(),
            cwd: ExecContext::default_cwd(&spec.parent_ctx.permissions),
        };
    }

    // ── 确认语义(公理 4 单向收紧)──
    // 子代理无法交互:Guarded(需用户确认)调用默认即拒 ——
    // confirmation_override 是 HumanPlate 既有的非交互裁决口。
    // false(默认)= 拒绝并回馈模型;true = delegate 参数显式授权提升。
    // Direct/Sandbox 本不经确认,worktree 隔离下照常自动执行。
    let mut human_plate = HumanPlate::with_state(
        agent.exec_ctx.permissions.clone(),
        earth.session_bus.clone(),
    );
    human_plate.confirmation_override = Some(spec.allow_guarded);
    // #15 · Verifier shell 硬约束:身份提示词的"只读"由人盘白名单结构承载
    // (qian_permission::verifier,默认拒绝;只收紧)。仅 Verifier 置位——
    // 主 agent 与 Explore/Plan/Coder 的人盘实例此位恒 false,行为不变。
    human_plate.verifier_shell_only = matches!(spec.kind, SubagentType::Verifier);

    // ── aux model 路由:默认 secondary = aux_core,无 aux 配置回落 primary ──
    let core: &JiaCore = match spec.model {
        SubagentModel::Secondary => earth.aux_core.as_deref().unwrap_or(&earth.main_core),
        SubagentModel::Primary => &earth.main_core,
    };

    // ── 事件收集(子代理事件不透出父级流;报告经 delegate 结果返回)──
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let collect = tokio::spawn(async move {
        let mut rx = UnboundedReceiverStream::new(rx);
        let mut response = String::new();
        // 重试回滚锚点(同 cron 收集器,审计 W1-1)。
        let mut attempt_start = 0usize;
        let mut error: Option<String> = None;
        while let Some(event) = rx.next().await {
            match event {
                AgentEvent::Delta(content) => response.push_str(&content),
                AgentEvent::Retrying { .. } => response.truncate(attempt_start),
                AgentEvent::StreamEnd => attempt_start = response.len(),
                AgentEvent::Error(msg) => {
                    // 记录但继续排空(turn 上限/LLM 错误后主循环即返回,
                    // 通道随 RunContext 下落关闭)——已有产出优于全废。
                    if error.is_none() {
                        error = Some(msg);
                    }
                }
                AgentEvent::Done => break,
                _ => {}
            }
        }
        (response, error)
    });

    {
        let ctx = RunContext {
            core,
            human_plate: &human_plate,
            event_bus: &earth.spirit.event_bus,
            hook_registry: &earth.spirit.hook_registry,
            tx,
            cancel_token: &cancel,
        };
        agent
            .run(vec![Message::text(Role::User, spec.user_message)], &ctx)
            .await;
        // 位识边界:不调 post_loop —— 子代理不熏习、不蒸馏、不写种子。
    } // ctx(tx) 下落 → 收集器通道关闭,排空结束

    let (response, error) = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        collect,
    )
    .await
    {
        Ok(Ok(v)) => v,
        _ => (String::new(), Some("sub-agent event collector failed".to_string())),
    };
    // P0-4 · 父级取消优先:取消中的部分产出(含 XML 协议残片)不冒充报告。
    if cancel.is_cancelled() {
        return Err("Sub-agent cancelled".into());
    }
    // 成功完成(含部分产出)后渐进恢复限流容量。
    earth.subagent_batch.maybe_recover();

    match (response.trim().is_empty(), error) {
        (true, Some(e)) => Err(e),
        (true, None) => Err("Sub-agent returned empty response".into()),
        (false, e) => {
            let mut response = response;
            if let Some(e) = e {
                response.push_str(&format!("\n\n[注意: 子代理终止于错误 — {e}]"));
            }
            Ok(SubagentReport {
                response,
                history: agent.history,
                worktree_path,
                worktree_branch,
            })
        }
    }
}

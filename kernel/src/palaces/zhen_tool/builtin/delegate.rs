//! delegate — 子代理委派工具(U4)。
//!
//! 子代理复用天盘主 Agent 循环(native tools API,XML 为 provider 不支持时
//! 的自动回退),每个工具调用过与主循环同一套门禁(谋划短路 → GeJu → hooks
//! → HumanPlate 分发,公理 3)——TOOL-C1(2026-07-05 审计,公理 4 违规:
//! 旧 XML 旁路循环绕过门禁)由此结构性修复。运行编排在
//! `tian_heaven::spawn::run_subagent`(天盘);本工具只做参数解析、批量派发
//! 的结果聚合与会话持久化(delegate 始终是单甲工具,万甲归宗边界)。
//!
//! 类型:
//!   - Explore / Plan:只读子代理(只读注册表 + 谋划短路),变更类调用被
//!     拒并提示改用只读替代;
//!   - Coder:可写注册表(read/write/patch/shell/git/grep/glob/lsp/revert),
//!     强制 worktree 隔离(自动 enter,完成/失败后保留在盘上供审阅);
//!   - Verifier(#15):只读注册表 + shell(验证命令),对抗性复核"声称
//!     完成"——独立重跑测试/检查声明产物,报告 Verdict: PASS/FAIL。
//!     写工具结构性缺席;谋划态下与 Coder 同被拦截(可执行 shell)。
//!
//! 确认语义(公理 4,单向收紧):子代理无法交互,Guarded(需用户确认)调用
//! 默认即拒,不自动确认;`allow_guarded: true` 为显式授权提升。Direct/
//! Sandbox 不经确认,worktree 隔离下照常自动执行。

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::ToolError;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhen_tool::builtin::exec::background_task::{
    BackgroundTask, BackgroundTaskStore, TaskStatus, TaskType,
};
use crate::plates::di_earth::EarthPlate;
use crate::plates::tian_heaven::spawn::{
    SubagentReport, SubagentSpec, Verdict, WorktreeBinding, run_subagent,
};
use crate::stems::{AgentEvent, CeremoniesIntent};
use crate::stems::action::ExecContext;
use crate::types::{HistoryEntry, Message, Role};
use tokio::sync::mpsc::UnboundedSender;

/// 子代理类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentType {
    Explore,
    Plan,
    Coder,
    /// #15 · 对抗性复核"声称完成":只读注册表 + shell(独立跑测试/检查
    /// 声明的产物)。写工具结构性缺席;shell 由人盘验证命令白名单硬约束
    /// (qian_permission::verifier,默认拒绝)。
    Verifier,
}

impl SubagentType {
    pub(crate) fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "explore" => Ok(Self::Explore),
            "plan" => Ok(Self::Plan),
            "coder" => Ok(Self::Coder),
            "verifier" => Ok(Self::Verifier),
            other => Err(format!(
                "Unknown subagent_type '{other}'. Use 'Explore', 'Plan', 'Coder' or 'Verifier'."
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explore => "Explore",
            Self::Plan => "Plan",
            Self::Coder => "Coder",
            Self::Verifier => "Verifier",
        }
    }
}

/// 子代理模型路由:secondary = aux_core(默认,无 aux 配置回落 primary)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubagentModel {
    Primary,
    #[default]
    Secondary,
}

impl SubagentModel {
    pub(crate) fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "primary" => Ok(Self::Primary),
            "secondary" => Ok(Self::Secondary),
            other => Err(format!(
                "Unknown model '{other}'. Use 'primary' or 'secondary'."
            )),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }
}

/// 谋划态门禁(loop_dispatch.gate_one_tool)用:delegate 入参是否请求
/// 可写/可执行型子代理——Coder(单个或 tasks[] 任一)或 #15 Verifier
/// (可执行 shell)。Coder 可写、Verifier 可跑命令,谋划态下均按变更类
/// 拦截(公理 4 只收紧;函数名保留——门禁调用点不变)。
pub(crate) fn requests_coder(params: &Value) -> bool {
    let is_mutable = |v: &Value| {
        v["subagent_type"]
            .as_str()
            .map(|s| s.eq_ignore_ascii_case("coder") || s.eq_ignore_ascii_case("verifier"))
            .unwrap_or(false)
    };
    if is_mutable(params) {
        return true;
    }
    params["tasks"]
        .as_array()
        .is_some_and(|arr| arr.iter().any(is_mutable))
}

/// #15 · delegate 入参是否请求 Verifier(单个或 tasks[] 任一)——天盘
/// 据此识别复核委派(验证信号 + 复核结论回流)。
pub(crate) fn requests_verifier(params: &Value) -> bool {
    let is_verifier = |v: &Value| {
        v["subagent_type"]
            .as_str()
            .map(|s| s.eq_ignore_ascii_case("verifier"))
            .unwrap_or(false)
    };
    if is_verifier(params) {
        return true;
    }
    params["tasks"]
        .as_array()
        .is_some_and(|arr| arr.iter().any(is_verifier))
}

/// P8 · a persisted sub-agent session, continuable via `send_message`.
///
/// 类型/模型/确认授权/worktree 路径在创建时绑定并持久化;resume 一律恢复
/// 原绑定,不被 send_message 的新参数覆盖。
pub struct SubagentSession {
    /// 主循环格式的会话历史(system prompt 由类型确定性重建,不入列)。
    pub history: Vec<HistoryEntry>,
    pub subagent_type: SubagentType,
    pub model: SubagentModel,
    pub allow_guarded: bool,
    /// Coder 隔离绑定:resume 时 Reattach 同一 worktree。
    pub worktree_path: Option<PathBuf>,
    pub created_at: i64,
    pub last_used: i64,
}

/// 持久化信封(v2):subagent_sessions.messages_json 列承载。旧行(XML 协议
/// 时代的 Vec<Message> JSON)按 legacy 路径转换为 HistoryEntry。
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionEnvelope {
    v: u32,
    history: Vec<HistoryEntry>,
    model: String,
    allow_guarded: bool,
    worktree_path: Option<String>,
}

impl SubagentSession {
    pub fn to_stored_json(&self) -> Option<String> {
        serde_json::to_string(&SessionEnvelope {
            v: 2,
            history: self.history.clone(),
            model: self.model.as_str().to_string(),
            allow_guarded: self.allow_guarded,
            worktree_path: self
                .worktree_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
        })
        .ok()
    }

    /// 崩溃恢复 hydration:先按 v2 信封解析,失败回退 legacy Vec<Message>。
    pub fn from_stored(
        json: &str,
        subagent_type: &str,
        created_at: i64,
        last_used: i64,
    ) -> Option<Self> {
        let kind = SubagentType::from_str(subagent_type).unwrap_or(SubagentType::Explore);
        if let Ok(env) = serde_json::from_str::<SessionEnvelope>(json)
            && env.v == 2
        {
            return Some(Self {
                history: env.history,
                subagent_type: kind,
                model: SubagentModel::from_str(&env.model).unwrap_or_default(),
                allow_guarded: env.allow_guarded,
                worktree_path: env.worktree_path.map(PathBuf::from),
                created_at,
                last_used,
            });
        }
        // Legacy: XML 协议时代的 Vec<Message>。
        if let Ok(messages) = serde_json::from_str::<Vec<Message>>(json) {
            let history = messages
                .into_iter()
                .map(|m| match m.role {
                    Role::User => HistoryEntry::User {
                        content: m.content,
                        images: m.images,
                    },
                    Role::Assistant => HistoryEntry::assistant(m.content),
                    Role::System => HistoryEntry::system(m.content),
                })
                .collect();
            return Some(Self {
                history,
                subagent_type: kind,
                model: SubagentModel::Secondary,
                allow_guarded: false,
                worktree_path: None,
                created_at,
                last_used,
            });
        }
        None
    }
}

/// 一次派发的任务输入(单任务 = tasks[1] 的特例)。
struct TaskInput {
    kind: SubagentType,
    prompt: String,
    max_turns: u32,
    model: SubagentModel,
    allow_guarded: bool,
}

fn parse_one_task(v: &Value) -> Result<TaskInput, String> {
    let kind = SubagentType::from_str(
        v["subagent_type"]
            .as_str()
            .ok_or("Missing 'subagent_type' parameter")?,
    )?;
    let prompt = v["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?
        .to_string();
    let max_turns = v["max_turns"].as_u64().unwrap_or(25).clamp(1, 50) as u32;
    let model = match v["model"].as_str() {
        Some(s) => SubagentModel::from_str(s)?,
        None => SubagentModel::Secondary,
    };
    let allow_guarded = v["allow_guarded"].as_bool().unwrap_or(false);
    Ok(TaskInput {
        kind,
        prompt,
        max_turns,
        model,
        allow_guarded,
    })
}

/// 单任务(subagent_type+prompt)或并行批量(tasks[])。
fn parse_tasks(input: &Value) -> Result<Vec<TaskInput>, String> {
    if let Some(arr) = input["tasks"].as_array() {
        if arr.is_empty() {
            return Err("'tasks' must be a non-empty array".to_string());
        }
        return arr.iter().map(parse_one_task).collect();
    }
    Ok(vec![parse_one_task(input)?])
}

/// P2 · 子代理生命周期透出(最小可观测,神盘语义:只观测不阻塞——
/// 发送失败静默丢弃)。承载在途子代理的 started/completed/failed;
/// 进度(progress)事件由 run_subagent 收集器经 progress_tx 发出。
fn emit_lifecycle(
    events: Option<&UnboundedSender<AgentEvent>>,
    id: &str,
    kind: SubagentType,
    status: &str,
    summary: String,
) {
    if let Some(tx) = events {
        let _ = tx.send(AgentEvent::SubagentLifecycle {
            id: id.to_string(),
            kind: kind.as_str().to_string(),
            status: status.to_string(),
            summary,
        });
    }
}

/// #15/P2 · Verifier 复核结论在 delegate 输出中的协议行(loop 侧严格
/// 解析;UNPARSEABLE = 子代理未遵循 verdict 协议,回落启发式)。
fn verdict_marker(verdict: Option<Verdict>) -> &'static str {
    match verdict {
        Some(Verdict::Pass) => "Verifier verdict: PASS",
        Some(Verdict::Fail) => "Verifier verdict: FAIL",
        None => "Verifier verdict: UNPARSEABLE",
    }
}

/// loop 侧消费:严格解析 delegate 输出中的结构化 verdict 协议行。
/// None = 无协议行(旧输出)或 UNPARSEABLE —— 调用方回落既有启发式。
pub(crate) fn delegate_output_verdict(output: &str) -> Option<Verdict> {
    for line in output.lines() {
        match line.trim() {
            "Verifier verdict: PASS" => return Some(Verdict::Pass),
            "Verifier verdict: FAIL" => return Some(Verdict::Fail),
            // UNPARSEABLE 是明确的"未遵循协议"信号,不再扫描后续行。
            "Verifier verdict: UNPARSEABLE" => return None,
            _ => {}
        }
    }
    None
}

/// #15/P2 · Verifier 复核失败判定(loop 消费路径,协议化):
/// 优先读结构化协议行;None(旧输出/UNPARSEABLE)回落"文本含
/// Verdict: FAIL"启发式 —— 过渡兼容,只收紧不放松。
pub(crate) fn verifier_failed(output: &str) -> bool {
    match delegate_output_verdict(output) {
        Some(Verdict::Fail) => true,
        Some(Verdict::Pass) => false,
        None => output.contains("Verdict: FAIL"),
    }
}

/// 新鲜派发的统一执行路径(inline / parallel / background 共用):
/// 运行子代理并把结果打包为可持久化会话(类型/模型/授权/worktree 绑定)。
async fn run_one(
    earth: &Arc<EarthPlate>,
    session_id: String,
    task: TaskInput,
    parent_ctx: ExecContext,
    progress_tx: Option<UnboundedSender<AgentEvent>>,
) -> Result<(SubagentReport, SubagentSession), String> {
    let spec = SubagentSpec {
        session_id,
        kind: task.kind,
        model: task.model,
        allow_guarded: task.allow_guarded,
        max_turns: task.max_turns,
        history: Vec::new(),
        user_message: task.prompt,
        worktree: match task.kind {
            // Coder 强制 worktree 隔离(嵌套拒绝沿用工具层判定)。
            SubagentType::Coder => WorktreeBinding::Enter,
            _ => WorktreeBinding::None,
        },
        parent_ctx,
        progress_tx,
    };
    let report = run_subagent(earth, spec).await?;
    let now = crate::utils::unix_now();
    let session = SubagentSession {
        history: report.history.clone(),
        subagent_type: task.kind,
        model: task.model,
        allow_guarded: task.allow_guarded,
        worktree_path: report.worktree_path.clone(),
        created_at: now,
        last_used: now,
    };
    Ok((report, session))
}

/// P8/P1 · 会话持久化:内存表(LRU 64)+ SQLite 崩溃恢复(类型/模型绑定随
/// 信封落盘)。
fn persist_session(earth: &Arc<EarthPlate>, subagent_id: &str, session: SubagentSession) {
    let json = session.to_stored_json();
    let ty = session.subagent_type.as_str().to_string();
    let created_at = session.created_at;
    let last_used = session.last_used;
    {
        let mut sessions = earth
            .session_bus
            .subagent_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Light capacity gate: drop the least-recently-used session when full.
        if sessions.len() >= 64
            && !sessions.contains_key(subagent_id)
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(k, _)| k.clone())
        {
            sessions.remove(&oldest);
        }
        sessions.insert(subagent_id.to_string(), session);
    }
    if let Some(json) = json {
        let _ = earth
            .store
            .save_subagent_session(subagent_id, &json, &ty, created_at, last_used);
    }
}

/// delegate 结果表述:Coder 报告 worktree 位置(完成/失败后保留在盘上,
/// 供审阅/合并/清理);Verifier 附结构化 verdict 协议行(loop 侧严格解析,
/// 见 delegate_output_verdict)。
fn format_report(subagent_id: &str, kind: SubagentType, report: &SubagentReport) -> String {
    let mut out = format!("Sub-agent {subagent_id} ({}) completed.", kind.as_str());
    if kind == SubagentType::Verifier {
        out.push_str(&format!("\n{}", verdict_marker(report.verdict)));
    }
    if let Some(path) = &report.worktree_path {
        let branch = report.worktree_branch.as_deref().unwrap_or("(resumed)");
        out.push_str(&format!(
            "\nWorktree: {} (branch: {branch}) — left on disk for review; merge or `git worktree remove` when done.",
            path.display()
        ));
    }
    out.push_str(&format!(
        "\n\n{}\n\nTo continue this sub-agent, call send_message with subagent_id=\"{subagent_id}\".",
        report.response
    ));
    out
}

/// Delegate a task to a sub-agent (Explore / Plan / Coder).
///
/// 地盘部件经 Weak<EarthPlate> 迟绑定(装配顺序:工具先于 EarthPlate 进
/// 注册表;zhen→di 边与既有 SubagentBatch/Store 注入同向,仅换成整盘弱
/// 引用以取用子代理注册表/spirit/aux_core 等)。
pub struct DelegateTool {
    earth: std::sync::OnceLock<std::sync::Weak<EarthPlate>>,
}

impl DelegateTool {
    pub fn new() -> Self {
        Self {
            earth: std::sync::OnceLock::new(),
        }
    }

    /// 起局装配完成后由 di_earth 调用一次(测试可直接绑定 mock 盘)。
    pub fn bind_earth(&self, earth: &Arc<EarthPlate>) {
        let _ = self.earth.set(Arc::downgrade(earth));
    }

    fn earth(&self) -> Result<Arc<EarthPlate>, ToolError> {
        self.earth
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| ToolError::exec(self.name(), "delegate tool is not bound to the Earth plate"))
    }
}

impl Default for DelegateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BaseTool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> String {
        "Delegate a task to a sub-agent that runs the full agent loop with its own tool set. \
         Types: 'Explore' (read-only codebase research), 'Plan' (read-only design), \
         'Coder' (reads/writes code inside an auto-created isolated git worktree — requires \
         a git repository; the worktree is left on disk for review), \
         'Verifier' (read-only tools + shell for verification commands — adversarially \
         re-runs tests/checks against a claimed completion and reports Verdict: PASS/FAIL). \
         Sub-agents cannot ask the user: confirmation-gated (Guarded) tool calls are denied \
         unless allow_guarded=true explicitly authorizes auto-approval. \
         Use tasks=[{subagent_type, prompt, ...}] to dispatch several sub-agents in parallel \
         and get aggregated results; model='primary'|'secondary' (default secondary = aux \
         model, falling back to primary); run_in_background=true runs detached and reports \
         completion as a background task notification. Returns a subagent_id for \
         continuation via send_message (type/model bindings are preserved on resume)."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        // 戊仪(只读型委派):Explore/Plan 子代理只读。Coder 可写——谋划态下
        // 由 gate_one_tool 按名+入参拦截(requests_coder),见 loop_dispatch。
        CeremoniesIntent::Wu
    }

    fn target_palace(&self, input: &Value) -> crate::palaces::Palace {
        match input["subagent_type"].as_str() {
            Some("Explore") | Some("Plan") => crate::palaces::Palace::Dui,
            _ => crate::palaces::Palace::Xun,
        }
    }

    fn parameters_schema(&self) -> Value {
        let task_props = serde_json::json!({
            "subagent_type": {
                "type": "string",
                "description": "Type of sub-agent: 'Explore' (read-only research), 'Plan' (read-only planning), 'Coder' (writes code in an isolated git worktree), 'Verifier' (read-only + shell verification commands; re-runs tests against a claimed completion)"
            },
            "prompt": {
                "type": "string",
                "description": "The task description and instructions for the sub-agent"
            },
            "max_turns": {
                "type": "integer",
                "description": "Maximum reasoning turns (default: 25)",
                "minimum": 1,
                "maximum": 50
            },
            "model": {
                "type": "string",
                "enum": ["primary", "secondary"],
                "description": "Model routing (default: 'secondary' = aux model, falls back to primary when unconfigured)"
            },
            "allow_guarded": {
                "type": "boolean",
                "description": "Explicitly authorize auto-approval of confirmation-gated (Guarded) tool calls (default: false — denied, sub-agents cannot ask)"
            }
        });
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_type": task_props["subagent_type"].clone(),
                "prompt": task_props["prompt"].clone(),
                "max_turns": task_props["max_turns"].clone(),
                "model": task_props["model"].clone(),
                "allow_guarded": task_props["allow_guarded"].clone(),
                "tasks": {
                    "type": "array",
                    "description": "Parallel dispatch: several sub-agents at once (each entry takes subagent_type/prompt/max_turns/model/allow_guarded). Mutually exclusive with the single-task parameters.",
                    "items": {
                        "type": "object",
                        "properties": task_props,
                        "required": ["subagent_type", "prompt"]
                    }
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Run detached (default: false); completion is reported as a background task notification in a later turn"
                }
            }
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        // P2 · Parallel: independent session creation, brief lock only for insert.
        true
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        self.execute_inner(input, ctx, None).await
    }

    async fn execute_with_tx(
        &self,
        input: Value,
        tx: &UnboundedSender<AgentEvent>,
        ctx: &ExecContext,
    ) -> Result<String, ToolError> {
        // P2 · 子代理生命周期/进度事件经父级事件通道透出(最小可观测)。
        self.execute_inner(input, ctx, Some(tx)).await
    }
}

impl DelegateTool {
    async fn execute_inner(
        &self,
        input: Value,
        ctx: &ExecContext,
        events: Option<&UnboundedSender<AgentEvent>>,
    ) -> Result<String, ToolError> {
        let earth = self.earth()?;
        let tasks = parse_tasks(&input)?;
        let run_in_background = input["run_in_background"].as_bool().unwrap_or(false);

        // ── 后台统一:复用在途 BackgroundTaskStore(类型前缀 a),完成通知由
        // 主循环既有的后台通知机制注入后续 turn。后台子代理独立于父级 turn
        // 的取消令牌(detached,同后台 shell 语义)。
        // P2 · 事件分工(不重复):后台子代理此处只发 started 生命周期事件;
        // 完成/失败通知走 BackgroundTaskStore → TaskCompleted(任务体系),
        // 不再发 completed/failed,进度事件也不发(父级 turn 可能已结束)。
        if run_in_background {
            let mut lines = Vec::new();
            for task in tasks {
                let subagent_id = uuid::Uuid::new_v4().to_string();
                let task_id = BackgroundTaskStore::generate_id(TaskType::Agent);
                let out_path =
                    crate::palaces::zhen_tool::builtin::exec::disk_output::task_output_path(
                        &task_id,
                    );
                earth.background_tasks.register(BackgroundTask {
                    id: task_id.clone(),
                    task_type: TaskType::Agent,
                    status: TaskStatus::Pending,
                    description: crate::utils::truncate_chars(&task.prompt, 80),
                    output_file: out_path.clone(),
                    output_offset: 0,
                    notified: false,
                    started_at: std::time::Instant::now(),
                    ended_at: None,
                    tool_use_id: None,
                    agent_id: Some(subagent_id.clone()),
                    exit_code: None,
                });
                emit_lifecycle(
                    events,
                    &subagent_id,
                    task.kind,
                    "started",
                    crate::utils::truncate_chars(&task.prompt, 80),
                );
                let earth = earth.clone();
                let mut parent_ctx = ctx.clone();
                parent_ctx.cancel_token = tokio_util::sync::CancellationToken::new();
                let bg_task_id = task_id.clone();
                let bg_id = subagent_id.clone();
                let bg_out_path = out_path.clone();
                tokio::spawn(async move {
                    let outcome = run_one(&earth, bg_id.clone(), task, parent_ctx, None).await;
                    let (text, status) = match outcome {
                        Ok((report, session)) => {
                            let text = format_report(&bg_id, session.subagent_type, &report);
                            persist_session(&earth, &bg_id, session);
                            (text, TaskStatus::Completed)
                        }
                        Err(e) => (
                            format!("Sub-agent {bg_id} failed: {e}"),
                            TaskStatus::Failed,
                        ),
                    };
                    if let Some(parent) = bg_out_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&bg_out_path, &text);
                    earth
                        .background_tasks
                        .update_status(&bg_task_id, status, None);
                });
                lines.push(format!(
                    "Sub-agent ({}) started in background: task_id={task_id}, subagent_id={subagent_id}.\n\
                     Completion will arrive as a background task notification; output file: {}.",
                    match input["tasks"].as_array() {
                        Some(_) => "batch",
                        None => "single",
                    },
                    out_path.display()
                ));
            }
            return Ok(lines.join("\n"));
        }

        // ── 单任务:inline 运行 ──
        if tasks.len() == 1 {
            let Some(task) = tasks.into_iter().next() else {
                return Err(ToolError::exec(self.name(), "'tasks' must be a non-empty array"));
            };
            let subagent_id = uuid::Uuid::new_v4().to_string();
            emit_lifecycle(
                events,
                &subagent_id,
                task.kind,
                "started",
                crate::utils::truncate_chars(&task.prompt, 80),
            );
            let kind = task.kind;
            let outcome = run_one(
                &earth,
                subagent_id.clone(),
                task,
                ctx.clone(),
                events.cloned(),
            )
            .await;
            let (report, session) = match outcome {
                Ok(v) => v,
                Err(e) => {
                    emit_lifecycle(events, &subagent_id, kind, "failed", e.clone());
                    return Err(ToolError::exec(self.name(), e));
                }
            };
            let out = format_report(&subagent_id, session.subagent_type, &report);
            emit_lifecycle(
                events,
                &subagent_id,
                kind,
                "completed",
                crate::utils::truncate_chars(report.response.trim(), 120),
            );
            persist_session(&earth, &subagent_id, session);
            return Ok(out);
        }

        // ── 并行派发:JoinSet(U1 模式),许可在 run_subagent 内获取
        // (burst-then-throttle 复用);逐任务独立成败,按声明序聚合返回。
        let n = tasks.len();
        let mut join_set: tokio::task::JoinSet<
            (usize, String, SubagentType, Result<(SubagentReport, SubagentSession), String>),
        > = tokio::task::JoinSet::new();
        for (i, task) in tasks.into_iter().enumerate() {
            let earth = earth.clone();
            let pctx = ctx.clone();
            let subagent_id = uuid::Uuid::new_v4().to_string();
            emit_lifecycle(
                events,
                &subagent_id,
                task.kind,
                "started",
                crate::utils::truncate_chars(&task.prompt, 80),
            );
            let kind = task.kind;
            let progress = events.cloned();
            join_set.spawn(async move {
                let report = run_one(&earth, subagent_id.clone(), task, pctx, progress).await;
                (i, subagent_id, kind, report)
            });
        }
        let mut results: Vec<
            Option<(String, SubagentType, Result<(SubagentReport, SubagentSession), String>)>,
        > = (0..n).map(|_| None).collect();
        loop {
            tokio::select! {
                joined = join_set.join_next() => match joined {
                    Some(Ok((i, id, kind, r))) => {
                        // 完成态即时透出(不等聚合),成败如实。
                        match &r {
                            Ok((report, _)) => emit_lifecycle(
                                events,
                                &id,
                                kind,
                                "completed",
                                crate::utils::truncate_chars(report.response.trim(), 120),
                            ),
                            Err(e) => emit_lifecycle(events, &id, kind, "failed", e.clone()),
                        }
                        results[i] = Some((id, kind, r));
                    }
                    Some(Err(e)) => {
                        // Aborted (parent cancel) or panicked — slot stays None.
                        if e.is_panic() {
                            tracing::error!(error = %e, "parallel sub-agent task panicked");
                        }
                    }
                    None => break,
                },
                _ = ctx.cancel_token.cancelled() => {
                    join_set.abort_all();
                    while join_set.join_next().await.is_some() {}
                    break;
                }
            }
        }
        let mut sections = Vec::new();
        for (i, slot) in results.into_iter().enumerate() {
            match slot {
                Some((id, kind, Ok((report, session)))) => {
                    let text = format_report(&id, kind, &report);
                    persist_session(&earth, &id, session);
                    sections.push(format!("## [{}] {text}", i + 1));
                }
                Some((id, _, Err(e))) => {
                    sections.push(format!("## [{}] Sub-agent {id} failed: {e}", i + 1));
                }
                None => {
                    sections.push(format!("## [{}] Sub-agent cancelled before completion.", i + 1));
                }
            }
        }
        Ok(sections.join("\n\n"))
    }
}

/// P8 · Continue a previously delegated sub-agent by id (SendMessage pattern).
///
/// 类型/模型/确认授权/worktree 绑定自持久化会话恢复(不被新参数覆盖);
/// 循环与门禁与 delegate 完全同径。注意:谋划态下 send_message 被门禁按名
/// 拦截(loop_dispatch)——续聊的可能是类型绑定的 Coder 会话,无法在门禁
/// 处判定,一律拒(公理 4 只收紧);退出谋划态后再续聊。
pub struct SendMessageTool {
    earth: std::sync::OnceLock<std::sync::Weak<EarthPlate>>,
}

impl SendMessageTool {
    pub fn new() -> Self {
        Self {
            earth: std::sync::OnceLock::new(),
        }
    }

    /// 起局装配完成后由 di_earth 调用一次。
    pub fn bind_earth(&self, earth: &Arc<EarthPlate>) {
        let _ = self.earth.set(Arc::downgrade(earth));
    }

    fn earth(&self) -> Result<Arc<EarthPlate>, ToolError> {
        self.earth
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| ToolError::exec(self.name(), "send_message tool is not bound to the Earth plate"))
    }
}

impl Default for SendMessageTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BaseTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> String {
        "Continue a previously delegated sub-agent (identified by subagent_id \
         returned from delegate) with a follow-up message. The sub-agent keeps \
         its full prior context and its original type/model/worktree bindings. \
         Use this to refine or extend a sub-agent's work without re-delegating \
         from scratch."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        // 戊仪(只读型续聊)。续聊 Coder 会话可写——谋划态下由 gate_one_tool
        // 按名拦截(见 loop_dispatch 与模块文档)。
        CeremoniesIntent::Wu
    }

    fn target_palace(&self, _input: &Value) -> crate::palaces::Palace {
        crate::palaces::Palace::Dui
    }

    fn is_concurrency_safe(&self) -> bool {
        // P2 · Parallel: independent session continuation, brief lock only.
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_id": {
                    "type": "string",
                    "description": "The subagent_id returned by a prior delegate call"
                },
                "message": {
                    "type": "string",
                    "description": "The follow-up message for the sub-agent"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum reasoning turns (default: 25)",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["subagent_id", "message"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        self.execute_inner(input, ctx, None).await
    }

    async fn execute_with_tx(
        &self,
        input: Value,
        tx: &UnboundedSender<AgentEvent>,
        ctx: &ExecContext,
    ) -> Result<String, ToolError> {
        // P2 · 续聊子代理的生命周期/进度事件同样透出父级(同 delegate)。
        self.execute_inner(input, ctx, Some(tx)).await
    }
}

impl SendMessageTool {
    async fn execute_inner(
        &self,
        input: Value,
        ctx: &ExecContext,
        events: Option<&UnboundedSender<AgentEvent>>,
    ) -> Result<String, ToolError> {
        let earth = self.earth()?;
        let subagent_id = input["subagent_id"]
            .as_str()
            .ok_or("Missing 'subagent_id' parameter")?;
        let message = input["message"]
            .as_str()
            .ok_or("Missing 'message' parameter")?;
        let max_turns = input["max_turns"].as_u64().unwrap_or(25).clamp(1, 50) as u32;

        // Load the bound session (clone out so the lock is not held during
        // the run). 类型/模型/授权/worktree 绑定随会话恢复,不被新参数覆盖。
        let (history, kind, model, allow_guarded, worktree_path, created_at) = {
            let mut sessions = earth
                .session_bus
                .subagent_sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let session = sessions
                .get_mut(subagent_id)
                .ok_or_else(|| format!("Unknown subagent_id '{subagent_id}'"))?;
            session.last_used = crate::utils::unix_now();
            (
                session.history.clone(),
                session.subagent_type,
                session.model,
                session.allow_guarded,
                session.worktree_path.clone(),
                session.created_at,
            )
        };

        let worktree = match (kind, worktree_path) {
            (SubagentType::Coder, Some(path)) => WorktreeBinding::Reattach(path),
            // 防御:绑定丢失(如 legacy 会话)的 Coder 续聊重新 enter。
            (SubagentType::Coder, None) => WorktreeBinding::Enter,
            _ => WorktreeBinding::None,
        };
        let spec = SubagentSpec {
            session_id: subagent_id.to_string(),
            kind,
            model,
            allow_guarded,
            max_turns,
            history,
            user_message: message.to_string(),
            worktree,
            parent_ctx: ctx.clone(),
            progress_tx: events.cloned(),
        };
        emit_lifecycle(
            events,
            subagent_id,
            kind,
            "started",
            crate::utils::truncate_chars(message, 80),
        );
        let outcome = run_subagent(&earth, spec).await;
        let report = match outcome {
            Ok(r) => r,
            Err(e) => {
                emit_lifecycle(events, subagent_id, kind, "failed", e.clone());
                return Err(ToolError::exec(self.name(), e));
            }
        };
        emit_lifecycle(
            events,
            subagent_id,
            kind,
            "completed",
            crate::utils::truncate_chars(report.response.trim(), 120),
        );

        // Store the updated conversation back with the SAME bindings.
        let session = SubagentSession {
            history: report.history.clone(),
            subagent_type: kind,
            model,
            allow_guarded,
            worktree_path: report.worktree_path.clone(),
            created_at,
            last_used: crate::utils::unix_now(),
        };
        persist_session(&earth, subagent_id, session);

        let mut out = String::new();
        if let Some(path) = &report.worktree_path {
            out.push_str(&format!("Worktree: {} (resumed)\n\n", path.display()));
        }
        out.push_str(&report.response);
        // Verifier 续聊同样附结构化 verdict 协议行(与 format_report 同径)。
        if kind == SubagentType::Verifier {
            out.push_str(&format!("\n\n{}", verdict_marker(report.verdict)));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use crate::palaces::gen_store::Store;
    use crate::palaces::kan_io::ChannelManager;
    use crate::palaces::kun_config::{AppConfig, CognitionSection, SecuritySection};
    use crate::palaces::li_skill::SkillRegistry;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::palaces::zhen_tool::ToolRegistry;
    use crate::palaces::zhen_tool::builtin::fs::read_file::ReadFileTool;
    use crate::palaces::zhen_tool::builtin::fs::write_file::WriteFileTool;
    use crate::palaces::zhong_core::JiaCore;
    use crate::plates::shen_spirit::SpiritPlate;
    use crate::plates::shen_spirit::completion_check::CompletionChecklist;

    /// Mock 地盘:主/辅模型皆 mock provider(kind "mock" → XML 回退路径,
    /// 与 native 共享同一循环与门禁)。readonly 注册表默认 [read_file],
    /// coder 注册表默认 [read_file, write_file](worktree 测试用)。
    fn mock_earth(
        tmp: &std::path::Path,
        main_responses: Vec<String>,
        aux_responses: Option<Vec<String>>,
        readonly_extra: Vec<Arc<dyn BaseTool>>,
    ) -> Arc<EarthPlate> {
        mock_earth_with_core(
            tmp,
            Arc::new(JiaCore::with_mock(main_responses)),
            aux_responses,
            readonly_extra,
        )
    }

    /// 自定义主模型核心的变体(native tools 路径测试注入 scripted provider)。
    fn mock_earth_with_core(
        tmp: &std::path::Path,
        main_core: Arc<JiaCore>,
        aux_responses: Option<Vec<String>>,
        readonly_extra: Vec<Arc<dyn BaseTool>>,
    ) -> Arc<EarthPlate> {
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
        let config_loader = Arc::new(crate::palaces::kun_config::ConfigLoader::from_app_config(config));
        let permissions = Arc::new(PermissionMatrix::from_config(
            &security,
            &tmp.join("workspace"),
            tmp.join("backups"),
        ));
        let mut readonly = ToolRegistry::new();
        readonly.register(Arc::new(ReadFileTool::new()));
        for t in readonly_extra {
            readonly.register(t);
        }
        let mut coder = ToolRegistry::new();
        coder.register(Arc::new(ReadFileTool::new()));
        coder.register(Arc::new(WriteFileTool::new()));
        let store = Arc::new(Store::open(tmp.join("store.db").to_str().unwrap()));
        Arc::new(EarthPlate {
            io: Arc::new(ChannelManager::default()),
            config: config_loader,
            tools: Arc::new(ToolRegistry::new()),
            subagent_readonly_tools: Arc::new(readonly),
            subagent_coder_tools: Arc::new(coder),
            main_core,
            aux_core: aux_responses.map(|r| Arc::new(JiaCore::with_mock(r))),
            permissions,
            skills: Arc::new(std::sync::RwLock::new(SkillRegistry::new())),
            cron: crate::palaces::zhen_tool::builtin::cron::CronStore::new(tmp.join("cron")),
            task_store: crate::palaces::zhen_tool::builtin::exec::task::TaskStore::new(),
            background_tasks: BackgroundTaskStore::new(),
            subagent_batch: Arc::new(crate::plates::tian_heaven::subagent_batch::SubagentBatch::new()),
            store_async: crate::palaces::gen_store::async_store::StoreAsync::new(store.clone()),
            store,
            spirit: Arc::new(SpiritPlate::new()),
            completion_checklist: Arc::new(CompletionChecklist::new()),
            user_hooks: Arc::new(Vec::new()),
            session_bus: Arc::new(crate::plates::ren_human::SessionBus::new()),
            data_dir: tmp.to_path_buf(),
            pid_path: tmp.join("gateway.pid"),
            backup_dir: tmp.join("backups"),
        })
    }

    fn bound_delegate(earth: &Arc<EarthPlate>) -> Arc<DelegateTool> {
        let tool = Arc::new(DelegateTool::new());
        tool.bind_earth(earth);
        tool
    }

    fn bound_send_message(earth: &Arc<EarthPlate>) -> Arc<SendMessageTool> {
        let tool = Arc::new(SendMessageTool::new());
        tool.bind_earth(earth);
        tool
    }

    fn parent_ctx(earth: &Arc<EarthPlate>) -> ExecContext {
        let mut ctx = ExecContext::new(earth.permissions.clone());
        ctx.session_id = "parent-session".to_string();
        ctx
    }

    fn extract_id(out: &str) -> &str {
        out.split("subagent_id=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("subagent_id in output")
    }

    /// 会话历史中的工具调用条目 (tool, output, error)。
    fn tool_entries(
        earth: &Arc<EarthPlate>,
        id: &str,
    ) -> Vec<(String, String, Option<String>)> {
        let sessions = earth.session_bus.subagent_sessions.lock().unwrap();
        let session = sessions.get(id).expect("session stored");
        session
            .history
            .iter()
            .filter_map(|e| match e {
                HistoryEntry::ToolCall {
                    tool,
                    output,
                    error,
                    ..
                } => Some((tool.clone(), output.clone(), error.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn subagent_type_from_str() {
        assert!(matches!(
            SubagentType::from_str("Explore"),
            Ok(SubagentType::Explore)
        ));
        assert!(matches!(
            SubagentType::from_str("PLAN"),
            Ok(SubagentType::Plan)
        ));
        assert!(matches!(
            SubagentType::from_str("coder"),
            Ok(SubagentType::Coder)
        ));
        assert!(SubagentType::from_str("invalid").is_err());
    }

    #[test]
    fn requests_coder_detects_single_and_batch() {
        assert!(requests_coder(&serde_json::json!({"subagent_type": "Coder"})));
        assert!(requests_coder(&serde_json::json!({"subagent_type": "coder"})));
        assert!(requests_coder(&serde_json::json!({
            "tasks": [{"subagent_type": "Explore", "prompt": "a"},
                      {"subagent_type": "Coder", "prompt": "b"}]
        })));
        assert!(!requests_coder(&serde_json::json!({"subagent_type": "Explore"})));
        assert!(!requests_coder(&serde_json::json!({
            "tasks": [{"subagent_type": "Explore", "prompt": "a"}]
        })));
    }

    #[test]
    fn session_envelope_roundtrip_preserves_bindings() {
        let session = SubagentSession {
            history: vec![HistoryEntry::user("task"), HistoryEntry::assistant("done")],
            subagent_type: SubagentType::Coder,
            model: SubagentModel::Primary,
            allow_guarded: true,
            worktree_path: Some(PathBuf::from("/repo/.jia/worktrees/coder-x")),
            created_at: 1,
            last_used: 2,
        };
        let json = session.to_stored_json().unwrap();
        let restored = SubagentSession::from_stored(&json, "Coder", 1, 2).unwrap();
        assert_eq!(restored.subagent_type, SubagentType::Coder);
        assert_eq!(restored.model, SubagentModel::Primary);
        assert!(restored.allow_guarded);
        assert_eq!(
            restored.worktree_path.as_deref(),
            Some(std::path::Path::new("/repo/.jia/worktrees/coder-x"))
        );
        assert_eq!(restored.history.len(), 2);
    }

    #[test]
    fn session_from_stored_legacy_messages() {
        let messages = vec![
            Message::text(Role::System, "sys"),
            Message::text(Role::User, "task"),
            Message::text(Role::Assistant, "answer"),
        ];
        let json = serde_json::to_string(&messages).unwrap();
        let restored = SubagentSession::from_stored(&json, "Explore", 1, 2).unwrap();
        assert_eq!(restored.subagent_type, SubagentType::Explore);
        // legacy 会话绑定默认:secondary 模型、无授权提升、无 worktree。
        assert_eq!(restored.model, SubagentModel::Secondary);
        assert!(!restored.allow_guarded);
        assert!(restored.worktree_path.is_none());
        assert_eq!(restored.history.len(), 3);
    }

    #[tokio::test]
    async fn delegate_missing_params() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(tmp.path(), vec![], None, vec![]);
        let tool = bound_delegate(&earth);
        assert!(tool.execute(serde_json::json!({}), &parent_ctx(&earth)).await.is_err());
        assert!(
            tool.execute(serde_json::json!({"subagent_type": "Explore"}), &parent_ctx(&earth))
                .await
                .is_err()
        );
        assert!(
            tool.execute(serde_json::json!({"tasks": []}), &parent_ctx(&earth))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delegate_unknown_type() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(tmp.path(), vec![], None, vec![]);
        let tool = bound_delegate(&earth);
        let result = tool
            .execute(
                serde_json::json!({"subagent_type": "invalid", "prompt": "test"}),
                &parent_ctx(&earth),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown subagent_type"));
    }

    #[tokio::test]
    async fn delegate_stores_session_and_returns_id() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(tmp.path(), vec!["analysis: found X".into()], None, vec![]);
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "find X"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("Sub-agent "), "expected id in output: {out}");
        assert!(out.contains("analysis: found X"), "report in output: {out}");
        let id = extract_id(&out);
        let sessions = earth.session_bus.subagent_sessions.lock().unwrap();
        let session = sessions.get(id).expect("session stored");
        // 创建时绑定:类型 Explore、默认 secondary 模型、无授权提升。
        assert_eq!(session.subagent_type, SubagentType::Explore);
        assert_eq!(session.model, SubagentModel::Secondary);
        assert!(!session.allow_guarded);
        assert!(session.worktree_path.is_none());
    }

    /// TOOL-C1 回归(公理 4):子代理的工具调用必须过与主循环同一套门禁。
    /// 只读子代理请求需用户确认的调用(.env 敏感文件强制 ask)→
    /// HumanPlate 确认门在子代理上下文即拒(不挂起等待),拒绝进入历史,
    /// 子代理继续给出最终报告。
    #[tokio::test]
    async fn tool_c1_guarded_call_denied_in_subagent() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec![
                r#"<tool_call>
{"name": "read_file", "parameters": {"path": ".env"}}
</tool_call>"#
                    .into(),
                "final report: cannot read .env".into(),
            ],
            None,
            vec![],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "read secrets"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("final report"), "output: {out}");
        let id = extract_id(&out);
        let entries = tool_entries(&earth, id);
        let (tool, _out, err) = entries
            .iter()
            .find(|(t, _, _)| t == "read_file")
            .expect("read_file call recorded");
        let err = err.as_ref().expect("guarded call must be denied");
        assert!(
            err.contains("denied") || err.contains("拒"),
            "denial reason: {err}"
        );
        assert_eq!(tool, "read_file");
    }

    /// TOOL-C1 回归:只读子代理(谋划态)发起变更类工具 → 谋划短路拒绝并
    /// 提示(与主循环同一 gate_one_tool 代码路径)。
    #[tokio::test]
    async fn tool_c1_destructive_call_blocked_in_readonly_subagent() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec![
                r#"<tool_call>
{"name": "write_file", "parameters": {"path": "x.txt", "content": "y"}}
</tool_call>"#
                    .into(),
                "report: stayed read-only".into(),
            ],
            None,
            // 测试注册表故意放入 write_file:验证的是门禁而非注册表缺失。
            vec![Arc::new(WriteFileTool::new())],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "write something"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("stayed read-only"), "output: {out}");
        let id = extract_id(&out);
        let entries = tool_entries(&earth, id);
        let (_, _, err) = entries
            .iter()
            .find(|(t, _, _)| t == "write_file")
            .expect("write_file call recorded");
        assert!(
            err.as_ref().unwrap().contains("谋划态"),
            "plan-mode short-circuit reason: {err:?}"
        );
        // 文件不得被写出。
        assert!(!tmp.path().join("workspace").join("x.txt").exists());
        assert!(!tmp.path().join("x.txt").exists());
    }

    /// U4 · native tools 路径:子代理 loop 复用主循环(native API),
    /// 工具执行与门禁同径。
    #[tokio::test]
    async fn native_tools_path_executes_through_gate() {
        use crate::error::ProviderError;
        use crate::palaces::zhong_core::{LlmProvider, StreamChunk};

        struct ScriptProvider {
            scripts: Mutex<Vec<Vec<StreamChunk>>>,
        }
        impl LlmProvider for ScriptProvider {
            fn infer_stream(
                &self,
                _messages: Vec<Message>,
                _tools: Option<&[crate::stems::action::ToolSchema]>,
                _cancel_token: Option<tokio_util::sync::CancellationToken>,
            ) -> std::pin::Pin<
                Box<dyn futures::Stream<Item = Result<StreamChunk, ProviderError>> + Send>,
            > {
                let script = {
                    let mut guard = self.scripts.lock().unwrap();
                    if guard.is_empty() { vec![] } else { guard.remove(0) }
                };
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                tokio::spawn(async move {
                    for chunk in script {
                        let _ = tx.send(Ok(chunk));
                    }
                });
                Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("real.txt"), "HELLO-NATIVE").unwrap();
        // kind "anthropic" → native tools 路径。
        let provider: Box<dyn LlmProvider> = Box::new(ScriptProvider {
            scripts: Mutex::new(vec![
                vec![
                    StreamChunk::NativeToolCall {
                        id: "c1".into(),
                        name: "read_file".into(),
                        arguments: serde_json::json!({"path": "real.txt"}).to_string(),
                    },
                    StreamChunk::Delta("reading.".into()),
                ],
                vec![StreamChunk::Delta("NATIVE-DONE".into())],
            ]),
        });
        let router = crate::palaces::zhong_core::ProviderRouter::new(vec![(0u32, provider)]);
        let native_core = Arc::new(JiaCore::with_router(router, "anthropic".into(), "mock".into(), 8192));
        let earth = mock_earth_with_core(tmp.path(), native_core, None, vec![]);
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "read real.txt"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("NATIVE-DONE"), "output: {out}");
        let id = extract_id(&out);
        let entries = tool_entries(&earth, id);
        let (_, output, err) = entries
            .iter()
            .find(|(t, _, _)| t == "read_file")
            .expect("read_file call recorded");
        assert!(err.is_none(), "read must succeed: {err:?}");
        assert!(output.contains("HELLO-NATIVE"), "tool output: {output}");
    }

    /// aux 路由:默认 secondary → aux_core;model=primary → 主模型。
    #[tokio::test]
    async fn aux_model_routes_secondary_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec!["MAIN-RESPONSE".into()],
            Some(vec!["AUX-RESPONSE".into()]),
            vec![],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "p"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("AUX-RESPONSE"), "default must route to aux: {out}");

        let tmp2 = tempfile::tempdir().unwrap();
        let earth2 = mock_earth(
            tmp2.path(),
            vec!["MAIN-RESPONSE".into()],
            Some(vec!["AUX-RESPONSE".into()]),
            vec![],
        );
        let tool2 = bound_delegate(&earth2);
        let out2 = tool2
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "p", "model": "primary"}),
                &parent_ctx(&earth2),
            )
            .await
            .expect("delegate failed");
        assert!(out2.contains("MAIN-RESPONSE"), "explicit primary: {out2}");
    }

    /// aux 路由回落:无 aux 配置时 secondary 回落 primary。
    #[tokio::test]
    async fn aux_model_falls_back_to_primary() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(tmp.path(), vec!["MAIN-RESPONSE".into()], None, vec![]);
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "p", "model": "secondary"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("MAIN-RESPONSE"), "fallback to primary: {out}");
    }

    /// resume 绑定:send_message 恢复创建时的类型/模型,不被默认值覆盖。
    #[tokio::test]
    async fn resume_restores_model_binding() {
        let tmp = tempfile::tempdir().unwrap();
        // 主模型两次应答(首发 + 续聊);aux 一次。绑定若丢失(回落 secondary),
        // 续聊会命中 aux 的 "A1"。
        let earth = mock_earth(
            tmp.path(),
            vec!["P1".into(), "P2".into()],
            Some(vec!["A1".into()]),
            vec![],
        );
        let delegate = bound_delegate(&earth);
        let out = delegate
            .execute(
                serde_json::json!({"subagent_type": "Explore", "prompt": "p", "model": "primary"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("P1"), "first answer: {out}");
        let id = extract_id(&out).to_string();

        let sm = bound_send_message(&earth);
        let resumed = sm
            .execute(
                serde_json::json!({"subagent_id": id, "message": "more?"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("send_message failed");
        assert_eq!(resumed, "P2", "resume must keep primary binding, not aux");
        let sessions = earth.session_bus.subagent_sessions.lock().unwrap();
        assert_eq!(sessions.get(&id).unwrap().model, SubagentModel::Primary);
    }

    #[tokio::test]
    async fn send_message_unknown_id() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(tmp.path(), vec![], None, vec![]);
        let sm = bound_send_message(&earth);
        let res = sm
            .execute(
                serde_json::json!({"subagent_id": "nonexistent", "message": "x"}),
                &parent_ctx(&earth),
            )
            .await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Unknown subagent_id"));
    }

    /// 并行派发:tasks[] 并行运行,结果按声明序聚合,会话各自持久化。
    #[tokio::test]
    async fn parallel_tasks_aggregate_results() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec!["R-ONE".into(), "R-TWO".into()],
            None,
            vec![],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({
                    "tasks": [
                        {"subagent_type": "Explore", "prompt": "task one"},
                        {"subagent_type": "Explore", "prompt": "task two"}
                    ]
                }),
                &parent_ctx(&earth),
            )
            .await
            .expect("parallel delegate failed");
        assert!(out.contains("## [1]"), "section 1: {out}");
        assert!(out.contains("## [2]"), "section 2: {out}");
        assert!(out.contains("R-ONE"), "first result: {out}");
        assert!(out.contains("R-TWO"), "second result: {out}");
        assert_eq!(
            earth.session_bus.subagent_sessions.lock().unwrap().len(),
            2,
            "both sessions persisted"
        );
    }

    /// 后台统一:run_in_background 注册 BackgroundTaskStore(前缀 a),
    /// 完成后产出落盘、状态终态、通知待注入、会话可续聊。
    #[tokio::test]
    async fn background_delegate_registers_agent_task() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(tmp.path(), vec!["BG-REPORT".into()], None, vec![]);
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({
                    "subagent_type": "Explore",
                    "prompt": "background research",
                    "run_in_background": true
                }),
                &parent_ctx(&earth),
            )
            .await
            .expect("background delegate failed");
        let task_id = out
            .split("task_id=")
            .nth(1)
            .and_then(|s| s.split([',', '\n', ' ']).next())
            .expect("task_id in output")
            .to_string();
        assert!(task_id.starts_with('a'), "agent task prefix: {task_id}");

        // 等待后台完成(最多 10s)。
        let mut terminal = None;
        for _ in 0..200 {
            if let Some(t) = earth.background_tasks.get(&task_id)
                && t.status.is_terminal()
            {
                terminal = Some(t);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let task = terminal.expect("background task must reach terminal status");
        assert!(matches!(task.status, TaskStatus::Completed), "status: {:?}", task.status);
        let content = std::fs::read_to_string(&task.output_file).expect("output file written");
        assert!(content.contains("BG-REPORT"), "output file: {content}");
        // 完成通知待主循环注入(notified=false 的终态任务)。
        assert!(
            earth
                .background_tasks
                .unnotified_terminal_tasks()
                .iter()
                .any(|t| t.id == task_id),
            "completion notification pending"
        );
        assert_eq!(earth.session_bus.subagent_sessions.lock().unwrap().len(), 1);
    }

    // ── 移植的回归:P0-4 取消 / 首轮 user 消息 ──────────────────

    use crate::error::ProviderError;
    use crate::palaces::zhong_core::{LlmProvider, StreamChunk};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Provider that always answers with a read_file tool_call (so the
    /// sub-agent loop would otherwise run to max_turns), and cancels the
    /// given token on the Nth invocation.
    struct CancellingProvider {
        calls: Arc<AtomicUsize>,
        cancel_on_call: usize,
        token: tokio_util::sync::CancellationToken,
    }

    impl LlmProvider for CancellingProvider {
        fn infer_stream(
            &self,
            _messages: Vec<Message>,
            _tools: Option<&[crate::stems::action::ToolSchema]>,
            _cancel_token: Option<tokio_util::sync::CancellationToken>,
        ) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, ProviderError>> + Send>>
        {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.cancel_on_call {
                self.token.cancel();
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                let text = r#"<tool_call>
{"name": "read_file", "parameters": {"path": "/nonexistent-p0-4"}}
</tool_call>"#;
                let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
            });
            Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
        }
    }

    /// P0-4 · 子代理运行中取消 → 提前退出(轮数远小于 50),返回错误。
    #[tokio::test]
    async fn delegate_cancel_stops_subagent_early() {
        let tmp = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let provider: Box<dyn LlmProvider> = Box::new(CancellingProvider {
            calls: calls.clone(),
            cancel_on_call: 2,
            token: token.clone(),
        });
        let router = crate::palaces::zhong_core::ProviderRouter::new(vec![(0u32, provider)]);
        let core = Arc::new(JiaCore::with_router(
            router,
            "mock".into(),
            "mock".into(),
            8192,
        ));
        let earth = mock_earth_with_core(tmp.path(), core, None, vec![]);
        let tool = bound_delegate(&earth);
        let mut ctx = parent_ctx(&earth);
        ctx.cancel_token = token;

        let res = tool
            .execute(
                serde_json::json!({
                    "subagent_type": "Explore",
                    "prompt": "loop forever",
                    "max_turns": 50
                }),
                &ctx,
            )
            .await;

        let n = calls.load(Ordering::SeqCst);
        assert!(
            n < 50,
            "cancelled sub-agent must exit early, not run to max_turns (calls={n})"
        );
        assert!(res.is_err(), "cancelled sub-agent must return an error");
    }

    /// Provider that captures the messages of its first invocation.
    struct CapturingProvider {
        seen: Arc<Mutex<Option<Vec<Message>>>>,
    }

    impl LlmProvider for CapturingProvider {
        fn infer_stream(
            &self,
            messages: Vec<Message>,
            _tools: Option<&[crate::stems::action::ToolSchema]>,
            _cancel_token: Option<tokio_util::sync::CancellationToken>,
        ) -> std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, ProviderError>> + Send>>
        {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if seen.is_none() {
                *seen = Some(messages);
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                let _ = tx.send(Ok(StreamChunk::Delta("done".to_string())));
            });
            Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
        }
    }

    /// 回归:delegate 首轮 LLM 调用必须包含 user 消息——system-only 请求会被
    /// LMStudio 等 provider 的提示词模板拒绝("No user query found in messages.")。
    #[tokio::test]
    async fn delegate_first_infer_includes_user_message() {
        let tmp = tempfile::tempdir().unwrap();
        let seen = Arc::new(Mutex::new(None));
        let provider: Box<dyn LlmProvider> = Box::new(CapturingProvider { seen: seen.clone() });
        let router = crate::palaces::zhong_core::ProviderRouter::new(vec![(0u32, provider)]);
        let core = Arc::new(JiaCore::with_router(
            router,
            "mock".into(),
            "mock".into(),
            8192,
        ));
        let earth = mock_earth_with_core(tmp.path(), core, None, vec![]);
        let tool = bound_delegate(&earth);

        let res = tool
            .execute(
                serde_json::json!({
                    "subagent_type": "Explore",
                    "prompt": "find the flux capacitor",
                    "max_turns": 1
                }),
                &parent_ctx(&earth),
            )
            .await;
        assert!(res.is_ok(), "delegate failed: {res:?}");

        let msgs = seen
            .lock()
            .unwrap()
            .clone()
            .expect("provider must be called at least once");
        assert!(
            msgs.iter().any(|m| m.role == Role::System),
            "must include a system message"
        );
        let user_msg = msgs
            .iter()
            .find(|m| m.role == Role::User)
            .expect("first infer call must include a user message");
        assert!(
            user_msg.content.contains("flux capacitor"),
            "user message should carry the task prompt"
        );
    }

    /// Coder 强制 worktree 隔离:自动 enter,产物 confined 在 worktree,
    /// 主 checkout 不动;worktree 保留在盘上并在报告中给出路径。
    #[tokio::test]
    async fn coder_enters_worktree_isolation() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        // 初始化 git 仓库(git 不可用则跳过)。
        let ok = std::process::Command::new("git")
            .arg("init")
            .current_dir(&repo)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("git unavailable, skipping coder worktree test");
            return;
        }
        for args in [
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            let _ = std::process::Command::new("git").args(&args).current_dir(&repo).output();
        }
        std::fs::write(repo.join("README"), "init").unwrap();
        let _ = std::process::Command::new("git").args(["add", "."]).current_dir(&repo).output();
        let committed = std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&repo)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !committed {
            eprintln!("git commit failed, skipping coder worktree test");
            return;
        }

        let earth = mock_earth(&repo, vec!["coding done".into()], None, vec![]);
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Coder", "prompt": "implement feature"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("coder delegate failed");
        assert!(out.contains("Worktree:"), "worktree reported: {out}");
        assert!(out.contains(".jia/worktrees/coder-"), "worktree path: {out}");
        assert!(out.contains("coding done"), "report: {out}");

        let id = extract_id(&out);
        let sessions = earth.session_bus.subagent_sessions.lock().unwrap();
        let session = sessions.get(id).expect("session stored");
        assert_eq!(session.subagent_type, SubagentType::Coder);
        let wt = session.worktree_path.clone().expect("worktree binding recorded");
        drop(sessions);
        assert!(wt.is_dir(), "worktree left on disk: {}", wt.display());
        assert!(wt.join("README").exists(), "worktree carries the checkout");

        // 主 checkout 未被触碰:除 worktree 目录(.jia/)与测试脚手架
        // (store.db 等)外无任何改动/新文件,且无任何已跟踪文件变更。
        let status = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&repo)
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&status.stdout);
        for line in status.lines() {
            assert!(
                line.starts_with("?? .jia/") || line.starts_with("?? store.db"),
                "main checkout must be untouched, got: {line}"
            );
        }
    }

    // ── #15 · Verifier 子代理 ─────────────────────────────────

    #[test]
    fn subagent_type_verifier_from_str() {
        assert!(matches!(
            SubagentType::from_str("Verifier"),
            Ok(SubagentType::Verifier)
        ));
        assert!(matches!(
            SubagentType::from_str("verifier"),
            Ok(SubagentType::Verifier)
        ));
        assert_eq!(SubagentType::Verifier.as_str(), "Verifier");
    }

    #[test]
    fn plan_gate_covers_verifier_and_requests_verifier_detects() {
        // 谋划态拦截(公理 4 只收紧):Verifier 可执行 shell,与 Coder 同被拦。
        assert!(requests_coder(&serde_json::json!({"subagent_type": "Verifier"})));
        assert!(requests_coder(&serde_json::json!({
            "tasks": [{"subagent_type": "Explore", "prompt": "a"},
                      {"subagent_type": "verifier", "prompt": "b"}]
        })));
        assert!(!requests_coder(&serde_json::json!({"subagent_type": "Plan"})));
        // 复核委派识别(天盘验证信号用)。
        assert!(requests_verifier(&serde_json::json!({"subagent_type": "verifier"})));
        assert!(requests_verifier(&serde_json::json!({
            "tasks": [{"subagent_type": "Coder", "prompt": "a"},
                      {"subagent_type": "Verifier", "prompt": "b"}]
        })));
        assert!(!requests_verifier(&serde_json::json!({"subagent_type": "Coder"})));
    }

    /// Verifier 门禁(只读约束):注册表结构性缺席写工具——write_file 调用
    /// 被同一门禁拒绝(未知工具即拒),shell 验证命令可用,最终照常出报告。
    #[tokio::test]
    async fn verifier_readonly_gate_denies_write_allows_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec![
                r#"<tool_call>
{"name": "write_file", "parameters": {"path": "x.txt", "content": "y"}}
</tool_call>"#
                    .into(),
                r#"<tool_call>
{"name": "shell", "parameters": {"command": "echo verified-output"}}
</tool_call>"#
                    .into(),
                "Verdict: PASS — write denied, verification command ran".into(),
            ],
            None,
            vec![],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Verifier", "prompt": "verify the claims"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("verifier delegate failed");
        assert!(out.contains("Verdict: PASS"), "report: {out}");

        let id = extract_id(&out);
        let entries = tool_entries(&earth, id);
        // 写调用被拒(注册表结构性只读——同一 gate_one_tool 门禁)。
        let (_, _, write_err) = entries
            .iter()
            .find(|(t, _, _)| t == "write_file")
            .expect("write_file call recorded");
        assert!(write_err.is_some(), "write must be denied for Verifier");
        // shell 验证命令放行并真实执行。
        let (_, shell_out, shell_err) = entries
            .iter()
            .find(|(t, _, _)| t == "shell")
            .expect("shell call recorded");
        assert!(shell_err.is_none(), "shell must run: {shell_err:?}");
        assert!(shell_out.contains("verified-output"), "{shell_out}");
        // 文件不得被写出。
        assert!(!tmp.path().join("workspace").join("x.txt").exists());
        assert!(!tmp.path().join("x.txt").exists());
        // 会话绑定类型正确(续聊恢复 Verifier 绑定)。
        let sessions = earth.session_bus.subagent_sessions.lock().unwrap();
        assert_eq!(sessions.get(id).unwrap().subagent_type, SubagentType::Verifier);
    }

    // ── P2 · Verdict 协议化(loop 消费路径)─────────────────────

    /// delegate 输出中的结构化协议行严格解析:PASS/FAIL/UNPARSEABLE/缺失。
    #[test]
    fn delegate_output_verdict_strict_marker_parse() {
        assert_eq!(
            delegate_output_verdict("Sub-agent x (Verifier) completed.\nVerifier verdict: PASS\n\nbody"),
            Some(Verdict::Pass)
        );
        assert_eq!(
            delegate_output_verdict("header\nVerifier verdict: FAIL\nbody"),
            Some(Verdict::Fail)
        );
        // 行首空白可容忍(trim),大小写/变形不可。
        assert_eq!(
            delegate_output_verdict("  Verifier verdict: FAIL  "),
            Some(Verdict::Fail)
        );
        assert_eq!(delegate_output_verdict("verifier verdict: fail"), None);
        assert_eq!(delegate_output_verdict("Verifier verdict: FAILED"), None);
        // UNPARSEABLE 与缺失协议行同为 None(调用方回落启发式)。
        assert_eq!(delegate_output_verdict("Verifier verdict: UNPARSEABLE\nVerifier verdict: FAIL"), None);
        assert_eq!(delegate_output_verdict("no marker here"), None);
        // 子代理原始 verdict 行不是协议行(前缀不同),不被误读。
        assert_eq!(delegate_output_verdict("Verdict: FAIL"), None);
    }

    /// loop 消费路径:结构化优先;None(旧输出/UNPARSEABLE)回落启发式。
    #[test]
    fn verifier_failed_structured_first_heuristic_fallback() {
        // 结构化 FAIL → true;PASS → false(即使正文他处出现 Verdict: FAIL,
        // 中间行出现的旧启发式命中被结构化判定否决 —— 协议化只收紧)。
        assert!(verifier_failed("Verifier verdict: FAIL\n\nclaims do not hold"));
        assert!(!verifier_failed(
            "Verifier verdict: PASS\n\nearlier draft said Verdict: FAIL"
        ));
        // UNPARSEABLE → 回落:正文含 "Verdict: FAIL" 仍记异常(保持现启发式)。
        assert!(verifier_failed(
            "Verifier verdict: UNPARSEABLE\n\nreport body\nVerdict: FAIL"
        ));
        assert!(!verifier_failed(
            "Verifier verdict: UNPARSEABLE\n\nreport body without verdict"
        ));
        // 无协议行(旧输出)→ 完全保持旧启发式。
        assert!(verifier_failed("some report\nVerdict: FAIL"));
        assert!(!verifier_failed("some report\nVerdict: PASS"));
    }

    /// Verifier 遵循协议(最后一行精确 verdict)→ 输出含结构化协议行。
    #[tokio::test]
    async fn verifier_report_carries_structured_verdict_line() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec!["re-ran cargo test: 12 passed\n\nVerdict: PASS".into()],
            None,
            vec![],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Verifier", "prompt": "verify"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("verifier delegate failed");
        assert!(out.contains("Verifier verdict: PASS"), "marker: {out}");
        assert_eq!(delegate_output_verdict(&out), Some(Verdict::Pass));
        assert!(!verifier_failed(&out));
    }

    /// Verifier 未遵循协议(verdict 不在最后一行/缺 verdict)→ UNPARSEABLE
    /// 标注,loop 消费回落启发式。
    #[tokio::test]
    async fn verifier_unparseable_verdict_annotated_and_falls_back() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            // verdict 出现在中间行,最后一行是普通文字 → 严格解析 None。
            vec!["Verdict: FAIL\n\nbut see the caveats above".into()],
            None,
            vec![],
        );
        let tool = bound_delegate(&earth);
        let out = tool
            .execute(
                serde_json::json!({"subagent_type": "Verifier", "prompt": "verify"}),
                &parent_ctx(&earth),
            )
            .await
            .expect("verifier delegate failed");
        assert!(out.contains("Verifier verdict: UNPARSEABLE"), "marker: {out}");
        assert_eq!(delegate_output_verdict(&out), None);
        // 回落启发式:正文含 "Verdict: FAIL" → 仍记异常(只收紧不放松)。
        assert!(verifier_failed(&out));
    }

    // ── P2 · 子代理生命周期事件透出 ──────────────────────────

    /// execute_with_tx:started/completed 生命周期事件 + 工具调用计数进度
    /// 经父级事件通道透出;execute(无 tx)静默不报错。
    #[tokio::test]
    async fn delegate_emits_subagent_lifecycle_events() {
        let tmp = tempfile::tempdir().unwrap();
        let earth = mock_earth(
            tmp.path(),
            vec![
                r#"<tool_call>
{"name": "read_file", "parameters": {"path": "a.txt"}}
</tool_call>"#
                    .into(),
                "analysis: found X".into(),
            ],
            None,
            vec![],
        );
        std::fs::write(tmp.path().join("workspace/a.txt"), "hello").ok();
        let tool = bound_delegate(&earth);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let out = tool
            .execute_with_tx(
                serde_json::json!({"subagent_type": "Explore", "prompt": "find X"}),
                &tx,
                &parent_ctx(&earth),
            )
            .await
            .expect("delegate failed");
        assert!(out.contains("analysis: found X"), "report: {out}");
        drop(tx);
        let mut statuses = Vec::new();
        while let Ok(AgentEvent::SubagentLifecycle { status, kind, .. }) = rx.try_recv() {
            assert_eq!(kind, "Explore");
            statuses.push(status);
        }
        assert_eq!(statuses.first().map(String::as_str), Some("started"));
        assert_eq!(statuses.last().map(String::as_str), Some("completed"));
        // 一次工具调用 → 一条计数进度(不含详情)。
        assert!(
            statuses.iter().filter(|s| s.as_str() == "progress").count() >= 1,
            "progress events: {statuses:?}"
        );
    }
}

//! di_earth — Earth Plate (地盘)

use std::path::PathBuf;
use std::sync::Arc;

use crate::palaces::gen_store::{Store, async_store::StoreAsync};
use crate::palaces::kan_io::ChannelManager;
use crate::palaces::kun_config::{AppConfig, ConfigLoader, default_workspace_dir};
use crate::palaces::li_skill::SkillRegistry;
use crate::palaces::li_skill::loader::SkillLoader;
use crate::palaces::li_skill::spawn_skill_watcher;
use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::builtin::browser::browser_click::BrowserClickTool;
use crate::palaces::zhen_tool::builtin::browser::browser_console::BrowserConsoleTool;
use crate::palaces::zhen_tool::builtin::browser::browser_dialog::BrowserDialogTool;
use crate::palaces::zhen_tool::builtin::browser::browser_navigate::BrowserNavigateTool;
use crate::palaces::zhen_tool::builtin::browser::browser_press::BrowserPressKeyTool;
use crate::palaces::zhen_tool::builtin::browser::browser_screenshot::BrowserScreenshotTool;
use crate::palaces::zhen_tool::builtin::browser::browser_scroll::BrowserScrollTool;
use crate::palaces::zhen_tool::builtin::browser::browser_snapshot::BrowserSnapshotTool;
use crate::palaces::zhen_tool::builtin::browser::browser_type::BrowserTypeTool;
use crate::palaces::zhen_tool::builtin::browser::web_execute_js::WebExecuteJsTool;
use crate::palaces::zhen_tool::builtin::browser::web_fetch::WebFetchTool;
use crate::palaces::zhen_tool::builtin::computer::computer_use::ComputerUseTool;
#[cfg(feature = "agent-tool")]
use crate::palaces::zhen_tool::builtin::delegate::SendMessageTool;
use crate::palaces::zhen_tool::builtin::exec::lsp::LspTool;
use crate::palaces::zhen_tool::builtin::exec::shell::ShellTool;
use crate::palaces::zhen_tool::builtin::exec::task::{TaskStore, TaskTool};
use crate::palaces::zhen_tool::builtin::exec::worktree::{EnterWorktreeTool, ExitWorktreeTool};
use crate::palaces::zhen_tool::builtin::fs::glob::GlobTool;
use crate::palaces::zhen_tool::builtin::fs::grep::GrepTool;
use crate::palaces::zhen_tool::builtin::fs::patch_file::EditTool;
use crate::palaces::zhen_tool::builtin::fs::read_file::ReadFileTool;
use crate::palaces::zhen_tool::builtin::fs::scratchpad::{ScratchpadReadTool, ScratchpadWriteTool};
use crate::palaces::zhen_tool::builtin::fs::write_file::WriteFileTool;
use crate::palaces::zhen_tool::builtin::plan_mode::{EnterPlanModeTool, ExitPlanModeTool};
use crate::palaces::zhen_tool::builtin::skill::SkillTool;
use crate::palaces::zhong_core::{JiaCore, LlmProvider};
use crate::stems::action::ExecContext;

use crate::palaces::zhen_tool::ToolRegistry;
use crate::palaces::zhen_tool::builtin::ask_user::AskUserQuestionTool;
#[cfg(feature = "web-search")]
use crate::palaces::zhen_tool::builtin::browser::web_search::WebSearchTool;
use crate::palaces::zhen_tool::builtin::cron::CronStore;
#[cfg(feature = "cron")]
use crate::palaces::zhen_tool::builtin::cron::CronTool;
use crate::palaces::zhen_tool::builtin::cron_runner;
#[cfg(feature = "agent-tool")]
use crate::palaces::zhen_tool::builtin::delegate::{DelegateTool, SubagentSession, SubagentType};
#[cfg(feature = "git")]
use crate::palaces::zhen_tool::builtin::exec::git::GitTool;
use crate::palaces::zhen_tool::builtin::namarupa::NamaRupaTool;
#[cfg(feature = "mcp")]
use crate::palaces::zhen_tool::mcp::McpManager;
#[cfg(feature = "wasm-plugin")]
use crate::palaces::zhen_tool::plugin_manager::PluginManager;
use crate::plates::ren_human::SessionBus;
use crate::plates::shen_spirit::SpiritPlate;
use crate::plates::shen_spirit::baihu::{BaiHuConfig, BaiHuHook};
use crate::plates::shen_spirit::completion_check::{CompletionCheckHook, CompletionChecklist};
use crate::plates::shen_spirit::jiudi::JiudiHook;
use crate::plates::shen_spirit::jiutian::JiuTianHook;
use crate::plates::shen_spirit::liuhe::LiuheHook;
use crate::plates::shen_spirit::taiyin::TaiYinHook;
use crate::plates::shen_spirit::tengshe::TengsheHook;
use crate::plates::shen_spirit::xuanwu::XuanWuHook;
use crate::plates::shen_spirit::zhifu::ZhifuHook;

/// 地盘 (Earth Plate) — Static infrastructure assembled once at startup.
///
/// All infrastructure is behind `Arc<T>` for shared access across the runtime.
/// The plate is immutable after assembly — 一局不变 (unchanging for one session).
pub struct EarthPlate {
    pub io: Arc<ChannelManager>,                       // 坎一
    pub config: Arc<ConfigLoader>,                     // 坤二
    pub tools: Arc<ToolRegistry>,                      // 震三
    pub main_core: Arc<JiaCore>,                       // 中五 (主模型)
    pub aux_core: Option<Arc<JiaCore>>, // 辅模型: 用于 consolidation/distillation/reflection
    pub permissions: Arc<PermissionMatrix>, // 乾六
    pub skills: Arc<std::sync::RwLock<SkillRegistry>>, // 离九
    pub cron: Arc<CronStore>,           // (cron runner)
    pub task_store: Arc<TaskStore>,     // 任务管理
    pub store: Arc<Store>,              // 艮八
    pub store_async: StoreAsync,        // 艮八 · async facade
    pub spirit: Arc<SpiritPlate>,       // 神盘
    /// CompletionChecklist — shared between hook (observation) and Agent (assessment).
    pub completion_checklist: Arc<CompletionChecklist>,
    /// P4 · compiled user-configurable hooks (人盘门规 / 神盘观测). Empty by
    /// default; regexes pre-compiled at assemble to avoid hot-path cost (O4).
    /// 类型居天干层(stems::CompiledHook)——地盘持有、天盘消费的共享语义。
    pub user_hooks: Arc<Vec<crate::stems::CompiledHook>>,
    /// P2-1 · 会话总线 — 可变会话状态五簇(pending 确认/提问、会话模式、
    /// 会话锁、子代理会话)归人盘:人盘 = 人机交互边界。
    pub session_bus: Arc<SessionBus>,
    /// Root data directory (`~/.jia/`).
    pub data_dir: PathBuf,
    /// Path to the PID file for gateway process management.
    pub pid_path: PathBuf,
    /// Directory for file backups (write_file / edit tools).
    pub backup_dir: PathBuf,
}

impl EarthPlate {
    /// 起局 (qi ju) — Assemble the Earth plate from configuration.
    ///
    /// This is called once at startup. The returned `Arc<EarthPlate>`
    /// is shared throughout the runtime lifetime.
    pub fn assemble(config: AppConfig) -> Arc<Self> {
        let data_dir = crate::palaces::kun_config::default_data_dir();
        let db_path = data_dir.join("store.db");
        let cron_dir = data_dir.join("cron");
        let pid_path = data_dir.join("gateway.pid");
        let backup_dir = data_dir.join("backups");

        let config_loader = Arc::new(ConfigLoader::from_app_config(config));

        // Ensure workspace dir for cron/bot agents exists
        std::fs::create_dir_all(default_workspace_dir())
            .unwrap_or_else(|e| tracing::warn!("cannot create workspace dir: {e}"));
        std::fs::create_dir_all(&backup_dir)
            .unwrap_or_else(|e| tracing::warn!("cannot create backup dir: {e}"));

        let default_profile = config_loader
            .app_config
            .default_main_provider()
            .expect("no default provider configured");
        let default_model = default_profile.default_main_model().to_string();
        let default_kind = default_profile.kind.clone();
        let context_window = default_profile.context_window.unwrap_or(128000);

        // Build ProviderRouter from all configured providers for failover.
        // Providers without priority default to lowest — they are tried last.
        // Sort ascending: lower priority = higher precedence.
        let mut router_providers: Vec<(u32, Box<dyn LlmProvider>)> = config_loader
            .app_config
            .providers
            .iter()
            .map(|(_name, profile)| {
                let model = profile.default_main_model().to_string();
                let p = crate::palaces::zhong_core::create_provider(profile, &model);
                let pri = profile.priority.unwrap_or(u32::MAX);
                (pri, p)
            })
            .collect();
        router_providers.sort_by_key(|(pri, _)| *pri);

        let router = crate::palaces::zhong_core::ProviderRouter::new(router_providers);
        let main_core = Arc::new(JiaCore::with_router(
            router,
            default_kind,
            default_model,
            context_window,
        ));

        let aux_core = config_loader
            .app_config
            .default_aux_model_provider
            .as_ref()
            .and_then(
                |aux_name| match config_loader.app_config.provider(aux_name) {
                    Ok(aux_profile) => {
                        let aux_model = aux_profile
                            .default_aux_model
                            .as_deref()
                            .unwrap_or_else(|| aux_profile.default_main_model())
                            .to_string();
                        Some(Arc::new(JiaCore::new(&aux_profile, &aux_model)))
                    }
                    Err(e) => {
                        tracing::warn!("aux_provider '{}' not found: {e}", aux_name);
                        None
                    }
                },
            );

        // Build PermissionMatrix from security config
        let permissions = Arc::new(
            PermissionMatrix::from_config(
                &config_loader.app_config.security,
                &default_workspace_dir(),
                backup_dir.clone(),
            )
            .with_sandbox(&config_loader.app_config.security.sandbox),
        );

        // Read-only subtools for sub-agents (Explore/Plan)
        let mut subtool_registry = ToolRegistry::new();
        subtool_registry.register(Arc::new(ReadFileTool::new()));
        subtool_registry.register(Arc::new(GrepTool::new()));
        subtool_registry.register(Arc::new(GlobTool::new()));
        subtool_registry.register(Arc::new(WebFetchTool::new()));
        subtool_registry.register(Arc::new(WebExecuteJsTool::new()));
        subtool_registry.register(Arc::new(BrowserNavigateTool::new()));
        subtool_registry.register(Arc::new(BrowserSnapshotTool::new()));
        subtool_registry.register(Arc::new(BrowserClickTool::new()));
        subtool_registry.register(Arc::new(BrowserTypeTool::new()));
        subtool_registry.register(Arc::new(BrowserPressKeyTool::new()));
        subtool_registry.register(Arc::new(BrowserScreenshotTool::new()));
        subtool_registry.register(Arc::new(BrowserScrollTool::new()));
        subtool_registry.register(Arc::new(BrowserConsoleTool::new()));
        subtool_registry.register(Arc::new(BrowserDialogTool::new()));
        subtool_registry.register(Arc::new(ComputerUseTool::new()));
        #[cfg(feature = "web-search")]
        subtool_registry.register(Arc::new(WebSearchTool::new()));
        let _subtools = Arc::new(subtool_registry);

        // P2-1 · 会话总线(人盘)——五簇可变会话状态,create early:
        // DelegateTool/SendMessageTool/AskUserQuestionTool below hold clones.
        let session_bus = Arc::new(SessionBus::new());

        let mut tool_registry = ToolRegistry::new();
        tool_registry.register(Arc::new(ReadFileTool::new()));
        tool_registry.register(Arc::new(WriteFileTool::new()));
        tool_registry.register(Arc::new(ShellTool::new()));
        tool_registry.register(Arc::new(GrepTool::new()));
        tool_registry.register(Arc::new(GlobTool::new()));
        tool_registry.register(Arc::new(EditTool::new()));
        tool_registry.register(Arc::new(LspTool::new()));
        // P3 · plan-mode control tools (read-only, non-destructive — D1)
        tool_registry.register(Arc::new(EnterPlanModeTool));
        tool_registry.register(Arc::new(ExitPlanModeTool));
        // P6 · worktree isolation tools
        tool_registry.register(Arc::new(EnterWorktreeTool::new()));
        tool_registry.register(Arc::new(ExitWorktreeTool));
        tool_registry.register(Arc::new(WebFetchTool::new()));
        tool_registry.register(Arc::new(WebExecuteJsTool::new()));
        tool_registry.register(Arc::new(BrowserNavigateTool::new()));
        tool_registry.register(Arc::new(BrowserSnapshotTool::new()));
        tool_registry.register(Arc::new(BrowserClickTool::new()));
        tool_registry.register(Arc::new(BrowserTypeTool::new()));
        tool_registry.register(Arc::new(BrowserPressKeyTool::new()));
        tool_registry.register(Arc::new(BrowserScreenshotTool::new()));
        tool_registry.register(Arc::new(BrowserScrollTool::new()));
        tool_registry.register(Arc::new(BrowserConsoleTool::new()));
        tool_registry.register(Arc::new(BrowserDialogTool::new()));
        tool_registry.register(Arc::new(ComputerUseTool::new()));
        #[cfg(feature = "web-search")]
        tool_registry.register(Arc::new(WebSearchTool::new()));
        // P8 · send_message (continue a sub-agent) + cross-worker scratchpad
        #[cfg(feature = "agent-tool")]
        tool_registry.register(Arc::new(SendMessageTool::new(
            main_core.clone(),
            _subtools.clone(),
            session_bus.subagent_sessions.clone(),
        )));
        tool_registry.register(Arc::new(ScratchpadWriteTool::new()));
        tool_registry.register(Arc::new(ScratchpadReadTool::new()));

        // Cron store shared between tool, runner, and REST API
        let cron_store = CronStore::new(cron_dir);
        #[cfg(feature = "cron")]
        tool_registry.register(Arc::new(CronTool::new(cron_store.clone())));
        #[cfg(feature = "git")]
        tool_registry.register(Arc::new(GitTool::new()));

        // Task store — CRUD task tracking, shared for potential REST API access
        let task_store = TaskStore::new();
        tool_registry.register(Arc::new(TaskTool::new(task_store.clone())));

        // Pending questions — shared between AskUserQuestionTool and REST /answer endpoint
        tool_registry.register(Arc::new(AskUserQuestionTool::new(
            session_bus.pending_questions.clone(),
        )));

        // Connect MCP servers and register their tools
        #[cfg(feature = "mcp")]
        if !config_loader.app_config.mcp_servers.is_empty() {
            McpManager::connect_all(&config_loader.app_config.mcp_servers, &mut tool_registry);
        }

        // I/O channel — keep receiver for the IO consumer task
        let (io, io_rx) = ChannelManager::new();
        let io = Arc::new(io);

        // Load skills from skills/ directory
        let mut skill_registry = SkillRegistry::new();
        // Resolve skills/ directory. The kernel crate's CARGO_MANIFEST_DIR
        // points to kernel/, so we try the parent directory first, then fall
        // back to CWD-relative for development convenience.
        let skills_dir = {
            let manifest_parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(|p| p.join("skills"))
                .unwrap_or_default();
            if manifest_parent.is_dir() {
                manifest_parent
            } else {
                std::path::PathBuf::from("skills")
            }
        };
        if skills_dir.is_dir() {
            match SkillLoader::load_directory_sync(&skills_dir, &mut skill_registry) {
                Ok(n) => tracing::info!("Loaded {n} skills from skills/"),
                Err(e) => tracing::warn!("Failed to load skills: {e}"),
            }
        } else {
            tracing::warn!("Skills directory not found: {}", skills_dir.display());
        }
        let skills = Arc::new(std::sync::RwLock::new(skill_registry));

        if skills_dir.is_dir() {
            spawn_skill_watcher(skills.clone(), skills_dir.clone());
        }

        // Register SkillTool — bridges ToolRegistry ↔ SkillRegistry
        tool_registry.register(Arc::new(SkillTool::new(skills.clone())));

        // Open Store before tool registration so tools can receive it
        let store = Arc::new(Store::open(&db_path.to_string_lossy()));
        let store_async = StoreAsync::new(store.clone());

        // P8 · crash recovery: hydrate subagent sessions from SQLite
        if let Ok(rows) = store.load_all_subagent_sessions() {
            if !rows.is_empty() {
                let mut guard = session_bus
                    .subagent_sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                for (id, messages_json, subagent_type, created_at, last_used) in rows {
                    if let Ok(messages) =
                        serde_json::from_str::<Vec<crate::types::Message>>(&messages_json)
                    {
                        let st =
                            SubagentType::from_str(&subagent_type).unwrap_or(SubagentType::Explore);
                        guard.insert(
                            id,
                            SubagentSession {
                                messages,
                                subagent_type: st,
                                created_at,
                                last_used,
                            },
                        );
                    }
                }
                tracing::info!(
                    count = guard.len(),
                    "restored subagent sessions from crash recovery"
                );
            }
        }

        // Register NamaRupaTool — agentic graph memory (nāma-rūpa)
        tool_registry.register(Arc::new(NamaRupaTool::new(store.clone())));

        // P1 · Register DelegateTool with Store for sub-agent session persistence
        #[cfg(feature = "agent-tool")]
        tool_registry.register(Arc::new(DelegateTool::new(
            main_core.clone(),
            _subtools.clone(),
            store.clone(),
            session_bus.subagent_sessions.clone(),
        )));

        // Load WASM plugins from plugins/ directory
        #[cfg(feature = "wasm-plugin")]
        if let Ok(pm) = PluginManager::new() {
            let plugins_dir = data_dir.join("plugins");
            match pm.load_from_dir(&plugins_dir, &mut tool_registry) {
                Ok(n) if n > 0 => tracing::info!("Loaded {n} WASM plugin tool(s)"),
                Err(e) => tracing::warn!("WASM plugin loading error: {e}"),
                _ => {}
            }
        }

        let mut tools = Arc::new(tool_registry);
        // P9 · register ToolSearchTool. It searches this same registry, so it
        // holds a Weak<ToolRegistry>; Arc::get_mut works here while `tools` is
        // the sole strong owner (before any clones).
        {
            let weak = std::sync::Arc::downgrade(&tools);
            if let Some(reg) = std::sync::Arc::get_mut(&mut tools) {
                reg.register(Arc::new(
                    crate::palaces::zhen_tool::builtin::toolsearch::ToolSearchTool::new(weak),
                ));
            }
        }

        // P4 · compile user-configurable hooks (regex pre-compiled once).
        let user_hooks: Vec<crate::stems::CompiledHook> = config_loader
            .app_config
            .hooks
            .iter()
            .filter_map(|cfg| match crate::stems::CompiledHook::compile(cfg) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!(hook = %cfg.command, error = %e, "skipping invalid hook");
                    None
                }
            })
            .collect();
        if !user_hooks.is_empty() {
            tracing::info!(count = user_hooks.len(), "compiled user hooks");
        }
        let user_hooks = Arc::new(user_hooks);

        let mut spirit = SpiritPlate::new();
        let event_bus = spirit.event_bus.clone();
        // 八神 — eight spirit hooks (one file per spirit, pinyin naming)
        spirit.hook_registry.register(Box::new(ZhifuHook));
        spirit.hook_registry.register(Box::new(TengsheHook));
        spirit
            .hook_registry
            .register(Box::new(TaiYinHook::new(event_bus.clone())));
        spirit.hook_registry.register(Box::new(LiuheHook));
        spirit.hook_registry.register(Box::new(BaiHuHook::new(
            BaiHuConfig::default(),
            event_bus.clone(),
        )));
        spirit
            .hook_registry
            .register(Box::new(XuanWuHook::new(event_bus.clone())));
        spirit.hook_registry.register(Box::new(JiudiHook));
        spirit
            .hook_registry
            .register(Box::new(JiuTianHook::new(event_bus.clone(), false)));
        // CompletionChecklist — shared between hook and Agent
        let completion_checklist = Arc::new(CompletionChecklist::new());
        spirit
            .hook_registry
            .register(Box::new(CompletionCheckHook::new(
                completion_checklist.clone(),
            )));

        let earth = Arc::new(Self {
            io,
            config: config_loader,
            tools,
            main_core,
            aux_core,
            permissions,
            skills,
            cron: cron_store.clone(),
            task_store: task_store.clone(),
            store_async,
            store,
            spirit: Arc::new(spirit),
            completion_checklist,
            user_hooks,
            session_bus,
            data_dir,
            pid_path,
            backup_dir,
        });

        // ── P2-2 · 点火运行时编排(天盘)──
        // 起局装配期的单向点火(组装根语义):cron 触发的会话编排上天盘,
        // 此处仅注入触发闭包(C13 解:cron_runner 不再持 EarthPlate);
        // IO 接收端移交天盘消费循环。运行期地盘不反向回调天盘。
        let earth_cron = earth.clone();
        cron_runner::spawn_cron_runner(
            cron_store.clone(),
            Arc::new(move |job_name: String, prompt: String| {
                crate::plates::tian_heaven::spawn::spawn_cron_agent(
                    earth_cron.clone(),
                    job_name,
                    prompt,
                );
            }),
        );
        crate::plates::tian_heaven::spawn::spawn_io_consumer(earth.clone(), io_rx);

        earth
    }

    /// P6 · rebuild a tool registry scoped to a worktree root.
    ///
    /// Build an ExecContext scoped to `root` (worktree or project workspace).
    /// Tools are stateless singletons on `self.tools` (六仪不动); only the
    /// ExecContext is replaced — O(1) instead of O(n) tool rebuild.
    /// `session_id`/`cancel_token` 归属该次 run：断连清扫按 session_id 匹配,
    /// 长等待工具(ask_user/delegate/确认)经 cancel_token 响应取消。
    pub fn build_worktree_exec_ctx(
        &self,
        root: &std::path::Path,
        session_id: &str,
        cancel_token: tokio_util::sync::CancellationToken,
    ) -> ExecContext {
        let mut sec = self.config.app_config.security.clone();
        sec.workspace_root = Some(root.to_string_lossy().to_string());
        // Per-project backup dir: <workspace_root>/.jia/backups/
        let backup_dir = root.join(".jia/backups");
        let _ = std::fs::create_dir_all(&backup_dir);
        let matrix = Arc::new(
            PermissionMatrix::from_config(&sec, root, backup_dir).with_sandbox(&sec.sandbox),
        );
        ExecContext {
            permissions: matrix,
            session_id: session_id.to_string(),
            cancel_token,
        }
    }
}

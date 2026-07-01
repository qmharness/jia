use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::palaces::gen_store::Store;
use crate::palaces::kan_io::ChannelManager;
use crate::palaces::kun_config::{AppConfig, ConfigLoader};
use crate::palaces::li_skill::SkillRegistry;
use crate::palaces::li_skill::loader::SkillLoader;
use crate::palaces::li_skill::spawn_skill_watcher;
use crate::palaces::qian_permission::PermissionMatrix;
use crate::palaces::zhen_tool::builtin::browser_click::BrowserClickTool;
use crate::palaces::zhen_tool::builtin::browser_console::BrowserConsoleTool;
use crate::palaces::zhen_tool::builtin::browser_dialog::BrowserDialogTool;
use crate::palaces::zhen_tool::builtin::browser_navigate::BrowserNavigateTool;
use crate::palaces::zhen_tool::builtin::browser_press::BrowserPressKeyTool;
use crate::palaces::zhen_tool::builtin::browser_screenshot::BrowserScreenshotTool;
use crate::palaces::zhen_tool::builtin::browser_scroll::BrowserScrollTool;
use crate::palaces::zhen_tool::builtin::browser_snapshot::BrowserSnapshotTool;
use crate::palaces::zhen_tool::builtin::browser_type::BrowserTypeTool;
use crate::palaces::zhen_tool::builtin::computer_use::ComputerUseTool;
#[cfg(feature = "agent-tool")]
use crate::palaces::zhen_tool::builtin::delegate::SendMessageTool;
use crate::palaces::zhen_tool::builtin::glob::GlobTool;
use crate::palaces::zhen_tool::builtin::grep::GrepTool;
use crate::palaces::zhen_tool::builtin::lsp::LspTool;
use crate::palaces::zhen_tool::builtin::patch_file::EditTool;
use crate::palaces::zhen_tool::builtin::plan_mode::{EnterPlanModeTool, ExitPlanModeTool};
use crate::palaces::zhen_tool::builtin::read_file::ReadFileTool;
use crate::palaces::zhen_tool::builtin::scratchpad::{ScratchpadReadTool, ScratchpadWriteTool};
use crate::palaces::zhen_tool::builtin::shell::ShellTool;
use crate::palaces::zhen_tool::builtin::skill::SkillTool;
use crate::palaces::zhen_tool::builtin::task::{TaskStore, TaskTool};
use crate::palaces::zhen_tool::builtin::web_execute_js::WebExecuteJsTool;
use crate::palaces::zhen_tool::builtin::web_fetch::WebFetchTool;
use crate::palaces::zhen_tool::builtin::worktree::{EnterWorktreeTool, ExitWorktreeTool};
use crate::palaces::zhen_tool::builtin::write_file::WriteFileTool;
use crate::palaces::zhong_core::JiaCore;

use crate::palaces::zhen_tool::ToolRegistry;
use crate::palaces::zhen_tool::builtin::ask_user::{AskUserQuestionTool, PendingQuestion};
use crate::palaces::zhen_tool::builtin::cron::CronStore;
#[cfg(feature = "cron")]
use crate::palaces::zhen_tool::builtin::cron::CronTool;
use crate::palaces::zhen_tool::builtin::cron_runner;
#[cfg(feature = "agent-tool")]
use crate::palaces::zhen_tool::builtin::delegate::DelegateTool;
#[cfg(feature = "git")]
use crate::palaces::zhen_tool::builtin::git::GitTool;
use crate::palaces::zhen_tool::builtin::namarupa::NamaRupaTool;
#[cfg(feature = "web-search")]
use crate::palaces::zhen_tool::builtin::web_search::WebSearchTool;
#[cfg(feature = "mcp")]
use crate::palaces::zhen_tool::mcp::McpManager;
#[cfg(feature = "wasm-plugin")]
use crate::palaces::zhen_tool::plugin_manager::PluginManager;
use crate::plates::ren_human::{HumanPlate, PendingConfirmation};
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::shen_spirit::SpiritPlate;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use crate::plates::tian_heaven::Agent;
use crate::plates::tian_heaven::r#loop::AgentEvent;
use crate::types::{HistoryEntry, Message, Role};
use crate::vijnana::manas::Manas;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// UUID v5 namespace for Jia IO sessions — deterministically maps a source key
/// (e.g. "webhook:wechat:wxid_xxx") to a session ID.  Generated once, fixed forever.
const JIA_SESSION_NS: uuid::Uuid = uuid::Uuid::from_bytes([
    0xA3, 0xE2, 0x91, 0x7C, 0x8F, 0x4D, 0x42, 0xB1, 0x9E, 0x56, 0xDC, 0x73, 0xFA, 0x10, 0x8B, 0x2F,
]);

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
    pub spirit: Arc<SpiritPlate>,       // 神盘
    /// P4 · compiled user-configurable hooks (人盘门规 / 神盘观测). Empty by
    /// default; regexes pre-compiled at assemble to avoid hot-path cost (O4).
    pub user_hooks: Arc<Vec<crate::plates::tian_heaven::r#loop::CompiledHook>>,
    pub pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    pub pending_questions: Arc<Mutex<HashMap<String, PendingQuestion>>>,
    /// P8 · persisted sub-agent sessions for continuation via send_message.
    pub subagent_sessions:
        Arc<Mutex<HashMap<String, crate::palaces::zhen_tool::builtin::delegate::SubagentSession>>>,
    /// P3 · per-session interaction mode (谋划态), set by user slash command
    /// (/plan) and read when the next agent run starts. Kept in sync with the
    /// agent's actual mode via InteractionModeChanged events.
    pub session_modes: Arc<Mutex<HashMap<String, crate::plates::tian_heaven::InteractionMode>>>,
    /// Per-session locks — serializes concurrent messages from the same source
    /// so they don't race on history read/write in post_loop.
    pub session_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
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
        std::fs::create_dir_all(&data_dir.join("workspace"))
            .unwrap_or_else(|e| tracing::warn!("cannot create workspace dir: {e}"));
        std::fs::create_dir_all(&backup_dir)
            .unwrap_or_else(|e| tracing::warn!("cannot create backup dir: {e}"));

        let default_profile = config_loader
            .app_config
            .default_main_provider()
            .expect("no default provider configured");
        let default_model = default_profile.default_main_model().to_string();
        let main_core = Arc::new(JiaCore::new(&default_profile, &default_model));
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
                &data_dir.join("workspace"),
                backup_dir.clone(),
            )
            .with_sandbox(&config_loader.app_config.security.sandbox),
        );

        // Read-only subtools for sub-agents (Explore/Plan)
        let mut subtool_registry = ToolRegistry::new();
        subtool_registry.register(Arc::new(ReadFileTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(GrepTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(GlobTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(WebFetchTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(WebExecuteJsTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserNavigateTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserSnapshotTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserClickTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserTypeTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserPressKeyTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserScreenshotTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserScrollTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserConsoleTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(BrowserDialogTool::new(permissions.clone())));
        subtool_registry.register(Arc::new(ComputerUseTool::new(permissions.clone())));
        #[cfg(feature = "web-search")]
        subtool_registry.register(Arc::new(WebSearchTool::new(permissions.clone())));
        let _subtools = Arc::new(subtool_registry);

        // P8 · sub-agent session table (created early — DelegateTool below needs it)
        let subagent_sessions: Arc<
            Mutex<HashMap<String, crate::palaces::zhen_tool::builtin::delegate::SubagentSession>>,
        > = Arc::new(Mutex::new(HashMap::new()));
        // P3 · per-session interaction modes (for /plan slash entry)
        let session_modes: Arc<
            Mutex<HashMap<String, crate::plates::tian_heaven::InteractionMode>>,
        > = Arc::new(Mutex::new(HashMap::new()));

        let mut tool_registry = ToolRegistry::new();
        tool_registry.register(Arc::new(ReadFileTool::new(permissions.clone())));
        tool_registry.register(Arc::new(WriteFileTool::new(permissions.clone())));
        tool_registry.register(Arc::new(ShellTool::new(permissions.clone())));
        tool_registry.register(Arc::new(GrepTool::new(permissions.clone())));
        tool_registry.register(Arc::new(GlobTool::new(permissions.clone())));
        tool_registry.register(Arc::new(EditTool::new(permissions.clone())));
        tool_registry.register(Arc::new(LspTool::new(permissions.clone())));
        // P3 · plan-mode control tools (read-only, non-destructive — D1)
        tool_registry.register(Arc::new(EnterPlanModeTool));
        tool_registry.register(Arc::new(ExitPlanModeTool));
        // P6 · worktree isolation tools
        tool_registry.register(Arc::new(EnterWorktreeTool::new(permissions.clone())));
        tool_registry.register(Arc::new(ExitWorktreeTool));
        tool_registry.register(Arc::new(WebFetchTool::new(permissions.clone())));
        tool_registry.register(Arc::new(WebExecuteJsTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserNavigateTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserSnapshotTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserClickTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserTypeTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserPressKeyTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserScreenshotTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserScrollTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserConsoleTool::new(permissions.clone())));
        tool_registry.register(Arc::new(BrowserDialogTool::new(permissions.clone())));
        tool_registry.register(Arc::new(ComputerUseTool::new(permissions.clone())));
        #[cfg(feature = "web-search")]
        tool_registry.register(Arc::new(WebSearchTool::new(permissions.clone())));
        // P8 · send_message (continue a sub-agent) + cross-worker scratchpad
        #[cfg(feature = "agent-tool")]
        tool_registry.register(Arc::new(SendMessageTool::new(
            main_core.clone(),
            _subtools.clone(),
            subagent_sessions.clone(),
        )));
        tool_registry.register(Arc::new(ScratchpadWriteTool::new(permissions.clone())));
        tool_registry.register(Arc::new(ScratchpadReadTool::new(permissions.clone())));

        // Cron store shared between tool, runner, and REST API
        let cron_store = CronStore::new(cron_dir);
        #[cfg(feature = "cron")]
        tool_registry.register(Arc::new(CronTool::new(cron_store.clone())));
        #[cfg(feature = "git")]
        tool_registry.register(Arc::new(GitTool::new(permissions.clone())));

        // Task store — CRUD task tracking, shared for potential REST API access
        let task_store = TaskStore::new();
        tool_registry.register(Arc::new(TaskTool::new(task_store.clone())));

        // Pending questions — shared between AskUserQuestionTool and REST /answer endpoint
        let pending_questions: Arc<Mutex<HashMap<String, PendingQuestion>>> =
            Arc::new(Mutex::new(HashMap::new()));
        tool_registry.register(Arc::new(AskUserQuestionTool::new(
            pending_questions.clone(),
            permissions.clone(),
        )));

        // Connect MCP servers and register their tools
        #[cfg(feature = "mcp")]
        if !config_loader.app_config.mcp_servers.is_empty() {
            McpManager::connect_all(
                &config_loader.app_config.mcp_servers,
                &mut tool_registry,
                permissions.clone(),
            );
        }

        // I/O channel — keep receiver for the IO consumer task
        let (io, io_rx) = ChannelManager::new();
        let io = Arc::new(io);

        // Load skills from skills/ directory
        let mut skill_registry = SkillRegistry::new();
        let skills_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        if skills_dir.is_dir() {
            match SkillLoader::load_directory_sync(&skills_dir, &mut skill_registry) {
                Ok(n) => tracing::info!("Loaded {n} skills from skills/"),
                Err(e) => tracing::warn!("Failed to load skills: {e}"),
            }
        }
        let skills = Arc::new(std::sync::RwLock::new(skill_registry));

        if skills_dir.is_dir() {
            spawn_skill_watcher(skills.clone(), skills_dir.clone());
        }

        // Register SkillTool — bridges ToolRegistry ↔ SkillRegistry
        tool_registry.register(Arc::new(SkillTool::new(skills.clone())));

        // Open Store before tool registration so tools can receive it
        let store = Arc::new(Store::open(&db_path.to_string_lossy()));

        // Register NamaRupaTool — agentic graph memory (nāma-rūpa)
        tool_registry.register(Arc::new(NamaRupaTool::new(store.clone())));

        // P1 · Register DelegateTool with Store for sub-agent session persistence
        #[cfg(feature = "agent-tool")]
        tool_registry.register(Arc::new(DelegateTool::new(
            permissions.clone(),
            main_core.clone(),
            _subtools.clone(),
            store.clone(),
            subagent_sessions.clone(),
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
                    crate::palaces::zhen_tool::builtin::toolsearch::ToolSearchTool::new(
                        weak,
                        permissions.clone(),
                    ),
                ));
            }
        }

        let pending_confirmations = Arc::new(Mutex::new(HashMap::new()));
        let session_locks = Arc::new(Mutex::new(HashMap::new()));

        // P4 · compile user-configurable hooks (regex pre-compiled once).
        let user_hooks: Vec<crate::plates::tian_heaven::r#loop::CompiledHook> = config_loader
            .app_config
            .hooks
            .iter()
            .filter_map(|cfg| {
                match crate::plates::tian_heaven::r#loop::CompiledHook::compile(cfg) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        tracing::warn!(hook = %cfg.command, error = %e, "skipping invalid hook");
                        None
                    }
                }
            })
            .collect();
        if !user_hooks.is_empty() {
            tracing::info!(count = user_hooks.len(), "compiled user hooks");
        }
        let user_hooks = Arc::new(user_hooks);

        let mut spirit = SpiritPlate::new();
        spirit.hook_registry.register(Box::new(TracingHook));

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
            store,
            spirit: Arc::new(spirit),
            user_hooks,
            pending_confirmations,
            pending_questions,
            subagent_sessions,
            session_modes,
            session_locks,
            data_dir,
            pid_path,
            backup_dir,
        });

        // Spawn cron runner (needs Arc<EarthPlate>)
        cron_runner::spawn_cron_runner(cron_store.clone(), earth.clone());

        // Spawn IO consumer — reads from ChannelManager and spawns Agent sessions
        // for bot messages (WeChat, Telegram, Discord, webhooks, etc.)
        {
            let earth_io = earth.clone();
            tokio::spawn(async move {
                let mut rx = UnboundedReceiverStream::new(io_rx);
                while let Some(input) = rx.next().await {
                    let earth = earth_io.clone();
                    tokio::spawn(async move {
                        run_io_agent(earth, input).await;
                    });
                }
                tracing::info!("IO consumer stopped");
            });
        }

        earth
    }

    /// P6 · rebuild a tool registry scoped to a worktree root.
    ///
    /// Constructs a fresh `ToolRegistry` with a sub-domain `PermissionMatrix`
    /// rooted at `root` (file/shell/git tools execute against the worktree),
    /// reusing the shared handles (Store/JiaCore/TaskStore/SkillRegistry/Cron/
    /// pending_questions) — only the matrix differs (E3). MCP/WASM tools are
    /// copied from `earth.tools` (they keep the global matrix; v1 limitation).
    /// LSP servers are shared via the per-process `LspManager` inside `LspTool`
    /// (D3: not restarted, just re-didOpen on the new root).
    pub fn rebuild_tools_for_root(&self, root: &std::path::Path) -> Arc<ToolRegistry> {
        // Force project_root = worktree by overriding the cloned security section.
        let mut sec = self.config.app_config.security.clone();
        sec.project_root = Some(root.to_string_lossy().to_string());
        let matrix = Arc::new(
            PermissionMatrix::from_config(&sec, root, self.backup_dir.clone())
                .with_sandbox(&sec.sandbox),
        );

        // Read-only subtools for delegate (rebuilt with the sub-matrix)
        let mut subtools = ToolRegistry::new();
        subtools.register(Arc::new(ReadFileTool::new(matrix.clone())));
        subtools.register(Arc::new(GrepTool::new(matrix.clone())));
        subtools.register(Arc::new(GlobTool::new(matrix.clone())));
        subtools.register(Arc::new(WebFetchTool::new(matrix.clone())));
        subtools.register(Arc::new(WebExecuteJsTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserNavigateTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserSnapshotTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserClickTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserTypeTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserPressKeyTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserScreenshotTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserScrollTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserConsoleTool::new(matrix.clone())));
        subtools.register(Arc::new(BrowserDialogTool::new(matrix.clone())));
        subtools.register(Arc::new(ComputerUseTool::new(matrix.clone())));
        #[cfg(feature = "web-search")]
        subtools.register(Arc::new(WebSearchTool::new(matrix.clone())));
        let subtools = Arc::new(subtools);

        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadFileTool::new(matrix.clone())));
        reg.register(Arc::new(WriteFileTool::new(matrix.clone())));
        reg.register(Arc::new(ShellTool::new(matrix.clone())));
        reg.register(Arc::new(GrepTool::new(matrix.clone())));
        reg.register(Arc::new(GlobTool::new(matrix.clone())));
        reg.register(Arc::new(EditTool::new(matrix.clone())));
        reg.register(Arc::new(LspTool::new(matrix.clone())));
        reg.register(Arc::new(EnterPlanModeTool));
        reg.register(Arc::new(ExitPlanModeTool));
        reg.register(Arc::new(EnterWorktreeTool::new(matrix.clone())));
        reg.register(Arc::new(ExitWorktreeTool));
        reg.register(Arc::new(WebFetchTool::new(matrix.clone())));
        reg.register(Arc::new(WebExecuteJsTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserNavigateTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserSnapshotTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserClickTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserTypeTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserPressKeyTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserScreenshotTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserScrollTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserConsoleTool::new(matrix.clone())));
        reg.register(Arc::new(BrowserDialogTool::new(matrix.clone())));
        reg.register(Arc::new(ComputerUseTool::new(matrix.clone())));
        #[cfg(feature = "web-search")]
        reg.register(Arc::new(WebSearchTool::new(matrix.clone())));
        #[cfg(feature = "agent-tool")]
        reg.register(Arc::new(DelegateTool::new(
            matrix.clone(),
            self.main_core.clone(),
            subtools.clone(),
            self.store.clone(),
            self.subagent_sessions.clone(),
        )));
        #[cfg(feature = "agent-tool")]
        reg.register(Arc::new(SendMessageTool::new(
            self.main_core.clone(),
            subtools.clone(),
            self.subagent_sessions.clone(),
        )));
        reg.register(Arc::new(ScratchpadWriteTool::new(matrix.clone())));
        reg.register(Arc::new(ScratchpadReadTool::new(matrix.clone())));
        #[cfg(feature = "cron")]
        reg.register(Arc::new(CronTool::new(self.cron.clone())));
        #[cfg(feature = "git")]
        reg.register(Arc::new(GitTool::new(matrix.clone())));
        reg.register(Arc::new(TaskTool::new(self.task_store.clone())));
        reg.register(Arc::new(AskUserQuestionTool::new(
            self.pending_questions.clone(),
            matrix.clone(),
        )));
        reg.register(Arc::new(NamaRupaTool::new(self.store.clone())));
        reg.register(Arc::new(SkillTool::new(self.skills.clone())));

        // Reuse external (MCP/WASM) tools from the global registry — they keep
        // the global matrix (v1 limitation: MCP tools in a worktree use the
        // global root). Builtins above are rebuilt with the sub-matrix. Preserve
        // external-ness so toolsearch finds them. Skip toolsearch (added below
        // scoped to THIS registry via Weak).
        for name in self.tools.list_names() {
            if name.as_str() == "toolsearch" {
                continue;
            }
            if reg.get(name).is_none()
                && let Some(t) = self.tools.get(name)
            {
                if self.tools.is_external(name) {
                    reg.register_external(t.clone());
                } else {
                    reg.register(t.clone());
                }
            }
        }

        let mut reg = Arc::new(reg);
        // P9 · ToolSearchTool scoped to this (worktree) registry.
        {
            let weak = std::sync::Arc::downgrade(&reg);
            if let Some(r) = std::sync::Arc::get_mut(&mut reg) {
                r.register(Arc::new(
                    crate::palaces::zhen_tool::builtin::toolsearch::ToolSearchTool::new(
                        weak,
                        matrix.clone(),
                    ),
                ));
            }
        }
        reg
    }

    /// Spawn a background agent task for a cron job prompt.
    ///
    /// Runs the full agent loop, logs the response, and stores it on
    /// the CronJob so the frontend can retrieve it.
    pub fn spawn_cron_agent(self: &Arc<Self>, job_name: String, prompt: String) {
        let earth = self.clone();
        let cron = self.cron.clone();
        tokio::spawn(async move {
            let session_id = uuid::Uuid::new_v4().to_string();
            let human_plate = HumanPlate::with_state(
                earth.permissions.clone(),
                earth.pending_confirmations.clone(),
            );
            let distilled_hashes = earth.store.load_distilled_hashes(&session_id);
            let workspace = earth.data_dir.join("workspace");
            let scoped_tools = earth.rebuild_tools_for_root(&workspace);
            let mut agent = Agent::with_session(
                session_id.clone(),
                earth.clone(),
                Vec::new(),
                Manas::default(),
                distilled_hashes,
                scoped_tools,
            );
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

            agent
                .run(
                    messages,
                    &earth.main_core,
                    &human_plate,
                    &event_bus,
                    &earth.spirit.hook_registry,
                    tx,
                    &CancellationToken::new(),
                )
                .await;
            agent
                .post_loop(store, &earth.main_core, earth.aux_core.as_deref())
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
        });
    }
}

/// Run an Agent session for a single ChannelInput and log the response.
///
/// Shared path for IO-triggered agent invocations
/// (bots, webhooks, file-watch).  The response is logged via tracing.
async fn run_io_agent(earth: Arc<EarthPlate>, input: crate::palaces::kan_io::ChannelInput) {
    use crate::palaces::kan_io::ChannelSource;
    let crate::palaces::kan_io::ChannelInput {
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
        let mut map = earth.session_locks.lock().unwrap();
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

    let human_plate = HumanPlate::with_state(
        earth.permissions.clone(),
        earth.pending_confirmations.clone(),
    );
    let distilled_hashes = earth.store.load_distilled_hashes(&session_id);
    let workspace = earth.data_dir.join("workspace");
    let scoped_tools = earth.rebuild_tools_for_root(&workspace);
    let mut agent = Agent::with_session(
        session_id.clone(),
        earth.clone(),
        history,
        manas,
        distilled_hashes,
        scoped_tools,
    );
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

    agent
        .run(
            messages,
            &earth.main_core,
            &human_plate,
            &earth.spirit.event_bus,
            &earth.spirit.hook_registry,
            tx,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await;
    agent
        .post_loop(
            earth.store.clone(),
            &earth.main_core,
            earth.aux_core.as_deref(),
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

// ── Built-in Hook: TracingHook ────────────────────────────────

/// Logs tool execution events via the tracing subsystem.
///
/// ZhiFu: tool lifecycle (pre/post execute)
/// TengShe: LLM response observation
/// JiuDi: context compaction events
struct TracingHook;

#[async_trait::async_trait]
impl Hook for TracingHook {
    fn name(&self) -> &str {
        "tracing"
    }

    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::ZhiFu, SpiritType::TengShe, SpiritType::JiuDi]
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        match &event {
            HookEvent::ToolPreExecute { tool_name, input } => {
                tracing::info!(tool = %tool_name, input = %input, "hook: tool pre-execute");
            }
            HookEvent::ToolPostExecute {
                tool_name,
                output,
                error,
                duration_ms,
            } => {
                if let Some(err) = error {
                    tracing::warn!(tool = %tool_name, error = %err, duration_ms = duration_ms, "hook: tool post-execute (error)");
                } else {
                    tracing::info!(tool = %tool_name, output_len = output.len(), duration_ms = duration_ms, "hook: tool post-execute");
                }
            }
            HookEvent::LlmResponse {
                response_len,
                tool_call_count,
            } => {
                tracing::info!(
                    response_len = response_len,
                    tool_call_count = tool_call_count,
                    "hook: LLM response"
                );
            }
            HookEvent::BatchEnded { tool_count, turn } => {
                tracing::info!(tool_count = tool_count, turn = turn, "hook: batch ended");
            }
            HookEvent::CompactionTriggered {
                messages_before,
                messages_after,
                tokens_before,
                tokens_after,
                method,
            } => {
                tracing::info!(
                    messages_before = messages_before,
                    messages_after = messages_after,
                    tokens_before = tokens_before,
                    tokens_after = tokens_after,
                    method = method.as_str(),
                    "hook: context compacted"
                );
            }
        }
        HookResult::Ok
    }
}

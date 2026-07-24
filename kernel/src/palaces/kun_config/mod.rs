//! kun_config — Configuration (坤二)

use std::{collections::HashMap, path::PathBuf};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

use crate::error::JiaError;

/// 甲（Jia）— AI Agent runtime
#[derive(Parser, Debug)]
#[command(name = "jia", about = "甲 — AI Agent runtime based on Qimen Dunjia")]
pub struct CliArgs {
    /// Config file path (default: config.toml in current directory)
    #[arg(long = "config", env = "JIA_CONFIG")]
    pub config_path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Shortcut for `jia gateway start` — start the API gateway in background
    #[command(visible_alias = "s")]
    Start {
        #[arg(long = "config", env = "JIA_CONFIG")]
        config_path: Option<PathBuf>,
        #[arg(long, env = "JIA_HOST")]
        host: Option<String>,
        #[arg(long, env = "JIA_PORT")]
        port: Option<u16>,
    },
    /// Shortcut for `jia gateway stop` — stop the running gateway
    Stop,
    /// Shortcut for `jia gateway status` — show gateway status
    Status,
    /// Shortcut for `jia gateway restart` — restart the gateway
    Restart {
        #[arg(long = "config", env = "JIA_CONFIG")]
        config_path: Option<PathBuf>,
        #[arg(long, env = "JIA_HOST")]
        host: Option<String>,
        #[arg(long, env = "JIA_PORT")]
        port: Option<u16>,
    },
    /// Start the API gateway server
    Gateway {
        #[command(subcommand)]
        action: GatewayAction,
    },
    /// WeChat QR login setup — scan with WeChat to obtain bot credentials
    WechatSetup,
    /// Launch the terminal UI (explicit, same as bare `jia`)
    Tui,
    /// Diagnose installation health: config, LLM, data dir, SQLite, disk
    Doctor,
    /// Initialize a Jia project in a directory (creates .jia/config.toml)
    Init {
        /// Path to initialize (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Start the gateway with the web dashboard (foreground).
    /// Shortcut for `jia gateway daemon --web-dir <frontend/dist>`.
    Web {
        /// Config file path (default: config.toml in current directory)
        #[arg(long = "config", env = "JIA_CONFIG")]
        config_path: Option<PathBuf>,

        /// HTTP server listen address (overrides config file)
        #[arg(long, env = "JIA_HOST")]
        host: Option<String>,

        /// HTTP server listen port (overrides config file)
        #[arg(long, env = "JIA_PORT")]
        port: Option<u16>,

        /// Frontend directory (default: CARGO_MANIFEST_DIR/frontend/dist)
        #[arg(long = "web-dir", env = "JIA_WEB_DIR")]
        web_dir: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum GatewayAction {
    /// Start the API gateway server (background daemon). Use `jia web` for the dashboard.
    Start {
        /// Config file path (default: config.toml in current directory)
        #[arg(long = "config", env = "JIA_CONFIG")]
        config_path: Option<PathBuf>,

        /// HTTP server listen address (overrides config file)
        #[arg(long, env = "JIA_HOST")]
        host: Option<String>,

        /// HTTP server listen port (overrides config file)
        #[arg(long, env = "JIA_PORT")]
        port: Option<u16>,
    },
    /// Stop the running HTTP server
    Stop,
    /// Show the running server status
    Status,
    /// Restart the HTTP server (stop then start)
    Restart {
        /// Config file path (default: config.toml in current directory)
        #[arg(long = "config", env = "JIA_CONFIG")]
        config_path: Option<PathBuf>,

        /// HTTP server listen address (overrides config file)
        #[arg(long, env = "JIA_HOST")]
        host: Option<String>,

        /// HTTP server listen port (overrides config file)
        #[arg(long, env = "JIA_PORT")]
        port: Option<u16>,
    },
    /// Internal: daemon process spawned by start/restart
    #[command(hide = true)]
    Daemon {
        /// Config file path
        #[arg(long = "config", env = "JIA_CONFIG")]
        config_path: Option<PathBuf>,

        /// HTTP server listen address
        #[arg(long, env = "JIA_HOST")]
        host: Option<String>,

        /// HTTP server listen port
        #[arg(long, env = "JIA_PORT")]
        port: Option<u16>,
    },
}

/// Config file schema (config.toml)
#[derive(Debug, Deserialize)]
pub struct JiaToml {
    #[serde(default)]
    pub server: ServerSection,
    #[serde(default)]
    pub llm: LlmSection,
    /// Named provider profiles: [providers.default], [providers.claude], etc.
    /// Required — must be defined in config.toml.
    pub providers: HashMap<String, ProviderProfile>,
    #[serde(default)]
    pub security: SecuritySection,
    /// MCP (Model Context Protocol) server definitions
    #[serde(default, rename = "mcp_server")]
    pub mcp_servers: Vec<McpServerConfig>,
    /// IM bot configuration
    #[serde(default)]
    pub bots: BotsSection,
    /// P4 · user-configurable hooks ([[hooks]] array)
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
    /// Cognitive architecture feature flags
    #[serde(default)]
    pub cognition: CognitionSection,
}

#[derive(Debug, Deserialize)]
pub struct ServerSection {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Path to web dashboard static files (frontend/dist/).
    /// Leave empty to disable web UI.
    #[serde(default)]
    pub web_dir: Option<String>,
}

/// LLM configuration section ([llm] in config.toml).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LlmSection {
    /// Name of the default main model provider (must match a key in [providers]).
    /// If unset, the first provider key alphabetically is used.
    #[serde(default)]
    pub default_main_model_provider: Option<String>,
    /// Name of the aux provider for background tasks (consolidation,
    /// distillation, skill reflection). If unset, aux_core is None and
    /// all aux tasks fall back to main_core.
    #[serde(default)]
    pub default_aux_model_provider: Option<String>,
    /// System prompt for the gateway /agent passthrough endpoint.
    /// Defaults to [`DEFAULT_SYSTEM_PROMPT`].
    #[serde(default)]
    pub system_prompt: Option<String>,
}

/// Default system prompt for the gateway /agent passthrough endpoint,
/// used when `[llm].system_prompt` is not configured.
pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are Jia (甲), Just Intelligence Agent (正是智能体). Respond concisely and directly.";

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderProfile {
    /// "openai", "anthropic", "gemini"
    pub kind: String,
    /// Model list. If empty or omitted, models are fetched from the provider API on startup.
    #[serde(default)]
    pub models: Vec<String>,
    /// Default model to use when none is specified. Falls back to models[0] if set.
    pub default_main_model: Option<String>,
    /// Model override when this provider is used as the aux provider.
    /// Falls back to default_main_model if not set.
    pub default_aux_model: Option<String>,
    pub api_key: String,

    pub base_url: String,
    /// Max output tokens per request. Defaults to 4096 if unset.
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Context window size (total tokens). Overrides security.max_context_tokens.
    /// Defaults to 8192 if unset.
    #[serde(default)]
    pub context_window: Option<usize>,
    /// Failover priority (lower = higher priority). Providers with lower values
    /// are tried first. None = lowest priority (tried last).
    #[serde(default)]
    pub priority: Option<u32>,
    /// Cost multiplier relative to baseline (1.0 = standard). Used by cost-aware
    /// routing (e.g. cheapest healthy provider for background tasks).
    #[serde(default)]
    pub cost_multiplier: Option<f32>,
}

impl ProviderProfile {
    pub fn default_main_model(&self) -> &str {
        self.default_main_model
            .as_deref()
            .unwrap_or_else(|| self.models.first().map(|s| s.as_str()).unwrap_or(""))
    }
}

/// P4 · user-configurable hook entry (人盘门规 / 神盘观测).
///
/// Configured as a `[[hooks]]` TOML array on AppConfig. Blocking pre-tool hooks
/// run synchronously in the loop after GeJu and before dispatch (人盘门规);
/// non-blocking observation hooks (post_tool_use, etc.) run via 神盘.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HookConfig {
    /// Event kind: "pre_tool_use" (人盘, may block) | "post_tool_use" (神盘, observe).
    #[serde(default)]
    pub event: String,
    /// Optional regex matched against the tool name. Empty/absent = match all.
    #[serde(default)]
    pub tool_pattern: Option<String>,
    /// External shell command. Hook context is passed via the `JIA_HOOK_CONTEXT`
    /// env var (JSON: {tool, input, ...}) and `JIA_HOOK_TOOL`.
    pub command: String,
    /// For pre_tool_use: a non-zero exit code blocks the tool (白虎守门).
    #[serde(default)]
    pub block_on_exit: bool,
}

/// Sandbox isolation mode.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    /// Must have true isolation — refuse execution if unavailable.
    #[default]
    Required,
    /// Try isolation, fall back to Process sandbox if unavailable.
    BestEffort,
    /// Skip sandbox entirely.
    Disabled,
}

/// Security configuration for tool sandboxing and permissions.
#[derive(Debug, Clone, Deserialize)]
pub struct SecuritySection {
    /// Root directory for path sandboxing. Default: current working directory.
    /// (alias: 重命名前为 project_root——无 alias 时旧配置键会被静默丢弃,
    /// 沙箱根回落默认目录,边界悄悄移动。)
    #[serde(default, alias = "project_root")]
    pub workspace_root: Option<String>,
    /// Additional directories (outside workspace_root) where tools can read/write.
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    /// Path prefixes that are always blocked, even within workspace_root.
    #[serde(default = "default_blocked_prefixes")]
    pub blocked_path_prefixes: Vec<String>,
    /// If non-empty, only these shell commands are allowed.
    #[serde(default)]
    pub command_allowlist: Vec<String>,
    /// Shell command patterns that are always blocked.
    #[serde(default = "default_blocked_commands")]
    pub command_blocklist: Vec<String>,
    /// Timeout in seconds for user confirmation prompts. Default: 30.
    #[serde(default = "default_confirmation_timeout")]
    pub confirmation_timeout_secs: u64,
    /// Sandbox isolation mode: Required (default), BestEffort, or Disabled.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,
    /// Maximum context window token budget. Default: 8192.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Truncation threshold (0.0–1.0). At 75%, old messages are dropped. Default: 0.75.
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: f64,
    /// Optional API key for Bearer token auth. When set, all HTTP requests require
    /// `Authorization: Bearer <key>`. When None, all requests are allowed (dev mode).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Max requests per minute per client IP for the /agent endpoint. 0 disables.
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_minute: u32,
    /// Execution sandbox configuration.
    #[serde(default)]
    pub sandbox: SandboxSection,
}

/// Execution sandbox backend selection and resource limits.
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxSection {
    /// Sandbox backend: "process", "docker", "seatbelt", "landlock".
    /// Default: "process" (always available, no external dependencies).
    #[serde(default = "default_sandbox_backend")]
    pub backend: String,
    /// Command timeout in seconds. Default: 30.
    #[serde(default = "default_sandbox_timeout")]
    pub timeout_seconds: u64,
    /// Memory limit in MB. Default: 512.
    #[serde(default = "default_sandbox_memory_mb")]
    pub memory_limit_mb: u64,
    /// Max child processes. Default: 50.
    #[serde(default = "default_sandbox_max_procs")]
    pub max_processes: u64,
    /// Max output file size in MB. Default: 100.
    #[serde(default = "default_sandbox_fsize_mb")]
    pub file_size_limit_mb: u64,
    /// Allow network access in sandbox (Docker only). Default: false.
    #[serde(default)]
    pub network_enabled: bool,
    /// Docker image for sandbox execution. Default: "alpine:3.20".
    #[serde(default = "default_docker_image")]
    pub docker_image: String,
    /// CPU limit for Docker sandbox (fractional allowed, e.g. 0.5 = half a core). Default: 1.0.
    #[serde(default = "default_cpu_limit")]
    pub cpu_limit: f64,
}

impl Default for SandboxSection {
    fn default() -> Self {
        Self {
            backend: default_sandbox_backend(),
            timeout_seconds: default_sandbox_timeout(),
            memory_limit_mb: default_sandbox_memory_mb(),
            max_processes: default_sandbox_max_procs(),
            file_size_limit_mb: default_sandbox_fsize_mb(),
            network_enabled: false,
            docker_image: default_docker_image(),
            cpu_limit: default_cpu_limit(),
        }
    }
}

fn default_sandbox_backend() -> String {
    "process".into()
}
fn default_sandbox_timeout() -> u64 {
    30
}
fn default_sandbox_memory_mb() -> u64 {
    512
}
fn default_sandbox_max_procs() -> u64 {
    50
}
fn default_sandbox_fsize_mb() -> u64 {
    100
}
fn default_docker_image() -> String {
    "alpine:3.20".into()
}
fn default_cpu_limit() -> f64 {
    1.0
}

/// IM bot configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct BotsSection {
    #[serde(default)]
    pub telegram: Option<TelegramBotConfig>,
    #[serde(default)]
    pub wechat: Option<WeChatBotConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramBotConfig {
    pub token: String,
    /// Allowed chat IDs (user or group). Empty = no one can interact.
    /// Each entry is a numeric chat ID string, e.g. ["123456789"].
    #[serde(default)]
    pub allowed_chat_ids: Vec<String>,
}

/// WeChat personal bot configuration (iLink Bot API).
#[derive(Debug, Clone, Deserialize)]
pub struct WeChatBotConfig {
    /// iLink Bot account ID (obtained from QR login).
    pub account_id: String,
    /// iLink Bot token (obtained from QR login).
    pub token: String,
    /// API base URL. Defaults to iLink production endpoint.
    #[serde(default = "default_wechat_base_url")]
    pub base_url: String,
    /// DM policy: "open" (anyone), "allowlist", or "disabled".
    #[serde(default = "default_dm_policy")]
    pub dm_policy: String,
    /// Group chat policy: "open", "allowlist", or "disabled".
    #[serde(default = "default_group_disabled")]
    pub group_policy: String,
    /// Comma-separated allowed WeChat user IDs (when dm_policy = "allowlist").
    #[serde(default)]
    pub allowed_users: String,
}

fn default_wechat_base_url() -> String {
    "https://ilinkai.weixin.qq.com".into()
}
fn default_dm_policy() -> String {
    "disabled".into()
}
fn default_group_disabled() -> String {
    "disabled".into()
}

fn default_blocked_prefixes() -> Vec<String> {
    vec![".git".into(), ".env".into()]
}
fn default_blocked_commands() -> Vec<String> {
    vec![
        "rm -rf /".into(),
        "mkfs.".into(),
        "dd if=".into(),
        "sudo rm".into(),
    ]
}
fn default_confirmation_timeout() -> u64 {
    30
}
fn default_max_context_tokens() -> usize {
    8192
}
fn default_compaction_threshold() -> f64 {
    0.75
}
fn default_rate_limit() -> u32 {
    30
}

/// MCP (Model Context Protocol) server configuration.
///
/// Each entry spawns an MCP-compliant subprocess and discovers its tools.
///
/// ```toml
/// [[mcp_server]]
/// name = "filesystem"
/// command = "npx"
/// args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
/// sandbox_params = ["path", "directory"]
/// read_only_tools = ["read_file", "list_dir"]
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Parameter names that carry filesystem paths — these are run through verify_path.
    #[serde(default)]
    pub sandbox_params: Vec<String>,
    /// Tool names that are read-only — classified as Wu (Read) instead of Geng (Exec).
    #[serde(default)]
    pub read_only_tools: Vec<String>,
    /// If true, run the subprocess under OS sandbox blocking network access.
    /// macOS: uses `sandbox-exec -n no-network`. Linux: uses `unshare -n`.
    /// Default false — opt-in for backward compatibility.
    #[serde(default)]
    pub isolated: bool,
}

impl Default for SecuritySection {
    fn default() -> Self {
        Self {
            workspace_root: None,
            allowed_paths: Vec::new(),
            blocked_path_prefixes: default_blocked_prefixes(),
            command_allowlist: Vec::new(),
            command_blocklist: default_blocked_commands(),
            confirmation_timeout_secs: default_confirmation_timeout(),
            sandbox_mode: crate::palaces::kun_config::SandboxMode::Required,
            max_context_tokens: default_max_context_tokens(),
            compaction_threshold: default_compaction_threshold(),
            api_key: None,
            rate_limit_per_minute: default_rate_limit(),
            sandbox: SandboxSection::default(),
        }
    }
}

fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    3000
}

/// Resolve the data directory: `$JIA_HOME` or `$HOME/.jia`.
pub fn default_data_dir() -> std::path::PathBuf {
    std::env::var("JIA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".into());
            std::path::PathBuf::from(home).join(".jia")
        })
}

/// Path to the gateway PID file: `$JIA_HOME/gateway.pid`.
pub fn pid_file_path() -> std::path::PathBuf {
    default_data_dir().join("gateway.pid")
}

/// Default project workspace root: `$HOME/Documents/jia-workspace`.
/// Used when `security.workspace_root` is not explicitly configured.
pub fn default_workspace_dir() -> std::path::PathBuf {
    std::env::var("JIA_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".into());
            std::path::PathBuf::from(home).join("Documents/jia-workspace")
        })
}

/// Default web dashboard directory.
///
/// Resolved at compile time relative to the crate root so `jia web` finds
/// the frontend without `--web-dir`. Within the repo layout, this is:
/// `<jia>/frontend/dist`.
pub fn default_web_dir() -> PathBuf {
    // 优先用户安装的前端(<data_dir>/frontend,如 ~/.jia/frontend),
    // 其次仓内构建产物(<crate_root>/frontend/dist)。
    let user = default_data_dir().join("frontend");
    if user.join("index.html").exists() {
        return user;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("frontend").join("dist"))
        .unwrap_or_else(|| PathBuf::from("frontend/dist"))
}

impl Default for ServerSection {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            web_dir: None,
        }
    }
}
/// Cognitive architecture feature flags and tuning parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct CognitionSection {
    pub certainty_enabled: bool,
    pub coactivation_enabled: bool,
    pub observation_enabled: bool,
    pub checklist_enabled: bool,
    pub reset_enabled: bool,
}

impl Default for CognitionSection {
    fn default() -> Self {
        Self {
            certainty_enabled: true,
            coactivation_enabled: true,
            observation_enabled: true,
            checklist_enabled: true,
            reset_enabled: true,
        }
    }
}

// ── AppConfig (resolved) ─────────────────────────────────────

pub struct AppConfig {
    pub host: String,
    pub port: u16,
    pub web_dir: Option<String>,
    pub providers: HashMap<String, ProviderProfile>,
    pub default_main_model_provider: Option<String>,
    pub default_aux_model_provider: Option<String>,
    /// System prompt for the gateway /agent endpoint (from `[llm].system_prompt`,
    /// falling back to [`DEFAULT_SYSTEM_PROMPT`]).
    pub system_prompt: String,
    pub security: SecuritySection,
    pub mcp_servers: Vec<McpServerConfig>,
    pub bots: BotsSection,
    /// P4 · user-configurable hooks (人盘门规 / 神盘观测). Default empty.
    pub hooks: Vec<HookConfig>,
    /// Cognitive architecture features (certainty, coactivation, observation, etc.)
    pub cognition: CognitionSection,
}

impl AppConfig {
    /// Load config from a TOML file with optional CLI overrides.
    pub fn load(
        config_path: Option<PathBuf>,
        host_override: Option<String>,
        port_override: Option<u16>,
    ) -> Result<Self, JiaError> {
        let _ = dotenvy::dotenv();

        let toml_path = config_path.unwrap_or_else(|| {
            let cwd_cfg = std::env::current_dir()
                .unwrap_or_default()
                .join("config.toml");
            if cwd_cfg.exists() {
                return cwd_cfg;
            }
            default_data_dir().join("config.toml")
        });
        let toml_str = std::fs::read_to_string(&toml_path).map_err(|e| {
            JiaError::Config(format!("Cannot read config file {}: {e}", toml_path.display()).into())
        })?;
        let mut toml: JiaToml = toml::from_str(&toml_str).map_err(|e| {
            JiaError::Config(format!("Invalid config file {}: {e}", toml_path.display()).into())
        })?;

        if toml.providers.is_empty() {
            return Err(JiaError::Config(
                format!(
                    "Config file {} has no [providers] section",
                    toml_path.display()
                )
                .into(),
            ));
        }
        for (name, p) in &toml.providers {
            if p.models.is_empty() {
                return Err(JiaError::Config(
                    format!(
                        "Provider '{name}' in {} has empty models list",
                        toml_path.display()
                    )
                    .into(),
                ));
            }
        }

        if let Some(ref dp) = toml.llm.default_main_model_provider
            && !toml.providers.contains_key(dp.as_str())
        {
            return Err(JiaError::Config(
                format!(
                    "Config file {}: default_provider '{}' not found in [providers] section",
                    toml_path.display(),
                    dp,
                )
                .into(),
            ));
        }

        // Env var takes priority over config file for api_key
        let mut security = toml.security;
        if let Ok(env_key) = std::env::var("JIA_API_KEY")
            && !env_key.is_empty()
        {
            security.api_key = Some(env_key);
        }

        // Env vars for provider API keys: {NAME}_API_KEY (e.g. ANTHROPIC_API_KEY)
        for (name, profile) in toml.providers.iter_mut() {
            let env_var_name = format!("{}_API_KEY", name.to_uppercase());
            if let Ok(env_key) = std::env::var(&env_var_name)
                && !env_key.is_empty()
            {
                tracing::info!("Using {env_var_name} for provider '{name}'");
                profile.api_key = env_key;
            }
        }

        let host = host_override.unwrap_or(toml.server.host);
        let port = port_override.unwrap_or(toml.server.port);

        Ok(Self {
            host,
            port,
            web_dir: toml.server.web_dir.clone(),
            providers: toml.providers,
            default_main_model_provider: toml.llm.default_main_model_provider,
            default_aux_model_provider: toml.llm.default_aux_model_provider,
            system_prompt: toml
                .llm
                .system_prompt
                .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string()),
            security,
            mcp_servers: toml.mcp_servers,
            bots: toml.bots,
            hooks: toml.hooks,
            cognition: toml.cognition,
        })
    }

    /// The effective default provider name (configured value or first alphabetically).
    pub fn default_main_provider_name(&self) -> &str {
        self.default_main_model_provider
            .as_deref()
            .unwrap_or_else(|| {
                let mut keys: Vec<&String> = self.providers.keys().collect();
                keys.sort();
                keys.first().expect("no providers configured").as_str()
            })
    }

    /// Resolve the default provider profile.
    pub fn default_main_provider(&self) -> Result<ProviderProfile, JiaError> {
        let name = self.default_main_provider_name().to_string();
        self.provider(&name)
    }

    /// Resolve a provider profile by name (falls back to the configured default_provider).
    pub fn provider(&self, name: &str) -> Result<ProviderProfile, JiaError> {
        let default_name = self.default_main_provider_name();
        self.providers
            .get(name)
            .or_else(|| self.providers.get(default_name))
            .cloned()
            .ok_or_else(|| {
                JiaError::Config(
                    format!("no provider '{name}' or default provider '{default_name}'").into(),
                )
            })
    }

    /// List provider names for the frontend selector
    pub fn provider_names(&self) -> Vec<&String> {
        let mut names: Vec<_> = self.providers.keys().collect();
        names.sort();
        names
    }
}

// ── ConfigLoader wrapper (坤二宫) ─────────────────────────────

/// 坤二宫 — Configuration Loader
///
/// Holds the resolved application configuration. Part of the Earth Plate.
pub struct ConfigLoader {
    pub app_config: AppConfig,
}

impl ConfigLoader {
    pub fn from_app_config(config: AppConfig) -> Self {
        Self { app_config: config }
    }

    pub fn provider(&self, name: &str) -> Result<ProviderProfile, JiaError> {
        self.app_config.provider(name)
    }

    pub fn default_main_provider(&self) -> Result<ProviderProfile, JiaError> {
        self.app_config.default_main_provider()
    }
}

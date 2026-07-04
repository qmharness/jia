// ── Top-level JiaError ──────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum JiaError {
    #[error("config: {0}")]
    Config(#[from] ConfigError),
    #[error("provider: {0}")]
    Provider(#[from] ProviderError),
    #[error("tool: {0}")]
    Tool(#[from] ToolError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("dispatch: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("network: {0}")]
    Network(#[from] reqwest::Error),
    #[error("r2d2 pool: {0}")]
    R2d2(#[from] r2d2::Error),
    #[error("internal: {0}")]
    Internal(String),
}

// ── ProviderError ───────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("rate limited: {body}")]
    RateLimited { body: String },
    #[error("authentication failed (HTTP {status}). Check API key.")]
    AuthFailed { status: u16 },
    #[error("server error (HTTP {status}): {body}")]
    ServerError { status: u16, body: String },
    #[error("client error (HTTP {status}): {body}")]
    ClientError { status: u16, body: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("stream stalled — no data for 30s")]
    StreamStalled,
    #[error("stream error: {0}")]
    Stream(String),
    #[error("provider error: {0}")]
    Provider(String),
}

impl ProviderError {
    /// Whether this error is retryable (transient — retry with next provider).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimited { .. }
                | ProviderError::ServerError { .. }
                | ProviderError::Network(_)
                | ProviderError::StreamStalled
        )
    }
}

impl From<String> for ProviderError {
    fn from(s: String) -> Self {
        ProviderError::Provider(s)
    }
}

impl From<&str> for ProviderError {
    fn from(s: &str) -> Self {
        ProviderError::Provider(s.to_string())
    }
}

// ── ToolError ───────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, thiserror::Error)]
pub enum ToolError {
    #[error("{tool}: {message}")]
    Execution { tool: String, message: String },
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("invalid input for {tool}: {reason}")]
    InvalidInput { tool: String, reason: String },
    #[error("sandbox rejected: {0}")]
    SandboxRejected(String),
    #[error("{tool} timed out after {secs}s")]
    Timeout { tool: String, secs: u64 },
}

impl From<String> for ToolError {
    fn from(s: String) -> Self {
        ToolError::Execution { tool: String::new(), message: s }
    }
}

impl From<&str> for ToolError {
    fn from(s: &str) -> Self {
        ToolError::Execution { tool: String::new(), message: s.to_string() }
    }
}

impl ToolError {
    /// Create an Execution error for a named tool.
    pub fn exec(tool: &str, message: impl Into<String>) -> Self {
        ToolError::Execution { tool: tool.to_string(), message: message.into() }
    }
}

// ── ConfigError ─────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required key: {0}")]
    MissingKey(String),
    #[error("invalid value for {key}: {reason}")]
    InvalidValue { key: String, reason: String },
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("{0}")]
    Other(String),
}

impl From<String> for ConfigError {
    fn from(s: String) -> Self {
        ConfigError::Other(s)
    }
}

// ── StoreError (moved from gen_store/mod.rs) ────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("Pool error: {0}")]
    Pool(String),
}

impl From<r2d2::Error> for StoreError {
    fn from(e: r2d2::Error) -> Self {
        StoreError::Pool(e.to_string())
    }
}

// ── DispatchError (moved from ren_human/mod.rs) ─────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum DispatchError {
    #[error("Execution denied: {0}")]
    Denied(String),
    #[error("Tool error: {0}")]
    ToolError(String),
}

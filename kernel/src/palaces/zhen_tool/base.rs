use crate::error::ToolError;
use crate::stems::action::ExecContext;
use async_trait::async_trait;
use std::path::PathBuf;

/// 工具资源访问声明 (U1) — per-call resource declaration derived from the
/// tool's input. The Heaven Plate conflict matrix
/// (`tian_heaven::tool_scheduler`) uses this as the SOLE parallelism
/// criterion (audit A2: ceremony-derived resource domains are deprecated —
/// 并发判据与六仪正交).
///
/// `Default` is `all: true` — the most conservative declaration (公理 4:
/// 只收紧). A tool that declares nothing is globally exclusive and always
/// runs as a singleton barrier batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAccesses {
    /// Paths read by this call (as given in the input; not canonicalized).
    pub reads: Vec<PathBuf>,
    /// Paths written by this call.
    pub writes: Vec<PathBuf>,
    /// true → every declared path is a directory accessed recursively, so
    /// conflict detection treats it as a prefix (conservative).
    pub recursive: bool,
    /// true → unknown/unbounded access: globally exclusive barrier.
    pub all: bool,
}

impl Default for ToolAccesses {
    /// Conservative default: unknown access, globally exclusive.
    fn default() -> Self {
        Self::all()
    }
}

impl ToolAccesses {
    /// Unknown/unbounded access — conflicts with everything (barrier).
    pub fn all() -> Self {
        Self {
            reads: Vec::new(),
            writes: Vec::new(),
            recursive: false,
            all: true,
        }
    }

    /// Read-only declaration for the given paths.
    pub fn read_only(reads: Vec<PathBuf>, recursive: bool) -> Self {
        Self {
            reads,
            writes: Vec::new(),
            recursive,
            all: false,
        }
    }

    /// Write declaration for the given paths (no reads declared).
    pub fn write_only(writes: Vec<PathBuf>) -> Self {
        Self {
            reads: Vec::new(),
            writes,
            recursive: false,
            all: false,
        }
    }
}

/// 震三宫 — BaseTool trait
///
/// Every tool must implement this trait. The `ceremony()` method
/// declares which of the six ceremonial stems the tool belongs to,
/// enabling GeJu evaluation.
///
/// 工具自身为 stateless 单例（注册于地盘，六仪不动）。
/// 权限通过 ExecContext 在调用时注入（值符随时干旋转）。
#[async_trait]
pub trait BaseTool: Send + Sync {
    /// Unique tool name (e.g., "read_file", "write_file", "shell")
    fn name(&self) -> &str;

    /// Human-readable description for LLM function-calling
    fn description(&self) -> String;

    /// Category name for UI grouping (e.g., "文件操作", "浏览器", "Web").
    /// Default: "其他"
    fn category(&self) -> &str {
        "其他"
    }

    /// Which Ceremonies stem category this tool belongs to
    fn ceremony(&self) -> crate::stems::CeremoniesIntent;

    /// JSON Schema describing the tool's input parameters
    fn parameters_schema(&self) -> serde_json::Value;

    /// Whether this tool performs destructive (non-read-only) operations.
    /// Default: true for all CeremoniesIntent categories except Wu.
    fn is_destructive(&self) -> bool {
        !matches!(self.ceremony(), crate::stems::CeremoniesIntent::Wu)
    }

    /// Whether this tool can execute concurrently with other tools.
    /// Every tool MUST explicitly declare this — no default.
    fn is_concurrency_safe(&self) -> bool;

    /// Resource access declaration for this call (U1).
    ///
    /// The scheduler's conflict matrix uses ONLY this declaration to decide
    /// parallelism (A2): read-read never conflicts; write-write conflicts
    /// only on intersecting paths; read-write conflicts on intersection;
    /// `all: true` (the default) is a global barrier. Tools that hold
    /// session-level mutable state (e.g. the LSP manager) MUST keep the
    /// `All` default — 任何声明可并行的工具不得持有会话级可变状态.
    fn accesses(&self, _input: &serde_json::Value) -> ToolAccesses {
        ToolAccesses::all()
    }

    /// Execute the tool with the given JSON input and execution context.
    /// Permissions are injected via `ctx` rather than held by the tool struct.
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: &ExecContext,
    ) -> Result<String, ToolError>;

    /// Target palace for GeJu evaluation.
    ///
    /// Default: maps each ceremony stem to the palace where it sits in the
    /// active 局 (阳遁三局: 戊起震三顺排).  Override to route this tool to a
    /// different palace — e.g., based on the input action.
    fn target_palace(&self, _input: &serde_json::Value) -> crate::palaces::Palace {
        use crate::palaces::Palace;
        match self.ceremony() {
            // 阳遁三局: 戊→震3, 己→巽4, 庚→中5, 辛→乾6, 壬→兑7, 癸→艮8
            crate::stems::CeremoniesIntent::Wu => Palace::Zhen,
            crate::stems::CeremoniesIntent::Ji => Palace::Xun,
            crate::stems::CeremoniesIntent::Geng => Palace::Zhong,
            crate::stems::CeremoniesIntent::Xin => Palace::Qian,
            crate::stems::CeremoniesIntent::Ren => Palace::Dui,
            crate::stems::CeremoniesIntent::Gui => Palace::Gen,
        }
    }

    /// Execute the tool with access to the agent event channel.
    ///
    /// Default implementation delegates to `execute()`. Override only if
    /// the tool needs to emit SSE events (e.g., AskUserQuestion).
    async fn execute_with_tx(
        &self,
        input: serde_json::Value,
        _tx: &tokio::sync::mpsc::UnboundedSender<crate::stems::AgentEvent>,
        ctx: &ExecContext,
    ) -> Result<String, ToolError> {
        self.execute(input, ctx).await
    }
}

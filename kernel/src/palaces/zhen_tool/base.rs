use crate::error::ToolError;
use crate::stems::action::ExecContext;
use async_trait::async_trait;

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
    /// Default: true for all CeremoniesIntent categories except Wu (ReadAction).
    fn is_destructive(&self) -> bool {
        !matches!(self.ceremony(), crate::stems::CeremoniesIntent::Wu(_))
    }

    /// Whether this tool can execute concurrently with other tools.
    /// Every tool MUST explicitly declare this — no default.
    fn is_concurrency_safe(&self) -> bool;

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
            crate::stems::CeremoniesIntent::Wu(_) => Palace::Zhen,
            crate::stems::CeremoniesIntent::Ji(_) => Palace::Xun,
            crate::stems::CeremoniesIntent::Geng(_) => Palace::Zhong,
            crate::stems::CeremoniesIntent::Xin(_) => Palace::Qian,
            crate::stems::CeremoniesIntent::Ren(_) => Palace::Dui,
            crate::stems::CeremoniesIntent::Gui(_) => Palace::Gen,
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

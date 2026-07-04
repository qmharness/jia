use serde::{Deserialize, Serialize};

/// 工具调用 — LLM 发起的工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub parameters: serde_json::Value,
}

/// 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
    pub error: Option<String>,
}

/// Tool definition schema for native tools APIs (OpenAI / Anthropic / Gemini).
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// 执行上下文 — 天盘值符携带的"时令"
///
/// 工具自身为 stateless 单例（注册于地盘，六仪不动）。
/// 权限矩阵由 Agent 通过 ctx 参数在调用时注入（值符随时干旋转）。
#[derive(Clone)]
pub struct ExecContext {
    pub permissions: std::sync::Arc<crate::palaces::qian_permission::PermissionMatrix>,
}

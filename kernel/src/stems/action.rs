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
///
/// `cancel_token` 是该次 run 的取消令牌（与 RunContext 同源），
/// 供长等待工具（ask_user/delegate/确认）select! 响应取消；
/// `session_id` 标识本次 run 所属会话，用于断连时按会话清扫
/// pending_questions / pending_confirmations（消除断连死锁）。
#[derive(Clone)]
pub struct ExecContext {
    pub permissions: std::sync::Arc<crate::palaces::qian_permission::PermissionMatrix>,
    pub session_id: String,
    pub cancel_token: tokio_util::sync::CancellationToken,
}

impl ExecContext {
    /// 构造一个无会话归属、不可取消的上下文（测试与默认场景用）。
    pub fn new(
        permissions: std::sync::Arc<crate::palaces::qian_permission::PermissionMatrix>,
    ) -> Self {
        Self {
            permissions,
            session_id: String::new(),
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }
}

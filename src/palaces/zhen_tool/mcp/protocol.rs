// ── MCP Protocol Types (JSON-RPC 2.0 + MCP messages) ──────────

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Constants ────────────────────────────────────────────────

pub const JSONRPC_VERSION: &str = "2.0";
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
pub const CLIENT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_INITIALIZED: &str = "notifications/initialized";
pub const METHOD_TOOLS_LIST: &str = "tools/list";
pub const METHOD_TOOLS_CALL: &str = "tools/call";

pub const CONTENT_TYPE_TEXT: &str = "text";

/// JSON-RPC 2.0 request
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 response (success)
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcResponse {
    Ok {
        jsonrpc: String,
        id: u64,
        result: Value,
    },
    Err {
        jsonrpc: String,
        id: u64,
        error: JsonRpcError,
    },
    // notifications have no id
    Notification {
        jsonrpc: String,
        method: String,
        #[serde(default)]
        params: Value,
    },
}

/// JSON-RPC 2.0 error object
#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Value,
}

/// MCP `initialize` result (server→client, capability negotiation)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: Value,
    pub server_info: ServerInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

/// MCP `tools/list` result
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListResult {
    pub tools: Vec<McpToolDef>,
}

/// Single tool definition from MCP server
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
}

/// MCP `tools/call` request params
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

/// MCP `tools/call` result
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCallResult {
    pub content: Vec<ContentBlock>,
}

/// A content block in a tools/call result
#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: String,
}

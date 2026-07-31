//! #10 · retrieve_tool_result — 按 tool_call_id 定向取回落盘的完整工具结果。
//!
//! 批量屏障截断工具输出时,完整结果由 `finalize_outcome` 落盘到
//! `<workspace>/.jia/tool-results/<session_id>/<tool_call_id>.txt`
//! (见 `disk_output::persist_tool_result`)。本工具提供分段翻页读取。
//!
//! 位识融合红线:落盘内容【不参与】熏习/召回 —— 工具结果 ≠ 记忆种子,
//! 仅在此按 id 定向取回(与种子分表)。

use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use super::disk_output;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// Default window size (64KB) and hard cap (1MB) per retrieve call.
const DEFAULT_MAX_BYTES: u64 = 64 * 1024;
const MAX_RETRIEVE_BYTES: u64 = 1024 * 1024;

pub struct RetrieveToolResultTool {}

impl Default for RetrieveToolResultTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RetrieveToolResultTool {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl BaseTool for RetrieveToolResultTool {
    fn name(&self) -> &str {
        "retrieve_tool_result"
    }

    fn description(&self) -> String {
        "Retrieve the full output of a previous tool call that was truncated \
         and persisted to disk (the truncated result tells you the \
         tool_call_id). Page through large outputs with offset/max_bytes."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        // 戊仪只读:仅读取本会话落盘目录,无任何副作用。
        CeremoniesIntent::Wu
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn accesses(&self, _input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // Reads only the internal per-session spill dir (recursive); no tool
        // writes there, so read-read concurrency is safe.
        crate::palaces::zhen_tool::base::ToolAccesses::read_only(
            vec![std::path::PathBuf::from(".jia/tool-results")],
            true,
        )
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_call_id": {
                    "type": "string",
                    "description": "The tool_call_id whose full output was persisted (shown in the truncation notice)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset to start reading from (default 0; use the next offset from a previous call to continue paging)"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes to read in this call (default 65536, capped at 1048576)"
                }
            },
            "required": ["tool_call_id"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let tool_call_id = input["tool_call_id"]
            .as_str()
            .ok_or("Missing 'tool_call_id' parameter")?;
        if tool_call_id.is_empty() {
            return Err("Empty 'tool_call_id' parameter".into());
        }
        let offset = input["offset"].as_u64().unwrap_or(0);
        let max_bytes = input["max_bytes"]
            .as_u64()
            .unwrap_or(DEFAULT_MAX_BYTES)
            .clamp(1, MAX_RETRIEVE_BYTES);

        // 本会话落盘目录(内部路径,与 backups 同约定,不走 verify_path)。
        let root = &ctx.permissions.sandbox.workspace_root;
        let path = disk_output::tool_result_path(root, &ctx.session_id, tool_call_id);

        if !path.exists() {
            let available = disk_output::list_tool_result_ids(root, &ctx.session_id);
            return Err(if available.is_empty() {
                format!(
                    "No persisted tool result for tool_call_id '{tool_call_id}' — \
                     this session has no persisted results at all. Only tool \
                     outputs that exceeded the output budget are persisted."
                )
            } else {
                format!(
                    "No persisted tool result for tool_call_id '{tool_call_id}'. \
                     Available ids this session: {}",
                    available.join(", ")
                )
            }
            .into());
        }

        let (content, new_offset, total) =
            disk_output::read_tool_result_window(&path, offset, max_bytes).map_err(ToolError::from)?;

        let mut out = format!(
            "[tool_call_id '{tool_call_id}' — bytes {offset}..{new_offset} of {total}]\n{content}"
        );
        if new_offset < total {
            out.push_str(&format!(
                "\n[{} bytes remaining — call again with offset={new_offset}]",
                total - new_offset
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::kun_config::SecuritySection;
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;

    /// ExecContext rooted at a temp workspace, session "s1".
    fn test_ctx(root: &std::path::Path) -> ExecContext {
        let matrix = PermissionMatrix::from_config(
            &SecuritySection::default(),
            root,
            root.join(".jia/backups"),
        );
        let mut ctx = ExecContext::new(Arc::new(matrix));
        ctx.session_id = "s1".to_string();
        ctx
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn retrieve_segmented_paging() {
        let dir = tempdir();
        let ctx = test_ctx(dir.path());
        let content: String = (0..1000).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        disk_output::persist_tool_result(dir.path(), "s1", "call_1", &content).unwrap();

        let tool = RetrieveToolResultTool::new();

        // First page.
        let out1 = tool
            .execute(
                serde_json::json!({"tool_call_id": "call_1", "offset": 0, "max_bytes": 400}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out1.contains("bytes 0..400 of 1000"), "header: {out1}");
        assert!(out1.contains(&content[..400]), "page content: {out1}");
        assert!(out1.contains("offset=400"), "continuation hint: {out1}");

        // Second page via the reported offset.
        let out2 = tool
            .execute(
                serde_json::json!({"tool_call_id": "call_1", "offset": 400, "max_bytes": 400}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out2.contains("bytes 400..800 of 1000"), "header: {out2}");
        assert!(out2.contains(&content[400..800]), "page content: {out2}");

        // Tail page: no continuation hint.
        let out3 = tool
            .execute(
                serde_json::json!({"tool_call_id": "call_1", "offset": 800, "max_bytes": 400}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out3.contains("bytes 800..1000 of 1000"), "header: {out3}");
        assert!(out3.contains(&content[800..]), "page content: {out3}");
        assert!(!out3.contains("remaining"), "no hint at EOF: {out3}");

        // Defaults: offset=0, max_bytes=64K.
        let out4 = tool
            .execute(serde_json::json!({"tool_call_id": "call_1"}), &ctx)
            .await
            .unwrap();
        assert!(out4.contains("bytes 0..1000 of 1000"), "header: {out4}");
    }

    #[tokio::test]
    async fn retrieve_unknown_id_lists_available() {
        let dir = tempdir();
        let ctx = test_ctx(dir.path());
        disk_output::persist_tool_result(dir.path(), "s1", "call_a", "x").unwrap();
        disk_output::persist_tool_result(dir.path(), "s1", "call_b", "y").unwrap();

        let tool = RetrieveToolResultTool::new();
        let err = tool
            .execute(serde_json::json!({"tool_call_id": "call_zzz"}), &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("call_zzz"), "error: {err}");
        assert!(err.contains("call_a") && err.contains("call_b"), "error: {err}");
    }

    #[tokio::test]
    async fn retrieve_unknown_id_no_results_this_session() {
        let dir = tempdir();
        let ctx = test_ctx(dir.path());
        let tool = RetrieveToolResultTool::new();
        let err = tool
            .execute(serde_json::json!({"tool_call_id": "call_x"}), &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("no persisted results"), "error: {err}");
    }

    #[tokio::test]
    async fn retrieve_session_isolation() {
        let dir = tempdir();
        disk_output::persist_tool_result(dir.path(), "other_session", "call_1", "secret").unwrap();
        // Session s1 must not see another session's spill files.
        let ctx = test_ctx(dir.path());
        let tool = RetrieveToolResultTool::new();
        let err = tool
            .execute(serde_json::json!({"tool_call_id": "call_1"}), &ctx)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("call_1"), "error: {err}");
    }

    #[tokio::test]
    async fn retrieve_missing_params() {
        let dir = tempdir();
        let ctx = test_ctx(dir.path());
        let tool = RetrieveToolResultTool::new();
        assert!(
            tool.execute(serde_json::json!({}), &ctx).await.is_err(),
            "missing tool_call_id must error"
        );
    }
}

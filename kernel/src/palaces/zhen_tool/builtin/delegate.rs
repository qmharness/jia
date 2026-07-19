use crate::error::ToolError;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use crate::palaces::gen_store::Store;
use crate::palaces::zhen_tool::base::BaseTool;

use crate::palaces::zhen_tool::registry::ToolRegistry;
use crate::palaces::zhong_core::JiaCore;
use crate::stems::action::ExecContext;
use crate::stems::CeremoniesIntent;
use crate::stems::parse_tool_calls;
use crate::types::{Message, Role};

pub struct DelegateTool {
    core: Arc<JiaCore>,
    /// Read-only tools available to sub-agents
    subtools: Arc<ToolRegistry>,
    /// P8 · persisted sub-agent sessions for continuation via send_message.
    sessions: Arc<Mutex<HashMap<String, SubagentSession>>>,
    /// P1 · SQLite-backed persistence for crash recovery.
    store: Arc<Store>,
}

impl DelegateTool {
    pub fn new(
        core: Arc<JiaCore>,
        subtools: Arc<ToolRegistry>,
        store: Arc<Store>,
        sessions: Arc<Mutex<HashMap<String, SubagentSession>>>,
    ) -> Self {
        Self {
            core,
            subtools,
            sessions,
            store,
        }
    }
}

/// P8 · a persisted sub-agent session, continuable via `send_message`.
pub struct SubagentSession {
    /// Conversation messages (system prompt at [0]).
    pub messages: Vec<Message>,
    pub subagent_type: SubagentType,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentType {
    Explore,
    Plan,
}

impl SubagentType {
    pub(crate) fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "explore" => Ok(Self::Explore),
            "plan" => Ok(Self::Plan),
            other => Err(format!(
                "Unknown subagent_type '{}'. Use 'Explore' or 'Plan'.",
                other
            )),
        }
    }
}

/// Build the system prompt for a sub-agent, including available tools.
fn build_subagent_system(
    subagent_type: SubagentType,
    prompt: &str,
    subtools: &ToolRegistry,
) -> String {
    let mut tool_section = String::new();
    let tools = subtools.list_all();
    if !tools.is_empty() {
        tool_section.push_str("\n\n## Available Tools\n\n");
        tool_section.push_str(
            "You have access to the following read-only tools. To use a tool, output:\n\n",
        );
        tool_section.push_str(
            "<tool_call>\n{\"name\": \"tool_name\", \"parameters\": {...}}\n</tool_call>\n\n",
        );
        for tool in &tools {
            let schema = tool.parameters_schema();
            tool_section.push_str(&format!(
                "### {}\n{}\nParameters: {}\n\n",
                tool.name(),
                tool.description(),
                serde_json::to_string_pretty(&schema).unwrap_or_default()
            ));
        }
    }

    let role_instruction = match subagent_type {
        SubagentType::Explore => format!(
            "You are an Explore sub-agent. Research the codebase to answer the task. \
             Use available tools to read files and search the codebase. \
             Be thorough: look at multiple files, follow references, and trace logic.\n\n\
             Task: {prompt}\n\n\
             After researching, provide a detailed analysis with specific file paths and line numbers."
        ),
        SubagentType::Plan => format!(
            "You are a Plan sub-agent. Design an implementation plan for the task. \
             Use available tools to understand the existing codebase before planning. \
             Read relevant files to understand patterns and architecture.\n\n\
             Task: {prompt}\n\n\
             After researching, provide a step-by-step implementation plan with specific \
             file changes, architectural considerations, and dependencies."
        ),
    };

    format!("{role_instruction}{tool_section}")
}

#[async_trait]
impl BaseTool for DelegateTool {
    fn name(&self) -> &str {
        "delegate"
    }

    fn description(&self) -> String {
        "Delegate a multi-step analysis task to a sub-agent with actual tool execution. \
         Subagent types: 'Explore' for codebase research (reads files, searches code), \
         'Plan' for architectural design (researches then plans). \
         Sub-agents have read-only access to read_file, grep, web_search, and web_fetch tools. \
         They run multiple turns of reasoning and tool use before returning consolidated results."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        // Delegation is read-only (sub-agents only have read tools).
        // Wu ceremony ensures delegate is not blocked in planning mode.
        CeremoniesIntent::Wu
    }

    fn target_palace(&self, input: &Value) -> crate::palaces::Palace {
        match input["subagent_type"].as_str() {
            Some("Explore") | Some("Plan") => crate::palaces::Palace::Dui,
            _ => crate::palaces::Palace::Xun,
        }
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "description": "Type of sub-agent: 'Explore' for codebase research, 'Plan' for architectural planning"
                },
                "prompt": {
                    "type": "string",
                    "description": "The task description and instructions for the sub-agent"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum reasoning turns (default: 25)",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["subagent_type", "prompt"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        // P2 · Parallel: independent session creation, brief lock only for insert.
        true
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let subagent_type = SubagentType::from_str(
            input["subagent_type"]
                .as_str()
                .ok_or("Missing 'subagent_type' parameter")?,
        )?;

        let prompt = input["prompt"]
            .as_str()
            .ok_or("Missing 'prompt' parameter")?;

        let max_turns = input["max_turns"].as_u64().unwrap_or(25).min(50) as usize;

        let system_content = build_subagent_system(subagent_type, prompt, &self.subtools);
        let mut messages = vec![Message::text(Role::System, system_content)];

        let result =
            run_subagent_loop(&self.core, &self.subtools, &mut messages, max_turns, ctx).await?;

        // P8 · persist the sub-agent conversation for send_message continuation.
        let subagent_id = uuid::Uuid::new_v4().to_string();
        let now = crate::utils::unix_now();
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("session lock error: {e}"))?;
        // Light capacity gate: drop the least-recently-used session when full.
        if sessions.len() >= 64
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(k, _)| k.clone())
        {
            sessions.remove(&oldest);
        }
        sessions.insert(
            subagent_id.clone(),
            SubagentSession {
                messages: messages.clone(),
                subagent_type,
                created_at: now,
                last_used: now,
            },
        );

        // P1 · Persist sub-agent session to SQLite for crash recovery.
        if let Ok(json) = serde_json::to_string(&messages) {
            let _ = self.store.save_subagent_session(
                &subagent_id,
                &json,
                &format!("{:?}", subagent_type),
                now,
                now,
            );
        }

        Ok(format!(
            "Sub-agent {subagent_id} completed.\n\n{result}\n\n\
             To continue this sub-agent, call send_message with subagent_id=\"{subagent_id}\"."
        ))
    }
}

/// P8 · shared sub-agent turn loop. Mutates `messages` in place (appends
/// assistant turns + tool results); returns the accumulated assistant text.
/// Used by both `DelegateTool` (fresh session) and `SendMessageTool` (continuation).
async fn run_subagent_loop(
    core: &JiaCore,
    subtools: &ToolRegistry,
    messages: &mut Vec<Message>,
    max_turns: usize,
    exec_ctx: &ExecContext,
) -> Result<String, String> {
    let mut total_response = String::new();

    // P1 · scratchpad-based progress reporting: after each turn, write
    // a progress summary to `~/.jia/scratchpad/subagent_{id}.md` so the
    // parent agent (or user) can monitor progress via the scratchpad tool.
    // Key is stable across calls so the parent can poll it.
    let progress_key = format!(
        "subagent_{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );

    for turn in 0..max_turns {
        // P0-4 · 子代理响应父级取消:每轮 LLM 调用前检查。
        if exec_ctx.cancel_token.is_cancelled() {
            tracing::info!(turn, "Sub-agent cancelled by parent run");
            return Err("Sub-agent cancelled".into());
        }
        tracing::debug!(turn = turn + 1, max = max_turns, "Sub-agent turn");

        let mut stream = core.infer(
            messages.clone(),
            None,
            Some(exec_ctx.cancel_token.clone()),
        );
        let mut full_response = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta)) => {
                    full_response.push_str(&delta)
                }
                Err(e) => return Err(format!("Sub-agent inference error: {e}")),
                _ => {}
            }
        }

        let subtool_names: Vec<&str> = subtools.list_names().iter().map(|s| s.as_str()).collect();
        let (clean_text, tool_calls) = parse_tool_calls(&full_response, &subtool_names);
        total_response.push_str(&clean_text);

        if tool_calls.is_empty() {
            break;
        }

        messages.push(Message::text(Role::Assistant, full_response.clone()));

        for tc in &tool_calls {
            let tool = match subtools.get(&tc.name) {
                Some(t) => t.clone(),
                None => {
                    messages.push(Message::text(
                        Role::User,
                        format!("Error: Sub-agent: unknown tool '{}'", tc.name),
                    ));
                    continue;
                }
            };
            // Subagent safety gate: reject destructive tools.
            // Subagents run in a loop without GeJu/hook/planning-mode checks.
            // Destructive operations (shell, write, patch, computer_use) are
            // always denied here; subagents should only use read/observe tools.
            if tool.is_destructive() {
                messages.push(Message::text(
                    Role::User,
                    format!(
                        "Error: Explore subagent cannot use destructive tool '{}'. Use a different subagent_type.",
                        tc.name
                    ),
                ));
                continue;
            }
            match tool.execute(tc.parameters.clone(), exec_ctx).await {
                Ok(output) => messages.push(Message::text(Role::User, output)),
                Err(e) => messages.push(Message::text(Role::User, format!("Error: {e}"))),
            }
        }

        // P1 · Scratchpad progress: write turn summary so parent can poll.
        let _ = std::fs::create_dir_all(
            crate::palaces::kun_config::default_data_dir().join("scratchpad"),
        );
        let progress_file = crate::palaces::kun_config::default_data_dir()
            .join("scratchpad")
            .join(format!("{progress_key}.md"));
        let summary = format!(
            "## Sub-agent turn {} / {}\n\n### Response\n{}\n\n{} tools called\n",
            turn + 1,
            max_turns,
            &crate::utils::truncate_chars(&clean_text, 500),
            tool_calls.len(),
        );
        let _ = std::fs::write(&progress_file, &summary);
    }

    if total_response.is_empty() {
        return Err("Sub-agent returned empty response".into());
    }
    Ok(total_response)
}

/// P8 · Continue a previously delegated sub-agent by id (SendMessage pattern).
///
/// Loads the persisted conversation, appends a new user message, runs another
/// turn loop with the same JiaCore + read-only subtools, and stores the
/// updated conversation back. 戊仪 (Wu, read-only) — subtools are read-only, so
/// this is safe in plan mode (C6). NOTE: if writable subtools are ever
/// introduced, send_message must propagate InteractionMode::Planning to the
/// sub-agent to avoid bypassing plan mode.
pub struct SendMessageTool {
    core: Arc<JiaCore>,
    subtools: Arc<ToolRegistry>,
    sessions: Arc<Mutex<HashMap<String, SubagentSession>>>,
}

impl SendMessageTool {
    pub fn new(
        core: Arc<JiaCore>,
        subtools: Arc<ToolRegistry>,
        sessions: Arc<Mutex<HashMap<String, SubagentSession>>>,
    ) -> Self {
        Self {
            core,
            subtools,
            sessions,
        }
    }
}

#[async_trait]
impl BaseTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> String {
        "Continue a previously delegated sub-agent (identified by subagent_id \
         returned from delegate) with a follow-up message. The sub-agent keeps \
         its full prior context. Use this to refine or extend a sub-agent's \
         analysis without re-delegating from scratch."
            .to_string()
    }

    fn category(&self) -> &str {
        "agent"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        // Read-only continuation (subtools are read-only). Wu → is_destructive=false,
        // so plan mode permits it (C6).
        CeremoniesIntent::Wu
    }

    fn target_palace(&self, _input: &Value) -> crate::palaces::Palace {
        crate::palaces::Palace::Dui
    }

    fn is_concurrency_safe(&self) -> bool {
        // P2 · Parallel: independent session creation, brief lock only for insert.
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_id": {
                    "type": "string",
                    "description": "The subagent_id returned by a prior delegate call"
                },
                "message": {
                    "type": "string",
                    "description": "The follow-up message for the sub-agent"
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Maximum reasoning turns (default: 25)",
                    "minimum": 1,
                    "maximum": 50
                }
            },
            "required": ["subagent_id", "message"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let subagent_id = input["subagent_id"]
            .as_str()
            .ok_or("Missing 'subagent_id' parameter")?;
        let message = input["message"]
            .as_str()
            .ok_or("Missing 'message' parameter")?;
        let max_turns = input["max_turns"].as_u64().unwrap_or(25).min(50) as usize;

        // Load the session (clone messages out so we don't hold the lock during inference)
        let mut messages = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| format!("session lock error: {e}"))?;
            let session = sessions
                .get_mut(subagent_id)
                .ok_or_else(|| format!("Unknown subagent_id '{subagent_id}'"))?;
            session.last_used = crate::utils::unix_now();
            session.messages.clone()
        };

        messages.push(Message::text(Role::User, message.to_string()));
        let result =
            run_subagent_loop(&self.core, &self.subtools, &mut messages, max_turns, ctx).await?;

        // Store the updated conversation back
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|e| format!("session lock error: {e}"))?;
            if let Some(session) = sessions.get_mut(subagent_id) {
                session.messages = messages;
                session.last_used = crate::utils::unix_now();
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;
    fn test_ctx() -> crate::stems::action::ExecContext {
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;
    use crate::palaces::zhen_tool::builtin;
    use crate::stems::action::ExecContext;

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

    fn test_core() -> Arc<JiaCore> {
        use crate::palaces::kun_config::ProviderProfile;
        let profile = ProviderProfile {
            kind: "openai".into(),
            models: vec!["gpt-4".into()],
            default_aux_model: None,
            default_main_model: None,
            api_key: "sk-test".into(),
            base_url: "https://api.openai.com/v1".into(),
            max_tokens: Some(256),
            context_window: None,
            priority: None,
            cost_multiplier: None,
        };
        Arc::new(JiaCore::new(&profile, profile.default_main_model()))
    }

    fn test_subtools() -> Arc<ToolRegistry> {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(builtin::read_file::ReadFileTool::new()));
        reg.register(Arc::new(builtin::grep::GrepTool::new()));
        Arc::new(reg)
    }

    fn test_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        // Leak tempdir for test lifetime
        std::mem::forget(dir);
        Arc::new(Store::open(path.to_str().unwrap()))
    }

    fn test_sessions() -> Arc<Mutex<HashMap<String, SubagentSession>>> {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[test]
    fn test_subagent_type_from_str() {
        assert!(matches!(
            SubagentType::from_str("Explore"),
            Ok(SubagentType::Explore)
        ));
        assert!(matches!(
            SubagentType::from_str("PLAN"),
            Ok(SubagentType::Plan)
        ));
        assert!(SubagentType::from_str("invalid").is_err());
    }

    #[tokio::test]
    async fn delegate_missing_params() {
        let tool = DelegateTool::new(test_core(), test_subtools(), test_store(), test_sessions());
        assert!(
            tool.execute(serde_json::json!({}), &test_ctx())
                .await
                .is_err()
        );
        assert!(
            tool.execute(serde_json::json!({"subagent_type": "Explore"}), &test_ctx())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delegate_unknown_type() {
        let tool = DelegateTool::new(test_core(), test_subtools(), test_store(), test_sessions());
        let result = tool
            .execute(
                serde_json::json!({
                    "subagent_type": "invalid",
                    "prompt": "test"
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown subagent_type")
        );
    }

    /// P8 · delegate stores a session and returns a subagent_id (mock core, no
    /// tool calls → loop ends after one turn).
    #[tokio::test]
    async fn delegate_stores_session_and_returns_id() {
        let core = Arc::new(crate::palaces::zhong_core::JiaCore::with_mock(vec![
            "analysis: found X".to_string(),
        ]));
        let sessions = test_sessions();
        let tool = DelegateTool::new(core, test_subtools(), test_store(), sessions.clone());
        let res = tool
            .execute(
                serde_json::json!({
                    "subagent_type": "Explore",
                    "prompt": "find X"
                }),
                &test_ctx(),
            )
            .await;
        assert!(res.is_ok(), "delegate failed: {:?}", res.err());
        let out = res.unwrap();
        assert!(
            out.to_string().contains("Sub-agent "),
            "expected id in output: {out}"
        );
        assert_eq!(
            sessions.lock().unwrap().len(),
            1,
            "session should be stored"
        );
    }

    /// P8 · send_message continues a stored session with a follow-up.
    #[tokio::test]
    async fn send_message_continues_session() {
        let core = Arc::new(crate::palaces::zhong_core::JiaCore::with_mock(vec![
            "initial analysis".to_string(),
        ]));
        let sessions = test_sessions();
        let delegate = DelegateTool::new(core, test_subtools(), test_store(), sessions.clone());
        let out = delegate
            .execute(
                serde_json::json!({
                    "subagent_type": "Explore",
                    "prompt": "p"
                }),
                &test_ctx(),
            )
            .await
            .unwrap();
        let id = out
            .split("subagent_id=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("subagent_id in output");

        // Continue with a fresh mock core that returns a follow-up answer.
        let core2 = Arc::new(crate::palaces::zhong_core::JiaCore::with_mock(vec![
            "follow-up answer".to_string(),
        ]));
        let sm = SendMessageTool::new(core2, test_subtools(), sessions.clone());
        let res = sm
            .execute(
                serde_json::json!({
                    "subagent_id": id,
                    "message": "more?"
                }),
                &test_ctx(),
            )
            .await;
        assert!(res.is_ok(), "send_message failed: {:?}", res.err());
        assert_eq!(res.unwrap(), "follow-up answer");
    }

    #[tokio::test]
    async fn send_message_unknown_id() {
        let sm = SendMessageTool::new(test_core(), test_subtools(), test_sessions());
        let res = sm
            .execute(
                serde_json::json!({
                    "subagent_id": "nonexistent",
                    "message": "x"
                }),
                &test_ctx(),
            )
            .await;
        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("Unknown subagent_id"));
    }

    // ── P0-4 · 子代理可取消 ────────────────────────────────────

    use crate::error::ProviderError;
    use crate::palaces::zhong_core::{LlmProvider, StreamChunk};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Provider that always answers with a read_file tool_call (so the
    /// sub-agent loop would otherwise run to max_turns), and cancels the
    /// given token on the Nth invocation.
    struct CancellingProvider {
        calls: Arc<AtomicUsize>,
        cancel_on_call: usize,
        token: tokio_util::sync::CancellationToken,
    }

    impl LlmProvider for CancellingProvider {
        fn infer_stream(
            &self,
            _messages: Vec<Message>,
            _tools: Option<&[crate::stems::action::ToolSchema]>,
            _cancel_token: Option<tokio_util::sync::CancellationToken>,
        ) -> std::pin::Pin<
            Box<dyn futures::Stream<Item = Result<StreamChunk, ProviderError>> + Send>,
        > {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.cancel_on_call {
                self.token.cancel();
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                let text = r#"<tool_call>
{"name": "read_file", "parameters": {"file_path": "/nonexistent-p0-4"}}
</tool_call>"#;
                let _ = tx.send(Ok(StreamChunk::Delta(text.to_string())));
            });
            Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
        }
    }

    /// P0-4 · 子代理运行中取消 → 提前退出(轮数远小于 50),返回取消错误。
    #[tokio::test]
    async fn delegate_cancel_stops_subagent_early() {
        let calls = Arc::new(AtomicUsize::new(0));
        let token = tokio_util::sync::CancellationToken::new();
        let provider: Box<dyn LlmProvider> = Box::new(CancellingProvider {
            calls: calls.clone(),
            cancel_on_call: 2,
            token: token.clone(),
        });
        let router = crate::palaces::zhong_core::ProviderRouter::new(vec![(0u32, provider)]);
        let core = Arc::new(JiaCore::with_router(router, "mock".into(), "mock".into(), 8192));

        let tool = DelegateTool::new(core, test_subtools(), test_store(), test_sessions());
        let mut ctx = test_ctx();
        ctx.cancel_token = token;

        let res = tool
            .execute(
                serde_json::json!({
                    "subagent_type": "Explore",
                    "prompt": "loop forever",
                    "max_turns": 50
                }),
                &ctx,
            )
            .await;

        let n = calls.load(Ordering::SeqCst);
        assert!(
            n < 50,
            "cancelled sub-agent must exit early, not run to max_turns (calls={n})"
        );
        assert!(res.is_err(), "cancelled sub-agent must return an error");
        assert!(
            res.unwrap_err().to_string().contains("cancel"),
            "error should mention cancellation"
        );
    }
}

//! hooks — P4 用户可配置门规钩子 (P2-2 自天盘 loop_hooks 下沉)
//!
//! 哲学依据:CompiledHook 的编译产物由地盘装配期持有
//! (EarthPlate.user_hooks,一局不变),消费在天盘 dispatch —— 跨盘
//! 共享的编译期语义,归天干层。运行函数 run_pre_tool_hooks 是纯
//! 子进程执行(无盘依赖),随类型一并居此。

/// P4 · a user-configurable hook compiled at startup (regex pre-compiled).
#[derive(Debug, Clone)]
pub struct CompiledHook {
    pub event: UserHookEvent,
    pub tool_pattern: Option<regex::Regex>,
    pub command: String,
    pub block_on_exit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserHookEvent {
    /// 人盘门规 — runs synchronously in the loop after GeJu, before dispatch;
    /// may block the tool (白虎守门).
    PreToolUse,
    /// 神盘观测 — observation only, never blocks.
    PostToolUse,
}

impl CompiledHook {
    /// Compile a `HookConfig` into a `CompiledHook` (pre-compiles the regex).
    pub fn compile(cfg: &crate::palaces::kun_config::HookConfig) -> Result<Self, String> {
        let event = match cfg.event.as_str() {
            "pre_tool_use" => UserHookEvent::PreToolUse,
            "post_tool_use" => UserHookEvent::PostToolUse,
            other => return Err(format!("unknown hook event: {other}")),
        };
        let tool_pattern = match cfg.tool_pattern.as_deref() {
            Some(p) if !p.is_empty() => Some(
                regex::Regex::new(p).map_err(|e| format!("bad hook tool_pattern '{p}': {e}"))?,
            ),
            _ => None,
        };
        Ok(Self {
            event,
            tool_pattern,
            command: cfg.command.clone(),
            block_on_exit: cfg.block_on_exit,
        })
    }

    pub(crate) fn matches_tool(&self, tool_name: &str) -> bool {
        match &self.tool_pattern {
            Some(p) => p.is_match(tool_name),
            None => true,
        }
    }
}

/// Run pre-tool-use hooks (人盘门规). Returns `Err(reason)` if a hook blocks
/// the tool (non-zero exit with `block_on_exit`). Runs the configured command
/// via `sh -c`, passing context in `JIA_HOOK_CONTEXT` (JSON) and `JIA_HOOK_TOOL`.
/// Blocking subprocess work is offloaded to `spawn_blocking` so the async
/// runtime is not stalled.
pub async fn run_pre_tool_hooks(
    hooks: &[CompiledHook],
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<(), String> {
    let ctx = serde_json::json!({ "tool": tool_name, "input": input }).to_string();
    for h in hooks {
        if h.event != UserHookEvent::PreToolUse || !h.matches_tool(tool_name) {
            continue;
        }
        let cmd = h.command.clone();
        let tool = tool_name.to_string();
        let ctx_clone = ctx.clone();
        let status_res = tokio::task::spawn_blocking(move || {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .env("JIA_HOOK_CONTEXT", &ctx_clone)
                .env("JIA_HOOK_TOOL", &tool)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
        })
        .await
        .map_err(|e| format!("hook join error: {e}"))?;

        let success = match status_res {
            Ok(s) => s.success(),
            Err(e) => {
                tracing::warn!(hook = %h.command, error = %e, "pre-tool hook failed to run");
                // A hook that fails to spawn does NOT block (fail-open for
                // misconfigured hooks); only an explicit non-zero exit blocks.
                true
            }
        };
        if h.block_on_exit && !success {
            return Err(format!(
                "Tool '{}' blocked by pre-tool-use hook (人盘门规): {}",
                tool_name, h.command
            ));
        }
    }
    Ok(())
}

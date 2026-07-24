// ── Git Tool — Execute safe git commands ─────────────────────

use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// Safe git subcommands (read-only or non-destructive).
const ALLOWED_COMMANDS: &[&str] = &[
    "status", "diff", "log", "branch", "add", "commit", "checkout", "stash", "show", "blame", "tag",
];

/// Dangerous patterns. Only `--no-index` is reachable: it lets `git diff`
/// operate on files outside a git repository and is read-only.
/// 拦截理由(勿删):`git diff --no-index <a> <b>` 可读取 workspace_root 之外
/// 的任意文件并把内容回显给模型——这是沙箱逃逸向量,必须拦截。
/// The other historically listed patterns (`push --force`, `reset --hard`,
/// `clean -f`, `clean -d`) are subcommands that are already rejected because
/// `push`, `reset`, and `clean` are not in ALLOWED_COMMANDS, making this
/// check dead.
const DANGEROUS_PATTERNS: &[&str] = &["--no-index"];

pub struct GitTool;

impl Default for GitTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BaseTool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> String {
        "Execute a git subcommand. Allowed: status, diff, log, branch, add, commit, checkout, stash, show, blame, tag. Dangerous operations (push --force, reset --hard, clean -f) are blocked.".to_string()
    }

    fn category(&self) -> &str {
        "system"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Geng
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subcommand": {
                    "type": "string",
                    "description": "Git subcommand to execute, e.g. 'status', 'diff', 'log --oneline -5'"
                }
            },
            "required": ["subcommand"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let subcmd = input["subcommand"]
            .as_str()
            .ok_or("Missing 'subcommand' parameter")?;

        let first_word = subcmd.split_whitespace().next().unwrap_or("");
        if !ALLOWED_COMMANDS.contains(&first_word) {
            return Err(format!(
                "Git subcommand '{}' is not allowed. Allowed: {}",
                first_word,
                ALLOWED_COMMANDS.join(", "),
            )
            .into());
        }

        for pattern in DANGEROUS_PATTERNS {
            if subcmd.contains(pattern) {
                return Err(format!("Dangerous git operation '{}' is blocked.", pattern).into());
            }
        }

        let workspace_root = &ctx.permissions.sandbox.workspace_root;

        let output = tokio::process::Command::new("git")
            .args(subcmd.split_whitespace())
            .current_dir(workspace_root)
            .output()
            .await
            .map_err(|e| format!("git error: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            return Err(format!("git failed ({}): {}", output.status, stderr.trim()).into());
        }

        let out = stdout.trim().to_string();
        if out.is_empty() && !stderr.trim().is_empty() {
            Ok(stderr.trim().to_string())
        } else if out.is_empty() {
            Ok("(no output)".into())
        } else {
            Ok(out)
        }
    }
}

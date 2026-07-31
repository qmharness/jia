use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;

/// Output from a sandboxed command execution.
#[derive(Debug, Clone)]
pub struct SandboxOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// A pluggable execution sandbox.
///
/// Implementations range from simple process rlimits (ProcessSandbox) to
/// full OS-level containerization (Docker, Landlock, Seatbelt).
#[async_trait]
pub trait ExecutionSandbox: Send + Sync {
    /// Execute a shell command inside the sandbox.
    async fn execute(
        &self,
        cmd: &str,
        cwd: &Path,
        env: &HashMap<String, String>,
    ) -> Result<SandboxOutput, String>;

    /// Human-readable sandbox backend name (e.g., "process", "docker").
    fn name(&self) -> &str;
}

/// N3 · environment hardening injected into EVERY sandboxed child, on top of
/// whatever the caller passes. Centralized here so all four backends (Docker /
/// Landlock / Seatbelt / Process) and the unsandboxed fallback share it.
///
/// - `NO_COLOR=1` / `TERM=dumb`: no ANSI escapes, no alternate-screen — tool
///   output is fed to the LLM as plain text; control sequences only waste
///   tokens and corrupt parsing.
/// - `GIT_TERMINAL_PROMPT=0`: git must never block on a credential prompt
///   (child stdin is /dev/null, so a prompt would hang until timeout).
pub fn hardened_env() -> HashMap<String, String> {
    HashMap::from([
        ("NO_COLOR".to_string(), "1".to_string()),
        ("TERM".to_string(), "dumb".to_string()),
        ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardened_env_contains_expected_keys() {
        let env = hardened_env();
        assert_eq!(env.get("NO_COLOR").map(String::as_str), Some("1"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("dumb"));
        assert_eq!(
            env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
            Some("0")
        );
    }
}

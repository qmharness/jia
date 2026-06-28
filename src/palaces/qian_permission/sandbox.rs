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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;

use super::sandbox::{ExecutionSandbox, SandboxOutput};

/// Docker-based execution sandbox.
///
/// Runs each command in a fresh container with resource limits, network isolation,
/// and a read-only root filesystem. Requires Docker daemon access.
pub struct DockerSandbox {
    /// Docker image to use (e.g., "alpine:3.20").
    pub image: String,
    /// Command timeout.
    pub timeout: Duration,
    /// Memory limit for the container (docker `--memory` flag).
    pub memory_limit_mb: u64,
    /// CPU limit (docker `--cpus` flag, fractional allowed).
    pub cpus: f64,
    /// Max output file size in MB (tmpfs /tmp size).
    pub tmpfs_size_mb: u64,
    /// Allow network access. Default false.
    pub network_enabled: bool,
    /// Host directory to mount as /workspace (read-only for sandbox safety).
    pub workspace_dir: PathBuf,
}

impl Default for DockerSandbox {
    fn default() -> Self {
        Self {
            image: "alpine:3.20".into(),
            timeout: Duration::from_secs(30),
            memory_limit_mb: 512,
            cpus: 1.0,
            tmpfs_size_mb: 100,
            network_enabled: false,
            workspace_dir: PathBuf::new(),
        }
    }
}

#[async_trait]
impl ExecutionSandbox for DockerSandbox {
    fn name(&self) -> &str {
        "docker"
    }

    async fn execute(
        &self,
        cmd: &str,
        _cwd: &Path,
        _env: &HashMap<String, String>,
    ) -> Result<SandboxOutput, String> {
        let mut args: Vec<String> = vec!["run".into(), "--rm".into()];

        // Resource limits
        args.push(format!("--memory={}m", self.memory_limit_mb));
        args.push(format!("--cpus={}", self.cpus));

        // Network isolation
        if !self.network_enabled {
            args.push("--network=none".into());
        }

        // Read-only root FS with writable tmpfs /tmp
        args.push("--read-only".into());
        args.push(format!(
            "--tmpfs=/tmp:rw,noexec,nosuid,size={}m",
            self.tmpfs_size_mb
        ));

        // Security hardening
        args.push("--security-opt=no-new-privileges".into());

        // Mount workspace read-only
        if !self.workspace_dir.as_os_str().is_empty() {
            let mount = format!("{}:/workspace:ro", self.workspace_dir.display());
            args.push("-v".into());
            args.push(mount);
            args.push("-w".into());
            args.push("/workspace".into());
        }

        // Image + shell command
        args.push(self.image.clone());
        args.push("sh".into());
        args.push("-c".into());
        args.push(cmd.to_string());

        let output = tokio::process::Command::new("docker")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Docker sandbox error: {e}"))?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        Ok(SandboxOutput {
            stdout,
            stderr,
            exit_code,
        })
    }
}

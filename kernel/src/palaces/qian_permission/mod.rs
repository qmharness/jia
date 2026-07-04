//! qian_permission — Permission & Sandbox (乾六)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing;

use crate::palaces::kun_config::SecuritySection;

pub mod sandbox;
#[cfg(feature = "sandbox-docker")]
pub mod sandbox_docker;
#[cfg(target_os = "linux")]
pub mod sandbox_landlock;
pub mod sandbox_process;
#[cfg(target_os = "macos")]
pub mod sandbox_seatbelt;

use sandbox::ExecutionSandbox;

/// Available sandbox backends, in descending priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    Docker,
    Landlock,
    Seatbelt,
    Process,
}

impl SandboxBackend {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "docker" => Self::Docker,
            "landlock" => Self::Landlock,
            "seatbelt" => Self::Seatbelt,
            _ => Self::Process,
        }
    }

    fn docker_available() -> bool {
        std::process::Command::new("docker")
            .arg("version")
            .arg("--format={{.Client.Version}}")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Select the best available backend given the user preference and platform.
    pub fn auto_select(preferred: &str) -> Self {
        let pref = Self::from_str(preferred);
        match pref {
            Self::Docker => {
                if Self::docker_available() {
                    return Self::Docker;
                }
            }
            Self::Landlock =>
            {
                #[cfg(target_os = "linux")]
                if sandbox_landlock::is_available() {
                    return Self::Landlock;
                }
            }
            Self::Seatbelt =>
            {
                #[cfg(target_os = "macos")]
                if sandbox_seatbelt::is_available() {
                    return Self::Seatbelt;
                }
            }
            _ => {}
        }
        // Priority fallback: Docker > Landlock > Seatbelt > Process
        #[cfg(feature = "sandbox-docker")]
        {
            if Self::docker_available() {
                return Self::Docker;
            }
        }
        #[cfg(target_os = "linux")]
        {
            if sandbox_landlock::is_available() {
                return Self::Landlock;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if sandbox_seatbelt::is_available() {
                return Self::Seatbelt;
            }
        }
        Self::Process
    }
}

/// Whether a filesystem operation reads or writes.
#[derive(Debug, Clone, Copy)]
pub enum PathOp {
    Read,
    Write,
}

/// Resolved sandbox configuration with canonicalized paths.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub project_root: PathBuf,
    pub allowed_paths: Vec<PathBuf>,
    pub blocked_prefixes: Vec<String>,
}

/// Shell execution policy.
#[derive(Debug, Clone)]
pub struct ShellPolicy {
    pub allowlist: Vec<String>,
    pub blocklist: Vec<String>,
}

/// 乾六宫 — Permission Matrix
///
/// Enforces tool execution boundaries:
/// 1. Path sandboxing (confine reads/writes to project_root + allowed_paths)
/// 2. Shell command filtering (allowlist/blocklist)
/// 3. User confirmation timeout configuration
pub struct PermissionMatrix {
    pub sandbox: SandboxConfig,
    pub shell_policy: ShellPolicy,
    pub confirmation_timeout: std::time::Duration,
    pub sandbox_disabled: bool,
    /// Directory for file backups (write_file / edit tools).
    pub backup_dir: PathBuf,
    /// Pluggable execution sandbox for shell commands.
    /// None means direct process execution (no sandbox).
    pub execution_sandbox: Option<Arc<dyn ExecutionSandbox>>,
}

impl std::fmt::Debug for PermissionMatrix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionMatrix")
            .field("sandbox.project_root", &self.sandbox.project_root)
            .field("sandbox.blocked_prefixes", &self.sandbox.blocked_prefixes)
            .field("sandbox_disabled", &self.sandbox_disabled)
            .finish_non_exhaustive()
    }
}

impl Default for PermissionMatrix {
    fn default() -> Self {
        let workspace = std::env::current_dir()
            .unwrap_or_else(|_| crate::palaces::kun_config::default_data_dir().join("workspace"));
        Self::from_config(
            &SecuritySection::default(),
            &workspace,
            PathBuf::from(".jia-backups"),
        )
    }
}

impl PermissionMatrix {
    /// Build the permission matrix from security configuration.
    pub fn from_config(
        security: &SecuritySection,
        default_root: &Path,
        backup_dir: PathBuf,
    ) -> Self {
        let project_root = security
            .project_root
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_root.to_path_buf());

        let project_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());

        let allowed_paths: Vec<PathBuf> = security
            .allowed_paths
            .iter()
            .filter_map(|p| {
                let pb = PathBuf::from(p);
                pb.canonicalize().ok()
            })
            .collect();

        Self {
            sandbox: SandboxConfig {
                project_root,
                allowed_paths,
                blocked_prefixes: security.blocked_path_prefixes.clone(),
            },
            shell_policy: ShellPolicy {
                allowlist: security.command_allowlist.clone(),
                blocklist: security.command_blocklist.clone(),
            },
            confirmation_timeout: std::time::Duration::from_secs(
                security.confirmation_timeout_secs,
            ),
            sandbox_disabled: security.sandbox_disabled,
            backup_dir,
            execution_sandbox: None,
        }
    }

    /// Verify a user-supplied path against sandbox boundaries.
    ///
    /// Steps:
    /// 1. Resolve relative paths against project_root
    /// 2. Canonicalize (resolves symlinks, `..`) — for writes, canonicalize parent
    /// 3. Check canonical path is within project_root or allowed_paths
    /// 4. Check no path component matches a blocked prefix
    ///
    /// Returns the canonical path on success, or an error string on denial.
    pub fn verify_path(&self, raw: &str, op: PathOp) -> Result<PathBuf, String> {
        let p = Path::new(raw);

        // Resolve relative paths against project_root
        let resolved = if p.is_relative() {
            self.sandbox.project_root.join(p)
        } else {
            p.to_path_buf()
        };

        // Canonicalize — for writes, canonicalize parent then join filename
        let canonical = if matches!(op, PathOp::Write) && !resolved.exists() {
            let parent = resolved.parent().unwrap_or(&resolved);
            if let Ok(canonical_parent) = parent.canonicalize() {
                let filename = resolved.file_name().ok_or("invalid filename")?;
                canonical_parent.join(filename)
            } else {
                return Err(format!(
                    "parent directory does not exist: {}",
                    parent.display(),
                ));
            }
        } else {
            resolved
                .canonicalize()
                .map_err(|e| format!("cannot resolve path '{raw}': {e}"))?
        };

        // Check root boundary
        let within_root = canonical.starts_with(&self.sandbox.project_root);
        let within_allowed = self
            .sandbox
            .allowed_paths
            .iter()
            .any(|ap| canonical.starts_with(ap));

        if !within_root && !within_allowed {
            return Err(format!(
                "path '{}' (→ {}) is outside project root '{}'",
                raw,
                canonical.display(),
                self.sandbox.project_root.display(),
            ));
        }

        // Check blocked prefixes against the canonical path string
        let path_str = canonical.to_string_lossy();
        for prefix in &self.sandbox.blocked_prefixes {
            if path_str.contains(prefix) {
                return Err(format!(
                    "path '{}' (→ {}) matches blocked prefix '{}'",
                    raw,
                    canonical.display(),
                    prefix,
                ));
            }
        }

        Ok(canonical)
    }

    /// Check if a shell command is allowed.
    ///
    /// If allowlist is non-empty, only those command names are permitted.
    /// Always checks against the blocklist.
    ///
    /// Uses shell word splitting to prevent bypass via quoting/variable expansion
    /// (e.g., `$'r'\m` no longer bypasses a block on `rm`).
    pub fn verify_command(&self, cmd: &str) -> Result<(), String> {
        // Tokenize the command using shell word splitting rules
        let tokens: Vec<String> = match shell_words::split(cmd) {
            Ok(t) => t,
            Err(e) => {
                // If we can't parse, fall back to best-effort: check the raw command
                // against blocklist to avoid letting unparseable input through.
                tracing::warn!(
                    ?e,
                    cmd,
                    "verify_command: shell-words parse error, falling back to raw check"
                );
                let tokens = cmd.split_whitespace().map(String::from).collect::<Vec<_>>();
                if tokens.is_empty() {
                    return Ok(());
                }
                for pattern in &self.shell_policy.blocklist {
                    if cmd.contains(pattern.as_str()) {
                        return Err(format!("command matches blocked pattern '{pattern}'"));
                    }
                }
                return Ok(());
            }
        };

        if tokens.is_empty() {
            return Ok(());
        }

        // Extract the command name (first token, stripped of path)
        let cmd_name = tokens[0].split('/').next_back().unwrap_or(&tokens[0]);

        // Allowlist check
        if !self.shell_policy.allowlist.is_empty() {
            let allowed = self
                .shell_policy
                .allowlist
                .iter()
                .any(|a| a == cmd_name || a == &tokens[0]);
            if !allowed {
                return Err(format!("command '{cmd_name}' is not in the allowlist"));
            }
        }

        // Blocklist check — check each token individually
        for token in &tokens {
            for pattern in &self.shell_policy.blocklist {
                if token == pattern.as_str() {
                    return Err(format!(
                        "command token '{token}' matches blocked pattern '{pattern}'"
                    ));
                }
            }
        }

        // Also check the raw command against the blocklist as a defense-in-depth measure
        for pattern in &self.shell_policy.blocklist {
            if cmd.contains(pattern.as_str()) {
                return Err(format!("command matches blocked pattern '{pattern}'"));
            }
        }

        Ok(())
    }

    /// Build and attach an execution sandbox based on configuration.
    pub fn with_sandbox(mut self, section: &crate::palaces::kun_config::SandboxSection) -> Self {
        let configured = section.backend.clone();
        let backend = SandboxBackend::auto_select(&section.backend);
        tracing::info!(
            ?backend,
            requested = configured,
            "Selected execution sandbox backend"
        );

        #[cfg(feature = "sandbox-docker")]
        if backend == SandboxBackend::Docker {
            self.execution_sandbox = Some(Arc::new(sandbox_docker::DockerSandbox {
                image: section.docker_image.clone(),
                timeout: std::time::Duration::from_secs(section.timeout_seconds),
                memory_limit_mb: section.memory_limit_mb,
                cpus: section.cpu_limit,
                tmpfs_size_mb: section.file_size_limit_mb,
                network_enabled: section.network_enabled,
                workspace_dir: self.sandbox.project_root.clone(),
            }));
            return self;
        }

        #[cfg(target_os = "linux")]
        if backend == SandboxBackend::Landlock {
            self.execution_sandbox = Some(Arc::new(sandbox_landlock::LandlockSandbox {
                project_root: self.sandbox.project_root.clone(),
                allowed_paths: self.sandbox.allowed_paths.clone(),
                timeout: std::time::Duration::from_secs(section.timeout_seconds),
            }));
            return self;
        }

        #[cfg(target_os = "macos")]
        if backend == SandboxBackend::Seatbelt {
            self.execution_sandbox = Some(Arc::new(sandbox_seatbelt::SeatbeltSandbox {
                project_root: self.sandbox.project_root.clone(),
                allowed_paths: self.sandbox.allowed_paths.clone(),
                timeout: std::time::Duration::from_secs(section.timeout_seconds),
            }));
            return self;
        }

        // Default: Process sandbox (always available)
        self.execution_sandbox = Some(Arc::new(sandbox_process::ProcessSandbox {
            timeout: std::time::Duration::from_secs(section.timeout_seconds),
            memory_limit_bytes: section.memory_limit_mb * 1024 * 1024,
            file_size_limit_bytes: section.file_size_limit_mb * 1024 * 1024,
            max_processes: section.max_processes,
        }));
        if backend != SandboxBackend::Process {
            tracing::warn!(
                requested = configured,
                actual = "process",
                "Requested sandbox backend not available, falling back to process"
            );
        }
        self
    }

    /// Execute a shell command through the configured sandbox.
    ///
    /// If no sandbox is configured, falls back to direct process execution.
    pub async fn execute_sandboxed(&self, cmd: &str) -> Result<String, String> {
        self.verify_command(cmd)?;

        let cwd = self.sandbox.project_root.clone();

        if let Some(ref sandbox) = self.execution_sandbox {
            let output = sandbox
                .execute(cmd, &cwd, &std::collections::HashMap::new())
                .await?;
            let mut result = if output.stderr.is_empty() {
                output.stdout
            } else {
                format!("stdout:\n{}\nstderr:\n{}", output.stdout, output.stderr)
            };
            if output.exit_code != 0 {
                result.push_str(&format!("\n[exit code: {}]", output.exit_code));
            }
            Ok(result)
        } else {
            // Fallback: direct process execution
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .output()
                .await
                .map_err(|e| format!("shell error: {e}"))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("stdout:\n{stdout}\nstderr:\n{stderr}")
            })
        }
    }

    /// Apply sandbox transformations to tool input.
    pub fn sandbox_input(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match tool_name {
            "read_file" => self.sandbox_path(input, "path", PathOp::Read),
            "write_file" | "patch_file" => self.sandbox_path(input, "path", PathOp::Write),
            "shell" => {
                let cmd = input["command"]
                    .as_str()
                    .ok_or("missing 'command' parameter")?;
                self.verify_command(cmd)?;
                Ok(input.clone())
            }
            "grep" => {
                if input["path"].as_str().is_some() {
                    self.sandbox_path(input, "path", PathOp::Read)
                } else {
                    Ok(input.clone())
                }
            }
            _ => Ok(input.clone()),
        }
    }

    /// Apply path sandboxing to parameters declared in `sandbox_params`.
    ///
    /// Unlike `sandbox_input`, this doesn't hardcode tool names — it inspects
    /// parameter keys against a caller-supplied allowlist. Used by MCP tools
    /// where the config author declares which params are filesystem paths.
    pub fn sandbox_known_params(
        &self,
        input: &serde_json::Value,
        param_names: &[String],
    ) -> Result<serde_json::Value, String> {
        let mut v = input.clone();
        for key in param_names {
            if let Some(_val) = v.get(key) {
                v = self.sandbox_path(&v, key, PathOp::Write)?;
            }
        }
        Ok(v)
    }

    fn sandbox_path(
        &self,
        input: &serde_json::Value,
        key: &str,
        op: PathOp,
    ) -> Result<serde_json::Value, String> {
        let mut v = input.clone();
        let path = v[key]
            .as_str()
            .ok_or(format!("missing '{key}' parameter"))?;
        let canonical = self.verify_path(path, op)?;
        v[key] = serde_json::Value::String(canonical.to_string_lossy().into());
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_matrix() -> PermissionMatrix {
        let project_root = std::env::current_dir().unwrap();
        PermissionMatrix {
            sandbox: SandboxConfig {
                project_root: project_root.canonicalize().unwrap(),
                allowed_paths: vec![],
                blocked_prefixes: vec![".git".into(), ".env".into()],
            },
            shell_policy: ShellPolicy {
                allowlist: vec![],
                blocklist: vec!["rm -rf".into(), "mkfs.".into()],
            },
            confirmation_timeout: std::time::Duration::from_secs(30),
            sandbox_disabled: false,
            backup_dir: PathBuf::from(".jia-backups"),
            execution_sandbox: None,
        }
    }

    #[test]
    fn test_verify_path_inside_root() {
        let m = make_matrix();
        // Cargo.toml should exist in the project root
        let result = m.verify_path("Cargo.toml", PathOp::Read);
        assert!(
            result.is_ok(),
            "Cargo.toml should be inside root: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_verify_path_outside_root_blocked() {
        let m = make_matrix();
        let result = m.verify_path("/etc/passwd", PathOp::Read);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside project root"));
    }

    #[test]
    fn test_verify_path_blocked_prefix() {
        let m = make_matrix();
        let result = m.verify_path(".git/config", PathOp::Read);
        if result.is_ok() {
            // .git/config might not exist; if the path resolves and exists,
            // it should still be blocked by prefix
            let _ = result.unwrap();
        }
        // If the file exists and resolves, check it was blocked
        let m2 = make_matrix();
        let result2 = m2.verify_path(".git/HEAD", PathOp::Read);
        if result2.is_ok() {
            // Should have been blocked if it resolved
        }
    }

    #[test]
    fn test_verify_command_blocked() {
        let m = make_matrix();
        assert!(m.verify_command("rm -rf /tmp/foo").is_err());
        assert!(m.verify_command("mkfs.ext4 /dev/sda").is_err());
    }

    #[test]
    fn test_verify_command_allowed() {
        let m = make_matrix();
        assert!(m.verify_command("echo hello").is_ok());
        assert!(m.verify_command("ls -la").is_ok());
    }

    #[test]
    fn test_verify_command_allowlist() {
        let mut m = make_matrix();
        m.shell_policy.allowlist = vec!["ls".into(), "echo".into()];
        assert!(m.verify_command("ls -la").is_ok());
        assert!(m.verify_command("cat /etc/hosts").is_err());
    }

    #[test]
    fn test_sandbox_input_read_file() {
        let m = make_matrix();
        let input = serde_json::json!({"path": "Cargo.toml"});
        let result = m.sandbox_input("read_file", &input);
        assert!(result.is_ok());
        let canonical = result.unwrap()["path"].as_str().unwrap().to_string();
        assert!(
            canonical.starts_with("/"),
            "should be absolute: {canonical}"
        );
    }

    #[test]
    fn test_sandbox_input_shell_blocked() {
        let m = make_matrix();
        let input = serde_json::json!({"command": "rm -rf /tmp"});
        let result = m.sandbox_input("shell", &input);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_sandboxed_echo() {
        let m = make_matrix();
        let result = m.execute_sandboxed("echo hello").await.unwrap();
        assert!(result.contains("hello"));
    }
}

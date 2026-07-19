use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use super::sandbox::{ExecutionSandbox, SandboxOutput};

/// macOS Seatbelt sandbox using the built-in `sandbox-exec` command.
///
/// Generates a temporary .sb profile file that restricts:
/// - Network access (denied)
/// - Process forking (denied)
/// - File writes (only project_root + allowed_paths)
///
/// Requires macOS 10.7+. Always available on macOS.
pub struct SeatbeltSandbox {
    pub project_root: PathBuf,
    pub allowed_paths: Vec<PathBuf>,
    pub timeout: Duration,
}

pub fn is_available() -> bool {
    std::process::Command::new("sandbox-exec")
        .arg("-h")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success() || s.code() == Some(1))
        .unwrap_or(false)
}

/// Escape a string for embedding inside a double-quoted Seatbelt (.sb)
/// profile literal. Without this, a path containing `"` or `\` would break
/// out of the string literal and corrupt (or inject into) the profile.
fn escape_sb_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out
}

fn build_profile(project_root: &PathBuf, allowed_paths: &[PathBuf]) -> String {
    let mut p = String::new();
    p.push_str("(version 1)\n");
    p.push_str("(allow default)\n");
    p.push_str("(deny network*)\n");
    p.push_str("(deny process-fork)\n");
    p.push_str("(deny file-write* (subpath \"/\"))\n");

    // Carve out write permissions for project_root and allowed_paths
    let mut paths: Vec<&PathBuf> = vec![project_root];
    paths.extend(allowed_paths.iter());

    for path in paths {
        p.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            escape_sb_string(&path.display().to_string())
        ));
    }
    p
}

#[async_trait]
impl ExecutionSandbox for SeatbeltSandbox {
    fn name(&self) -> &str {
        "seatbelt"
    }

    async fn execute(
        &self,
        cmd: &str,
        cwd: &Path,
        env: &HashMap<String, String>,
    ) -> Result<SandboxOutput, String> {
        let profile = build_profile(&self.project_root, &self.allowed_paths);

        let profile_path =
            std::env::temp_dir().join(format!("jia-seatbelt-{}.sb", uuid::Uuid::new_v4()));
        std::fs::write(&profile_path, &profile)
            .map_err(|e| format!("Failed to write Seatbelt profile: {e}"))?;

        let cmd_owned = cmd.to_string();
        let cwd_owned = cwd.to_path_buf();
        let env_owned = env.clone();
        let timeout = self.timeout;
        let profile_clone = profile_path.clone();

        let result = tokio::task::spawn_blocking(move || {
            run_seatbelt(&cmd_owned, &cwd_owned, &env_owned, &profile_clone, timeout)
        })
        .await
        .map_err(|e| format!("Seatbelt join error: {e}"))?;

        let _ = std::fs::remove_file(&profile_path);
        result
    }
}

fn run_seatbelt(
    cmd: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    profile_path: &std::path::Path,
    timeout: Duration,
) -> Result<SandboxOutput, String> {
    let mut cmd_builder = std::process::Command::new("sandbox-exec");
    cmd_builder
        .arg("-f")
        .arg(profile_path)
        .arg("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(cwd)
        .process_group(0);
    for (k, v) in env {
        cmd_builder.env(k, v);
    }

    let child = cmd_builder
        .spawn()
        .map_err(|e| format!("sandbox-exec spawn error: {e} (is sandbox-exec installed?)"))?;
    let pid = child.id();

    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();

    let handle = std::thread::spawn(move || {
        let result = child.wait_with_output();
        done_clone.store(true, Ordering::SeqCst);
        (pid, result)
    });

    let poll_interval = Duration::from_millis(100);
    let deadline = std::time::Instant::now() + timeout;

    loop {
        if done.load(Ordering::SeqCst) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let pgid = -(pid as i32);
            // SAFETY: SIGKILL to child process group after timeout.
            // pgid is the negated PID of the spawned child; the process group
            // exists because process_group(0) was set before spawning.
            unsafe { ::libc::kill(pgid, libc::SIGKILL) };
            let _ = handle.join();
            return Err(format!("Command timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(poll_interval);
    }

    let (_pid, output_result) = handle
        .join()
        .map_err(|_| "Seatbelt thread panicked".to_string())?;

    let output = output_result.map_err(|e| format!("wait error: {e}"))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(SandboxOutput {
        stdout,
        stderr,
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sb_string_escapes_quotes_and_backslashes() {
        assert_eq!(escape_sb_string("/plain/path"), "/plain/path");
        assert_eq!(escape_sb_string("/a\"b"), "/a\\\"b");
        assert_eq!(escape_sb_string("/a\\b"), "/a\\\\b");
        assert_eq!(
            escape_sb_string("/q\"\\\""),
            "/q\\\"\\\\\\\""
        );
    }

    #[test]
    fn build_profile_escapes_embedded_paths() {
        let root = PathBuf::from("/proj/with\"quote");
        let allowed = vec![PathBuf::from("/extra/back\\slash")];
        let profile = build_profile(&root, &allowed);
        // The raw metacharacters must not appear unescaped inside literals
        assert!(profile.contains("(subpath \"/proj/with\\\"quote\")"));
        assert!(profile.contains("(subpath \"/extra/back\\\\slash\")"));
        // Structural lines still present
        assert!(profile.starts_with("(version 1)\n"));
        assert!(profile.contains("(deny network*)\n"));
        assert!(profile.contains("(deny file-write* (subpath \"/\"))\n"));
    }

    #[test]
    fn build_profile_plain_paths_unchanged() {
        let root = PathBuf::from("/proj/root");
        let profile = build_profile(&root, &[]);
        assert!(profile.contains("(allow file-write* (subpath \"/proj/root\"))\n"));
    }
}

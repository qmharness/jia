//! 震三宫 · rg — shared ripgrep subprocess plumbing for `grep` and `glob`.
//!
//! Both tools shell out to the `rg` binary (zero new crate dependencies):
//! .gitignore respect, hidden-file handling, and raw search speed all come
//! from ripgrep itself. Sensitive files are filtered twice — an rg `--glob`
//! prefilter (`prefilter_globs`) plus a post-filter on every result path
//! (`is_sensitive_path`). When `rg` is not installed, callers fall back to
//! their built-in implementations and annotate the result.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// Search-class subprocess timeout (aligned with the 30s browser tools use).
pub const RG_TIMEOUT: Duration = Duration::from_secs(30);

/// Hard cap on captured stdout, so a runaway match set cannot OOM the kernel.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Smaller cap for stderr; it only feeds error messages.
const MAX_STDERR_BYTES: usize = 64 * 1024;

pub struct RgOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// `None` when the process was killed after a timeout.
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout_truncated: bool,
}

/// Distinguishes "rg is not installed" (caller falls back) from real errors.
pub enum RgError {
    NotFound,
    Failed(String),
}

/// Spawn `rg` with `args` in `dir`, draining stdout/stderr under byte caps.
pub async fn run(args: &[String], dir: &Path) -> Result<RgOutput, RgError> {
    let mut child = match tokio::process::Command::new("rg")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RgError::NotFound),
        Err(e) => return Err(RgError::Failed(format!("failed to spawn rg: {e}"))),
    };

    let Some(mut out_pipe) = child.stdout.take() else {
        return Err(RgError::Failed("rg stdout was not piped".into()));
    };
    let Some(mut err_pipe) = child.stderr.take() else {
        return Err(RgError::Failed("rg stderr was not piped".into()));
    };
    // Read both pipes concurrently; a full stderr pipe would otherwise
    // deadlock the child while we drain stdout.
    let stdout_task =
        tokio::spawn(async move { read_capped(&mut out_pipe, MAX_OUTPUT_BYTES).await });
    let stderr_task =
        tokio::spawn(async move { read_capped(&mut err_pipe, MAX_STDERR_BYTES).await });

    match tokio::time::timeout(RG_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout, stdout_truncated) = stdout_task.await.unwrap_or_default();
            let (stderr, _) = stderr_task.await.unwrap_or_default();
            Ok(RgOutput {
                stdout,
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: status.code(),
                timed_out: false,
                stdout_truncated,
            })
        }
        Ok(Err(e)) => Err(RgError::Failed(format!("failed to wait on rg: {e}"))),
        Err(_) => {
            // Timed out: kill, then return whatever was captured so callers
            // can surface partial results.
            let _ = child.kill().await;
            let (stdout, stdout_truncated) = stdout_task.await.unwrap_or_default();
            let (stderr, _) = stderr_task.await.unwrap_or_default();
            Ok(RgOutput {
                stdout,
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: None,
                timed_out: true,
                stdout_truncated,
            })
        }
    }
}

/// Read to EOF, keeping at most `cap` bytes but draining the rest so the
/// child never blocks on a full pipe. Returns `(kept, truncated)`.
async fn read_capped<R: tokio::io::AsyncRead + Unpin>(reader: &mut R, cap: usize) -> (Vec<u8>, bool) {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < cap {
                    let keep = (cap - buf.len()).min(n);
                    buf.extend_from_slice(&chunk[..keep]);
                    truncated |= keep < n;
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (buf, truncated)
}

/// rg `--glob` prefilters. `--hidden` is passed by callers, so VCS metadata
/// must be excluded explicitly. Every glob MUST stay negated (`!`): a single
/// positive glob turns the whole `--glob` set into a whitelist and rg then
/// searches only those files. `.env.*` variants (and the public template
/// exemptions) are therefore left to the post-filter — only the exact `.env`
/// basename is excluded here.
pub fn prefilter_globs() -> Vec<String> {
    let mut globs = Vec::new();
    for dir in [".git", ".svn", ".hg", ".bzr", ".jj", ".sl"] {
        globs.push(format!("!{dir}"));
    }
    // Dotenv files: exact `.env` here, `.env.*` variants in the post-filter.
    globs.push("!**/.env".into());
    // SSH private keys (exact name and suffixed variants; `*.pub` untouched).
    for key in ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"] {
        globs.push(format!("!**/{key}"));
        globs.push(format!("!**/{key}[-_]*"));
    }
    // Cloud CLI credential stores.
    for cloud in [".aws", ".gcp"] {
        globs.push(format!("!**/{cloud}/credentials"));
        globs.push(format!("!**/{cloud}/credentials/**"));
    }
    globs
}

/// Split rg stderr into fatal lines and transient traversal warnings.
/// A file that disappears mid-walk ("IO error for operation on ...") is a
/// race with the live filesystem, not a search failure — callers should
/// treat only the fatal lines as an error (rg still exits 2 for both).
pub fn partition_stderr(stderr: &str) -> (Vec<&str>, Vec<&str>) {
    stderr
        .lines()
        .filter(|l| !l.trim().is_empty())
        .partition(|l| !l.contains("IO error for operation on"))
}

/// Post-filter mirror of `prefilter_globs`, applied to every result path.
pub fn is_sensitive_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with(".env")
        && !matches!(name, ".env.example" | ".env.sample" | ".env.template")
    {
        return true;
    }
    // Private keys; the public `.pub` halves are exempt.
    if !name.ends_with(".pub")
        && ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"]
            .iter()
            .any(|key| name.starts_with(key))
    {
        return true;
    }
    if name == "credentials"
        && let Some(parent) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        && matches!(parent, ".aws" | ".gcp")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_env_files() {
        assert!(is_sensitive_path(Path::new("/proj/.env")));
        assert!(is_sensitive_path(Path::new("/proj/.env.local")));
        assert!(!is_sensitive_path(Path::new("/proj/.env.example")));
        assert!(!is_sensitive_path(Path::new("/proj/.env.sample")));
        assert!(!is_sensitive_path(Path::new("/proj/.env.template")));
        assert!(!is_sensitive_path(Path::new("/proj/environment.rs")));
    }

    #[test]
    fn sensitive_ssh_keys() {
        assert!(is_sensitive_path(Path::new("/home/u/.ssh/id_rsa")));
        assert!(is_sensitive_path(Path::new("/home/u/.ssh/id_ed25519")));
        assert!(is_sensitive_path(Path::new("/keys/id_rsa_backup")));
        assert!(!is_sensitive_path(Path::new("/home/u/.ssh/id_rsa.pub")));
        assert!(!is_sensitive_path(Path::new("/src/id_rsa.rs.txt.pub")));
    }

    #[test]
    fn sensitive_cloud_credentials() {
        assert!(is_sensitive_path(Path::new("/home/u/.aws/credentials")));
        assert!(is_sensitive_path(Path::new("/home/u/.gcp/credentials")));
        assert!(!is_sensitive_path(Path::new("/proj/src/credentials")));
    }

    #[test]
    fn prefilter_globs_are_all_negated() {
        // A positive glob turns the whole --glob set into a whitelist.
        assert!(prefilter_globs().iter().all(|g| g.starts_with('!')));
    }

    #[test]
    fn stderr_partition_separates_transient_io_errors() {
        let (fatal, transient) = partition_stderr(
            "rg: ./gone: IO error for operation on ./gone: No such file or directory (os error 2)\n\
             rg: badglob: glob parse error\n",
        );
        assert_eq!(fatal, ["rg: badglob: glob parse error"]);
        assert_eq!(transient.len(), 1);
    }
}

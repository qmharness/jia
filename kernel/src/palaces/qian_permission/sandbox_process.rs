use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use super::sandbox::{ExecutionSandbox, SandboxOutput};

/// Process-level sandbox using rlimit resource limits and process group isolation.
///
/// Resource limits are applied in a `pre_exec` closure (between fork and exec)
/// so they only affect the child process. Does not require Docker or any
/// container runtime.
pub struct ProcessSandbox {
    pub timeout: Duration,
    pub memory_limit_bytes: u64,
    pub file_size_limit_bytes: u64,
    pub max_processes: u64,
}

impl Default for ProcessSandbox {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            memory_limit_bytes: 512 * 1024 * 1024,    // 512 MB
            file_size_limit_bytes: 100 * 1024 * 1024, // 100 MB
            max_processes: 50,
        }
    }
}

#[async_trait]
impl ExecutionSandbox for ProcessSandbox {
    fn name(&self) -> &str {
        "process"
    }

    async fn execute(
        &self,
        cmd: &str,
        cwd: &Path,
        env: &HashMap<String, String>,
    ) -> Result<SandboxOutput, String> {
        let timeout = self.timeout;
        let mem_limit = self.memory_limit_bytes;
        let fsize_limit = self.file_size_limit_bytes;
        let nproc_limit = self.max_processes;

        let cmd_owned = cmd.to_string();
        let cwd_owned = cwd.to_path_buf();
        let env_owned = env.clone();

        tokio::task::spawn_blocking(move || {
            run_sandboxed(
                &cmd_owned,
                timeout,
                mem_limit,
                fsize_limit,
                nproc_limit,
                &cwd_owned,
                &env_owned,
            )
        })
        .await
        .map_err(|e| format!("ProcessSandbox join error: {e}"))?
    }
}

fn run_sandboxed(
    cmd: &str,
    timeout: Duration,
    mem_limit: u64,
    fsize_limit: u64,
    nproc_limit: u64,
    cwd: &Path,
    env: &HashMap<String, String>,
) -> Result<SandboxOutput, String> {
    let mut cmd_builder = std::process::Command::new("sh");
    cmd_builder
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .process_group(0)
        .current_dir(cwd);
    for (k, v) in env {
        cmd_builder.env(k, v);
    }

    // SAFETY: pre_exec closure runs in the child process between fork and exec.
    // Invalid pointer dereference or panic in this closure are undefined behavior per POSIX.
    // Our closure only calls setrlimit (safe FFI) and, on failure, child_warn
    // (write(2), async-signal-safe — no allocation, no locks), then returns Ok(()).
    unsafe {
        cmd_builder.pre_exec(move || {
            apply_child_rlimits(mem_limit, fsize_limit, nproc_limit);
            Ok(())
        });
    }

    let child = cmd_builder
        .spawn()
        .map_err(|e| format!("spawn error: {e}"))?;
    let pid = child.id();

    let done = Arc::new(AtomicBool::new(false));
    let done_clone = done.clone();

    let handle = std::thread::spawn(move || {
        let result = child.wait_with_output();
        done_clone.store(true, Ordering::SeqCst);
        (pid, result)
    });

    // Poll with timeout
    let poll_interval = Duration::from_millis(100);
    let deadline = std::time::Instant::now() + timeout;

    loop {
        if done.load(Ordering::SeqCst) {
            // Reap any remaining children in the process group
            let pgid = -(pid as i32);
            loop {
                let mut status: i32 = 0;
                let wpid = unsafe { ::libc::waitpid(pgid, &mut status, libc::WNOHANG) };
                if wpid <= 0 {
                    break;
                }
            }
            break;
        }
        if std::time::Instant::now() >= deadline {
            // Kill the entire process group and reap all children
            let pgid = -(pid as i32);
            // SAFETY: SIGKILL to the child process group kills all
            // subprocesses (shell pipelines, background jobs, etc.)
            unsafe { ::libc::kill(pgid, libc::SIGKILL) };
            let _ = handle.join();
            // Reap any remaining zombies in the process group
            loop {
                let mut status: i32 = 0;
                // SAFETY: waitpid(-pgid, WNOHANG) collects one child without blocking
                let wpid = unsafe { ::libc::waitpid(pgid, &mut status, libc::WNOHANG) };
                if wpid <= 0 {
                    break;
                }
            }
            return Err(format!("Command timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(poll_interval);
    }

    let (_pid, output_result) = handle
        .join()
        .map_err(|_| "ProcessSandbox thread panicked".to_string())?;

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

/// Async-signal-safe report from the forked child (pre-exec): tracing/format!
/// allocate and take subscriber locks, which can deadlock the child when
/// another thread held them at fork (same hazard as landlock's pre_exec, U3).
/// write(2) to the piped stderr is signal-safe and lands in SandboxOutput.stderr.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn child_warn(msg: &str) {
    // SAFETY: write(2) on a valid fd with a valid buffer; no allocation.
    unsafe {
        libc::write(
            libc::STDERR_FILENO,
            msg.as_ptr() as *const libc::c_void,
            msg.len(),
        );
    }
}

#[cfg(target_os = "linux")]
fn apply_child_rlimits(mem_limit: u64, fsize_limit: u64, nproc_limit: u64) {
    // SAFETY: setrlimit is a POSIX syscall that takes a valid resource type
    // and a pointer to a properly initialized rlimit struct. Both preconditions
    // are met — rlim_cur/rlim_max are set to the configured values. Called
    // only from pre_exec (child process, no threads); failures are reported
    // via child_warn (write(2), async-signal-safe), never tracing/alloc.
    if mem_limit > 0 {
        let lim = ::libc::rlimit {
            rlim_cur: mem_limit,
            rlim_max: mem_limit,
        };
        let ret = unsafe { ::libc::setrlimit(libc::RLIMIT_AS, &lim) };
        if ret != 0 {
            child_warn("jia-sandbox: setrlimit(RLIMIT_AS) failed; memory limit not enforced\n");
        }
    }
    if fsize_limit > 0 {
        let lim = ::libc::rlimit {
            rlim_cur: fsize_limit,
            rlim_max: fsize_limit,
        };
        let ret = unsafe { ::libc::setrlimit(libc::RLIMIT_FSIZE, &lim) };
        if ret != 0 {
            child_warn("jia-sandbox: setrlimit(RLIMIT_FSIZE) failed; file size limit not enforced\n");
        }
    }
    if nproc_limit > 0 {
        let lim = ::libc::rlimit {
            rlim_cur: nproc_limit,
            rlim_max: nproc_limit,
        };
        let ret = unsafe { ::libc::setrlimit(libc::RLIMIT_NPROC, &lim) };
        if ret != 0 {
            child_warn("jia-sandbox: setrlimit(RLIMIT_NPROC) failed; process count limit not enforced\n");
        }
    }
}

#[cfg(target_os = "macos")]
fn apply_child_rlimits(mem_limit: u64, fsize_limit: u64, nproc_limit: u64) {
    // SAFETY: Same as above. macOS uses RLIMIT_DATA instead of RLIMIT_AS
    // for memory limits (RLIMIT_AS is not enforced on macOS).
    if mem_limit > 0 {
        let lim = ::libc::rlimit {
            rlim_cur: mem_limit,
            rlim_max: mem_limit,
        };
        let ret = unsafe { ::libc::setrlimit(libc::RLIMIT_DATA, &lim) };
        if ret != 0 {
            child_warn("jia-sandbox: setrlimit(RLIMIT_DATA) failed; memory limit not enforced\n");
        }
    }
    if fsize_limit > 0 {
        let lim = ::libc::rlimit {
            rlim_cur: fsize_limit,
            rlim_max: fsize_limit,
        };
        let ret = unsafe { ::libc::setrlimit(libc::RLIMIT_FSIZE, &lim) };
        if ret != 0 {
            child_warn("jia-sandbox: setrlimit(RLIMIT_FSIZE) failed; file size limit not enforced\n");
        }
    }
    if nproc_limit > 0 {
        let lim = ::libc::rlimit {
            rlim_cur: nproc_limit,
            rlim_max: nproc_limit,
        };
        let ret = unsafe { ::libc::setrlimit(libc::RLIMIT_NPROC, &lim) };
        if ret != 0 {
            child_warn("jia-sandbox: setrlimit(RLIMIT_NPROC) failed; process count limit not enforced\n");
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn apply_child_rlimits(_mem_limit: u64, _fsize_limit: u64, _nproc_limit: u64) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_with_rlimits_succeeds() {
        // Generous limits the child can actually honor — exercises the
        // setrlimit return-value-checking path (no warn expected).
        let sandbox = ProcessSandbox {
            timeout: Duration::from_secs(10),
            memory_limit_bytes: 1024 * 1024 * 1024,
            file_size_limit_bytes: 100 * 1024 * 1024,
            max_processes: 256,
        };
        let cwd = std::env::current_dir().unwrap();
        let out = sandbox
            .execute("echo rlimit-ok", &cwd, &HashMap::new())
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("rlimit-ok"));
    }

    #[test]
    fn zero_limits_are_noop() {
        // 0 disables each limit — must not call setrlimit nor panic.
        apply_child_rlimits(0, 0, 0);
    }
}

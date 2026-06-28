use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use super::sandbox::{ExecutionSandbox, SandboxOutput};

// ── Landlock ABI constants (linux/landlock.h) ────────────────────

const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;

/// File opened as a directory descriptor for path_beneath rules.
#[cfg(target_os = "linux")]
const O_PATH: i32 = 0o10000000;

const LANDLOCK_RULE_PATH_BENEATH: u32 = 1;

// All filesystem access rights we control (everything except EXECUTE)
const ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
const ACCESS_FS_READ_FILE: u64 = 1 << 2;
const ACCESS_FS_READ_DIR: u64 = 1 << 3;
const ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
const ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
const ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
const ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
const ACCESS_FS_MAKE_REG: u64 = 1 << 8;
const ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
const ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
const ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
const ACCESS_FS_MAKE_SYM: u64 = 1 << 12;

const HANDLED_ACCESS_FS: u64 = ACCESS_FS_WRITE_FILE
    | ACCESS_FS_READ_FILE
    | ACCESS_FS_READ_DIR
    | ACCESS_FS_REMOVE_DIR
    | ACCESS_FS_REMOVE_FILE
    | ACCESS_FS_MAKE_CHAR
    | ACCESS_FS_MAKE_DIR
    | ACCESS_FS_MAKE_REG
    | ACCESS_FS_MAKE_SOCK
    | ACCESS_FS_MAKE_FIFO
    | ACCESS_FS_MAKE_BLOCK
    | ACCESS_FS_MAKE_SYM;

// ── repr(C) syscall structs ──────────────────────────────────────

#[repr(C)]
struct LandlockRulesetAttr {
    handled_access_fs: u64,
}

#[repr(C)]
struct LandlockPathBeneathAttr {
    allowed_access: u64,
    parent_fd: i32,
}

#[cfg(target_os = "linux")]
// SAFETY: Direct Linux Landlock syscalls. All pointer arguments reference
// valid stack-allocated structs. The syscall numbers match the Linux kernel
// ABI (landlock_create_ruleset = 444, landlock_add_rule = 445,
// landlock_restrict_self = 446 on x86_64). Called only from within a child
// process (between fork and exec) to sandbox command execution.
unsafe fn landlock_create_ruleset(attr: &LandlockRulesetAttr) -> libc::c_long {
    libc::syscall(
        SYS_LANDLOCK_CREATE_RULESET,
        attr as *const LandlockRulesetAttr,
        std::mem::size_of::<LandlockRulesetAttr>(),
        0u32,
    )
}

#[cfg(target_os = "linux")]
// SAFETY: See landlock_create_ruleset. ruleset_fd is a valid file descriptor
// returned by landlock_create_ruleset. attr points to a valid
// LandlockPathBeneathAttr struct.
unsafe fn landlock_add_rule(
    ruleset_fd: i32,
    rule_type: u32,
    attr: &LandlockPathBeneathAttr,
) -> libc::c_long {
    libc::syscall(
        SYS_LANDLOCK_ADD_RULE,
        ruleset_fd,
        rule_type,
        attr as *const LandlockPathBeneathAttr,
        0u32,
    )
}

#[cfg(target_os = "linux")]
// SAFETY: See landlock_create_ruleset. ruleset_fd must be a valid ruleset fd.
// This syscall is irreversible — it permanently restricts the calling process.
unsafe fn landlock_restrict_self(ruleset_fd: i32) -> libc::c_long {
    libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset_fd, 0u32)
}

// ── Public API ───────────────────────────────────────────────────

/// Linux Landlock sandbox using the Landlock LSM (kernel 5.13+).
///
/// Restricts filesystem access to project_root + allowed_paths only.
/// No network or process restrictions — purely filesystem.
pub struct LandlockSandbox {
    pub project_root: PathBuf,
    pub allowed_paths: Vec<PathBuf>,
    pub timeout: Duration,
}

/// Probe whether Landlock is supported by the current kernel.
///
/// Tries `landlock_create_ruleset` with an empty access mask. On
/// kernels >= 5.13 this returns a valid fd. On older kernels it
/// returns -ENOSYS.
pub fn is_available() -> bool {
    unsafe {
        let attr = LandlockRulesetAttr {
            handled_access_fs: 0,
        };
        let fd = landlock_create_ruleset(&attr);
        if fd >= 0 {
            libc::close(fd as i32);
            true
        } else {
            false
        }
    }
}

// ── Core Landlock logic ──────────────────────────────────────────

unsafe fn apply_landlock(project_root: &Path, allowed_paths: &[PathBuf]) -> Result<(), String> {
    let attr = LandlockRulesetAttr {
        handled_access_fs: HANDLED_ACCESS_FS,
    };
    let ruleset_fd = landlock_create_ruleset(&attr);

    if ruleset_fd < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOSYS) {
            return Err(
                "Landlock is not supported by this kernel (requires Linux 5.13+). \
                 Upgrade your kernel or use a different sandbox backend."
                    .into(),
            );
        }
        return Err(format!("landlock_create_ruleset failed: {err}"));
    }

    // Collect paths: project_root + all allowed_paths
    let mut paths: Vec<PathBuf> = vec![project_root.to_path_buf()];
    paths.extend_from_slice(allowed_paths);

    for path in &paths {
        let path_str = match path.to_str() {
            Some(s) => s,
            None => continue,
        };
        let cstr = match std::ffi::CString::new(path_str) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let dir_fd = libc::open(cstr.as_ptr(), O_PATH | libc::O_CLOEXEC);
        if dir_fd < 0 {
            tracing::warn!(?path, "Landlock: cannot open path for ruleset");
            continue;
        }

        let path_attr = LandlockPathBeneathAttr {
            allowed_access: HANDLED_ACCESS_FS,
            parent_fd: dir_fd as i32,
        };
        let ret = landlock_add_rule(ruleset_fd as i32, LANDLOCK_RULE_PATH_BENEATH, &path_attr);
        libc::close(dir_fd);

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!("landlock_add_rule for {path_str} failed: {err}"));
        }
    }

    // Enforce the ruleset (irreversible for this process)
    let ret = landlock_restrict_self(ruleset_fd as i32);
    libc::close(ruleset_fd as i32);

    if ret < 0 {
        let err = std::io::Error::last_os_error();
        return Err(format!("landlock_restrict_self failed: {err}"));
    }

    Ok(())
}

// ── ExecutionSandbox impl ────────────────────────────────────────

#[async_trait]
impl ExecutionSandbox for LandlockSandbox {
    fn name(&self) -> &str {
        "landlock"
    }

    async fn execute(
        &self,
        cmd: &str,
        cwd: &PathBuf,
        env: &HashMap<String, String>,
    ) -> Result<SandboxOutput, String> {
        let cmd_owned = cmd.to_string();
        let cwd_owned = cwd.clone();
        let env_owned = env.clone();
        let timeout = self.timeout;
        let project_root = self.project_root.clone();
        let allowed_paths = self.allowed_paths.clone();

        tokio::task::spawn_blocking(move || {
            run_landlock(
                &cmd_owned,
                timeout,
                &cwd_owned,
                &env_owned,
                &project_root,
                &allowed_paths,
            )
        })
        .await
        .map_err(|e| format!("Landlock join error: {e}"))?
    }
}

fn run_landlock(
    cmd: &str,
    timeout: Duration,
    cwd: &PathBuf,
    env: &HashMap<String, String>,
    project_root: &Path,
    allowed_paths: &[PathBuf],
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

    let proot = project_root.to_path_buf();
    let apaths = allowed_paths.to_vec();

    unsafe {
        cmd_builder.pre_exec(move || {
            apply_landlock(&proot, &apaths)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            Ok(())
        });
    }

    let child = cmd_builder
        .spawn()
        .map_err(|e| format!("Landlock spawn error: {e}"))?;
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
            unsafe { ::libc::kill(pgid, libc::SIGKILL) };
            let _ = handle.join();
            return Err(format!("Command timed out after {}s", timeout.as_secs()));
        }
        std::thread::sleep(poll_interval);
    }

    let (_pid, output_result) = handle
        .join()
        .map_err(|_| "Landlock thread panicked".to_string())?;

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

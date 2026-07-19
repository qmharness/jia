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
// landlock_restrict_self = 446 on x86_64). Called either from within a child
// process (between fork and exec) to sandbox command execution, or from a
// short-lived probe thread in `is_available`.
unsafe fn landlock_create_ruleset(attr: &LandlockRulesetAttr) -> libc::c_long {
    unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            attr as *const LandlockRulesetAttr,
            std::mem::size_of::<LandlockRulesetAttr>(),
            0u32,
        )
    }
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
    unsafe {
        libc::syscall(
            SYS_LANDLOCK_ADD_RULE,
            ruleset_fd,
            rule_type,
            attr as *const LandlockPathBeneathAttr,
            0u32,
        )
    }
}

#[cfg(target_os = "linux")]
// SAFETY: See landlock_create_ruleset. ruleset_fd must be a valid ruleset fd.
// This syscall is irreversible — it permanently restricts the calling process.
unsafe fn landlock_restrict_self(ruleset_fd: i32) -> libc::c_long {
    unsafe { libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset_fd, 0u32) }
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

/// Probe whether Landlock is usable by the current (unprivileged) process.
///
/// `landlock_restrict_self(2)` requires the calling thread to have
/// `no_new_privs` set (or hold CAP_SYS_ADMIN), otherwise it fails with
/// EPERM. Probing only `landlock_create_ruleset` is therefore not enough —
/// it would report "available" on kernels >= 5.13 even when `restrict_self`
/// would always fail, causing `auto_select` to pick a backend that breaks
/// every command.
///
/// The probe runs in a short-lived dedicated thread: `no_new_privs` is a
/// per-thread attribute, so setting it here does not affect the main
/// process, and the probe ruleset (which is irreversible) dies with the
/// thread.
pub fn is_available() -> bool {
    std::thread::spawn(|| unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return false;
        }
        let attr = LandlockRulesetAttr {
            handled_access_fs: 0,
        };
        let fd = landlock_create_ruleset(&attr);
        if fd < 0 {
            return false;
        }
        // Actually restrict this probe thread with the empty ruleset —
        // verifies the full create+restrict path an unprivileged child
        // would take (returns EPERM without no_new_privs/CAP_SYS_ADMIN).
        let ret = landlock_restrict_self(fd as i32);
        libc::close(fd as i32);
        ret == 0
    })
    .join()
    .unwrap_or(false)
}

// ── Core Landlock logic ──────────────────────────────────────────

/// Build a fully-populated Landlock ruleset and return its fd.
///
/// Runs in the parent process *before* fork so that all heap allocation
/// (path Vec, CString, format! error messages) and tracing happen outside
/// the `pre_exec` closure — between fork and exec only async-signal-safe
/// operations are allowed, and allocating there can deadlock the child of
/// a multithreaded (tokio) process. The returned fd is inherited by the
/// child, whose `pre_exec` closure only needs to `restrict_self` on it.
///
/// On error any partially-built ruleset fd is closed before returning.
// SAFETY: all raw syscalls below operate on valid fds/structs as documented
// on the individual wrappers; error paths close every fd they own.
unsafe fn prepare_ruleset(project_root: &Path, allowed_paths: &[PathBuf]) -> Result<i32, String> {
    let attr = LandlockRulesetAttr {
        handled_access_fs: HANDLED_ACCESS_FS,
    };
    let ruleset_fd = unsafe { landlock_create_ruleset(&attr) };

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
    let ruleset_fd = ruleset_fd as i32;

    // Prevent the fd leaking into unrelated concurrent spawns in this
    // multithreaded process: it is only needed until the child restricts
    // (pre-exec) and may vanish at exec. fork inheritance is unaffected by
    // CLOEXEC, so the child still sees it in pre_exec.
    unsafe { libc::fcntl(ruleset_fd, libc::F_SETFD, libc::FD_CLOEXEC) };

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

        let dir_fd = unsafe { libc::open(cstr.as_ptr(), O_PATH | libc::O_CLOEXEC) };
        if dir_fd < 0 {
            tracing::warn!(?path, "Landlock: cannot open path for ruleset");
            continue;
        }

        let path_attr = LandlockPathBeneathAttr {
            allowed_access: HANDLED_ACCESS_FS,
            parent_fd: dir_fd as i32,
        };
        let ret = unsafe { landlock_add_rule(ruleset_fd, LANDLOCK_RULE_PATH_BENEATH, &path_attr) };
        unsafe { libc::close(dir_fd) };

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            unsafe { libc::close(ruleset_fd) };
            return Err(format!("landlock_add_rule for {path_str} failed: {err}"));
        }
    }

    Ok(ruleset_fd)
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
        cwd: &Path,
        env: &HashMap<String, String>,
    ) -> Result<SandboxOutput, String> {
        let cmd_owned = cmd.to_string();
        let cwd_owned = cwd.to_path_buf();
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

    // Build the ruleset in the parent, before fork: all allocation and
    // tracing happens here, so the pre_exec closure below performs only
    // syscalls (async-signal-safe) and never touches the heap.
    let ruleset_fd = unsafe { prepare_ruleset(project_root, allowed_paths)? };

    // SAFETY: the pre_exec closure runs in the child between fork and exec,
    // where only async-signal-safe operations are permitted. The closure
    // performs no heap allocation — it only calls prctl, landlock syscalls
    // and close on the inherited ruleset fd. `ruleset_fd` is a valid fd
    // inherited across fork (CLOEXEC only takes effect at exec, after the
    // closure has already used and closed it).
    unsafe {
        cmd_builder.pre_exec(move || {
            // landlock_restrict_self requires no_new_privs (or CAP_SYS_ADMIN),
            // otherwise it fails with EPERM for unprivileged users. Must be
            // set in this same thread, right before restrict.
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                let err = std::io::Error::last_os_error();
                libc::close(ruleset_fd);
                return Err(err);
            }
            // Enforce the ruleset (irreversible for this process). Capture
            // errno before close(2) — an interrupted close could clobber it.
            let ret = landlock_restrict_self(ruleset_fd);
            let err = if ret < 0 {
                Some(std::io::Error::last_os_error())
            } else {
                None
            };
            libc::close(ruleset_fd);
            if let Some(err) = err {
                return Err(err);
            }
            Ok(())
        });
    }

    let spawn_result = cmd_builder.spawn();
    // The child got its own copy of the fd across fork; drop the parent's.
    unsafe { libc::close(ruleset_fd) };
    let child = spawn_result.map_err(|e| format!("Landlock spawn error: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_available_runs_and_is_consistent() {
        // The probe must not panic, must not affect the calling thread
        // (no_new_privs is set only inside the probe thread), and should
        // give a stable answer across calls.
        let first = is_available();
        // Verify the probe did not set no_new_privs on THIS thread.
        let nnp = unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) };
        assert_eq!(nnp, 0, "probe leaked no_new_privs into caller thread");
        assert_eq!(first, is_available());
    }

    #[test]
    fn prepare_ruleset_matches_probe_availability() {
        // If the probe says Landlock is usable, building a real ruleset in
        // the parent must succeed; if the probe says no, building may fail
        // but must return a clean Err (never panic, never leak a fd we then
        // fail to close — verified indirectly by not crashing).
        let result = unsafe { prepare_ruleset(Path::new("/tmp"), &[]) };
        if is_available() {
            let fd = result.expect("probe said available but prepare_ruleset failed");
            unsafe { libc::close(fd) };
        } else if let Ok(fd) = result {
            // Probe/build divergence: don't leak the fd we accidentally got.
            unsafe { libc::close(fd) };
        }
    }
}

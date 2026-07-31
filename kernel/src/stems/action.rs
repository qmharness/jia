use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;

/// 工具调用 — LLM 发起的工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub parameters: serde_json::Value,
}

/// 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
    pub error: Option<String>,
}

/// Tool definition schema for native tools APIs (OpenAI / Anthropic / Gemini).
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Freshness entry: (mtime, monotonic_timestamp_when_read).
type FreshnessEntry = (SystemTime, u64);

/// 执行上下文 — 天盘值符携带的"时令"
///
/// 工具自身为 stateless 单例（注册于地盘，六仪不动）。
/// 权限矩阵由 Agent 通过 ctx 参数在调用时注入（值符随时干旋转）。
///
/// `cancel_token` 是该次 run 的取消令牌（与 RunContext 同源），
/// 供长等待工具（ask_user/delegate/确认）select! 响应取消；
/// `session_id` 标识本次 run 所属会话，用于断连时按会话清扫
/// pending_questions / pending_confirmations（消除断连死锁）。
///
/// `read_state` 记录文件读取时刻（路径→mtime+时间戳），
/// 供新鲜度门(#4)和 Read 去重(#14)使用。
///
/// `cwd` 是 shell 的会话级工作目录(#6 · 末次 cd 继承)：shell 工具执行前
/// 读取、执行后按捕获到的 $PWD 更新（先过 verify_path 边界校验，逸出
/// workspace 则保持不变）。与 worktree swap 的交互：swap 整体更换
/// ExecContext，新 ctx 的 cwd 直接初始化为新根（重置而非映射相对路径，
/// 语义最干净）。内部可变性与 read_state 同法（Arc + 锁，`&ExecContext`
/// 共享引用下可写）。
#[derive(Clone)]
pub struct ExecContext {
    pub permissions: std::sync::Arc<crate::palaces::qian_permission::PermissionMatrix>,
    pub session_id: String,
    pub cancel_token: tokio_util::sync::CancellationToken,
    /// Tracks when files were last read: PathBuf → (mtime_at_read, read_nanos).
    /// Capacity 64 entries, LRU eviction. Wrapped in Arc so ExecContext stays Clone.
    pub read_state: Arc<Mutex<lru::LruCache<PathBuf, FreshnessEntry>>>,
    /// Session-level shell working directory. RwLock: read on every shell
    /// call, written only on an effective `cd` (and never concurrently —
    /// shell keeps the default `ToolAccesses::all`, a singleton batch per
    /// tool_scheduler.rs, so no two shell calls share a batch).
    pub cwd: Arc<RwLock<PathBuf>>,
}

impl ExecContext {
    /// Create a default read_state cache (64-entry LRU, empty).
    pub fn default_read_state() -> Arc<Mutex<lru::LruCache<PathBuf, FreshnessEntry>>> {
        Arc::new(Mutex::new(lru::LruCache::new(
            NonZeroUsize::new(64).unwrap(),
        )))
    }

    /// Create a session cwd initialized to the matrix's workspace_root.
    /// Used by every ExecContext construction site so a worktree swap
    /// (which rebuilds the matrix rooted at the worktree) resets the cwd
    /// to the new root.
    pub fn default_cwd(
        permissions: &crate::palaces::qian_permission::PermissionMatrix,
    ) -> Arc<RwLock<PathBuf>> {
        Arc::new(RwLock::new(permissions.sandbox.workspace_root.clone()))
    }

    /// 构造一个无会话归属、不可取消的上下文（测试与默认场景用）。
    pub fn new(
        permissions: std::sync::Arc<crate::palaces::qian_permission::PermissionMatrix>,
    ) -> Self {
        let cwd = Self::default_cwd(&permissions);
        Self {
            permissions,
            session_id: String::new(),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            read_state: Self::default_read_state(),
            cwd,
        }
    }

    /// Current session working directory (poison-tolerant, same as read_state).
    pub fn cwd(&self) -> PathBuf {
        self.cwd.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Persist a new session cwd. Callers must boundary-check the path first
    /// (shell tool: verify_path; worktree swap: rebuild via default_cwd) —
    /// 路径边界只收紧不放松.
    pub fn set_cwd(&self, path: PathBuf) {
        *self.cwd.write().unwrap_or_else(|e| e.into_inner()) = path;
    }

    /// Record that a file was read at the current moment.
    /// Captures both the file's mtime and a monotonic timestamp.
    pub fn record_read(&self, path: PathBuf, mtime: SystemTime) {
        let nanos = std::time::UNIX_EPOCH
            .elapsed()
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mut cache = self
            .read_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.put(path, (mtime, nanos));
    }

    /// Check freshness: has the file been read and hasn't changed since?
    /// Returns Ok if the file was read and mtime matches.
    /// Returns Err with a user-facing reason otherwise.
    pub fn check_freshness(
        &self,
        path: &PathBuf,
        current_mtime: SystemTime,
    ) -> Result<(), String> {
        let mut cache = self
            .read_state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match cache.get(path) {
            None => Err(format!(
                "Freshness gate: '{}' has not been read in this session. Please read_file first.",
                path.display()
            )),
            Some((cached_mtime, _read_nanos)) => {
                if *cached_mtime != current_mtime {
                    Err(format!(
                        "Freshness gate: '{}' was modified after your last read (mtime changed). Please re-read the file.",
                        path.display()
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::qian_permission::PermissionMatrix;

    #[test]
    fn cwd_defaults_to_workspace_root() {
        let perms = Arc::new(PermissionMatrix::default());
        let ctx = ExecContext::new(perms.clone());
        assert_eq!(ctx.cwd(), perms.sandbox.workspace_root);
    }

    /// #6 · worktree swap 语义：swap 整体更换 ExecContext，新 ctx 的 cwd
    /// 重置为新根（而非映射相对路径）。
    #[test]
    fn worktree_swap_resets_cwd_to_new_root() {
        let main_perms = Arc::new(PermissionMatrix::default());
        let ctx = ExecContext::new(main_perms);
        let sub = ctx.cwd().join("some/sub/dir");
        ctx.set_cwd(sub);

        // Simulate the loop.rs swap: a fresh ExecContext built from a
        // worktree-scoped matrix resets the cwd to the new root.
        let wt_root = PathBuf::from("/tmp/jia-test-worktree-root");
        let mut wt_perms = PermissionMatrix::default();
        wt_perms.sandbox.workspace_root = wt_root.clone();
        let wt_perms = Arc::new(wt_perms);
        let swapped = ExecContext {
            cwd: ExecContext::default_cwd(&wt_perms),
            permissions: wt_perms,
            session_id: ctx.session_id.clone(),
            cancel_token: ctx.cancel_token.clone(),
            read_state: ctx.read_state.clone(),
        };
        assert_eq!(swapped.cwd(), wt_root);
        // The old context keeps its own cwd (Arc sharing is per-context).
        assert!(ctx.cwd().ends_with("some/sub/dir"));
    }
}

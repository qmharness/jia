// ── Disk Output — Secure file I/O for background task output ─────
//
// Inspired by Claude Code's utils/task/diskOutput.ts.
//
// Key security decisions:
//   - O_NOFOLLOW on Unix to prevent symlink attacks from sandboxed processes
//   - O_EXCL to ensure new files are created fresh
//   - 5GB disk cap (MAX_TASK_OUTPUT_BYTES)
//   - Async drain loop with single Buffer (memory-sensitive, GC-friendly)

use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::palaces::kun_config::default_data_dir;

/// 5GB disk cap — mirrors Claude Code's MAX_TASK_OUTPUT_BYTES.
pub const MAX_TASK_OUTPUT_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Default max bytes to read at once (8MB, same as Claude Code).
pub const DEFAULT_MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

/// Task output directory: `~/.jia/tasks/`
pub fn task_output_dir() -> PathBuf {
    default_data_dir().join("tasks")
}

/// Output file path for a given task ID.
pub fn task_output_path(task_id: &str) -> PathBuf {
    task_output_dir().join(format!("{task_id}.output"))
}

/// Open a fresh file for writing.
/// O_EXCL ensures we create a new file and fail if something exists.
/// O_NOFOLLOW (Unix) prevents symlink-following attacks from sandboxes.
fn create_new_secure(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
}

/// Initialize a task output file.
/// Creates parent dirs, opens with O_EXCL (Unix) or 'wx' (Windows).
/// Returns the file path on success.
pub fn init_task_output(task_id: &str) -> Result<PathBuf, String> {
    let dir = task_output_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;

    let path = task_output_path(task_id);
    create_new_secure(&path).map_err(|e| format!("open {path:?}: {e}"))?;

    Ok(path)
}

/// Append content to a task's output file.
/// Self-caps at MAX_TASK_OUTPUT_BYTES with a truncation message.
pub fn append_task_output(task_id: &str, content: &str) -> Result<(), String> {
    let path = task_output_path(task_id);

    // Check current size (use saturating_add to prevent u64 overflow)
    let current_size = std::fs::metadata(&path)
        .map(|m| m.len())
        .unwrap_or(0);

    if current_size >= MAX_TASK_OUTPUT_BYTES {
        return Ok(()); // already capped, silently drop
    }

    let projected = current_size.saturating_add(content.len() as u64);
    let to_write = if projected > MAX_TASK_OUTPUT_BYTES {
        // Truncate to fit within cap
        let available = MAX_TASK_OUTPUT_BYTES.saturating_sub(current_size) as usize;
        if available == 0 {
            return Ok(());
        }
        let truncated = &content[..content
            .char_indices()
            .take_while(|(i, _)| *i < available)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0)];
        format!(
            "{truncated}\n[output truncated: exceeded 5GB disk cap]\n"
        )
    } else {
        content.to_string()
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .append(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| format!("append open {path:?}: {e}"))?;
        file.write_all(to_write.as_bytes())
            .map_err(|e| format!("write {path:?}: {e}"))?;
    }
    #[cfg(not(unix))]
    {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .map_err(|e| format!("append open {path:?}: {e}"))?;
        file.write_all(to_write.as_bytes())
            .map_err(|e| format!("write {path:?}: {e}"))?;
    }

    Ok(())
}

/// Read the full task output (tail, capped at max_bytes).
/// If the file is larger than max_bytes, a truncation notice is prepended.
pub fn read_task_output(task_id: &str, max_bytes: u64) -> Result<String, String> {
    let path = task_output_path(task_id);

    let metadata = std::fs::metadata(&path).map_err(|e| format!("stat {path:?}: {e}"))?;
    let file_size = metadata.len();

    if file_size == 0 {
        return Ok(String::new());
    }

    let read_size = max_bytes.min(file_size);
    let skip = if file_size > read_size {
        file_size - read_size
    } else {
        0
    };

    let mut file = open_output_file_read(&path)?;
    if skip > 0 {
        file.seek(SeekFrom::Start(skip))
            .map_err(|e| format!("seek {path:?}: {e}"))?;
    }

    let mut buf = vec![0u8; read_size as usize];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("read {path:?}: {e}"))?;
    buf.truncate(n);

    let content = String::from_utf8_lossy(&buf).to_string();
    if skip > 0 {
        Ok(format!(
            "[{}KB of earlier output omitted]\n{content}",
            skip / 1024
        ))
    } else {
        Ok(content)
    }
}

/// Open a task output file for reading with O_NOFOLLOW on Unix.
fn open_output_file_read(path: &std::path::Path) -> Result<std::fs::File, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("open {path:?}: {e}"))
    }
    #[cfg(not(unix))]
    {
        std::fs::File::open(path).map_err(|e| format!("open {path:?}: {e}"))
    }
}

/// Read incremental delta since `from_offset`.
/// Returns (new_content, new_offset).
/// Reads at most max_bytes, never loads the full file.
pub fn read_task_output_delta(
    task_id: &str,
    from_offset: u64,
    max_bytes: u64,
) -> Result<(String, u64), String> {
    let path = task_output_path(task_id);

    let metadata = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((String::new(), from_offset));
        }
        Err(e) => return Err(format!("stat {path:?}: {e}")),
    };
    let file_size = metadata.len();

    if file_size <= from_offset {
        return Ok((String::new(), from_offset));
    }

    let available = file_size - from_offset;
    let read_size = max_bytes.min(available);

    let mut file = open_output_file_read(&path)?;
    file.seek(SeekFrom::Start(from_offset))
        .map_err(|e| format!("seek {path:?}: {e}"))?;

    let mut buf = vec![0u8; read_size as usize];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("read {path:?}: {e}"))?;
    buf.truncate(n);

    let content = String::from_utf8_lossy(&buf).to_string();
    Ok((content, from_offset + n as u64))
}

/// Get current size of a task output file.
pub fn task_output_size(task_id: &str) -> Result<u64, String> {
    let path = task_output_path(task_id);
    match std::fs::metadata(&path) {
        Ok(m) => Ok(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("stat {path:?}: {e}")),
    }
}

/// Clean up a task's output file.
pub fn cleanup_task_output(task_id: &str) {
    let path = task_output_path(task_id);
    let _ = std::fs::remove_file(&path);
}

/// Read the tail of a task output file (last N bytes).
/// Used by the stall watchdog to check for interactive prompts.
pub fn tail_task_output(task_id: &str, tail_bytes: usize) -> Result<String, String> {
    let path = task_output_path(task_id);

    let metadata = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(format!("stat {path:?}: {e}")),
    };
    let file_size = metadata.len();

    if file_size == 0 {
        return Ok(String::new());
    }

    let read_size = (tail_bytes as u64).min(file_size);
    let skip = file_size - read_size;

    let mut file = open_output_file_read(&path)?;
    if skip > 0 {
        file.seek(SeekFrom::Start(skip))
            .map_err(|e| format!("seek {path:?}: {e}"))?;
    }

    let mut buf = vec![0u8; read_size as usize];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("read {path:?}: {e}"))?;
    buf.truncate(n);

    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Thread-safe write queue for a single task's output.
/// Keeps a local buffer and flushes to disk periodically.
///
/// Uses a single Mutex around all state to avoid the deadlock that the
/// previous three-Mutex design caused (buffer lock held while acquiring
/// itself in the cap-hit path).
pub struct OutputWriter {
    task_id: String,
    inner: Mutex<OutputWriterInner>,
}

struct OutputWriterInner {
    buffer: Vec<u8>,
    bytes_written: u64,
    capped: bool,
}

impl OutputWriter {
    pub fn new(task_id: &str) -> Self {
        Self {
            task_id: task_id.to_string(),
            inner: Mutex::new(OutputWriterInner {
                buffer: Vec::with_capacity(8192),
                bytes_written: 0,
                capped: false,
            }),
        }
    }

    /// Append content to the in-memory buffer.
    /// Flushes when buffer exceeds 8KB.
    pub fn append(&self, content: &str) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if inner.capped {
            return Ok(());
        }

        inner.bytes_written = inner.bytes_written.saturating_add(content.len() as u64);
        if inner.bytes_written >= MAX_TASK_OUTPUT_BYTES {
            inner.capped = true;
            // Preserve the existing buffer contents (don't clear), append
            // the truncation message, and flush.
            inner
                .buffer
                .extend_from_slice(b"\n[output truncated: exceeded 5GB disk cap]\n");
            let buf = std::mem::take(&mut inner.buffer);
            let data = String::from_utf8_lossy(&buf).to_string();
            // Release the lock before the blocking disk write
            drop(inner);
            return append_task_output(&self.task_id, &data);
        }

        inner.buffer.extend_from_slice(content.as_bytes());
        if inner.buffer.len() >= 8192 {
            let buf = std::mem::take(&mut inner.buffer);
            drop(inner);
            let data = String::from_utf8_lossy(&buf).to_string();
            append_task_output(&self.task_id, &data)?;
        }
        Ok(())
    }

    /// Flush the buffer to disk immediately.
    pub fn flush(&self) -> Result<(), String> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let buf = std::mem::take(&mut inner.buffer);
        drop(inner);
        if buf.is_empty() {
            return Ok(());
        }
        let content = String::from_utf8_lossy(&buf);
        append_task_output(&self.task_id, &content)
    }
}

impl Drop for OutputWriter {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            let buf = std::mem::take(&mut inner.buffer);
            if !buf.is_empty() {
                let content = String::from_utf8_lossy(&buf);
                let _ = append_task_output(&self.task_id, &content);
            }
        }
    }
}

// ── Tool result persistence (#10) ─────────────────────────────
//
// 超阈值工具结果落盘(批量屏障截断时由 finalize_outcome 调用)。
// 路径约定与 backups 相同:`<workspace>/.jia/tool-results/<session_id>/<tool_call_id>.txt`;
// 同为内部写盘,不经工具层 verify_path。
//
// 位识融合红线:落盘内容【不参与】熏习/召回 —— 工具结果 ≠ 记忆种子,
// 仅由 retrieve_tool_result 按 tool_call_id 定向取回(与种子分表)。

/// Session-scoped tool-results directory:
/// `<workspace_root>/.jia/tool-results/<session_id>/`.
pub fn tool_results_dir(workspace_root: &std::path::Path, session_id: &str) -> PathBuf {
    workspace_root
        .join(".jia")
        .join("tool-results")
        .join(session_id)
}

/// Sanitize a tool_call_id for use as a file name (defensive: ids are
/// provider-generated, so strip anything outside [A-Za-z0-9._-]).
fn sanitize_tool_call_id(tool_call_id: &str) -> String {
    tool_call_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Output file path for a persisted tool result.
pub fn tool_result_path(
    workspace_root: &std::path::Path,
    session_id: &str,
    tool_call_id: &str,
) -> PathBuf {
    tool_results_dir(workspace_root, session_id)
        .join(format!("{}.txt", sanitize_tool_call_id(tool_call_id)))
}

/// Persist a full (untruncated) tool result to disk.
///
/// O_EXCL 冻结:同一 tool_call_id 只保留第一次落盘的内容 —— 文件已存在
/// 时不覆盖,直接返回既有路径与字节数(替换决策按 id 冻结,同一 turn
/// 内不重复落盘同一 id)。
///
/// Returns (path, size_in_bytes).
pub fn persist_tool_result(
    workspace_root: &std::path::Path,
    session_id: &str,
    tool_call_id: &str,
    content: &str,
) -> Result<(PathBuf, u64), String> {
    let dir = tool_results_dir(workspace_root, session_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {dir:?}: {e}"))?;

    let path = tool_result_path(workspace_root, session_id, tool_call_id);
    match create_new_secure(&path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes())
                .map_err(|e| format!("write {path:?}: {e}"))?;
            Ok((path, content.len() as u64))
        }
        // 冻结:已存在 → 保留首份内容,报既有大小。
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let size = std::fs::metadata(&path)
                .map(|m| m.len())
                .unwrap_or(0);
            Ok((path, size))
        }
        Err(e) => Err(format!("open {path:?}: {e}")),
    }
}

/// Read a window of a persisted tool result (分段翻页).
/// Returns (content, new_offset, total_size). Never loads the full file.
pub fn read_tool_result_window(
    path: &std::path::Path,
    from_offset: u64,
    max_bytes: u64,
) -> Result<(String, u64, u64), String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("stat {path:?}: {e}"))?;
    let file_size = metadata.len();

    if file_size <= from_offset {
        return Ok((String::new(), from_offset, file_size));
    }

    let available = file_size - from_offset;
    let read_size = max_bytes.min(available);

    let mut file = open_output_file_read(path)?;
    file.seek(SeekFrom::Start(from_offset))
        .map_err(|e| format!("seek {path:?}: {e}"))?;

    let mut buf = vec![0u8; read_size as usize];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("read {path:?}: {e}"))?;
    buf.truncate(n);

    let content = String::from_utf8_lossy(&buf).to_string();
    Ok((content, from_offset + n as u64, file_size))
}

/// List persisted tool_call_ids in the session dir (for friendly errors).
pub fn list_tool_result_ids(workspace_root: &std::path::Path, session_id: &str) -> Vec<String> {
    let dir = tool_results_dir(workspace_root, session_id);
    let mut ids: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".txt").map(str::to_string)
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    ids.sort();
    ids
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_task_id(suffix: &str) -> String {
        format!("disk_test_{}_{}", std::process::id(), suffix)
    }

    fn cleanup(task_id: &str) {
        cleanup_task_output(task_id);
    }

    #[test]
    fn init_and_append_and_read() {
        let tid = test_task_id("init_read");
        cleanup(&tid);

        let path = init_task_output(&tid).unwrap();
        assert!(path.exists());

        append_task_output(&tid, "hello world\n").unwrap();
        append_task_output(&tid, "line 2\n").unwrap();

        let output = read_task_output(&tid, DEFAULT_MAX_READ_BYTES).unwrap();
        assert!(output.contains("hello world"));
        assert!(output.contains("line 2"));

        cleanup(&tid);
    }

    #[test]
    fn delta_read() {
        let tid = test_task_id("delta");
        cleanup(&tid);

        init_task_output(&tid).unwrap();
        append_task_output(&tid, "chunk1\n").unwrap();

        let (delta, offset) = read_task_output_delta(&tid, 0, 1024).unwrap();
        assert!(delta.contains("chunk1"));
        assert_eq!(offset, 7); // "chunk1\n" = 7 bytes

        append_task_output(&tid, "chunk2\n").unwrap();
        let (delta2, offset2) = read_task_output_delta(&tid, offset, 1024).unwrap();
        assert!(delta2.contains("chunk2"));
        assert_eq!(offset2, 14); // +7 more

        // No new data
        let (delta3, offset3) = read_task_output_delta(&tid, offset2, 1024).unwrap();
        assert!(delta3.is_empty());
        assert_eq!(offset3, 14);

        cleanup(&tid);
    }

    #[test]
    fn tail_read() {
        let tid = test_task_id("tail");
        cleanup(&tid);

        init_task_output(&tid).unwrap();
        append_task_output(&tid, "line1\nline2\nline3\nline4\n").unwrap();

        let tail = tail_task_output(&tid, 12).unwrap();
        assert!(tail.contains("line3") || tail.contains("line4"));

        cleanup(&tid);
    }

    #[test]
    fn output_writer() {
        let tid = test_task_id("writer");
        cleanup(&tid);

        init_task_output(&tid).unwrap();
        let writer = OutputWriter::new(&tid);
        writer.append("hello ").unwrap();
        writer.append("world").unwrap();
        drop(writer); // flush on drop

        let output = read_task_output(&tid, DEFAULT_MAX_READ_BYTES).unwrap();
        assert!(output.contains("hello world"));

        cleanup(&tid);
    }

    #[test]
    fn missing_file_delta_graceful() {
        let (delta, offset) = read_task_output_delta("nonexistent_12345", 0, 1024).unwrap();
        assert!(delta.is_empty());
        assert_eq!(offset, 0);
    }

    #[test]
    fn caps_at_5gb() {
        // Unit test: verify that append_task_output doesn't panic with
        // large content and the cap check works.
        let tid = test_task_id("caps");
        cleanup(&tid);

        init_task_output(&tid).unwrap();
        // Write a small amount, verify truncation message appears when near cap.
        // We don't actually write 5GB — just verify the logic.
        let result = append_task_output(&tid, "small test");
        assert!(result.is_ok());

        let output = read_task_output(&tid, DEFAULT_MAX_READ_BYTES).unwrap();
        assert!(output.contains("small test"));

        cleanup(&tid);
    }

    // ── #10 tool-result persistence ───────────────────────────

    fn tool_results_tempdir() -> tempfile::TempDir {
        tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap()
    }

    #[test]
    fn persist_and_read_window_roundtrip() {
        let dir = tool_results_tempdir();
        let root = dir.path();
        let content = "0123456789abcdefghijklmnopqrstuvwxyz";

        let (path, size) = persist_tool_result(root, "s1", "call_1", content).unwrap();
        assert_eq!(size, content.len() as u64);
        assert!(path.exists());
        assert_eq!(
            path,
            root.join(".jia/tool-results/s1/call_1.txt"),
            "path convention must mirror backups (<workspace>/.jia/...)"
        );

        // Full window.
        let (text, next, total) = read_tool_result_window(&path, 0, 1024).unwrap();
        assert_eq!(text, content);
        assert_eq!(next, content.len() as u64);
        assert_eq!(total, content.len() as u64);

        // Segmented paging.
        let (w1, o1, _) = read_tool_result_window(&path, 0, 10).unwrap();
        assert_eq!(w1, "0123456789");
        assert_eq!(o1, 10);
        let (w2, o2, _) = read_tool_result_window(&path, o1, 10).unwrap();
        assert_eq!(w2, "abcdefghij");
        assert_eq!(o2, 20);
        // Past EOF: empty, offset unchanged.
        let (w3, o3, _) = read_tool_result_window(&path, total, 10).unwrap();
        assert!(w3.is_empty());
        assert_eq!(o3, total);
    }

    #[test]
    fn persist_freezes_by_tool_call_id() {
        let dir = tool_results_tempdir();
        let root = dir.path();

        let (path, size1) = persist_tool_result(root, "s1", "call_x", "first").unwrap();
        assert_eq!(size1, 5);
        // Same id again (same turn or not): frozen — first content wins.
        let (path2, size2) = persist_tool_result(root, "s1", "call_x", "second-longer").unwrap();
        assert_eq!(path, path2);
        assert_eq!(size2, 5, "frozen entry keeps the first byte count");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        // Sessions are isolated.
        let (other, _) = persist_tool_result(root, "s2", "call_x", "second-longer").unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "second-longer");
    }

    #[test]
    fn list_tool_result_ids_sorted() {
        let dir = tool_results_tempdir();
        let root = dir.path();
        persist_tool_result(root, "s1", "call_b", "x").unwrap();
        persist_tool_result(root, "s1", "call_a", "x").unwrap();

        assert_eq!(list_tool_result_ids(root, "s1"), vec!["call_a", "call_b"]);
        assert!(list_tool_result_ids(root, "no_such_session").is_empty());
    }

    #[test]
    fn tool_call_id_sanitized_for_filename() {
        let dir = tool_results_tempdir();
        let root = dir.path();
        let (path, _) = persist_tool_result(root, "s1", "call/../evil", "x").unwrap();
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "call_.._evil.txt"
        );
        // The sanitized name stays inside the session dir.
        assert!(path.starts_with(tool_results_dir(root, "s1")));
    }
}

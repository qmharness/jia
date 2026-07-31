use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;
use std::path::Path;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// N5 · 艮八宫(癸·藏)——备份是"藏",回滚是"藏之取";备份只增不删。
pub struct RevertFileTool {}

impl Default for RevertFileTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RevertFileTool {
    pub fn new() -> Self {
        Self {}
    }
}

/// One available backup: the timestamp dir name (unix seconds) and file size.
#[derive(Debug)]
struct BackupEntry {
    ts: u64,
    size: u64,
}

/// Scan `backup_root/<ts>/<fname>` for backups of this file, newest first.
/// Internal path (same treatment as backup writes in write_file/patch_file):
/// not user-visible, so no verify_path.
async fn list_backups(backup_root: &Path, fname: &std::ffi::OsStr) -> Vec<BackupEntry> {
    let mut out = Vec::new();
    if let Ok(mut dirs) = tokio::fs::read_dir(backup_root).await {
        while let Ok(Some(entry)) = dirs.next_entry().await {
            let Ok(ts) = entry.file_name().to_string_lossy().parse::<u64>() else {
                continue;
            };
            let path = entry.path().join(fname);
            if let Ok(meta) = tokio::fs::metadata(&path).await
                && meta.is_file()
            {
                out.push(BackupEntry {
                    ts,
                    size: meta.len(),
                });
            }
        }
    }
    out.sort_by(|a, b| b.ts.cmp(&a.ts));
    out
}

/// Resolve the `backup` parameter to an index into `entries` (newest first).
/// An exact timestamp match wins; otherwise the value is a 1-based index
/// (1 = most recent). Absent parameter → most recent (index 0).
fn select_backup(entries: &[BackupEntry], backup: Option<&Value>) -> Result<usize, String> {
    match backup {
        None | Some(Value::Null) => Ok(0),
        Some(v) => {
            let s = v
                .as_str()
                .map(str::to_string)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
                .ok_or("Invalid 'backup' parameter: expected a timestamp or 1-based index")?;
            let n: u64 = s.trim().parse().map_err(|_| {
                format!(
                    "Invalid 'backup' parameter '{s}': expected a timestamp or 1-based index (1 = most recent)"
                )
            })?;
            if let Some(i) = entries.iter().position(|e| e.ts == n) {
                Ok(i)
            } else if n >= 1 && n as usize <= entries.len() {
                Ok((n - 1) as usize)
            } else {
                Err(format!(
                    "No backup matching '{s}' ({} available, newest first)",
                    entries.len()
                ))
            }
        }
    }
}

#[async_trait]
impl BaseTool for RevertFileTool {
    fn name(&self) -> &str {
        "revert_file"
    }

    fn description(&self) -> String {
        "Revert a file to a previous backup, undoing edits made by \
         write_file/patch_file (they automatically back up a file before \
         modifying it; backups are never deleted). Pass dry_run=true to list \
         the available backups for the file (timestamp, size) without \
         changing anything, then pick one with the backup parameter. Before \
         reverting, the current file content is itself backed up, so a \
         revert can be reverted. Freshness gate: like write_file, reverting \
         a file you have not read recently in this session is rejected; a \
         successful revert refreshes the gate."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ji
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to revert"
                },
                "backup": {
                    "type": "string",
                    "description": "Which backup to restore: a timestamp (as listed by dry_run) or a 1-based index where 1 is the most recent. Omit for the most recent backup."
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "List available backups without reverting (default false)"
                }
            },
            "required": ["path"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn accesses(&self, input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // U1: declares exactly the target path; the backup dir is internal
        // (timestamped per-second dirs keep disjoint names).
        match input["path"].as_str() {
            Some(p) if !p.is_empty() => {
                crate::palaces::zhen_tool::base::ToolAccesses::write_only(vec![
                    std::path::PathBuf::from(p),
                ])
            }
            _ => crate::palaces::zhen_tool::base::ToolAccesses::all(),
        }
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
        let dry_run = input["dry_run"].as_bool().unwrap_or(false);

        let canonical = ctx.permissions.verify_path(path, PathOp::Write)?;
        let fname = canonical.file_name().ok_or("invalid filename")?;

        let backups = list_backups(&ctx.permissions.backup_dir, fname).await;

        if backups.is_empty() {
            let msg = format!(
                "No backups found for '{}'. Backups are created automatically \
                 before write_file/patch_file modifies a file.",
                canonical.display()
            );
            return if dry_run { Ok(msg) } else { Err(msg.into()) };
        }

        if dry_run {
            let mut out = format!("Backups for '{}' (newest first):\n", canonical.display());
            for (i, b) in backups.iter().enumerate() {
                out.push_str(&format!("  [{}] ts={} size={} bytes\n", i + 1, b.ts, b.size));
            }
            out.push_str("Pass backup=<ts or index> without dry_run to restore one.");
            return Ok(out);
        }

        let idx = select_backup(&backups, input.get("backup"))?;
        let chosen = &backups[idx];
        let backup_path = ctx
            .permissions
            .backup_dir
            .join(chosen.ts.to_string())
            .join(fname);

        // Freshness gate (#4): reverting overwrites the current file — like
        // write_file, require a recent read if the file exists.
        if let Ok(meta) = tokio::fs::metadata(&canonical).await
            && let Ok(mtime) = meta.modified()
        {
            ctx.check_freshness(&canonical, mtime)
                .map_err(ToolError::PermissionDenied)?;
        }

        // 备份只增不删: back up the current state first, so the revert itself
        // can be reverted. If this second's dir already holds a backup of the
        // file (same-second collision), bump the timestamp — never overwrite
        // an existing backup.
        if let Ok(current) = tokio::fs::read(&canonical).await {
            let mut ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            while ctx
                .permissions
                .backup_dir
                .join(ts.to_string())
                .join(fname)
                .exists()
            {
                ts += 1;
            }
            let backup_dir = ctx.permissions.backup_dir.join(ts.to_string());
            if tokio::fs::create_dir_all(&backup_dir).await.is_ok() {
                let _ = tokio::fs::write(backup_dir.join(fname), &current).await;
            }
        }

        tokio::fs::copy(&backup_path, &canonical)
            .await
            .map_err(|e| format!("revert_file error: {e}"))?;

        // Update read_state after successful revert (#4 write-then-read rule)
        if let Ok(meta) = tokio::fs::metadata(&canonical).await
            && let Ok(mtime) = meta.modified()
        {
            ctx.record_read(canonical.clone(), mtime);
        }

        Ok(format!(
            "Reverted {} to backup ts={} ({} bytes)",
            canonical.display(),
            chosen.ts,
            chosen.size
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stems::action::ExecContext;
    use std::path::PathBuf;

    fn test_ctx(backup_dir: &Path) -> ExecContext {
        use crate::palaces::kun_config::SecuritySection;
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        let workspace = std::env::current_dir().unwrap();
        ExecContext::new(Arc::new(PermissionMatrix::from_config(
            &SecuritySection::default(),
            &workspace,
            backup_dir.to_path_buf(),
        )))
    }

    fn test_dir() -> tempfile::TempDir {
        tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap()
    }

    /// Temp dir holding `test.txt` plus a sibling `backups/` dir; returns
    /// (dir, file path, backup root).
    fn with_file(content: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = test_dir();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, content).unwrap();
        let backup_root = dir.path().join("backups");
        std::fs::create_dir_all(&backup_root).unwrap();
        (dir, path, backup_root)
    }

    fn seed_backup(backup_root: &Path, ts: u64, content: &str) {
        let d = backup_root.join(ts.to_string());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("test.txt"), content).unwrap();
    }

    fn record_fresh(ctx: &ExecContext, path: &Path) {
        let mtime = std::fs::metadata(path).unwrap().modified().unwrap();
        ctx.record_read(path.to_path_buf(), mtime);
    }

    #[tokio::test]
    async fn revert_no_backups_friendly_error() {
        let (_dir, path, backup_root) = with_file("hello\n");
        let ctx = test_ctx(&backup_root);
        record_fresh(&ctx, &path);

        let tool = RevertFileTool::new();
        let result = tool
            .execute(serde_json::json!({"path": path.to_string_lossy()}), &ctx)
            .await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("No backups found"), "unexpected error: {err}");
        assert!(err.contains("test.txt"));
    }

    #[tokio::test]
    async fn revert_dry_run_lists_backups() {
        let (_dir, path, backup_root) = with_file("current\n");
        seed_backup(&backup_root, 1000, "old\n");
        seed_backup(&backup_root, 2000, "older-content-v2\n");
        let ctx = test_ctx(&backup_root);

        let tool = RevertFileTool::new();
        let out = tool
            .execute(
                serde_json::json!({"path": path.to_string_lossy(), "dry_run": true}),
                &ctx,
            )
            .await
            .unwrap();
        // Newest first, with sizes.
        assert!(out.contains("[1] ts=2000"), "listing: {out}");
        assert!(out.contains("[2] ts=1000"), "listing: {out}");
        assert!(out.contains("size=4 bytes"), "listing: {out}");
        assert!(out.contains("size=17 bytes"), "listing: {out}");
        // dry_run must not touch the file.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "current\n");
    }

    #[tokio::test]
    async fn revert_dry_run_no_backups_is_ok_note() {
        let (_dir, path, backup_root) = with_file("hello\n");
        let ctx = test_ctx(&backup_root);

        let tool = RevertFileTool::new();
        let out = tool
            .execute(
                serde_json::json!({"path": path.to_string_lossy(), "dry_run": true}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("No backups found"), "note: {out}");
    }

    #[tokio::test]
    async fn revert_to_latest_by_default() {
        let (_dir, path, backup_root) = with_file("modified\n");
        seed_backup(&backup_root, 1000, "original\n");
        seed_backup(&backup_root, 2000, "second\n");
        let ctx = test_ctx(&backup_root);
        record_fresh(&ctx, &path);

        let tool = RevertFileTool::new();
        let out = tool
            .execute(serde_json::json!({"path": path.to_string_lossy()}), &ctx)
            .await
            .unwrap();
        assert!(out.contains("ts=2000"), "output: {out}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second\n");
    }

    #[tokio::test]
    async fn revert_by_index_and_timestamp() {
        let (_dir, path, backup_root) = with_file("modified\n");
        seed_backup(&backup_root, 1000, "original\n");
        seed_backup(&backup_root, 2000, "second\n");
        let ctx = test_ctx(&backup_root);
        record_fresh(&ctx, &path);

        let tool = RevertFileTool::new();
        // 1-based index, 2 = older backup.
        tool.execute(
            serde_json::json!({"path": path.to_string_lossy(), "backup": "2"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");

        // Explicit timestamp also works (restores ts=1000 again → same content).
        record_fresh(&ctx, &path);
        tool.execute(
            serde_json::json!({"path": path.to_string_lossy(), "backup": "1000"}),
            &ctx,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");

        // Out-of-range index is a friendly error.
        record_fresh(&ctx, &path);
        let err = tool
            .execute(
                serde_json::json!({"path": path.to_string_lossy(), "backup": "99"}),
                &ctx,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("No backup matching '99'"), "error: {err}");
    }

    #[tokio::test]
    async fn revert_refreshes_freshness_gate() {
        let (_dir, path, backup_root) = with_file("modified\n");
        seed_backup(&backup_root, 1000, "original world\n");
        let ctx = test_ctx(&backup_root);
        record_fresh(&ctx, &path);

        let tool = RevertFileTool::new();
        tool.execute(serde_json::json!({"path": path.to_string_lossy()}), &ctx)
            .await
            .unwrap();

        // After the revert, patch_file must not be rejected by the freshness
        // gate — revert updated read_state (write-then-read rule).
        let edit = crate::palaces::zhen_tool::builtin::fs::patch_file::EditTool::new();
        let result = edit
            .execute(
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "old_string": "world",
                    "new_string": "jia"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok(), "patch after revert failed: {:?}", result.err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original jia\n");
    }

    #[tokio::test]
    async fn revert_backs_up_current_state_first() {
        let (_dir, path, backup_root) = with_file("modified\n");
        seed_backup(&backup_root, 1000, "original\n");
        let ctx = test_ctx(&backup_root);
        record_fresh(&ctx, &path);

        let tool = RevertFileTool::new();
        tool.execute(serde_json::json!({"path": path.to_string_lossy()}), &ctx)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original\n");

        // The pre-revert state ("modified\n") was backed up: the newest backup
        // now holds it, and the seeded backup is untouched (只增不删).
        let entries = list_backups(&backup_root, std::ffi::OsStr::new("test.txt")).await;
        assert_eq!(entries.len(), 2, "expected 2 backups: {entries:?}");
        let newest = backup_root
            .join(entries[0].ts.to_string())
            .join("test.txt");
        assert_eq!(std::fs::read_to_string(newest).unwrap(), "modified\n");
        let seeded = backup_root.join("1000").join("test.txt");
        assert_eq!(std::fs::read_to_string(seeded).unwrap(), "original\n");
    }

    #[tokio::test]
    async fn revert_path_outside_root_rejected() {
        let (_dir, _path, backup_root) = with_file("hello\n");
        let ctx = test_ctx(&backup_root);

        let tool = RevertFileTool::new();
        let result = tool
            .execute(serde_json::json!({"path": "/etc/jia-revert-evil"}), &ctx)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("outside project root")
        );
    }

    #[tokio::test]
    async fn revert_missing_params() {
        let (_dir, _path, backup_root) = with_file("hello\n");
        let tool = RevertFileTool::new();
        assert!(
            tool.execute(serde_json::json!({}), &test_ctx(&backup_root))
                .await
                .is_err()
        );
    }
}

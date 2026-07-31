// ── Tool Scheduler — concurrent tool execution (U1) ────────────
//
// Conflict-matrix batching based on per-call `ToolAccesses` declarations
// (kimi-code tool-scheduler/tool-access pattern).
//
// Architecture:
//   - 居天盘 (Heaven Plate): called from the agent loop, not EarthPlate
//   - A2: the tool-level `accesses()` declaration is the SOLE parallelism
//     criterion. Ceremony-derived resource domains are deprecated — the
//     six ceremonies are orthogonal to concurrency (web_fetch is 壬仪 but
//     read-only; enter_worktree is 戊仪 but swaps write-level state).
//   - Every call still goes through the full gate pipeline (谋划短路 →
//     GeJu → pre-tool hooks → HumanPlate 分发模式判定) SERIALLY before
//     entering a batch; only the execute step runs concurrently (公理 3).
//
// Conflict rules:
//   read  vs read                  — never conflict
//   write vs write, disjoint paths — no conflict
//   read  vs write, intersecting   — conflict
//   either side `all: true`        — global barrier (singleton batch)
// Path intersection honors the `recursive` flag (directory = prefix).

use std::path::Path;

use crate::palaces::zhen_tool::base::ToolAccesses;
use crate::palaces::zhen_tool::registry::ToolRegistry;
use crate::stems::action::ToolCall;

// ── Conflict matrix ─────────────────────────────────────────────

/// Do two paths intersect? With `recursive`, a path is a directory prefix.
fn paths_intersect(a: &Path, a_recursive: bool, b: &Path, b_recursive: bool) -> bool {
    if a == b {
        return true;
    }
    if a_recursive && b.starts_with(a) {
        return true;
    }
    if b_recursive && a.starts_with(b) {
        return true;
    }
    false
}

/// Do two access declarations conflict (i.e. must NOT run concurrently)?
///
/// Pure function — unit-tested directly. Conservative on every edge:
/// `all` on either side always conflicts.
pub fn accesses_conflict(a: &ToolAccesses, b: &ToolAccesses) -> bool {
    if a.all || b.all {
        return true;
    }
    // write-write: only disjoint write sets may run in parallel
    for wa in &a.writes {
        for wb in &b.writes {
            if paths_intersect(wa, a.recursive, wb, b.recursive) {
                return true;
            }
        }
    }
    // read-write, both directions
    for r in &a.reads {
        for w in &b.writes {
            if paths_intersect(r, a.recursive, w, b.recursive) {
                return true;
            }
        }
    }
    for w in &a.writes {
        for r in &b.reads {
            if paths_intersect(w, a.recursive, r, b.recursive) {
                return true;
            }
        }
    }
    false
}

// ── plan_batches ────────────────────────────────────────────────

/// Plans execution batches for the tool calls of a single turn.
///
/// Preserves the LLM's intended order: batches run in sequence, and results
/// are written back in declaration order. A call is parallel-eligible iff
/// its `accesses()` declaration is not `all` (A2: the declaration is the
/// sole criterion). Eligible calls accumulate into a pending batch until a
/// call conflicts with any pending member; a non-eligible call (`all` —
/// e.g. shell, enter_worktree, unknown tools) is a barrier: the pending
/// batch is flushed and the barrier becomes a singleton batch.
pub fn plan_batches(calls: &[ToolCall], tools: &ToolRegistry) -> Vec<Vec<ToolCall>> {
    let mut batches: Vec<Vec<ToolCall>> = Vec::new();
    let mut pending: Vec<ToolCall> = Vec::new();
    let mut pending_accesses: Vec<ToolAccesses> = Vec::new();

    for call in calls {
        let accesses = tools
            .get(&call.name)
            // Unknown tool → conservative barrier (公理 4).
            .map(|t| t.accesses(&call.parameters))
            .unwrap_or_else(ToolAccesses::all);

        if accesses.all {
            // Barrier: emit accumulated batch, then this singleton.
            if !pending.is_empty() {
                batches.push(std::mem::take(&mut pending));
                pending_accesses.clear();
            }
            batches.push(vec![call.clone()]);
            continue;
        }

        if pending_accesses
            .iter()
            .any(|pa| accesses_conflict(pa, &accesses))
        {
            // Conflicts with a pending member: flush and start a new batch.
            batches.push(std::mem::take(&mut pending));
            pending_accesses.clear();
        }
        pending.push(call.clone());
        pending_accesses.push(accesses);
    }

    if !pending.is_empty() {
        batches.push(pending);
    }

    batches
}

/// Max concurrent executions within one batch (JoinSet window).
/// Overridable via `JIA_MAX_TOOL_CONCURRENCY`; default 10.
pub fn max_tool_concurrency() -> usize {
    std::env::var("JIA_MAX_TOOL_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(10)
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::zhen_tool::builtin::exec::shell::ShellTool;
    use crate::palaces::zhen_tool::builtin::exec::worktree::EnterWorktreeTool;
    use crate::palaces::zhen_tool::builtin::fs::glob::GlobTool;
    use crate::palaces::zhen_tool::builtin::fs::grep::GrepTool;
    use crate::palaces::zhen_tool::builtin::fs::patch_file::EditTool;
    use crate::palaces::zhen_tool::builtin::fs::read_file::ReadFileTool;
    use crate::palaces::zhen_tool::builtin::fs::write_file::WriteFileTool;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_registry() -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(ReadFileTool::new()));
        reg.register(Arc::new(WriteFileTool::new()));
        reg.register(Arc::new(EditTool::new()));
        reg.register(Arc::new(GrepTool::new()));
        reg.register(Arc::new(GlobTool::new()));
        reg.register(Arc::new(ShellTool::new()));
        reg.register(Arc::new(EnterWorktreeTool::new()));
        reg
    }

    fn tc(name: &str, path: &str) -> ToolCall {
        let parameters = match name {
            "shell" => serde_json::json!({"command": path}),
            "enter_worktree" => serde_json::json!({"name": path}),
            "grep" | "glob" => serde_json::json!({"pattern": "x", "path": path}),
            _ => serde_json::json!({"path": path}),
        };
        ToolCall {
            id: format!("call_{name}_{path}"),
            name: name.to_string(),
            parameters,
        }
    }

    // ── conflict matrix: pure logic ─────────────────────────────

    fn reads(paths: &[&str]) -> ToolAccesses {
        ToolAccesses::read_only(paths.iter().map(PathBuf::from).collect(), false)
    }

    fn writes(paths: &[&str]) -> ToolAccesses {
        ToolAccesses::write_only(paths.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn read_read_never_conflicts() {
        assert!(!accesses_conflict(&reads(&["a.rs"]), &reads(&["a.rs"])));
        assert!(!accesses_conflict(&reads(&["a.rs"]), &reads(&["b.rs"])));
    }

    #[test]
    fn write_write_disjoint_ok_same_path_conflicts() {
        assert!(!accesses_conflict(&writes(&["a.rs"]), &writes(&["b.rs"])));
        assert!(accesses_conflict(&writes(&["a.rs"]), &writes(&["a.rs"])));
    }

    #[test]
    fn read_write_intersection_conflicts() {
        assert!(accesses_conflict(&reads(&["a.rs"]), &writes(&["a.rs"])));
        assert!(accesses_conflict(&writes(&["a.rs"]), &reads(&["a.rs"])));
        assert!(!accesses_conflict(&reads(&["a.rs"]), &writes(&["b.rs"])));
    }

    #[test]
    fn recursive_read_covers_nested_write() {
        let mut rec = reads(&["src"]);
        rec.recursive = true;
        assert!(accesses_conflict(&rec, &writes(&["src/main.rs"])));
        assert!(!accesses_conflict(&rec, &writes(&["tests/t.rs"])));
        // Same flag on the write side: write into "src" dir conflicts with
        // a read of a file under it.
        let mut wrec = writes(&["src"]);
        wrec.recursive = true;
        assert!(accesses_conflict(&wrec, &reads(&["src/lib.rs"])));
    }

    #[test]
    fn all_is_global_barrier() {
        let all = ToolAccesses::all();
        assert!(accesses_conflict(&all, &reads(&["a.rs"])));
        assert!(accesses_conflict(&reads(&["a.rs"]), &all));
        assert!(accesses_conflict(&all, &all));
        // Default is the conservative barrier (公理 4).
        assert!(ToolAccesses::default().all);
    }

    // ── plan_batches ────────────────────────────────────────────

    #[test]
    fn empty_calls() {
        let reg = make_registry();
        assert!(plan_batches(&[], &reg).is_empty());
    }

    #[test]
    fn single_tool_one_batch() {
        let reg = make_registry();
        let batches = plan_batches(&[tc("read_file", "a.rs")], &reg);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
    }

    #[test]
    fn reads_parallel() {
        let reg = make_registry();
        let calls = [
            tc("read_file", "a.rs"),
            tc("grep", "src"),
            tc("glob", "src"),
        ];
        let batches = plan_batches(&calls, &reg);
        // grep/glob read "src" recursively — read-read never conflicts.
        assert_eq!(batches.len(), 1, "all reads should be parallel");
        assert_eq!(batches[0].len(), 3);
    }

    #[test]
    fn disjoint_writes_parallel() {
        let reg = make_registry();
        let calls = [tc("write_file", "a.rs"), tc("patch_file", "b.rs")];
        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 1, "disjoint writes should be parallel");
        assert_eq!(batches[0].len(), 2);
    }

    #[test]
    fn same_path_writes_serial() {
        let reg = make_registry();
        let calls = [tc("write_file", "a.rs"), tc("patch_file", "a.rs")];
        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 2, "same-path writes must serialize");
        assert_eq!(batches[0][0].name, "write_file");
        assert_eq!(batches[1][0].name, "patch_file");
    }

    #[test]
    fn read_write_same_path_serial() {
        let reg = make_registry();
        let calls = [tc("read_file", "a.rs"), tc("write_file", "a.rs")];
        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 2);
        // …and the reverse order serializes too.
        let calls = [tc("write_file", "a.rs"), tc("read_file", "a.rs")];
        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 2);
    }

    #[test]
    fn all_tools_are_barriers() {
        let reg = make_registry();
        // shell declares nothing → All → singleton barrier.
        let calls = [tc("shell", "ls"), tc("read_file", "a.rs"), tc("grep", "src")];
        let batches = plan_batches(&calls, &reg);
        assert!(
            batches
                .iter()
                .any(|b| b.len() == 1 && b[0].name == "shell")
        );
        // Unknown tool → barrier as well.
        let batches = plan_batches(&[tc("nonexistent_tool", "x")], &reg);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 1);
    }

    #[test]
    fn enter_worktree_is_a_barrier() {
        let reg = make_registry();
        // read(A), enter_worktree, read(B) — the worktree swap must land on
        // a serial barrier (B1): 3 batches, worktree alone in the middle.
        let calls = [
            tc("read_file", "a.rs"),
            tc("enter_worktree", "wt1"),
            tc("read_file", "b.rs"),
        ];
        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0][0].name, "read_file");
        assert_eq!(batches[1].len(), 1);
        assert_eq!(batches[1][0].name, "enter_worktree");
        assert_eq!(batches[2][0].name, "read_file");
        assert_eq!(batches[2][0].parameters["path"], "b.rs");
    }

    #[test]
    fn preserves_original_order() {
        let reg = make_registry();
        let calls = [
            tc("read_file", "A"),
            tc("shell", "x"),
            tc("read_file", "B"),
        ];
        let batches = plan_batches(&calls, &reg);
        assert_eq!(batches.len(), 3, "shell is a barrier: 3 batches");
        assert_eq!(batches[0][0].parameters["path"], "A");
        assert_eq!(batches[1][0].name, "shell");
        assert_eq!(batches[2][0].parameters["path"], "B");
    }

    #[test]
    fn missing_path_param_is_conservative_barrier() {
        let reg = make_registry();
        // read_file without a path cannot declare its access → All.
        let call = ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            parameters: serde_json::json!({}),
        };
        let batches = plan_batches(&[call, tc("read_file", "a.rs")], &reg);
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].len(), 1, "undeclared access = singleton");
    }
}

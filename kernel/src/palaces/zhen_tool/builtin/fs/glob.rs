use crate::error::ToolError;
use std::time::SystemTime;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhen_tool::builtin::fs::rg;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// 震三宫 · Glob — file discovery by name pattern.
///
/// Complements `grep` (content search): `glob` finds files by name,
/// `grep` finds text within files. Read-only (戊仪 Wu ceremony),
/// routes to 震三 (Zhen) palace. GeJu evaluates as Direct.
pub struct GlobTool;

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BaseTool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> String {
        "Find files by name pattern (e.g., '**/*.rs', 'src/**/*.toml'). \
         Returns matching file paths sorted by modification time (most \
         recent first). Powered by ripgrep when available; respects \
         .gitignore. Use this to discover files; use `grep` to search \
         their contents."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn accesses(&self, input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // glob walks the base directory → recursive prefix declaration.
        let path = input["path"].as_str().unwrap_or(".");
        crate::palaces::zhen_tool::base::ToolAccesses::read_only(
            vec![std::path::PathBuf::from(path)],
            true,
        )
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, e.g. '**/*.rs', 'src/**/*.toml', '*.md'"
                },
                "path": {
                    "type": "string",
                    "description": "Base directory to search in (default: current directory)"
                },
                "sort_by_mtime": {
                    "type": "boolean",
                    "description": "Sort results by modification time, most recent first (default: true with rg; honored in fallback only)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of paths to return (default: 100)"
                }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, input: Value, ctx: &ExecContext) -> Result<String, ToolError> {
        let pattern = input["pattern"]
            .as_str()
            .ok_or("Missing 'pattern' parameter")?;
        let raw_path = input["path"].as_str().unwrap_or(".");
        let sort_by_mtime = input["sort_by_mtime"].as_bool().unwrap_or(false);
        let max_results = input["max_results"].as_u64().unwrap_or(100) as usize;

        // Sandbox the base directory (confines traversal to project root)
        let search_root = ctx.permissions.verify_path(raw_path, PathOp::Read)?;
        let search_root = if search_root.is_dir() {
            search_root
        } else {
            return Err(format!("path is not a directory: {}", search_root.display()).into());
        };

        let (matches, fallback_used) = match glob_via_rg(pattern, &search_root).await {
            Ok(paths) => (paths, false),
            Err(rg::RgError::NotFound) => (glob_fallback(pattern, &search_root, sort_by_mtime)?, true),
            Err(rg::RgError::Failed(msg)) => return Err(msg.into()),
        };

        let total = matches.len();
        let truncated = total > max_results;
        let mut matches = matches;
        matches.truncate(max_results);

        if matches.is_empty() {
            return Ok(format!("No files matched pattern '{}'", pattern));
        }

        let mut lines: Vec<String> = matches
            .into_iter()
            .map(|p| p.display().to_string())
            .collect();
        if truncated {
            lines.push(format!(
                "... (truncated at {} of {} matches)",
                max_results, total
            ));
        }
        if fallback_used {
            lines.push("[note: rg not found, used built-in glob fallback]".to_string());
        }
        Ok(lines.join("\n"))
    }
}

/// ripgrep path: `rg --files` from the search root, mtime-descending, with
/// sensitive-file double filtering (rg `--glob` prefilter + post-filter).
async fn glob_via_rg(
    pattern: &str,
    search_root: &std::path::Path,
) -> Result<Vec<std::path::PathBuf>, rg::RgError> {
    let mut args: Vec<String> = vec![
        "--files".into(),
        "--hidden".into(),
        "--sortr=modified".into(),
    ];
    // User pattern first: rg globs are last-match-wins, so the sensitive
    // prefilters must come after it to stay effective.
    args.push("--glob".into());
    args.push(pattern.to_string());
    for g in rg::prefilter_globs() {
        args.push("--glob".into());
        args.push(g);
    }

    let mut out = rg::run(&args, search_root).await?;
    if out.exit_code == Some(2) && out.stderr.contains("--sortr") {
        // Older rg without `--sortr`: retry unsorted.
        args.retain(|a| a != "--sortr=modified");
        out = rg::run(&args, search_root).await?;
    }

    // rg exit codes: 0 = matches, 1 = no matches (not an error), 2+ = error.
    // Files vanishing mid-walk are a filesystem race, not a search failure.
    if let Some(code) = out.exit_code
        && code > 1
    {
        let (fatal, _) = rg::partition_stderr(&out.stderr);
        if !fatal.is_empty() {
            return Err(rg::RgError::Failed(format!(
                "ripgrep failed (exit {code}): {}",
                fatal.join("\n")
            )));
        }
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut matches = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let path = search_root.join(line);
        // Defense in depth: canonicalize to prevent `..` traversal bypass,
        // and re-check sensitivity on the concrete path.
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        if !canonical.starts_with(search_root) || rg::is_sensitive_path(&canonical) {
            continue;
        }
        matches.push(path);
    }
    Ok(matches)
}

/// Built-in fallback (glob crate) for systems without an rg binary.
fn glob_fallback(
    pattern: &str,
    search_root: &std::path::Path,
    sort_by_mtime: bool,
) -> Result<Vec<std::path::PathBuf>, ToolError> {
    // Compose full glob pattern: <root>/<pattern>
    let full_pattern = format!("{}/{}", search_root.display(), pattern);

    let mut matches: Vec<(std::path::PathBuf, Option<SystemTime>)> = glob::glob(&full_pattern)
        .map_err(|e| format!("invalid glob pattern '{pattern}': {e}"))?
        .filter_map(|r| r.ok())
        // Defense in depth: canonicalize to prevent `..` traversal bypass
        .filter(|p| {
            p.canonicalize()
                .map(|cp| cp.starts_with(search_root))
                .unwrap_or(false)
        })
        .filter(|p| p.is_file())
        .filter(|p| !rg::is_sensitive_path(p))
        .map(|p| {
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, mtime)
        })
        .collect();

    if sort_by_mtime {
        // Most recent first; entries without mtime sort last
        matches.sort_by(|a, b| b.1.cmp(&a.1));
    }

    Ok(matches.into_iter().map(|(p, _)| p).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;

    fn test_ctx() -> ExecContext {
        ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    #[tokio::test]
    async fn glob_finds_rs_files() {
        let tool = GlobTool::new();
        let ctx = test_ctx();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "*.rs",
                    "path": "src/palaces/zhen_tool/builtin/fs"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok(), "glob failed: {:?}", result.err());
        let out = result.unwrap();
        assert!(out.contains("grep.rs"), "expected grep.rs in: {out}");
        assert!(out.contains("glob.rs"), "expected glob.rs in: {out}");
    }

    #[tokio::test]
    async fn glob_recursive_double_star() {
        let tool = GlobTool::new();
        let ctx = test_ctx();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "**/*.toml",
                    "path": "."
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok(), "glob failed: {:?}", result.err());
        let out = result.unwrap();
        assert!(out.contains("Cargo.toml"), "expected Cargo.toml in: {out}");
    }

    #[tokio::test]
    async fn glob_no_match() {
        let tool = GlobTool::new();
        let ctx = test_ctx();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "this_does_not_exist_*.xyz"
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("No files matched"));
    }

    #[tokio::test]
    async fn glob_missing_pattern() {
        let tool = GlobTool::new();
        let ctx = test_ctx();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn glob_max_results_truncates() {
        let tool = GlobTool::new();
        let ctx = test_ctx();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "**/*.rs",
                    "path": "src",
                    "max_results": 2
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok());
        let out = result.unwrap();
        // Truncation banner present when more than 2 .rs files exist under src
        assert!(
            out.contains("truncated at 2 of") || out.lines().count() <= 2,
            "unexpected output: {out}"
        );
    }

    #[tokio::test]
    async fn glob_excludes_sensitive_env() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1\n").unwrap();
        std::fs::write(dir.path().join(".env.example"), "SECRET=\n").unwrap();
        std::fs::write(dir.path().join("visible.txt"), "hi\n").unwrap();

        let tool = GlobTool::new();
        let ctx = test_ctx();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "**/.env*",
                    "path": dir.path().to_string_lossy()
                }),
                &ctx,
            )
            .await;
        assert!(result.is_ok(), "glob failed: {:?}", result.err());
        let out = result.unwrap();
        assert!(
            !out.lines().any(|l| l.ends_with("/.env") || l.ends_with("\\.env")),
            "must not list .env: {out}"
        );
        assert!(
            out.contains(".env.example"),
            "template should stay discoverable: {out}"
        );
    }
}

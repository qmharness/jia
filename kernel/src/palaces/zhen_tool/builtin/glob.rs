use crate::error::ToolError;
use std::time::SystemTime;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
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
         Returns matching file paths, optionally sorted by modification time \
         (most recent first). Use this to discover files; use `grep` to search \
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
                    "description": "Sort results by modification time, most recent first (default: false)"
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

        // Compose full glob pattern: <root>/<pattern>
        let full_pattern = format!("{}/{}", search_root.display(), pattern);

        let mut matches: Vec<(std::path::PathBuf, Option<SystemTime>)> = glob::glob(&full_pattern)
            .map_err(|e| format!("invalid glob pattern '{pattern}': {e}"))?
            .filter_map(|r| r.ok())
            // Defense in depth: canonicalize to prevent `..` traversal bypass
            .filter(|p| {
                p.canonicalize()
                    .map(|cp| cp.starts_with(&search_root))
                    .unwrap_or(false)
            })
            .filter(|p| p.is_file())
            .map(|p| {
                let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
                (p, mtime)
            })
            .collect();

        if sort_by_mtime {
            // Most recent first; entries without mtime sort last
            matches.sort_by(|a, b| b.1.cmp(&a.1));
        }

        let total = matches.len();
        let truncated = total > max_results;
        matches.truncate(max_results);

        if matches.is_empty() {
            return Ok(format!("No files matched pattern '{}'", pattern));
        }

        let mut lines: Vec<String> = matches
            .into_iter()
            .map(|(p, _)| p.display().to_string())
            .collect();
        if truncated {
            lines.push(format!(
                "... (truncated at {} of {} matches)",
                max_results, total
            ));
        }
        Ok(lines.join("\n"))
    }
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
                    "path": "src/palaces/zhen_tool/builtin"
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
}

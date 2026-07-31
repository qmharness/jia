use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhen_tool::builtin::fs::rg;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

/// Pagination default, aligned with the kimi-code Grep tool.
const DEFAULT_HEAD_LIMIT: usize = 250;

pub struct GrepTool;

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GrepTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BaseTool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> String {
        "Search for a text pattern in files under a directory (powered by ripgrep \
         when available; respects .gitignore). Returns matching lines with file \
         path and line number. Supports glob filtering (e.g., '*.rs', '*.toml')."
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
        // grep recurses into directories → recursive prefix declaration.
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
                    "description": "Text pattern to search for (plain substring match, not regex)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in (default: current directory)"
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob pattern to filter files (e.g., '*.rs')"
                },
                "ignore_case": {
                    "type": "boolean",
                    "description": "Case-insensitive matching (default: true)"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Number of context lines to show around each match (rg -C)"
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (default: 250)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip this many matching lines before returning results (default: 0)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Legacy alias for head_limit"
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
        let glob = input["glob"].as_str();
        // Legacy behavior was a case-insensitive substring match; keep it default.
        let ignore_case = input["ignore_case"].as_bool().unwrap_or(true);
        let context_lines = input["context_lines"].as_u64();
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        // head_limit is the pagination knob; max_results survives as an alias.
        let head_limit = input["head_limit"]
            .as_u64()
            .or_else(|| input["max_results"].as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_HEAD_LIMIT);

        let search_root = ctx.permissions.verify_path(raw_path, PathOp::Read)?;

        match search_via_rg(
            &search_root,
            pattern,
            glob,
            ignore_case,
            context_lines,
            offset,
            head_limit,
            &ctx.permissions.sandbox.workspace_root,
        )
        .await
        {
            Ok(output) => Ok(output),
            Err(rg::RgError::NotFound) => {
                // rg is not installed: fall back to the built-in walker and
                // say so in the output.
                let mut results = if search_root.is_file() {
                    search_single_file(&search_root, pattern, offset + head_limit)?
                } else {
                    search_dir(
                        &search_root,
                        pattern,
                        glob,
                        offset + head_limit,
                        &ctx.permissions.sandbox.blocked_prefixes,
                    )?
                };
                if offset > 0 {
                    results = results.into_iter().skip(offset).collect();
                }
                let mut out = String::from("[note: rg not found, used built-in fallback search]");
                if results.is_empty() {
                    out.push_str(&format!("\nNo matches found for '{pattern}'"));
                } else {
                    out.push('\n');
                    out.push_str(&results.join("\n"));
                }
                Ok(out)
            }
            Err(rg::RgError::Failed(msg)) => Err(msg.into()),
        }
    }
}

/// ripgrep path: fixed-string content search with sensitive-file double
/// filtering (rg `--glob` prefilter + post-filter on each result path).
#[allow(clippy::too_many_arguments)]
async fn search_via_rg(
    search_root: &std::path::Path,
    pattern: &str,
    glob: Option<&str>,
    ignore_case: bool,
    context_lines: Option<u64>,
    offset: usize,
    head_limit: usize,
    workspace_root: &std::path::Path,
) -> Result<String, rg::RgError> {
    let mut args: Vec<String> = vec![
        "--hidden".into(),
        "--max-columns".into(),
        "500".into(),
        "--fixed-strings".into(),
        "--with-filename".into(),
        "--line-number".into(),
        // NUL after the path makes the post-filter split unambiguous.
        "--null".into(),
    ];
    if ignore_case {
        args.push("--ignore-case".into());
    }
    if let Some(c) = context_lines {
        args.push("--context".into());
        args.push(c.to_string());
    }
    if let Some(g) = glob {
        args.push("--glob".into());
        args.push(g.to_string());
    }
    for g in rg::prefilter_globs() {
        args.push("--glob".into());
        args.push(g);
    }
    args.push("--".into());
    args.push(pattern.to_string());
    args.push(search_root.to_string_lossy().into_owned());

    let out = rg::run(&args, workspace_root).await?;

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

    // Post-filter: drop lines from sensitive files, restore `path:line:...`.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines: Vec<String> = Vec::new();
    let mut filtered_sensitive = 0usize;
    for raw in stdout.lines() {
        if raw == "--" {
            lines.push(raw.to_string());
            continue;
        }
        if let Some(nul) = raw.find('\0') {
            let path = std::path::Path::new(&raw[..nul]);
            if rg::is_sensitive_path(path) {
                filtered_sensitive += 1;
                continue;
            }
            lines.push(raw.replacen('\0', ":", 1));
        } else if !raw.is_empty() {
            lines.push(raw.to_string());
        }
    }

    let total = lines.len();
    let after_offset: Vec<String> = lines.into_iter().skip(offset).collect();
    let limited = after_offset.len() > head_limit;
    let page: Vec<String> = after_offset.into_iter().take(head_limit).collect();

    if page.is_empty() && !out.timed_out {
        return Ok(if filtered_sensitive > 0 {
            "No non-sensitive matches found".to_string()
        } else {
            format!("No matches found for '{pattern}'")
        });
    }

    let mut result = page.join("\n");
    if limited {
        result.push_str(&format!(
            "\nResults truncated to {head_limit} lines (total: {total}). Use offset={} to see more.",
            offset + head_limit
        ));
    }
    if filtered_sensitive > 0 {
        result.push_str(&format!(
            "\nFiltered {filtered_sensitive} line(s) from sensitive files."
        ));
    }
    if out.stdout_truncated {
        result.push_str(&format!(
            "\n[stdout truncated at {} bytes; increase head_limit/offset paging or narrow the search]",
            rg::MAX_OUTPUT_BYTES
        ));
    }
    if out.timed_out {
        result.push_str(&format!(
            "\n[grep timed out after {}s; partial results returned]",
            rg::RG_TIMEOUT.as_secs()
        ));
    }
    Ok(result)
}

fn search_single_file(
    path: &std::path::Path,
    pattern: &str,
    max_results: usize,
) -> Result<Vec<String>, String> {
    let data = std::fs::read(path).map_err(|_| format!("Failed to read {}", path.display()))?;
    if is_binary(&data) {
        return Ok(Vec::new());
    }
    let content = String::from_utf8_lossy(&data);
    let pattern_lower = pattern.to_lowercase();
    let mut results = Vec::new();
    for (line_num, line) in content.lines().enumerate() {
        if line.to_lowercase().contains(&pattern_lower) {
            results.push(format!("{}:{}: {}", path.display(), line_num + 1, line));
            if results.len() >= max_results {
                break;
            }
        }
    }
    Ok(results)
}

fn search_dir(
    dir: &std::path::Path,
    pattern: &str,
    glob: Option<&str>,
    max_results: usize,
    blocked_prefixes: &[String],
) -> Result<Vec<String>, String> {
    let mut results = Vec::new();

    for entry in walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        // Check blocked prefixes for each file in traversal
        let path = entry.path();
        let blocked = blocked_prefixes
            .iter()
            .any(|p| path.to_string_lossy().contains(p.as_str()));
        if blocked || rg::is_sensitive_path(path) {
            continue;
        }

        if let Some(g) = glob
            && let Some(filename) = entry.file_name().to_str()
            && !glob_match(g, filename)
        {
            continue;
        }

        let file_results =
            search_in_file(&entry, pattern, max_results.saturating_sub(results.len()))?;
        results.extend(file_results);
        if results.len() >= max_results {
            results.push(format!("... (truncated at {} results)", max_results));
            break;
        }
    }

    Ok(results)
}

fn search_in_file(
    entry: &walkdir::DirEntry,
    pattern: &str,
    max_results: usize,
) -> Result<Vec<String>, String> {
    search_single_file(entry.path(), pattern, max_results)
}

fn is_binary(data: &[u8]) -> bool {
    data.iter().take(8192).any(|&b| b == 0)
}

/// Simple glob matching: supports `*` wildcard.
fn glob_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return name == pattern;
    }
    if !name.starts_with(parts[0]) {
        return false;
    }
    let mut cursor = parts[0].len();
    for part in &parts[1..parts.len() - 1] {
        match name[cursor..].find(part) {
            Some(pos) => cursor += pos + part.len(),
            None => return false,
        }
    }
    let last = parts.last().unwrap_or(&"");
    last.is_empty() || name[cursor..].ends_with(last)
}

#[cfg(test)]
mod tests {
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*.rs", "foo.rs"));
        assert!(glob_match("*.rs", "bar.rs"));
        assert!(!glob_match("*.rs", "foo.txt"));
        assert!(glob_match("test_*", "test_grep"));
        assert!(!glob_match("test_*", "tests/grep"));
        assert!(glob_match("*.toml", "Cargo.toml"));
    }

    #[test]
    fn test_is_binary() {
        assert!(is_binary(&[0, 1, 2, 3]));
        assert!(!is_binary(b"hello world"));
        assert!(!is_binary(b""));
    }

    #[tokio::test]
    async fn grep_cargo_toml() {
        let tool = GrepTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "package",
                    "path": "Cargo.toml"
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok(), "grep failed: {:?}", result.err());
        assert!(result.unwrap().contains("[package]"));
    }

    #[tokio::test]
    async fn grep_src_dir_rs_files() {
        let tool = GrepTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "pub struct",
                    "path": "src",
                    "glob": "*.rs",
                    "max_results": 10
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok(), "grep failed: {:?}", result.err());
        let output = result.unwrap();
        assert!(
            output.contains("pub struct"),
            "should find struct definitions: {output}"
        );
    }

    #[tokio::test]
    async fn grep_missing_pattern() {
        let tool = GrepTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn grep_no_match() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "xyznonexistent123",
                    "path": dir.path().to_string_lossy()
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok(), "grep failed: {:?}", result.err());
        let output = result.unwrap();
        assert!(
            output.contains("No matches found"),
            "unexpected output: {output}"
        );
    }

    #[tokio::test]
    async fn grep_head_limit_and_offset() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        let body: String = (1..=20).map(|i| format!("matchline {i}\n")).collect();
        std::fs::write(dir.path().join("a.txt"), body).unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "matchline",
                    "path": dir.path().to_string_lossy(),
                    "head_limit": 5,
                    "offset": 2
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok(), "grep failed: {:?}", result.err());
        let output = result.unwrap();
        assert!(
            output.contains("matchline 3"),
            "offset=2 should start at line 3: {output}"
        );
        assert!(
            !output.contains("matchline 2\n") && !output.contains("matchline 2:"),
            "offset=2 should skip first two lines: {output}"
        );
        assert!(
            output.contains("Use offset=7"),
            "truncation banner should guide paging: {output}"
        );
    }

    #[tokio::test]
    async fn grep_filters_sensitive_env() {
        let dir = tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET_TOKEN=hunter2\n").unwrap();
        std::fs::write(dir.path().join(".env.example"), "SECRET_TOKEN=\n").unwrap();
        std::fs::write(dir.path().join("ok.txt"), "SECRET_TOKEN is documented\n").unwrap();

        let tool = GrepTool::new();
        let result = tool
            .execute(
                serde_json::json!({
                    "pattern": "SECRET_TOKEN",
                    "path": dir.path().to_string_lossy()
                }),
                &test_ctx(),
            )
            .await;
        assert!(result.is_ok(), "grep failed: {:?}", result.err());
        let output = result.unwrap();
        assert!(
            !output.contains("hunter2"),
            "must not leak .env contents: {output}"
        );
        assert!(
            output.contains("ok.txt"),
            "non-sensitive file should match: {output}"
        );
    }
}

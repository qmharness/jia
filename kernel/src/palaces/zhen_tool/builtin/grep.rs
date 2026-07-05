use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent, ReadAction};

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
        "Search for a text pattern in files under a directory. \
         Returns matching lines with file path and line number. \
         Supports glob filtering (e.g., '*.rs', '*.toml')."
            .to_string()
    }

    fn category(&self) -> &str {
        "file"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu(ReadAction {
            target: String::new(),
        })
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
                    "description": "Text pattern to search for (plain substring match)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search in (default: current directory)"
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob pattern to filter files (e.g., '*.rs')"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (default: 50)"
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
        let max_results = input["max_results"].as_u64().unwrap_or(50) as usize;

        let search_root = ctx.permissions.verify_path(raw_path, PathOp::Read)?;

        let results = if search_root.is_file() {
            search_single_file(&search_root, pattern, max_results)?
        } else {
            search_dir(&search_root, pattern, glob, max_results)?
        };

        if results.is_empty() {
            Ok(format!("No matches found for '{}'", pattern))
        } else {
            Ok(results.join("\n"))
        }
    }
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
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext {
            permissions: Arc::new(PermissionMatrix::default()),
        }
    }

    use super::*;
    use crate::stems::action::ExecContext;

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

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
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
}

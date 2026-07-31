use crate::error::ToolError;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use crate::palaces::qian_permission::PathOp;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::palaces::zhen_tool::builtin::exec::lsp::{EditDiagnostics, append_post_edit_diagnostics};
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;

pub struct EditTool {
    /// N6 · 可选 LSP 诊断句柄(共享 LspManager)。None 时行为与注入前
    /// 完全一致;拉取失败/超时静默降级,不阻塞主流程。
    diagnostics: Option<Arc<dyn EditDiagnostics>>,
}

impl Default for EditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl EditTool {
    pub fn new() -> Self {
        Self { diagnostics: None }
    }

    pub fn with_diagnostics(diagnostics: Option<Arc<dyn EditDiagnostics>>) -> Self {
        Self { diagnostics }
    }
}

// ── #11 · 行尾视图 ──────────────────────────────────────────
//
// read_file 经 tokio `lines()` 输出 LF 视图(CRLF 行尾的 '\r' 被剥掉),
// 所以 patch 对纯 CRLF 文件也在 LF 视图内匹配/替换,写回时还原 CRLF —
// 两工具视图对齐,"读到的内容"必然能匹配上。混合行尾文件不做视图转换
// (保持现状精确匹配),失败消息中提示。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEndings {
    /// 纯 LF(或无换行)。
    Lf,
    /// 纯 CRLF。
    Crlf,
    /// CRLF 与裸 LF 并存。
    Mixed,
}

fn detect_line_endings(s: &str) -> LineEndings {
    let bytes = s.as_bytes();
    let mut crlf = 0usize;
    let mut bare_lf = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            if i > 0 && bytes[i - 1] == b'\r' {
                crlf += 1;
            } else {
                bare_lf += 1;
            }
        }
    }
    match (crlf, bare_lf) {
        (0, _) => LineEndings::Lf,
        (_, 0) => LineEndings::Crlf,
        (_, _) => LineEndings::Mixed,
    }
}

// ── #11 · 引号归一回退 ──────────────────────────────────────

/// 弯引号⇄直引号归一(char→char,全程保持 char 边界)。
/// 返回 (归一文本, 映射表):映射表每项是 (归一字节偏移, 原文字节偏移),
/// 末尾附 (len, len) 哨兵,供命中区间回译原文切片。
fn normalize_quotes(s: &str) -> (String, Vec<(usize, usize)>) {
    let mut out = String::with_capacity(s.len());
    let mut map: Vec<(usize, usize)> = Vec::new();
    for (orig_idx, c) in s.char_indices() {
        let nc = match c {
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            _ => c,
        };
        map.push((out.len(), orig_idx));
        out.push(nc);
    }
    map.push((out.len(), s.len()));
    (out, map)
}

fn orig_byte(map: &[(usize, usize)], norm_byte: usize) -> Option<usize> {
    map.iter()
        .find(|(n, _)| *n == norm_byte)
        .map(|(_, o)| *o)
}

// ── #11 · 匹配 ─────────────────────────────────────────────

struct Found {
    /// 命中区间(视图内字节偏移,char 边界)。
    start: usize,
    end: usize,
    quote_fallback: bool,
}

enum MatchResult {
    One(Found),
    NotFound,
    Multiple {
        count: usize,
        /// 第二处命中(视图内字节偏移)。
        second: usize,
        quote_fallback: bool,
    },
}

fn find_unique(view: &str, old: &str) -> MatchResult {
    let mut it = view.match_indices(old);
    match it.next() {
        Some((start, _)) => match it.next() {
            Some((second, _)) => MatchResult::Multiple {
                count: 2 + it.count(),
                second,
                quote_fallback: false,
            },
            None => MatchResult::One(Found {
                start,
                end: start + old.len(),
                quote_fallback: false,
            }),
        },
        None => quote_fallback(view, old),
    }
}

/// old_string 未命中时:弯引号⇄直引号归一后再匹配一次。命中区间回译
/// 到原文(视图)字节偏移,替换仍按文件原文进行。
fn quote_fallback(view: &str, old: &str) -> MatchResult {
    let (norm_view, map) = normalize_quotes(view);
    let (norm_old, _) = normalize_quotes(old);
    if norm_old == old && norm_view == view {
        // 没有可归一的引号,结果与直接匹配相同 —— 不再重复搜索。
        return MatchResult::NotFound;
    }
    let mut it = norm_view.match_indices(&norm_old);
    match it.next() {
        None => MatchResult::NotFound,
        Some((ns, _)) => match it.next() {
            Some((ns2, _)) => MatchResult::Multiple {
                count: 2 + it.count(),
                second: orig_byte(&map, ns2).unwrap_or(0),
                quote_fallback: true,
            },
            None => {
                let ne = ns + norm_old.len();
                match (orig_byte(&map, ns), orig_byte(&map, ne)) {
                    (Some(start), Some(end)) => MatchResult::One(Found {
                        start,
                        end,
                        quote_fallback: true,
                    }),
                    _ => MatchResult::NotFound,
                }
            }
        },
    }
}

// ── #11 · 教学化失败消息 ────────────────────────────────────

/// `byte_pos` 所在行 ±2 行上下文(带 1-based 行号)。全部在 char 边界
/// 切片;`view` 可能是 LF 视图,行号与 read_file 所见一致。
fn context_around(view: &str, byte_pos: usize) -> String {
    let line_no = view[..byte_pos].matches('\n').count(); // 0-based
    let lines: Vec<&str> = view.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let start = line_no.saturating_sub(2);
    let end = (line_no + 2).min(lines.len() - 1);
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate().take(end + 1).skip(start) {
        out.push_str(&format!("\n{:>4}| {}", i + 1, line.trim_end()));
    }
    out
}

/// 最相近行:与 old_string 首行共享最长公共前缀(>=6 字符才值得展示)。
/// 前缀长度只作评分(按字节计,不切片),展示整行 —— 无 CJK 切片风险。
fn closest_match_pos(view: &str, old: &str) -> Option<usize> {
    let needle = old.lines().next()?;
    if needle.len() < 6 {
        return None;
    }
    let mut best: Option<(usize, usize)> = None; // (byte_pos, score)
    let mut pos = 0usize;
    for line in view.split_inclusive('\n') {
        let text = line.strip_suffix('\n').unwrap_or(line);
        let common = text
            .bytes()
            .zip(needle.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if common >= 6 && best.is_none_or(|(_, b)| common > b) {
            best = Some((pos, common));
        }
        pos += line.len();
    }
    best.map(|(p, _)| p)
}

fn line_ending_hint(le: LineEndings) -> &'static str {
    match le {
        LineEndings::Mixed => {
            "\nNote: the file has mixed line endings (CRLF + LF); matching is exact \
             against raw bytes — copy the exact text from read_file output."
        }
        LineEndings::Crlf => {
            "\nNote: the file uses CRLF line endings; matching runs in a normalized \
             LF view (the same view read_file shows)."
        }
        LineEndings::Lf => "",
    }
}

#[async_trait]
impl BaseTool for EditTool {
    fn name(&self) -> &str {
        "patch_file"
    }

    fn description(&self) -> String {
        "Perform exact string replacements in an existing file. \
         The old_string must match exactly one location in the file — \
         if it is absent or occurs multiple times, the edit is rejected; \
         include more surrounding context to make it unique. \
         Line endings: CRLF files are matched in the same normalized LF view \
         that read_file shows and are written back as CRLF; mixed-ending files \
         are matched byte-exact. If old_string does not match, a quote-normalized \
         retry (curly quotes ⇄ straight quotes) is attempted automatically. \
         Freshness gate: you MUST read_file this file earlier in the session \
         before patching it; if the file was modified externally since your \
         last read (mtime changed), the edit is rejected and you must \
         read_file it again — your old_string was matched against bytes that \
         no longer exist. A successful patch refreshes the gate, so \
         consecutive edits by you do not require a re-read."
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
                    "description": "Path to the file to edit"
                },
                "old_string": {
                    "type": "string",
                    "description": "The exact text to replace (must be unique in the file)"
                },
                "new_string": {
                    "type": "string",
                    "description": "The text to replace it with"
                }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        false
    }

    fn accesses(&self, input: &Value) -> crate::palaces::zhen_tool::base::ToolAccesses {
        // U1: declares exactly the target path; patches to disjoint files may
        // run in parallel (freshness gate is per-path and read_state is
        // Arc<Mutex>, so concurrent checks are safe).
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
        let old_string = input["old_string"]
            .as_str()
            .ok_or("Missing 'old_string' parameter")?;
        let new_string = input["new_string"]
            .as_str()
            .ok_or("Missing 'new_string' parameter")?;

        let canonical = ctx.permissions.verify_path(path, PathOp::Write)?;

        // Freshness gate (#4): verify agent has read the file recently
        let meta = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| format!("Cannot stat file: {e}"))?;
        let current_mtime = meta.modified().ok();
        if let Some(mtime) = current_mtime {
            ctx.check_freshness(&canonical, mtime)
                .map_err(ToolError::PermissionDenied)?;
        }

        let raw = tokio::fs::read_to_string(&canonical)
            .await
            .map_err(|e| format!("read error: {e}"))?;

        // #11 · 行尾视图:纯 CRLF → LF 视图匹配,写回时还原;混合/LF → 精确匹配。
        let endings = detect_line_endings(&raw);
        let crlf_view = endings == LineEndings::Crlf;
        let view = if crlf_view {
            raw.replace("\r\n", "\n")
        } else {
            raw.clone()
        };
        let old = if crlf_view {
            old_string.replace("\r\n", "\n")
        } else {
            old_string.to_string()
        };
        let new = if crlf_view {
            new_string.replace("\r\n", "\n")
        } else {
            new_string.to_string()
        };

        // freshness 提示(失败消息用):记录 mtime 与当前磁盘 mtime 不一致。
        let stale_hint = {
            let recorded = {
                let mut cache = ctx.read_state.lock().unwrap_or_else(|e| e.into_inner());
                cache.get(&canonical).map(|(m, _)| *m)
            };
            matches!((current_mtime, recorded), (Some(c), Some(r)) if c != r)
        };
        let mut hints = String::new();
        if stale_hint {
            hints.push_str(
                "\nNote: the file was modified on disk since your last read — \
                 read_file it again before retrying.",
            );
        }
        hints.push_str(line_ending_hint(endings));

        match find_unique(&view, &old) {
            MatchResult::NotFound => {
                let mut msg =
                    format!("old_string not found in file '{}' (0 matches).", canonical.display());
                if let Some(pos) = closest_match_pos(&view, &old) {
                    msg.push_str(&format!(
                        " Closest match:{}",
                        context_around(&view, pos)
                    ));
                }
                msg.push_str(&hints);
                Err(msg.into())
            }
            MatchResult::Multiple {
                count,
                second,
                quote_fallback,
            } => {
                let line_no = view[..second].matches('\n').count() + 1;
                let scope = if quote_fallback {
                    " (after quote normalization)"
                } else {
                    ""
                };
                Err(format!(
                    "old_string matches multiple locations in '{}' ({count} matches{scope}). \
                     Must be unique — include more surrounding context. \
                     Second occurrence at line {line_no}:{}",
                    canonical.display(),
                    context_around(&view, second),
                )
                .into())
            }
            MatchResult::One(found) => {
                // Backup original content before mutation
                {
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let backup_dir = ctx.permissions.backup_dir.join(ts.to_string());
                    if tokio::fs::create_dir_all(&backup_dir).await.is_ok()
                        && let Some(fname) = canonical.file_name()
                    {
                        // Save original content (raw bytes, before any view transform)
                        let _ = tokio::fs::write(backup_dir.join(fname), &raw).await;
                    }
                }

                let mut patched = format!(
                    "{}{}{}",
                    &view[..found.start],
                    new,
                    &view[found.end..],
                );
                // 写回时还原 CRLF(视图内只剩 '\n',全部还原)。
                if crlf_view {
                    patched = patched.replace('\n', "\r\n");
                }

                tokio::fs::write(&canonical, &patched)
                    .await
                    .map_err(|e| format!("write error: {e}"))?;

                // Update read_state after successful patch (#4 write-then-read rule)
                if let Ok(meta) = tokio::fs::metadata(&canonical).await {
                    if let Ok(mtime) = meta.modified() {
                        ctx.record_read(canonical.clone(), mtime);
                    }
                }

                let mut result = format!(
                    "Successfully edited {} (1 replacement)",
                    canonical.display()
                );
                if found.quote_fallback {
                    result.push_str(
                        " [quote-normalized fallback: curly quotes matched as straight]",
                    );
                }
                // N6 · 编辑后 LSP 主动诊断(静默降级,不阻塞主流程)
                append_post_edit_diagnostics(&mut result, &self.diagnostics, &canonical).await;
                Ok(result)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    fn test_dir() -> tempfile::TempDir {
        tempfile::TempDir::new_in(std::env::current_dir().unwrap()).unwrap()
    }

    fn with_temp_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = test_dir();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    fn fresh_ctx(path: &std::path::Path) -> crate::stems::action::ExecContext {
        let ctx = test_ctx();
        let mtime = std::fs::metadata(path).unwrap().modified().unwrap();
        ctx.record_read(path.to_path_buf(), mtime);
        ctx
    }

    async fn run_edit(
        tool: &EditTool,
        path: &std::path::Path,
        old: &str,
        new: &str,
        ctx: &crate::stems::action::ExecContext,
    ) -> Result<String, ToolError> {
        tool.execute(
            serde_json::json!({
                "path": path.to_string_lossy(),
                "old_string": old,
                "new_string": new
            }),
            ctx,
        )
        .await
    }

    #[tokio::test]
    async fn edit_single_replacement() {
        let (_dir, path) = with_temp_file("Hello, world!\nThis is a test.\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "world", "Jia", &ctx).await;
        assert!(result.is_ok(), "edit failed: {:?}", result.err());

        let new_content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(new_content, "Hello, Jia!\nThis is a test.\n");
    }

    #[tokio::test]
    async fn edit_not_unique() {
        let (_dir, path) = with_temp_file("foo\nbar\nfoo\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "foo", "baz", &ctx).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("matches multiple locations"), "got: {err}");
        assert!(err.contains("2 matches"), "count in message, got: {err}");
        // 教学化:含第二处行号与上下文
        assert!(err.contains("Second occurrence at line 3"), "got: {err}");
        assert!(err.contains("1| foo"), "context lines, got: {err}");
    }

    #[tokio::test]
    async fn edit_not_found() {
        let (_dir, path) = with_temp_file("hello\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "nonexistent", "x", &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn edit_not_found_shows_closest_context() {
        let content = "fn main() {\n    let target_value = 1;\n    println!(\"{}\", target_value);\n}\n";
        let (_dir, path) = with_temp_file(content);
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "    let target_vAlue = 2;", "x", &ctx).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("(0 matches)"), "match count, got: {err}");
        assert!(err.contains("Closest match"), "closest context, got: {err}");
        assert!(err.contains("let target_value = 1;"), "context body, got: {err}");
    }

    #[tokio::test]
    async fn edit_missing_params() {
        let tool = EditTool::new();
        assert!(
            tool.execute(serde_json::json!({}), &test_ctx())
                .await
                .is_err()
        );
    }

    // ── #11 · 行尾归一往返 ──────────────────────────────────

    #[test]
    fn line_ending_detection() {
        assert_eq!(detect_line_endings("a\nb\n"), LineEndings::Lf);
        assert_eq!(detect_line_endings("a\r\nb\r\n"), LineEndings::Crlf);
        assert_eq!(detect_line_endings("a\r\nb\n"), LineEndings::Mixed);
        assert_eq!(detect_line_endings("no newlines"), LineEndings::Lf);
    }

    #[tokio::test]
    async fn edit_crlf_roundtrip_preserves_crlf() {
        let (_dir, path) = with_temp_file("line one\r\nline two world\r\nline three\r\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        // LF 视图下的 old_string(与 read_file 所见一致)应命中 CRLF 文件。
        let result = run_edit(&tool, &path, "line two world", "line two Jia", &ctx).await;
        assert!(result.is_ok(), "crlf edit failed: {:?}", result.err());

        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text, "line one\r\nline two Jia\r\nline three\r\n");
    }

    #[tokio::test]
    async fn edit_crlf_multiline_old_string() {
        let (_dir, path) = with_temp_file("fn a() {\r\n    body();\r\n}\r\nfn b() {}\r\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(
            &tool,
            &path,
            "fn a() {\n    body();\n}",
            "fn a() {\n    changed();\n}",
            &ctx,
        )
        .await;
        assert!(result.is_ok(), "crlf multiline edit failed: {:?}", result.err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "fn a() {\r\n    changed();\r\n}\r\nfn b() {}\r\n");
    }

    #[tokio::test]
    async fn edit_mixed_line_endings_hint() {
        let (_dir, path) = with_temp_file("alpha\r\nbeta\ngamma\r\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "not in file at all", "x", &ctx).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("mixed line endings"), "got: {err}");
    }

    #[tokio::test]
    async fn edit_mixed_line_endings_exact_match_still_works() {
        let (_dir, path) = with_temp_file("alpha\r\nbeta\ngamma\r\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "beta", "BETA", &ctx).await;
        assert!(result.is_ok(), "mixed edit failed: {:?}", result.err());
        // 混合行尾:精确匹配,不引入视图转换,原文行尾保持。
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "alpha\r\nBETA\ngamma\r\n");
    }

    // ── #11 · 引号归一回退 ──────────────────────────────────

    #[test]
    fn quote_normalization_mapping() {
        let (norm, map) = normalize_quotes("a\u{201C}b\u{201D}c");
        assert_eq!(norm, "a\"b\"c");
        // 弯引号是 3 字节:归一后 b 在偏移 2,原文在偏移 4。
        assert_eq!(orig_byte(&map, 2), Some(4));
        assert_eq!(orig_byte(&map, 0), Some(0));
        assert_eq!(orig_byte(&map, norm.len()), Some(9));
    }

    #[tokio::test]
    async fn edit_quote_fallback_hits_and_annotates() {
        // 文件含弯引号,agent 用直引号匹配 → 回退命中并按原文替换。
        let (_dir, path) = with_temp_file("say \u{201C}hello world\u{201D} loudly\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "\"hello world\"", "\"hi\"", &ctx).await;
        let out = result.expect("quote fallback should hit");
        assert!(out.contains("quote-normalized fallback"), "got: {out}");
        // 命中区间回译到文件原文(new_string 按给定内容插入)。
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "say \"hi\" loudly\n");
    }

    #[tokio::test]
    async fn edit_quote_fallback_curly_old_matches_straight_file() {
        // 反向:文件是直引号,agent 给了弯引号。
        let (_dir, path) = with_temp_file("say \"hello world\" loudly\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "\u{201C}hello world\u{201D}", "X", &ctx).await;
        assert!(result.is_ok(), "reverse quote fallback failed: {:?}", result.err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "say X loudly\n");
    }

    #[tokio::test]
    async fn edit_quote_fallback_still_reports_not_found() {
        let (_dir, path) = with_temp_file("totally different content here\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "\u{201C}missing\u{201D}", "x", &ctx).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("old_string not found"), "got: {err}");
    }

    // ── #11 · CJK char 边界 ─────────────────────────────────

    #[tokio::test]
    async fn edit_cjk_content_no_panic() {
        let (_dir, path) = with_temp_file("中文注释:你好世界\nfn main() {}\n中文尾巴\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "你好世界", "世界你好", &ctx).await;
        assert!(result.is_ok(), "cjk edit failed: {:?}", result.err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "中文注释:世界你好\nfn main() {}\n中文尾巴\n");
    }

    #[tokio::test]
    async fn edit_cjk_multiple_matches_context_no_panic() {
        let (_dir, path) = with_temp_file("你好\n世界\n你好\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::new();
        let result = run_edit(&tool, &path, "你好", "x", &ctx).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("matches multiple locations"), "got: {err}");
        assert!(err.contains("Second occurrence at line 3"), "got: {err}");
    }

    // ── N6 · 编辑后诊断注入 ─────────────────────────────────

    struct MockDiagnostics(Option<String>);
    impl EditDiagnostics for MockDiagnostics {
        fn post_edit_summary(&self, _path: &std::path::Path) -> Option<String> {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn edit_appends_diagnostics_summary() {
        let (_dir, path) = with_temp_file("fn main() { bad }\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::with_diagnostics(Some(Arc::new(MockDiagnostics(Some(
            "\n[LSP 诊断: 1 error — src/main.rs: 1: expected `}`]".to_string(),
        )))));
        let result = run_edit(&tool, &path, "bad", "good", &ctx).await;
        let out = result.unwrap();
        assert!(out.contains("Successfully edited"), "got: {out}");
        assert!(out.contains("[LSP 诊断: 1 error"), "got: {out}");
    }

    #[tokio::test]
    async fn edit_no_diagnostics_appends_nothing() {
        let (_dir, path) = with_temp_file("fn main() {}\n");
        let ctx = fresh_ctx(&path);

        let tool = EditTool::with_diagnostics(Some(Arc::new(MockDiagnostics(None))));
        let out = run_edit(&tool, &path, "main", "main2", &ctx).await.unwrap();
        assert!(!out.contains("LSP"), "silent degrade, got: {out}");

        // None 句柄:行为与注入前完全一致(相同成功消息,无附加)。
        let tool_plain = EditTool::new();
        let out2 = run_edit(&tool_plain, &path, "main2", "main3", &ctx)
            .await
            .unwrap();
        assert_eq!(out, out2);
    }
}

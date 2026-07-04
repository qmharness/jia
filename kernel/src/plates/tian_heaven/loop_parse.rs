//! Extract <tool_call> blocks from LLM responses.
//!
//! Supported formats (DeepSeek, Anthropic, and OpenAI-compatible models vary):
//!
//! 1. JSON body (original format):
//!    `<tool_call>{"tool":"X","parameters":{...}}</tool_call>`
//!
//! 2. XML attribute name + text body (DeepSeek native):
//!    `<tool_call name="X">body text</tool_call>`
//!
//! 3. Container wrapper (DeepSeek multi-tool batches):
//!    `<tool_calls>
//!       <tool_call name="X">...</tool_call>
//!       <tool_call name="Y">...</tool_call>
//!     </tool_calls>`
//!
//! 4. Markdown code blocks (fallback, for models that describe rather than call).
//!
//! 5. Raw tool-name XML tags (fallback, for models like Gemma that emit
//!    `<ask_user question="...">...</ask_user>` instead of `<tool_call>`).

use crate::stems::action::ToolCall;

/// Extract `<tool_call>...</tool_call>` or `<tool_calls>...</tool_calls>` blocks
/// from an LLM response.
///
/// Returns the cleaned text (with tool call blocks removed) and a list of
/// parsed tool calls.
///
/// `tool_names` is an optional list of known tool names. When provided, a
/// final fallback scans for raw XML tags matching those names (e.g.
/// `<ask_user question="...">...</ask_user>`) for models that don't follow
/// the `<tool_call>` convention.
pub fn parse_tool_calls(text: &str, tool_names: &[&str]) -> (String, Vec<ToolCall>) {
    let mut clean_text = String::new();
    let mut tool_calls = Vec::new();
    let mut remaining = text;

    while let Some(tm) = find_next_tag(remaining) {
        clean_text.push_str(&remaining[..tm.start]);

        let after_open = &remaining[tm.open_end..];

        if tm.tag_name == "tool_calls" {
            // ── Container: find matching </tool_calls> with nesting ──
            let (inner, rest) = find_tag_body(after_open, "tool_calls");
            remaining = rest;

            // Recursively scan container content for <tool_call> elements
            // (skip any nested <tool_calls> wrappers — they're transparent)
            if !inner.is_empty() {
                let (_inner_clean, inner_calls) = parse_tool_calls(inner, tool_names);
                tool_calls.extend(inner_calls);
            }
        } else {
            // ── Singular <tool_call> ─────────────────────────────
            let (body, rest) = find_tag_body(after_open, "tool_call");
            if rest.is_empty() && body == after_open {
                // No closing tag found — treat as plain text, don't consume
                clean_text.push_str(&remaining[tm.start..]);
                remaining = "";
                break;
            }
            remaining = rest;

            if let Some(tool_name) = tm.name_attr {
                // XML attribute format: name from attr, body text as param
                let body_trimmed = body.trim();
                let params = if body_trimmed.is_empty() {
                    serde_json::Value::Object(serde_json::Map::new())
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(body_trimmed)
                    && val.is_object()
                {
                    let mut obj = val.as_object().cloned().unwrap_or_default();
                    if !obj.contains_key("tool") && !obj.contains_key("name") {
                        obj.insert(
                            "input".to_string(),
                            serde_json::Value::String(body_trimmed.to_string()),
                        );
                    }
                    serde_json::Value::Object(obj)
                } else {
                    serde_json::json!({ "input": body_trimmed })
                };
                tool_calls.push(ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: tool_name,
                    parameters: params,
                });
            } else {
                // JSON format (existing behavior)
                let inner = body.trim();
                match parse_tool_call_json(inner) {
                    Ok(tc) => tool_calls.push(tc),
                    Err(e) => {
                        tracing::warn!("Failed to parse tool call JSON: {e}");
                    }
                }
            }
        }
    }

    clean_text.push_str(remaining);
    let mut clean_text = clean_text.trim().to_string();

    // ── Fallback: markdown fenced code blocks ──────────────────
    if tool_calls.is_empty() {
        let md_calls = parse_markdown_tool_calls(text);
        if !md_calls.is_empty() {
            tool_calls = md_calls;
        }
    }

    // ── Fallback: raw tool-name XML tags ─────────────────────
    if tool_calls.is_empty() && !tool_names.is_empty() {
        let (raw_clean, raw_calls) = parse_raw_tool_tags(text, tool_names);
        if !raw_calls.is_empty() {
            clean_text = raw_clean;
            tool_calls = raw_calls;
        }
    }

    (clean_text, tool_calls)
}

// ── Tag matching ──────────────────────────────────────────────

/// Info about a found opening tag.
struct TagMatch {
    /// "tool_call" or "tool_calls"
    tag_name: &'static str,
    /// Extracted `name="..."` attribute, if present
    name_attr: Option<String>,
    /// Byte offset of `<` in the original text
    start: usize,
    /// Byte offset just after the `>` of the opening tag
    open_end: usize,
}

/// Find the nearest `<tool_call` or `<tool_calls` opening tag.
///
/// Searches for `<tool_call` (without closing `>`) to match both
/// `<tool_call>` (no attrs) and `<tool_call name="X">` (XML attrs).
///
/// IMPORTANT: `<tool_call` is a prefix of `<tool_calls`, so a naive
/// `find("<tool_call")` would incorrectly match `<tool_calls>` as
/// singular. We must verify the character after the match to distinguish:
///   `<tool_call>`  or `<tool_call `  → singular
///   `<tool_calls>` or `<tool_calls ` → plural
fn find_next_tag(text: &str) -> Option<TagMatch> {
    // Search for singular `<tool_call` — but skip if followed by `s` (plural).
    let call_pos = find_tag_prefix(text, "tool_call", "tool_calls");
    let calls_pos = text.find("<tool_calls");

    let (tag_name, pos) = match (call_pos, calls_pos) {
        (Some(c), Some(cs)) if c <= cs => ("tool_call", c),
        (Some(_), Some(cs)) => ("tool_calls", cs),
        (Some(c), None) => ("tool_call", c),
        (None, Some(cs)) => ("tool_calls", cs),
        (None, None) => return None,
    };

    // Scan forward from pos to find the closing `>` of the opening tag.
    let after_tag = &text[pos..];
    let gt = after_tag.find('>')?;
    let open_tag = &after_tag[..=gt]; // e.g. "<tool_call>" or "<tool_call name=\"shell\">"
    let open_end = pos + gt + 1;

    // Extract name="..." attribute from opening tag
    let name_attr = extract_attr(open_tag, "name");

    // Only accept if this is actually a recognized tool call tag (not a stray
    // "<tool_call" substring in text). The tag must start with exactly
    // "<tool_call" followed by `>` or ` `.
    let valid = open_tag.starts_with("<tool_call>")
        || open_tag.starts_with("<tool_call ")
        || open_tag.starts_with("<tool_calls>")
        || open_tag.starts_with("<tool_calls ");
    if !valid {
        // Skip past this position and retry
        let next = find_next_tag(&text[pos + 1..])?;
        return Some(TagMatch {
            tag_name: next.tag_name,
            name_attr: next.name_attr,
            start: pos + 1 + next.start,
            open_end: pos + 1 + next.open_end,
        });
    }

    Some(TagMatch {
        tag_name,
        name_attr,
        start: pos,
        open_end,
    })
}

/// Extract an XML attribute value. Simple parser — handles `name="value"`.
fn extract_attr(open_tag: &str, attr_name: &str) -> Option<String> {
    let pattern = format!("{attr_name}=\"");
    let start = open_tag.find(&pattern)? + pattern.len();
    let rest = &open_tag[start..];
    let end = rest.find('"')?;
    let value = rest[..end].to_string();
    if value.is_empty() { None } else { Some(value) }
}

/// Find `<{tag_name}` in text, ensuring the match isn't a prefix of a
/// different tag. For example, `<tool_call` is a prefix of `<tool_calls`,
/// so when searching for singular `<tool_call`, we must reject `<tool_calls`.
fn find_tag_prefix(text: &str, tag_name: &str, _exclude: &str) -> Option<usize> {
    let prefix = format!("<{tag_name}");
    let mut search_from = 0;
    loop {
        let pos = text[search_from..].find(&prefix)?;
        let abs = search_from + pos;
        let after = &text[abs + prefix.len()..];
        // The next char must be `>` (no attrs), ` ` (has attrs), or newline
        // (unusual but defensive). If it's `s`, this is actually `<tool_calls>`
        // and we must skip it.
        let next = after.chars().next();
        match next {
            Some('>') | Some(' ') | Some('\n') | Some('\r') => return Some(abs),
            Some('s') => {
                // This is <tool_calls> — skip and keep searching
                search_from = abs + 1;
            }
            _ => return Some(abs), // unrecognised char, accept anyway
        }
    }
}

/// Find the body between an opening tag and its matching closing tag,
/// with proper nesting-depth counting.
///
/// Returns `(body_text, rest_of_text_after_closing_tag)`.
fn find_tag_body<'a>(text: &'a str, tag_name: &str) -> (&'a str, &'a str) {
    let open = format!("<{tag_name}");
    let close = format!("</{tag_name}>");

    let mut depth = 1u32;
    let mut search_pos = 0usize;

    loop {
        let remaining = &text[search_pos..];
        let open_pos = remaining.find(&open);
        let close_pos = remaining.find(&close);

        match (open_pos, close_pos) {
            (_, Some(cp)) if depth == 1 && open_pos.is_none_or(|op| cp < op) => {
                // Found matching close at depth 1
                return (
                    &text[..search_pos + cp],
                    &text[search_pos + cp + close.len()..],
                );
            }
            (Some(op), Some(cp)) if op < cp => {
                depth += 1;
                search_pos += op + open.len();
            }
            (_, Some(cp)) => {
                depth -= 1;
                search_pos += cp + close.len();
            }
            (Some(op), None) => {
                // Opening tag with no close — advance past it
                search_pos += op + open.len();
            }
            _ => {
                // No closing tag found at all
                return (text, "");
            }
        }
    }
}

// ── JSON parsing (existing) ──────────────────────────────────

/// Parse the JSON inside a `<tool_call>` tag into a `ToolCall`.
fn parse_tool_call_json(inner: &str) -> Result<ToolCall, serde_json::Error> {
    let val: serde_json::Value = serde_json::from_str(inner)?;

    let (tool_name, name_from_key) = {
        if let Some(t) = val["tool"].as_str() {
            (t.to_string(), false)
        } else if let Some(t) = val["name"].as_str() {
            (t.to_string(), false)
        } else if let Some((key, _)) = val.as_object().and_then(|o| o.iter().next()) {
            (key.clone(), true)
        } else {
            ("unknown".to_string(), false)
        }
    };

    let params = {
        let explicit_params = val
            .get("parameters")
            .and_then(|v| v.as_object())
            .map(|_| val["parameters"].clone());

        let mut extra: Vec<(String, serde_json::Value)> = Vec::new();
        if let Some(obj) = val.as_object() {
            for (k, v) in obj {
                let is_name_key = name_from_key && k == &tool_name;
                if k != "tool" && k != "name" && k != "parameters" && !is_name_key {
                    extra.push((k.clone(), v.clone()));
                }
            }
        }

        if extra.is_empty() {
            explicit_params.unwrap_or_else(|| {
                val.as_object()
                    .and_then(|o| o.iter().next())
                    .map(|(_, v)| v.clone())
                    .unwrap_or(serde_json::Value::Null)
            })
        } else if let Some(mut base) = explicit_params {
            if let Some(obj) = base.as_object_mut() {
                for (k, v) in extra {
                    if !obj.contains_key(&k) {
                        obj.insert(k, v);
                    }
                }
            }
            base
        } else {
            let obj: serde_json::Map<_, _> = extra.into_iter().collect();
            serde_json::Value::Object(obj)
        }
    };

    Ok(ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: tool_name,
        parameters: params,
    })
}

// ── Markdown fallback (existing) ─────────────────────────────

/// Scan for markdown fenced code blocks that look like tool calls.
fn parse_markdown_tool_calls(text: &str) -> Vec<ToolCall> {
    let mut tool_calls = Vec::new();
    let mut search_start = 0;

    while let Some(fence_start) = text[search_start..].find("```") {
        let abs_fence_start = search_start + fence_start;
        let after_fence = &text[abs_fence_start + 3..];
        let content_start = after_fence.find('\n').map(|n| n + 1).unwrap_or(0);
        let body = &after_fence[content_start..];
        let fence_end = match body.find("\n```") {
            Some(p) => p,
            None => break,
        };
        let code_content = body[..fence_end].trim();
        search_start = abs_fence_start + 3 + content_start + fence_end + 4;

        if code_content.is_empty() {
            if let Some(tool_name) = find_inline_code_tool_before(text, abs_fence_start) {
                tool_calls.push(build_tool_call(&tool_name, ""));
            }
            continue;
        }

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(code_content) {
            if let Some(tc) = build_tool_call_from_json(&val) {
                tool_calls.push(tc);
            } else if val.as_object().is_some_and(|o| o.is_empty())
                && let Some(tool_name) = find_inline_code_tool_before(text, abs_fence_start)
            {
                tool_calls.push(build_tool_call(&tool_name, code_content));
            }
        }
    }

    tool_calls
}

fn find_inline_code_tool_before(text: &str, pos: usize) -> Option<String> {
    let before = &text[..pos];
    let backtick_end = before.rfind('`')?;
    let backtick_start = before[..backtick_end].rfind('`')?;
    let candidate = before[backtick_start + 1..backtick_end].trim();
    if candidate.is_empty() || candidate.contains(' ') {
        return None;
    }
    Some(candidate.to_string())
}

fn build_tool_call(tool_name: &str, json_str: &str) -> ToolCall {
    let params = if json_str.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(json_str).unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    };
    ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: tool_name.to_string(),
        parameters: params,
    }
}

fn build_tool_call_from_json(val: &serde_json::Value) -> Option<ToolCall> {
    let obj = val.as_object()?;
    let (tool_name, params) = if let Some(t) = obj.get("tool").and_then(|v| v.as_str()) {
        let p = obj
            .get("parameters")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        (t.to_string(), p)
    } else if let Some(n) = obj.get("name").and_then(|v| v.as_str()) {
        let p = obj
            .get("parameters")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        (n.to_string(), p)
    } else {
        return None;
    };
    Some(ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: tool_name,
        parameters: params,
    })
}

// ── Raw tool-name XML tag fallback ─────────────────────────────

/// Scan for raw XML tags that match known tool names.
///
/// Some models (e.g. Gemma via LM Studio) emit tool calls as bare XML tags
/// without `<tool_call>` wrapping:
///
/// ```xml
/// <ask_user question="What do you like?">
/// Option A
/// Option B
/// </ask_user>
/// ```
///
/// This parser extracts tag name → `ToolCall.name`, attributes → `parameters`,
/// and body text → `parameters["input"]`.
fn parse_raw_tool_tags(text: &str, tool_names: &[&str]) -> (String, Vec<ToolCall>) {
    let mut clean_text = String::new();
    let mut tool_calls = Vec::new();
    let mut remaining = text;

    loop {
        // Find the earliest occurrence of any <tool_name ...> tag
        let mut best: Option<(usize, usize, &str)> = None; // (start, open_end, name)
        for &name in tool_names {
            let prefix = format!("<{name}");
            if let Some(pos) = remaining.find(&prefix) {
                // Must be followed by `>` or ` ` (not a continuation of tag name)
                let after = &remaining[pos + prefix.len()..];
                let next = after.chars().next().unwrap_or('\0');
                if next == '>' || next == ' ' || next == '\n' || next == '\r' {
                    let open_end = match after.find('>') {
                        Some(gt) => pos + prefix.len() + gt + 1,
                        None => continue,
                    };
                    match best {
                        None => best = Some((pos, open_end, name)),
                        Some((b_pos, _, _)) if pos < b_pos => best = Some((pos, open_end, name)),
                        _ => {}
                    }
                }
            }
        }

        let (start, open_end, name) = match best {
            Some(b) => b,
            None => break,
        };

        // Text before this tag
        clean_text.push_str(&remaining[..start]);

        let after_open = &remaining[open_end..];

        // Find matching closing tag with depth counting
        let (body, rest) = find_tag_body(after_open, name);

        // Extract attributes from opening tag
        let open_tag = &remaining[start..open_end];
        let mut params = extract_all_attrs(open_tag);

        // Body text goes into "input" (unless body is empty)
        let body_trimmed = body.trim();
        if !body_trimmed.is_empty() && !params.contains_key("input") {
            params.insert(
                "input".to_string(),
                serde_json::Value::String(body_trimmed.to_string()),
            );
        }

        tool_calls.push(ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            parameters: serde_json::Value::Object(params),
        });

        remaining = rest;
    }

    clean_text.push_str(remaining);
    (clean_text.trim().to_string(), tool_calls)
}

/// Extract all XML attributes from an opening tag like `<ask_user question="x" timeout="30">`.
fn extract_all_attrs(open_tag: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut attrs = serde_json::Map::new();
    let inner = open_tag
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(open_tag);

    // Skip the tag name
    let after_name = inner.find(' ').map(|p| &inner[p + 1..]).unwrap_or("");

    let mut pos = 0usize;
    let bytes = after_name.as_bytes();
    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        // Find `=`
        let eq = match bytes[pos..].iter().position(|&b| b == b'=') {
            Some(p) => pos + p,
            None => break,
        };
        let key = after_name[pos..eq].trim().to_string();
        pos = eq + 1;

        // Value must be quoted
        let quote = if pos < bytes.len() && bytes[pos] == b'"' {
            b'"'
        } else if pos < bytes.len() && bytes[pos] == b'\'' {
            b'\''
        } else {
            break;
        };
        pos += 1;
        let val_end = match bytes[pos..].iter().position(|&b| b == quote) {
            Some(p) => pos + p,
            None => break,
        };
        let value = &after_name[pos..val_end];
        let value = unescape_xml(value);
        attrs.insert(key, serde_json::Value::String(value));
        pos = val_end + 1;
    }

    attrs
}

/// Unescape basic XML entities.
fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── JSON format (existing) ────────────────────────────

    #[test]
    fn test_json_single() {
        let text = "Let me read.\n<tool_call>\n{\"tool\":\"read_file\",\"parameters\":{\"file_path\":\"/tmp/test.txt\"}}\n</tool_call>\nDone.";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(
            calls[0].parameters["file_path"].as_str().unwrap(),
            "/tmp/test.txt"
        );
        assert!(!_clean.contains("<tool_call>"));
    }

    #[test]
    fn test_json_multiple() {
        let text = "<tool_call>\n{\"tool\":\"read_file\",\"parameters\":{\"file_path\":\"/tmp/a.txt\"}}\n</tool_call>\n<tool_call>\n{\"tool\":\"write_file\",\"parameters\":{\"file_path\":\"/tmp/b.txt\",\"content\":\"hello\"}}\n</tool_call>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[1].name, "write_file");
    }

    #[test]
    fn test_json_none() {
        let text = "Just a regular response.";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 0);
        assert_eq!(_clean, text);
    }

    #[test]
    fn test_json_unclosed() {
        let text = "Start <tool_call> but never close";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 0);
    }

    // ── XML attribute format ──────────────────────────────

    #[test]
    fn test_xml_attr_single() {
        let text = "<tool_call name=\"shell\">\npwd && ls -la\n</tool_call>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(
            calls[0].parameters["input"].as_str().unwrap(),
            "pwd && ls -la"
        );
    }

    #[test]
    fn test_xml_attr_empty_body() {
        let text = "<tool_call name=\"enter_plan_mode\"></tool_call>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "enter_plan_mode");
    }

    #[test]
    fn test_xml_attr_json_body() {
        let text =
            "<tool_call name=\"read_file\">\n{\"file_path\":\"/tmp/test.txt\"}\n</tool_call>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(
            calls[0].parameters["file_path"].as_str().unwrap(),
            "/tmp/test.txt"
        );
    }

    // ── Container <tool_calls> ────────────────────────────

    #[test]
    fn test_container_single() {
        let text = "<tool_calls>\n<tool_call name=\"shell\">\nls\n</tool_call>\n</tool_calls>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn test_container_multiple() {
        let text = "<tool_calls>\n<tool_call name=\"shell\">\nls\n</tool_call>\n<tool_call name=\"glob\">\n**/*.md\n</tool_call>\n</tool_calls>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell");
        assert_eq!(calls[0].parameters["input"].as_str().unwrap(), "ls");
        assert_eq!(calls[1].name, "glob");
        assert_eq!(calls[1].parameters["input"].as_str().unwrap(), "**/*.md");
    }

    #[test]
    fn test_container_nested_double_wrapper() {
        // DeepSeek sometimes doubles the <tool_calls> wrapper
        let text = "<tool_calls>\n<tool_calls>\n<tool_call name=\"shell\">\nls\n</tool_call>\n</tool_calls>\n</tool_calls>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn test_mixed_json_and_xml_attr() {
        let text = "<tool_call>\n{\"tool\":\"enter_plan_mode\",\"parameters\":{}}\n</tool_call>\n\n好的。\n\n<tool_calls>\n<tool_call name=\"shell\">\nls\n</tool_call>\n</tool_calls>";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "enter_plan_mode");
        assert_eq!(calls[1].name, "shell");
    }

    // ── Markdown fallback ─────────────────────────────────

    #[test]
    fn test_markdown_inline_code_plus_empty_json() {
        let text = "Calling: `enter_plan_mode`\n\n```json\n{}\n```";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "enter_plan_mode");
    }

    #[test]
    fn test_markdown_tool_json_in_code_block() {
        let text = "```json\n{\"tool\":\"read_file\",\"parameters\":{\"file_path\":\"/tmp/test.txt\"}}\n```";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
    }

    #[test]
    fn test_markdown_fallback_not_triggered_when_xml_found() {
        let text = "<tool_call>\n{\"tool\":\"read_file\",\"parameters\":{\"file_path\":\"/tmp/a.txt\"}}\n</tool_call>\n\n`shell`\n```json\n{\"tool\":\"shell\",\"parameters\":{\"command\":\"ls\"}}\n```";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
    }

    #[test]
    fn test_markdown_no_tool_in_plain_code_block() {
        let text = "```rust\nfn main() {}\n```";
        let (_clean, calls) = parse_tool_calls(text, &[]);
        assert_eq!(calls.len(), 0);
    }
}

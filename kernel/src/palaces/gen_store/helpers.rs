//! Free helper functions for FTS5 indexing, JSON field extraction, and seed row parsing.

use rusqlite;

use crate::utils::truncate_title;

pub(crate) fn extract_content_text(content_type: &str, content_json: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(content_json) {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    match content_type {
        "KeyValue" => {
            let key = v["key"].as_str().unwrap_or_default();
            let value = v["value"].as_str().unwrap_or_default();
            format!("{key}: {value}")
        }
        "Triple" => {
            let subject = v["subject"].as_str().unwrap_or_default();
            let predicate = v["predicate"].as_str().unwrap_or_default();
            let object = v["object"].as_str().unwrap_or_default();
            format!("{subject} {predicate} {object}")
        }
        _ => v["text"].as_str().unwrap_or_default().to_string(),
    }
}

/// Escape a user query for safe FTS5 matching.
///
/// Wraps the query in double quotes for phrase search. Removes embedded
/// double quotes that would break the query.
pub(crate) fn escape_fts5_query(query: &str) -> String {
    let cleaned = query.replace('"', "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("\"{}\"", trimmed)
}

// ── graph_expand_multi helpers ────────────────────────────────

pub(crate) fn extract_seed_id(seed_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(seed_json)
        .ok()
        .and_then(|v| v["id"].as_str().map(String::from))
}

pub(crate) fn extract_strength(seed_json: &str) -> f32 {
    serde_json::from_str::<serde_json::Value>(seed_json)
        .ok()
        .and_then(|v| v["strength"].as_f64())
        .unwrap_or(0.0) as f32
}

pub(crate) fn extract_neighbor_values(seed_json: &str) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(seed_json) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let content = &v["content"];
    // Only Triple content has subject/object
    if content["type"] != "Triple" {
        return Vec::new();
    }
    let mut values = Vec::new();
    if let Some(s) = content["subject"].as_str()
        && !s.is_empty()
    {
        values.push(s.to_string());
    }
    if let Some(o) = content["object"].as_str()
        && !o.is_empty()
    {
        values.push(o.to_string());
    }
    values
}

pub(crate) fn extract_assertion_key(seed_json: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(seed_json).unwrap_or_default();
    let content = &v["content"];
    let subject = content["subject"].as_str().unwrap_or("");
    let predicate = content["predicate"].as_str().unwrap_or("");
    format!("{subject}|{predicate}")
}

pub(crate) fn extract_triple_object(seed_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(seed_json).ok()?;
    let content = &v["content"];
    content["object"].as_str().map(String::from)
}

pub(crate) fn parse_session_meta(messages_json: &str) -> (Option<String>, usize, bool) {
    let arr: Vec<serde_json::Value> = serde_json::from_str(messages_json).unwrap_or_default();
    let title = arr
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content").and_then(|c| c.as_str()))
        .map(truncate_title);
    let message_count = arr
        .iter()
        .filter(|m| m.get("role").and_then(|r| r.as_str()) != Some("tool_call"))
        .count();
    let has_error = arr.last().is_some_and(|m| {
        if m.get("status").and_then(|s| s.as_str()) == Some("error") {
            return true;
        }
        if m.get("status").and_then(|s| s.as_str()) == Some("running") {
            return true;
        }
        if m.get("error").is_some() {
            return true;
        }
        false
    });
    (title, message_count, has_error)
}

pub(crate) fn seed_row_to_json(row: &rusqlite::Row) -> String {
    let palace_int: i64 = row.get::<_, i64>(6).unwrap_or(0);
    let stem_int: i64 = row.get::<_, i64>(7).unwrap_or(0);
    let content_json_str: String = row.get::<_, String>(5).unwrap_or_default();
    let content: serde_json::Value = serde_json::from_str(&content_json_str).unwrap_or_default();

    let palace_str = match palace_int {
        0 => "Kan",
        1 => "Kun",
        2 => "Zhen",
        3 => "Xun",
        4 => "Zhong",
        5 => "Qian",
        6 => "Dui",
        7 => "Gen",
        8 => "Li",
        _ => "Zhen",
    };
    let stem_str = match stem_int {
        0 => "Jia",
        1 => "Yi",
        2 => "Bing",
        3 => "Ding",
        4 => "Wu",
        5 => "Ji",
        6 => "Geng",
        7 => "Xin",
        8 => "Ren",
        9 => "Gui",
        _ => "Wu",
    };

    serde_json::json!({
        "id": row.get::<_, String>(0).unwrap_or_default(),
        "session_id": row.get::<_, String>(1).unwrap_or_default(),
        "nature": row.get::<_, String>(2).unwrap_or_default(),
        "source": row.get::<_, String>(3).unwrap_or_default(),
        "content": content,
        "palace": palace_str,
        "intent_stem": stem_str,
        "geju_key": row.get::<_, String>(8).unwrap_or_default(),
        "created_at": row.get::<_, i64>(9).unwrap_or(0),
        "access_count": row.get::<_, i64>(10).unwrap_or(0),
        "last_accessed_at": row.get::<_, i64>(11).unwrap_or(0),
        "strength": row.get::<_, f64>(12).unwrap_or(1.0),
        "tier": row.get::<_, String>(13).unwrap_or_else(|_| "OnDemand".into()),
        "project_id": row.get::<_, String>(14).unwrap_or_default(),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_content_text_keyvalue() {
        let result = extract_content_text("KeyValue", r#"{"key":"editor","value":"vim"}"#);
        assert_eq!(result, "editor: vim");
    }

    #[test]
    fn extract_content_text_triple() {
        let result = extract_content_text(
            "Triple",
            r#"{"subject":"A","predicate":"depends_on","object":"B"}"#,
        );
        assert_eq!(result, "A depends_on B");
    }

    #[test]
    fn extract_content_text_freetext() {
        let result = extract_content_text("FreeText", r#"{"text":"hello world"}"#);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn extract_content_text_handles_escaped_quotes() {
        // This would have been truncated by the old hand-written json_field parser
        let result = extract_content_text("FreeText", r#"{"text":"say \"hello\" to the user"}"#);
        assert_eq!(result, "say \"hello\" to the user");
    }

    #[test]
    fn escape_fts5_query_wraps_in_quotes() {
        assert_eq!(escape_fts5_query("rust"), "\"rust\"");
    }

    #[test]
    fn escape_fts5_query_removes_embedded_quotes() {
        assert_eq!(escape_fts5_query(r#""hello" world"#), r#""hello world""#);
    }

    #[test]
    fn escape_fts5_query_empty_after_trim_returns_empty() {
        assert_eq!(escape_fts5_query("   "), "");
    }

    #[test]
    fn extract_seed_id_works() {
        let json = r#"{"id": "seed-123", "strength": 0.5}"#;
        assert_eq!(extract_seed_id(json), Some("seed-123".into()));
    }

    #[test]
    fn extract_assertion_key_combines_subject_predicate() {
        let json = r#"{"content": {"subject": "Cargo.toml", "predicate": "depends_on"}}"#;
        assert_eq!(extract_assertion_key(json), "Cargo.toml|depends_on");
    }

    #[test]
    fn extract_triple_object_works() {
        let json = r#"{"content": {"object": "serde"}}"#;
        assert_eq!(extract_triple_object(json), Some("serde".into()));
    }

    #[test]
    fn parse_session_meta_extracts_title() {
        let messages = serde_json::json!([
            {"role": "user", "content": "Hello world"},
            {"role": "assistant", "content": "Hi"}
        ]);
        let (title, count, has_error) = parse_session_meta(&messages.to_string());
        assert_eq!(title, Some("Hello world".into()));
        assert_eq!(count, 2);
        assert!(!has_error);
    }

    #[test]
    fn parse_session_meta_detects_error() {
        let messages = serde_json::json!([
            {"role": "user", "content": "test"},
            {"role": "assistant", "content": ""},
            {"role": "tool_call", "status": "error"}
        ]);
        let (_, _, has_error) = parse_session_meta(&messages.to_string());
        assert!(has_error);
    }

    #[test]
    fn parse_session_meta_detects_running() {
        let messages = serde_json::json!([
            {"role": "user", "content": "test"},
            {"role": "tool_call", "status": "running"}
        ]);
        let (_, _, has_error) = parse_session_meta(&messages.to_string());
        assert!(has_error);
    }
}

use std::sync::Arc;
/// Helper utilities for skill evolution pipeline — stripping, truncation, eligibility, and cleanup.

use std::sync::RwLock;

use crate::palaces::gen_store::Store;
use crate::palaces::li_skill::{Skill, SkillRegistry};

use super::{EvolutionConfig, EvolutionEngine, SkillReflection};

// ── Stage 1 helpers ───────────────────────────────────────────

impl EvolutionEngine {
    /// Remove stale SKILL.md.tmp.* files left behind by a previous crash.
    /// Only removes files older than 120 seconds to avoid racing with
    /// concurrent sessions' atomic writes.
    pub(super) fn cleanup_stale_temp_files(skills: &Arc<RwLock<SkillRegistry>>) {
        let dirs: Vec<std::path::PathBuf> = {
            let reg = match skills.read() {
                Ok(r) => r,
                Err(e) => e.into_inner(),
            };
            reg.list_all()
                .iter()
                .filter_map(|s| s.source_path.parent().map(|p| p.to_path_buf()))
                .collect()
        };
        let now = crate::utils::unix_now();
        for dir in &dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    if !name.to_string_lossy().starts_with("SKILL.md.tmp.") {
                        continue;
                    }
                    // Only clean files older than 120s to avoid races
                    if let Ok(meta) = entry.metadata()
                        && let Ok(mtime) = meta.modified()
                        && let Ok(dur) = mtime.duration_since(std::time::UNIX_EPOCH)
                        && now - (dur.as_secs() as i64) < 120
                    {
                        continue;
                    }
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    pub(super) fn check_eligibility(
        skill: &Skill,
        config: &EvolutionConfig,
        store: &Store,
        session_id: &str,
        skill_tool_calls: &[String],
    ) -> bool {
        // Must opt in
        if !config.auto_evolve {
            return false;
        }
        // v1: only tool-only skills (always=false, paths=None)
        if skill.always || skill.paths.is_some() {
            return false;
        }
        // Rate limit
        if let Ok(count) = store.count_revisions_this_session(&skill.name, session_id)
            && count >= config.max_revisions_per_session
        {
            return false;
        }
        // Cooldown
        if let Ok(Some(last_ts)) = store.last_revision_time(&skill.name) {
            let now = crate::utils::unix_now();
            if now - last_ts < 3600 {
                return false;
            }
        }
        // Sufficient signal: at least 2 skill() calls
        let call_count = skill_tool_calls
            .iter()
            .filter(|n| *n == &skill.name)
            .count();
        if call_count < 2 {
            return false;
        }
        true
    }

    // ── Stage 4: Persistence ────────────────────────────────

    pub(super) fn persist_reflection(
        reflection: &SkillReflection,
        store: &Store,
    ) -> Result<(), String> {
        let json =
            serde_json::to_string(reflection).map_err(|e| format!("serialize reflection: {e}"))?;
        store
            .save_skill_reflection(&json)
            .map_err(|e| e.to_string())
    }
}

// ── Free helper functions ──────────────────────────────────────

/// Strip ```json or ``` fences from an LLM response.
/// Returns the inner content, or the original string if no fences found.
pub(crate) fn strip_code_fences(text: &str) -> String {
    let t = text.trim();
    // Try ```json or ``` prefix, corresponding suffix
    for prefix in &["```json\n", "```json\r\n", "```\n", "```\r\n"] {
        if let Some(rest) = t.strip_prefix(prefix)
            && let Some(inner) = rest
                .strip_suffix("\n```")
                .or_else(|| rest.strip_suffix("\r\n```"))
                .or_else(|| rest.strip_suffix("```"))
        {
            return inner.trim().to_string();
        }
    }
    // Try single-line ```json{...}``` (no newlines)
    if let Some(rest) = t
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
    {
        return rest.trim().to_string();
    }
    if let Some(rest) = t.strip_prefix("```").and_then(|s| s.strip_suffix("```")) {
        return rest.trim().to_string();
    }
    t.to_string()
}

/// Strip ```markdown or ``` fences surrounding whole-file content.
/// Handles both "\n" and "\r\n" line endings.
pub(crate) fn strip_markdown_fence(content: &str) -> &str {
    let t = content.trim();
    // Strip leading ```markdown or ```
    let t = t
        .strip_prefix("```markdown\n")
        .or_else(|| t.strip_prefix("```markdown\r\n"))
        .or_else(|| t.strip_prefix("```\n"))
        .or_else(|| t.strip_prefix("```\r\n"))
        .map(|s| s.trim())
        .unwrap_or(t);
    // Strip trailing ```

    (t.strip_suffix("\n```")
        .or_else(|| t.strip_suffix("\r\n```"))
        .or_else(|| t.strip_suffix("```"))
        .map(|s| s.trim())
        .unwrap_or(t)) as _
}

/// Truncate text at a character boundary for audit prompts.
pub(crate) fn truncate_for_audit(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        text
    } else {
        let mut end = max_chars;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }
}

/// Safe byte-range truncation that avoids panicking on multi-byte character boundaries.
pub(crate) fn safe_snippet(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        text.to_string()
    } else {
        let mut end = max_bytes;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_string()
    }
}

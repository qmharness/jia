//! Stage 4–5: Revision trigger logic, LLM revision, frontmatter protection, and independent audit.

use std::time::Duration;

use futures::StreamExt;

use crate::palaces::gen_store::Store;
use crate::palaces::li_skill::Skill;
use crate::palaces::zhong_core::JiaCore;
use crate::types::{Message, Role};

use super::helpers::{safe_snippet, strip_markdown_fence, truncate_for_audit};
use super::{
    EvolutionConfig, EvolutionEngine, SkillRevisionDiff, SkillRevisionRecord, compute_diff,
    extract_frontmatter_str, inject_evolution_fields, revision_semantically_equal,
    split_frontmatter_parts,
};

impl EvolutionEngine {
    // ── Stage 4: Persistence & threshold check ────────────

    /// P2 · quick post-revision verification: feed the revised skill prompt to
    /// the LLM and check that it produces a valid response without errors.
    /// Returns true if the skill passes basic validation.
    pub(super) async fn verify_revision(
        core: &JiaCore,
        skill_name: &str,
        skill_prompt: &str,
    ) -> bool {
        let prompt = format!(
            "You have the following skill active:\n\n{skill_prompt}\n\n\
             Respond with 'OK' to confirm you understand this skill."
        );
        let messages = vec![crate::types::Message::text(
            crate::types::Role::User,
            prompt,
        )];
        let inference = async {
            let mut stream = core.infer(messages, None, None);
            let mut text = String::new();
            while let Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) =
                stream.next().await
            {
                text.push_str(&delta);
            }
            text
        };
        match tokio::time::timeout(std::time::Duration::from_secs(30), inference).await {
            Ok(text) => !text.is_empty() && !text.to_lowercase().contains("error"),
            Err(_) => {
                tracing::warn!(
                    "EvolutionEngine: post-revision verification timed out for {skill_name}"
                );
                true // don't block on timeout
            }
        }
    }

    pub(super) fn should_trigger_revision(
        skill_name: &str,
        session_id: &str,
        config: &EvolutionConfig,
        store: &Store,
    ) -> bool {
        let reflections = match store.load_skill_reflections(skill_name, session_id) {
            Ok(r) => r,
            Err(_) => return false,
        };

        // High-confidence single reflection triggers immediately
        if reflections
            .iter()
            .any(|r| r.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0) >= 0.85)
        {
            return true;
        }

        // Count by type
        let mut type_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for r in &reflections {
            if let Some(t) = r.get("reflection_type").and_then(|v| v.as_str()) {
                *type_counts.entry(t.to_string()).or_insert(0) += 1;
            }
        }

        let threshold = config.reflection_threshold as usize;

        // Same-type accumulation
        if type_counts.values().any(|&c| c >= threshold) {
            return true;
        }

        // Cross-type total
        let total: usize = type_counts.values().sum();
        let cross_threshold = std::cmp::max(threshold * 2, 4);
        total >= cross_threshold
    }

    // ── Stage 5: Skill revision ────────────────────────────

    pub(super) async fn revise(
        skill: &Skill,
        session_id: &str,
        config: &EvolutionConfig,
        main_core: &JiaCore,
        aux_core: Option<&JiaCore>,
        store: &Store,
        current_reflection_id: &str,
    ) -> Result<Option<SkillRevisionDiff>, String> {
        // Double-check cooldown (narrow race window)
        if let Ok(Some(last_ts)) = store.last_revision_time(&skill.name) {
            let now = crate::utils::unix_now();
            if now - last_ts < 3600 {
                tracing::info!(
                    "EvolutionEngine: cooldown double-check blocked revision for {}",
                    skill.name
                );
                return Ok(None);
            }
        }

        // Read raw SKILL.md
        let old_content = std::fs::read_to_string(&skill.source_path)
            .map_err(|e| format!("read skill file: {e}"))?;

        // Load reflections for prompt
        let reflections_json = store
            .load_skill_reflections(&skill.name, session_id)
            .map_err(|e| e.to_string())?;

        // Pre-compute aggregates from BATCHED reflections: exclude the one just persisted
        // (identified by ID, not position — positional exclusion is fragile under
        // created_at timestamp ties where ORDER BY produces non-deterministic ordering).
        let batch_reflections: Vec<&serde_json::Value> = reflections_json
            .iter()
            .filter(|r| r.get("id").and_then(|v| v.as_str()) != Some(current_reflection_id))
            .collect();
        let avg_conf: f64 = if batch_reflections.is_empty() {
            0.0
        } else {
            batch_reflections
                .iter()
                .filter_map(|r| r.get("confidence").and_then(|v| v.as_f64()))
                .sum::<f64>()
                / batch_reflections.len() as f64
        };
        let ids: Vec<String> = batch_reflections
            .iter()
            .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect();
        let pre_error_rate: Option<f32> = if !batch_reflections.is_empty() {
            let error_turns: usize = batch_reflections
                .iter()
                .filter_map(|r| {
                    r.get("turn_numbers")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                })
                .sum();
            Some(error_turns as f32 / batch_reflections.len() as f32)
        } else {
            None
        };

        // Build revision prompt
        let mut prompt = String::from("You are revising a skill based on execution feedback.\n\n");
        prompt.push_str("## Current SKILL.md\n```\n");
        prompt.push_str(&old_content);
        prompt.push_str("\n```\n\n## Accumulated Reflections\n```json\n");
        prompt.push_str(&serde_json::to_string_pretty(&reflections_json).unwrap_or_default());
        prompt.push_str("\n```\n\n## Revision Rules\n");
        prompt.push_str("1. Preserve the YAML frontmatter structure (--- ... ---)\n");
        prompt.push_str("2. Keep the skill name and general structure\n");
        prompt.push_str("3. Address ONLY the issues raised in reflections\n");
        prompt.push_str("4. Add/update ## Emphasis section for critical rules\n");
        prompt.push_str("5. Do NOT add rules for scenarios not observed\n");
        prompt.push_str("6. Do NOT modify evolution-related frontmatter fields\n\n");
        prompt.push_str("Output the complete revised SKILL.md (with frontmatter).");

        let messages = vec![Message::text(Role::User, prompt)];
        let inference = async {
            let mut stream = main_core.infer(messages, None, None);
            let mut content = String::new();
            while let Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) =
                stream.next().await
            {
                content.push_str(&delta);
            }
            content
        };
        let new_content = match tokio::time::timeout(Duration::from_secs(60), inference).await {
            Ok(c) => c,
            Err(_) => {
                tracing::warn!("EvolutionEngine: revision LLM timed out after 60s");
                return Ok(None);
            }
        };
        if new_content.is_empty() {
            tracing::warn!("EvolutionEngine: revision LLM returned empty");
            return Ok(None);
        }

        // Frontmatter protection: parse YAML, force evolution fields to old values
        let protected = match Self::protect_frontmatter(&new_content, skill) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    "EvolutionEngine: protect_frontmatter failed for {}: {e}",
                    skill.name
                );
                // Save logged-only record so LLM output is not lost
                let diff = compute_diff(&old_content, &new_content);
                let diff_outcome = SkillRevisionDiff {
                    skill_name: skill.name.clone(),
                    old_snippet: safe_snippet(&old_content, 300),
                    new_snippet: safe_snippet(&new_content, 300),
                    diff: diff.clone(),
                    confidence: avg_conf,
                    applied: false,
                };
                let record = SkillRevisionRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    skill_name: skill.name.clone(),
                    session_id: session_id.to_string(),
                    old_content,
                    new_content,
                    diff_text: diff,
                    avg_confidence: avg_conf,
                    reflection_ids: ids.clone(),
                    pre_revision_error_rate: pre_error_rate,
                    post_revision_error_rate: None,
                    applied: false,
                    created_at: crate::utils::unix_now(),
                };
                let json = serde_json::to_string(&record)
                    .map_err(|e| format!("serialize revision: {e}"))?;
                store
                    .save_skill_revision(&json)
                    .map_err(|e| e.to_string())?;
                if let Some(rate) = pre_error_rate {
                    let _ = store.backfill_post_revision_error_rate(&skill.name, rate);
                }
                return Ok(Some(diff_outcome));
            }
        };

        // YAML validation
        if let Err(e) = serde_yaml::from_str::<super::super::loader::SkillFrontmatter>(
            extract_frontmatter_str(&protected),
        ) {
            tracing::warn!(
                "EvolutionEngine: revised YAML invalid for {}: {e}",
                skill.name
            );
            // Save as logged-only
            let diff = compute_diff(&old_content, &protected);
            let diff_outcome = SkillRevisionDiff {
                skill_name: skill.name.clone(),
                old_snippet: safe_snippet(&old_content, 300),
                new_snippet: safe_snippet(&protected, 300),
                diff: diff.clone(),
                confidence: avg_conf,
                applied: false,
            };
            let record = SkillRevisionRecord {
                id: uuid::Uuid::new_v4().to_string(),
                skill_name: skill.name.clone(),
                session_id: session_id.to_string(),
                old_content,
                new_content: protected,
                diff_text: diff,
                avg_confidence: avg_conf,
                reflection_ids: ids.clone(),
                pre_revision_error_rate: pre_error_rate,
                post_revision_error_rate: None,
                applied: false,
                created_at: crate::utils::unix_now(),
            };
            let json =
                serde_json::to_string(&record).map_err(|e| format!("serialize revision: {e}"))?;
            store
                .save_skill_revision(&json)
                .map_err(|e| e.to_string())?;
            if let Some(rate) = pre_error_rate {
                let _ = store.backfill_post_revision_error_rate(&skill.name, rate);
            }
            return Ok(Some(diff_outcome));
        }

        // Confidence check
        let applied = avg_conf >= config.min_confidence;

        // Optional independent Auditor (uses light model)
        if applied && config.min_confidence >= 0.85 {
            let lc = aux_core.unwrap_or(main_core);
            if !Self::independent_audit(lc, &old_content, &protected).await {
                tracing::info!(
                    "EvolutionEngine: independent auditor rejected revision for {}",
                    skill.name
                );
                let diff = compute_diff(&old_content, &protected);
                let diff_outcome = SkillRevisionDiff {
                    skill_name: skill.name.clone(),
                    old_snippet: safe_snippet(&old_content, 300),
                    new_snippet: safe_snippet(&protected, 300),
                    diff: diff.clone(),
                    confidence: avg_conf,
                    applied: false,
                };
                let record = SkillRevisionRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    skill_name: skill.name.clone(),
                    session_id: session_id.to_string(),
                    old_content,
                    new_content: protected,
                    diff_text: diff,
                    avg_confidence: avg_conf,
                    reflection_ids: ids.clone(),
                    pre_revision_error_rate: pre_error_rate,
                    post_revision_error_rate: None,
                    applied: false,
                    created_at: crate::utils::unix_now(),
                };
                let json = serde_json::to_string(&record)
                    .map_err(|e| format!("serialize revision: {e}"))?;
                store
                    .save_skill_revision(&json)
                    .map_err(|e| e.to_string())?;
                if let Some(rate) = pre_error_rate {
                    let _ = store.backfill_post_revision_error_rate(&skill.name, rate);
                }
                return Ok(Some(diff_outcome));
            }
        }

        // Diff check — skip if semantically unchanged (identity or cosmetic YAML reformat)
        if old_content == protected || revision_semantically_equal(&old_content, &protected) {
            tracing::info!(
                "EvolutionEngine: revision unchanged for {}, skipping",
                skill.name
            );
            // Still backfill so error-rate tracking isn't lost
            if let Some(rate) = pre_error_rate {
                let _ = store.backfill_post_revision_error_rate(&skill.name, rate);
            }
            return Ok(None);
        }

        let diff = compute_diff(&old_content, &protected);

        let actually_applied = if applied {
            // Atomic write: session-scoped temp file + rename.
            // Session prefix prevents cleanup_stale_temp_files from deleting
            // another session's in-flight temp file.
            let session_prefix = &session_id[..session_id.len().min(12)];
            let temp_name = format!("SKILL.md.tmp.{}.{}", session_prefix, uuid::Uuid::new_v4());
            let temp_path = skill
                .source_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join(&temp_name);
            if std::fs::write(&temp_path, &protected).is_err() {
                false
            } else {
                match std::fs::rename(&temp_path, &skill.source_path) {
                    Ok(()) => {
                        tracing::info!("EvolutionEngine: revision applied for {}", skill.name);
                        true
                    }
                    Err(e) => {
                        tracing::warn!("EvolutionEngine: rename failed for {}: {e}", skill.name);
                        let _ = std::fs::remove_file(&temp_path);
                        false
                    }
                }
            }
        } else {
            false
        };

        // Backfill previous revision's post_revision_error_rate
        if let Some(rate) = pre_error_rate {
            let _ = store.backfill_post_revision_error_rate(&skill.name, rate);
        }

        let diff_outcome = SkillRevisionDiff {
            skill_name: skill.name.clone(),
            old_snippet: safe_snippet(&old_content, 300),
            new_snippet: safe_snippet(&protected, 300),
            diff: diff.clone(),
            confidence: avg_conf,
            applied: actually_applied,
        };

        let record = SkillRevisionRecord {
            id: uuid::Uuid::new_v4().to_string(),
            skill_name: skill.name.clone(),
            session_id: session_id.to_string(),
            old_content,
            new_content: protected.clone(),
            diff_text: diff,
            avg_confidence: avg_conf,
            reflection_ids: ids,
            pre_revision_error_rate: pre_error_rate,
            post_revision_error_rate: None,
            applied: actually_applied,
            created_at: crate::utils::unix_now(),
        };
        let json =
            serde_json::to_string(&record).map_err(|e| format!("serialize revision: {e}"))?;
        store
            .save_skill_revision(&json)
            .map_err(|e| e.to_string())?;

        // P2 · Post-revision verification: smoke-test the revised skill.
        if actually_applied {
            let vcore = aux_core.unwrap_or(main_core);
            let ok = Self::verify_revision(vcore, &skill.name, &protected).await;
            tracing::info!(
                skill = %skill.name,
                passed = ok,
                "EvolutionEngine: post-revision verification {}",
                if ok { "passed" } else { "failed" }
            );
        }

        Ok(Some(diff_outcome))
    }

    /// Protect evolution frontmatter fields: force them to old values.
    pub(super) fn protect_frontmatter(new_content: &str, skill: &Skill) -> Result<String, String> {
        // Strip markdown code fences if present
        let cleaned = strip_markdown_fence(new_content);

        if !cleaned.starts_with("---") {
            return Err("revised content missing frontmatter".into());
        }

        let (fm_str, body, le) = split_frontmatter_parts(cleaned)?;

        // Parse and force evolution fields
        let mut fm: serde_yaml::Value =
            serde_yaml::from_str(fm_str).map_err(|e| format!("YAML parse: {e}"))?;

        inject_evolution_fields(&mut fm, skill);

        let new_fm = serde_yaml::to_string(&fm).map_err(|e| format!("YAML serialize: {e}"))?;

        // Preserve original line endings
        Ok(format!("---{le}{new_fm}---{le}{body}", le = le))
    }

    /// Independent auditor: extra haiku call to verify revision quality.
    pub(super) async fn independent_audit(core: &JiaCore, old: &str, new: &str) -> bool {
        let prompt = format!(
            "Audit this skill revision.\n\n## Before\n```\n{}\n```\n\n## After\n```\n{}\n```\n\n\
             Check only these 3 rules:\n\
             1. No contradictory rules introduced\n\
             2. No important rules removed\n\
             3. Frontmatter is complete and valid YAML\n\n\
             Answer ONLY 'pass' or 'fail':",
            truncate_for_audit(old, 2000),
            truncate_for_audit(new, 2000),
        );

        let messages = vec![Message::text(Role::User, prompt)];
        let inference = async {
            let mut stream = core.infer(messages, None, None);
            let mut text = String::new();
            while let Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) =
                stream.next().await
            {
                text.push_str(&delta);
            }
            text
        };
        let text = match tokio::time::timeout(Duration::from_secs(30), inference).await {
            Ok(t) => t,
            Err(_) => {
                tracing::warn!("EvolutionEngine: independent auditor timed out");
                return true; // don't block on timeout (conservative)
            }
        };
        if text.is_empty() {
            return true;
        }
        text.trim().to_lowercase().starts_with("pass")
    }
}

//! Stage 2–3: Trajectory compilation, user correction detection, and LLM reflection.

use std::time::Duration;

use futures::StreamExt;

use crate::palaces::li_skill::Skill;
use crate::palaces::zhong_core::JiaCore;
use crate::types::{Message, Role};
use crate::vijnana::mano::TurnSnapshot;

use super::helpers::strip_code_fences;
use super::{
    EvolutionEngine, GeJuEventRef, SkillReflection, SkillTrajectory, TurnErrorRef, UserCorrection,
};

impl EvolutionEngine {
    // ── Stage 2: Trajectory compilation ──────────────────

    pub(super) fn compile_trajectory(
        skill: &Skill,
        snapshots: &[TurnSnapshot],
        skill_tool_calls: &[String],
        user_messages: &[(u64, String)],
    ) -> SkillTrajectory {
        let mut errors = Vec::new();
        let mut geju_events = Vec::new();

        // Find the first turn where this tool-only skill was invoked via skill().
        // Only include snapshots from that turn onward — errors before the skill
        // was activated cannot be attributed to it.
        let first_invocation_turn = snapshots
            .iter()
            .find(|snap| {
                snap.tool_name == "skill"
                    && snap.tool_input.get("skill").and_then(|v| v.as_str()) == Some(&skill.name)
            })
            .map(|snap| snap.turn_number);

        let first_invoked = skill_tool_calls.contains(&skill.name);

        for snap in snapshots {
            // Skip snapshots before the skill was first invoked
            if let Some(first_turn) = first_invocation_turn {
                if snap.turn_number < first_turn {
                    continue;
                }
            } else if !first_invoked {
                break;
            }

            if let Some(ref err) = snap.tool_error {
                errors.push(TurnErrorRef {
                    turn_number: snap.turn_number,
                    tool_name: snap.tool_name.clone(),
                    error: err.clone(),
                    geju_name: snap.geju_name.clone(),
                    execution_mode: snap.execution_mode.clone(),
                });
            }

            if matches!(
                snap.execution_mode.as_str(),
                "Guarded" | "Sandbox" | "Denied"
            ) {
                geju_events.push(GeJuEventRef {
                    turn_number: snap.turn_number,
                    geju_name: snap.geju_name.clone(),
                    execution_mode: snap.execution_mode.clone(),
                });
            }
        }

        // H1: User correction heuristic — three-rule A+B+C detection.
        let user_corrections = Self::detect_user_corrections(snapshots, user_messages);

        SkillTrajectory {
            errors,
            geju_events,
            user_corrections,
        }
    }

    // ── Stage 3: Skill-aware reflection ──────────────────

    pub(super) async fn reflect(
        skill: &Skill,
        trajectory: &SkillTrajectory,
        session_id: &str,
        core: &JiaCore,
    ) -> Option<SkillReflection> {
        if trajectory.errors.is_empty() && trajectory.geju_events.is_empty() {
            return None;
        }

        let mut prompt =
            String::from("You are analyzing a skill's execution traces to improve it.\n\n");
        prompt.push_str("## Current Skill\n");
        prompt.push_str(&skill.prompt);
        prompt.push_str("\n\n## Current Emphasis\n");
        prompt.push_str(skill.emphasis.as_deref().unwrap_or("None"));
        prompt.push_str("\n\n## Execution Traces\n");

        for err in &trajectory.errors {
            prompt.push_str(&format!(
                "- Turn {}: tool '{}' failed with error: {}. GeJu: {} (mode: {})\n",
                err.turn_number, err.tool_name, err.error, err.geju_name, err.execution_mode
            ));
        }
        for geju in &trajectory.geju_events {
            prompt.push_str(&format!(
                "- Turn {}: GeJu guard triggered: {} (mode: {})\n",
                geju.turn_number, geju.geju_name, geju.execution_mode
            ));
        }

        if !trajectory.user_corrections.is_empty() {
            prompt.push_str("\n## User Corrections\n");
            for corr in &trajectory.user_corrections {
                prompt.push_str(&format!("- Turn {}: {}\n", corr.turn_number, corr.message));
            }
        }

        prompt.push_str("\n## Task\n");
        prompt.push_str("Identify what went wrong and how the skill should be improved.\n");
        prompt.push_str("Output ONLY a valid JSON object (no markdown fences):\n");
        prompt.push_str(r#"{"type":"Discovery|Optimization|SkillDefect|ExecutionLapse","summary":"...","detail":"...","confidence":0.0}"#);

        let messages = vec![Message::text(Role::User, prompt)];
        let inference = async {
            let mut stream = core.infer(messages, None, None);
            let mut response = String::new();
            while let Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) =
                stream.next().await
            {
                response.push_str(&delta);
            }
            response
        };
        let response = match tokio::time::timeout(Duration::from_secs(60), inference).await {
            Ok(r) => r,
            Err(_) => {
                tracing::warn!("EvolutionEngine: reflection LLM timed out after 60s");
                return None;
            }
        };
        if response.is_empty() {
            tracing::warn!("EvolutionEngine: reflection LLM returned empty");
            return None;
        }

        let response = strip_code_fences(&response);

        let parsed: serde_json::Value = match serde_json::from_str(&response) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("EvolutionEngine: failed to parse reflection JSON: {e}");
                return None;
            }
        };

        let rtype = parsed["type"].as_str().unwrap_or("Unknown").to_string();
        if !matches!(
            rtype.as_str(),
            "Discovery" | "Optimization" | "SkillDefect" | "ExecutionLapse"
        ) {
            tracing::warn!("EvolutionEngine: unknown reflection type: {rtype}");
            return None;
        }

        Some(SkillReflection {
            id: uuid::Uuid::new_v4().to_string(),
            skill_name: skill.name.clone(),
            session_id: session_id.to_string(),
            reflection_type: rtype,
            content_json: response,
            confidence: parsed["confidence"].as_f64().unwrap_or(0.5),
            turn_numbers: trajectory.errors.iter().map(|e| e.turn_number).collect(),
            created_at: crate::utils::unix_now(),
        })
    }

    /// Detect user corrections from session history.
    ///
    /// Three-rule heuristic (A+B+C):
    /// - A: Any snapshot in the trajectory has a `tool_error` AND this message isn't a '/'-command
    /// - B: Message contains correction keywords
    /// - C: Message is short (< 50 chars) or in the shortest 30% of session messages
    ///
    /// Trigger: A AND (B OR C) → user correction detected.
    ///
    /// Note: `turn_number` in the returned corrections is the 1-based user-message
    /// position in history (not the agent turn_count), used only for identification.
    pub(super) fn detect_user_corrections(
        snapshots: &[TurnSnapshot],
        user_messages: &[(u64, String)],
    ) -> Vec<UserCorrection> {
        let mut corrections = Vec::new();

        if user_messages.is_empty() {
            return corrections;
        }

        // Rule A (global): any tool error in the trajectory
        let has_any_error = snapshots.iter().any(|s| s.tool_error.is_some());
        if !has_any_error {
            return corrections;
        }

        // Compute shortness threshold: 30th percentile of message lengths, min 50
        let mut lens: Vec<usize> = user_messages.iter().map(|(_, c)| c.len()).collect();
        lens.sort();
        let short_threshold = if lens.len() >= 3 {
            lens[(lens.len() as f64 * 0.3) as usize].max(50)
        } else {
            50
        };

        // Multi-char keywords only — single-char Chinese words like "不" or "错"
        // appear in too many non-correction contexts (不错=good, 不能=cannot, etc.)
        // and would cause massive false positives.
        const CORRECTION_KEYWORDS: &[&str] = &[
            "no",
            "wrong",
            "incorrect",
            "should",
            "instead",
            "don't",
            "not",
            "不要",
            "应该",
            "不对",
            "错了",
            "错误",
            "不行",
        ];

        for (turn, content) in user_messages.iter() {
            // Skip commands
            if content.starts_with('/') {
                continue;
            }

            // Rule B: contains correction keywords
            let content_lower = content.to_lowercase();
            let has_keyword = CORRECTION_KEYWORDS
                .iter()
                .any(|kw| content_lower.contains(kw));

            // Rule C: short message
            let is_short = content.len() < 50 || content.len() <= short_threshold;

            // Trigger: A AND (B OR C)
            if (has_keyword || is_short)
                && !corrections
                    .iter()
                    .any(|c: &UserCorrection| c.turn_number == *turn)
            {
                corrections.push(UserCorrection {
                    turn_number: *turn,
                    message: content.clone(),
                });
            }
        }

        corrections
    }
}

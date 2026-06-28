use std::sync::Arc;
use std::sync::RwLock;

use crate::palaces::gen_store::Store;
use crate::palaces::li_skill::{Skill, SkillRegistry};
use crate::palaces::zhong_core::JiaCore;
use crate::vijnana::mano::WorkingMemory;

// ── Submodules ──────────────────────────────────────────────────

mod diff;
mod frontmatter;
mod helpers;
mod reflect;
mod revise;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;

pub(crate) use diff::*;
pub(crate) use frontmatter::*;

// ── Types ──────────────────────────────────────────────────────

/// Evolution configuration derived from skill frontmatter.
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    pub auto_evolve: bool,
    pub min_confidence: f64,
    pub max_revisions_per_session: u32,
    pub reflection_threshold: u32,
}

impl From<&Skill> for EvolutionConfig {
    fn from(s: &Skill) -> Self {
        Self {
            auto_evolve: s.auto_evolve,
            min_confidence: s.evolve_min_confidence,
            max_revisions_per_session: s.evolve_max_revisions_per_session,
            reflection_threshold: s.evolve_reflection_threshold,
        }
    }
}

/// Aggregate report returned from an evolution run.
#[derive(Debug, Default)]
pub struct EvolutionReport {
    pub skills_analyzed: usize,
    pub reflections: usize,
    pub revisions: usize,
    pub revision_diffs: Vec<SkillRevisionDiff>,
}

/// A/B comparison snapshot for one revision.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillRevisionDiff {
    pub skill_name: String,
    pub old_snippet: String,
    pub new_snippet: String,
    pub diff: String,
    pub confidence: f64,
    pub applied: bool,
}

// ── Trajectory data ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct TurnErrorRef {
    pub(crate) turn_number: u64,
    pub(crate) tool_name: String,
    pub(crate) error: String,
    pub(crate) geju_name: String,
    pub(crate) execution_mode: String,
}

#[derive(Debug, Clone)]
pub(crate) struct GeJuEventRef {
    pub(crate) turn_number: u64,
    pub(crate) geju_name: String,
    pub(crate) execution_mode: String,
}

#[derive(Debug, Clone)]
pub(crate) struct UserCorrection {
    pub(crate) turn_number: u64,
    pub(crate) message: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillTrajectory {
    pub(crate) errors: Vec<TurnErrorRef>,
    pub(crate) geju_events: Vec<GeJuEventRef>,
    pub(crate) user_corrections: Vec<UserCorrection>,
}

// ── Reflection ──────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SkillReflection {
    pub(crate) id: String,
    pub(crate) skill_name: String,
    pub(crate) session_id: String,
    pub(crate) reflection_type: String, // Discovery|Optimization|SkillDefect|ExecutionLapse
    pub(crate) content_json: String,
    pub(crate) confidence: f64,
    pub(crate) turn_numbers: Vec<u64>,
    pub(crate) created_at: i64,
}

// ── Revision ────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SkillRevisionRecord {
    pub(crate) id: String,
    pub(crate) skill_name: String,
    pub(crate) session_id: String,
    pub(crate) old_content: String,
    pub(crate) new_content: String,
    pub(crate) diff_text: String,
    pub(crate) avg_confidence: f64,
    pub(crate) reflection_ids: Vec<String>,
    /// Average error-turns per reflection before this revision.
    /// Computed as sum(turn_numbers.len) / reflection count.
    pub(crate) pre_revision_error_rate: Option<f32>,
    /// Error rate measured after the *next* revision is applied.
    /// Backfilled by the subsequent revision cycle.
    pub(crate) post_revision_error_rate: Option<f32>,
    pub(crate) applied: bool,
    pub(crate) created_at: i64,
}

// ── Engine ──────────────────────────────────────────────────

pub struct EvolutionEngine;

impl EvolutionEngine {
    /// Main entry point — called from post_loop.
    ///
    /// `main_core` is used for Stage 5 revision (high-quality).
    /// `aux_core` is used for Stage 3 reflection + auditor (low-cost).
    /// Falls back to `main_core` if not provided.
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        skills: &Arc<RwLock<SkillRegistry>>,
        working_memory: &WorkingMemory,
        skill_tool_calls: &[String],
        user_messages: &[(u64, String)],
        store: &Arc<Store>,
        session_id: &str,
        main_core: &JiaCore,
        aux_core: Option<&JiaCore>,
    ) -> EvolutionReport {
        let mut report = EvolutionReport::default();

        // Snapshot list for trajectory compilation
        let snapshots = working_memory.snapshots.clone();

        // Cleanup stale temp files from previous crashes
        Self::cleanup_stale_temp_files(skills);

        // Collect eligible skills while holding the lock, then drop before awaits
        let eligible_skills: Vec<Arc<Skill>> = {
            let reg = match skills.read() {
                Ok(r) => r,
                Err(e) => e.into_inner(),
            };
            reg.list_all()
                .into_iter()
                .filter(|skill| {
                    let config = EvolutionConfig::from(skill.as_ref());
                    Self::check_eligibility(skill, &config, store, session_id, skill_tool_calls)
                })
                .collect()
        };
        // RwLockReadGuard dropped here — safe to await below

        for skill in &eligible_skills {
            let config = EvolutionConfig::from(skill.as_ref());
            report.skills_analyzed += 1;

            // Stage 2: Compile trajectory
            let trajectory =
                Self::compile_trajectory(skill, &snapshots, skill_tool_calls, user_messages);

            // Stage 3: Reflect (LLM call — uses light model if provided)
            let lc = aux_core.unwrap_or(main_core);
            let reflection = match Self::reflect(skill, &trajectory, session_id, lc).await {
                Some(r) => r,
                None => continue,
            };
            report.reflections += 1;

            // Stage 4: Accumulate
            if let Err(e) = Self::persist_reflection(&reflection, store) {
                tracing::warn!("EvolutionEngine: failed to persist reflection: {e}");
                continue;
            }

            if !Self::should_trigger_revision(&reflection.skill_name, session_id, &config, store) {
                continue;
            }

            // Stage 5: Revise (LLM call)
            match Self::revise(
                skill,
                session_id,
                &config,
                main_core,
                aux_core,
                store,
                &reflection.id,
            )
            .await
            {
                Ok(Some(diff)) => {
                    report.revisions += 1;
                    report.revision_diffs.push(diff);
                }
                Ok(None) => { /* skipped by safety gate */ }
                Err(e) => {
                    tracing::warn!("EvolutionEngine: revision failed for {}: {e}", skill.name);
                }
            }
        }

        report
    }
}

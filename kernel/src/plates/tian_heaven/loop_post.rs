/// Post-loop lifecycle: consolidation, distillation, zuowang, and skill evolution.
use crate::palaces::li_skill::evolution::EvolutionEngine;
use crate::palaces::zhong_core::JiaCore;
use crate::principles::SystemPrinciple;
use crate::telemetry::metrics::{JIA_ATMA_GRAHA, JIA_SEEDS_TOTAL};
use crate::vijnana::alaya::SeedStore;
use crate::vijnana::vasana::ConsolidationEngine;
use crate::vijnana::vasana::distillation::DistillationEngine;
use crate::zuowang::pipeline::VasanaScheduler;
use std::sync::Arc;

impl super::Agent {
    pub async fn post_loop(
        &mut self,
        store: Arc<crate::palaces::gen_store::Store>,
        main_core: &JiaCore,
        aux_core: Option<&JiaCore>,
    ) {
        // Final flush: touch any remaining seed IDs from the last round
        let ids: Vec<String> = self.touched_seed_ids.drain(..).collect();
        if !ids.is_empty() {
            let seed_store = SeedStore::new(store.clone());
            seed_store.touch_batch(&ids);
        }

        // Persist session history immediately so refreshes don't lose content.
        if let Ok(json) = serde_json::to_string(&self.history) {
            let store_async =
                crate::palaces::gen_store::async_store::StoreAsync::new(store.clone());
            if let Err(e) = store_async.save_session(&self.id, &json).await {
                tracing::warn!(session = %self.id, error = %e, "Failed to save session");
            }
        }

        // L4 · 自进化 — derive system principles from this session's error patterns
        // before snapshots are consumed by consolidation below.
        let snapshots = self.working_memory.snapshots.clone();
        if !snapshots.is_empty() {
            let new_principles = SystemPrinciple::derive(&self.id, &snapshots, &self.manas);
            if !new_principles.is_empty()
                && let Ok(json) = serde_json::to_string(&new_principles)
            {
                if let Err(e) = store.save_principles(&self.id, &json) {
                    tracing::warn!(error = %e, "Layer4: persist failed");
                } else {
                    tracing::info!(count = new_principles.len(), "Layer4: derived");
                }
            }
        }

        // L2 batch consolidation: extract cross-turn causal/entity facts from snapshots.
        // Requires ≥3 snapshots for meaningful pattern extraction.
        if snapshots.len() >= 3 {
            match ConsolidationEngine::run(
                self.id.clone(),
                snapshots,
                store.clone(),
                aux_core.unwrap_or(main_core),
            )
            .await
            {
                Ok(n) if n > 0 => {
                    tracing::info!(session = %self.id, seeds = n, "L2 consolidation created seeds");
                    self.manas.on_consolidation(n);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(session = %self.id, error = %e, "L2 consolidation failed");
                }
            }
        }

        // Thought distillation — extract reusable knowledge from completed exchanges.
        // Seeds produced here are surfaced to the LLM via top_influence_prompt()
        // in build_tool_prompt() when atma_graha is below threshold.
        // Uses content-hash set to skip pairs already processed in prior sessions.
        {
            let (new_hashes, seeds_created) = DistillationEngine::run(
                &self.id,
                &self.history,
                &self.distilled_hashes,
                &store,
                aux_core.unwrap_or(main_core),
            )
            .await;
            if seeds_created > 0 {
                tracing::info!(session = %self.id, seeds = seeds_created, "Distillation created seeds");
            }
            self.distilled_hashes.extend(new_hashes);
            // Persist immediately so future sessions skip these pairs.
            if let Err(e) = store.save_distilled_hashes(&self.id, &self.distilled_hashes) {
                tracing::warn!(session = %self.id, error = %e, "Failed to save distilled hashes");
            }
        }

        // ── VasanaScheduler (熏习调度) ──
        // Orchestrates: zuowang dissolution → tier budgets → dormancy detection.
        // Replaces the separate dissolve + enforce_tier_budgets calls.
        match VasanaScheduler::schedule(store.clone(), Some(&self.coactivation)) {
            Ok(report) => {
                // Zuowang dissolution results
                if let Some(ref zw) = report.zuowang {
                    if zw.seeds_examined > 0 {
                        tracing::info!(
                            session = %self.id,
                            examined = zw.seeds_examined,
                            dissolved = zw.seeds_dissolved,
                            weakened = zw.seeds_weakened,
                            downgraded = zw.seeds_downgraded,
                            entropy_before = %zw.entropy_before,
                            entropy_after = %zw.entropy_after,
                            dormant_detected = report.dormant_count,
                            "VasanaScheduler: zuowang dissolution complete"
                        );
                        let remaining = report.total_seeds_after;
                        self.manas.recalibrate(&zw.entropy_dimensions, remaining);
                        JIA_ATMA_GRAHA.set(self.manas.atma_graha as f64);
                        let _ = store.insert_manas_snapshot(
                            &self.id,
                            self.manas.atma_graha,
                            zw.entropy_dimensions.total,
                            remaining,
                        );
                    }
                }
                // Tier budget enforcement results
                if let Some(ref budget) = report.budget {
                    if budget.ondemand_demoted > 0 || budget.archive_deleted > 0 {
                        tracing::info!(
                            session = %self.id,
                            ondemand_total = budget.ondemand_total,
                            ondemand_demoted = budget.ondemand_demoted,
                            archive_total = budget.archive_total,
                            archive_deleted = budget.archive_deleted,
                            "VasanaScheduler: tier budget enforced"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(session = %self.id, error = %e, "VasanaScheduler failed");
            }
        }

        // Update seeds gauge after post-loop mutations
        if let Ok(count) = store.count_seeds() {
            JIA_SEEDS_TOTAL.set(count as f64);
        }

        // Prune old file backups (keep last N directories by timestamp).
        if let Ok(dirs) = std::fs::read_dir(&self.earth.backup_dir) {
            let mut entries: Vec<_> = dirs
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .collect();
            if entries.len() > 30 {
                entries.sort_by_key(|e| e.file_name().to_string_lossy().into_owned());
                for entry in entries.iter().take(entries.len() - 30) {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }

        // Persist Manas self-model after all lifecycle updates (record_turn,
        // on_consolidation, recalibrate, tier budget) have been applied.
        if let Ok(json) = serde_json::to_string(&self.manas)
            && let Err(e) = store.save_manas(&json)
        {
            tracing::warn!(error = %e, "Failed to save manas");
        }

        // Report skill usage at session end (Phase 2)
        self.report_skill_usage();

        // Extract user messages for evolution pipeline (H1 heuristic)
        let user_messages: Vec<(u64, String)> = self
            .history
            .iter()
            .filter_map(|e| match e {
                crate::types::HistoryEntry::User { content, .. } => Some(content.clone()),
                _ => None,
            })
            .enumerate()
            .map(|(i, c)| (i as u64 + 1, c))
            .collect();

        // Skill evolution pipeline (Phase 0)
        let evo_report = EvolutionEngine::run(
            &self.earth.skills,
            &self.working_memory,
            &self.skill_tool_calls,
            &user_messages,
            &store,
            &self.id,
            main_core,
            aux_core,
        )
        .await;
        if evo_report.skills_analyzed > 0 || evo_report.reflections > 0 || evo_report.revisions > 0
        {
            tracing::info!(
                session = %self.id,
                skills_analyzed = evo_report.skills_analyzed,
                reflections = evo_report.reflections,
                revisions = evo_report.revisions,
                "Skill evolution pipeline complete"
            );
        }
    }
}

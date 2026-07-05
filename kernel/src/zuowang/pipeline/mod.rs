use crate::error::JiaError;
use std::sync::Arc;
use std::sync::Mutex;

use crate::palaces::gen_store::Store;
use crate::vijnana::alaya::{Seed, SeedNature, SeedSource, SeedTier};
use crate::zuowang::trigger::AlayaEntropy;

/// 坐忘管道 — The forgetting pipeline.
///
/// Implements a four-layer dissolution transaction:
///   1. SNAPSHOT — load seeds, compute entropy
///   2. COMPUTE  — score each seed, identify candidates for dissolution
///   3. APPLY    — weaken or delete candidate seeds
///   4. VERIFY   — ensure user-stated facts are preserved
///
/// Uses the Store's per-agent dissolve_lock to prevent overlapping dissolves
/// on the same agent file while allowing concurrent dissolves on different agents.
pub struct ZuowangPipeline;

/// Ring buffer of recent dissolution events for the dashboard.
static DISSOLUTION_HISTORY: Mutex<Option<Vec<ZuowangReport>>> = Mutex::new(None);
const HISTORY_MAX: usize = 20;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeedDigest {
    pub nature: String,
    pub source: String,
    pub primary_dim: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ZuowangReport {
    pub seeds_examined: usize,
    pub seeds_dissolved: usize,
    pub seeds_weakened: usize,
    pub seeds_downgraded: usize,
    pub entropy_before: f32,
    pub entropy_after: f32,
    /// Dimensional entropy breakdown after dissolution (for manas recalibration).
    pub entropy_dimensions: AlayaEntropy,
    pub timestamp: i64,
    pub score_kept: usize,
    pub score_protected: usize,
    pub dissolved_sample: Vec<SeedDigest>,
}

impl ZuowangPipeline {
    /// Read the dissolution history for the dashboard.
    /// Falls back to DB when the in-memory buffer is empty (e.g. after restart).
    pub fn history(store: Arc<Store>) -> Vec<ZuowangReport> {
        // Fast path: in-memory buffer already populated
        if let Ok(guard) = DISSOLUTION_HISTORY.lock() {
            if let Some(ref entries) = *guard
                && !entries.is_empty()
            {
                return entries.clone();
            }
            // Slow path: load from DB and backfill the ring buffer
            drop(guard);
            if let Ok(reports) = store.load_dissolution_history(HISTORY_MAX)
                && !reports.is_empty()
            {
                if let Ok(mut hist) = DISSOLUTION_HISTORY.lock() {
                    *hist = Some(reports.clone());
                }
                return reports;
            }
        }
        Vec::new()
    }

    /// Run the four-layer dissolution pipeline.
    ///
    /// Acquires an exclusive lock to prevent concurrent dissolve runs.
    /// Returns early if entropy does not exceed the threshold.
    pub fn dissolve(store: Arc<Store>, threshold: f32) -> Result<ZuowangReport, JiaError> {
        // Prevent concurrent dissolve runs on this agent's store
        let Ok(_guard) = store.dissolve_lock.try_lock() else {
            tracing::debug!("Zuowang: another dissolve already in progress, skipping");
            return Ok(ZuowangReport {
                seeds_examined: 0,
                seeds_dissolved: 0,
                seeds_weakened: 0,
                seeds_downgraded: 0,
                entropy_before: 0.0,
                entropy_after: 0.0,
                entropy_dimensions: AlayaEntropy {
                    staleness: 0.0,
                    contradiction: 0.0,
                    redundancy: 0.0,
                    access_decay: 0.0,
                    total: 0.0,
                },
                timestamp: crate::utils::unix_now(),
                score_kept: 0,
                score_protected: 0,
                dissolved_sample: vec![],
            });
        };

        // ── Layer 1: SNAPSHOT (agent-wide) ────────────────
        let seed_jsons = store.load_all_seeds()?;

        // Adaptive threshold: more seeds → naturally higher entropy → lower bar.
        // Only lowers the threshold (never raises), with a floor of 0.05.
        // New agents (few seeds) dissolve at caller's threshold; mature agents
        // with many seeds get a progressively lower threshold.
        let threshold = (threshold - 0.03 * (seed_jsons.len() as f32).ln()).max(0.05);

        if seed_jsons.is_empty() {
            return Ok(ZuowangReport {
                seeds_examined: 0,
                seeds_dissolved: 0,
                seeds_weakened: 0,
                seeds_downgraded: 0,
                entropy_before: 0.0,
                entropy_after: 0.0,
                entropy_dimensions: AlayaEntropy {
                    staleness: 0.0,
                    contradiction: 0.0,
                    redundancy: 0.0,
                    access_decay: 0.0,
                    total: 0.0,
                },
                timestamp: crate::utils::unix_now(),
                score_kept: 0,
                score_protected: 0,
                dissolved_sample: vec![],
            });
        }

        let seeds: Vec<Seed> = seed_jsons
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        let now = crate::utils::unix_now();

        let entropy = AlayaEntropy::compute(&seeds, now);
        if !entropy.exceeds_threshold(threshold) {
            tracing::debug!(
                "Zuowang: entropy {:.3} below threshold {:.3}, skipping",
                entropy.total,
                threshold
            );
            return Ok(ZuowangReport {
                seeds_examined: seeds.len(),
                seeds_dissolved: 0,
                seeds_weakened: 0,
                seeds_downgraded: 0,
                entropy_before: entropy.total,
                entropy_after: entropy.total,
                entropy_dimensions: entropy.clone(),
                timestamp: crate::utils::unix_now(),
                score_kept: seeds.len(),
                score_protected: 0,
                dissolved_sample: vec![],
            });
        }

        tracing::info!(
            "Zuowang: entropy {:.3} exceeds threshold {:.3}, dissolving",
            entropy.total,
            threshold
        );

        // ── Layer 2: COMPUTE ───────────────────────────────
        // Score each seed: higher = more worth keeping.
        // Apply nature_weight to make Fact more resistant and Inference more dissolvable.
        let scored: Vec<(&Seed, f32)> = seeds
            .iter()
            .map(|s| {
                let raw = s.relevance_score(now);
                let weight = match s.nature {
                    crate::vijnana::alaya::SeedNature::Fact => 1.25,
                    crate::vijnana::alaya::SeedNature::Inference => 0.80,
                    _ => 1.0,
                };
                (s, (raw * weight).min(1.0))
            })
            .collect();

        // Count original protected seeds for VERIFY layer
        let original_protected = seeds
            .iter()
            .filter(|s| {
                matches!(
                    s.source,
                    SeedSource::UserStatement | SeedSource::RenSoul | SeedSource::Handoff
                ) || matches!(s.nature, SeedNature::Preference)
            })
            .count();

        // ── Layer 3: APPLY ─────────────────────────────────
        let mut dissolved = 0usize;
        let mut weakened = 0usize;
        let mut downgraded = 0usize;

        // Helper: check if seed is protected (never deleted/downgraded/weakened)
        fn is_prot(seed: &&Seed) -> bool {
            matches!(
                seed.source,
                SeedSource::UserStatement | SeedSource::RenSoul | SeedSource::Handoff
            ) || matches!(seed.nature, SeedNature::Preference)
        }

        // Archive seeds with score < 0.1 → delete
        // Always seeds exempt from dissolution
        let to_delete: Vec<String> = scored
            .iter()
            .filter(|(seed, score)| {
                *score < 0.1 && matches!(seed.tier, SeedTier::Archive) && !is_prot(seed)
            })
            .map(|(s, _)| s.id.clone())
            .collect();

        if !to_delete.is_empty() {
            store.delete_seeds(&to_delete)?;
            dissolved = to_delete.len();
        }

        // OnDemand seeds with score < 0.1 → downgrade to Archive (not delete)
        let to_downgrade: Vec<String> = scored
            .iter()
            .filter(|(seed, score)| {
                *score < 0.1 && matches!(seed.tier, SeedTier::OnDemand) && !is_prot(seed)
            })
            .map(|(s, _)| s.id.clone())
            .collect();

        if !to_downgrade.is_empty() {
            store.set_tier_batch(&to_downgrade, "Archive")?;
            downgraded = to_downgrade.len();
        }

        // OnDemand seeds with score ∈ [0.1, 0.2) → weaken
        // Always exempt, Archive seeds don't get weakened (already cold)
        let to_weaken: Vec<String> = scored
            .iter()
            .filter(|(seed, score)| {
                *score >= 0.1
                    && *score < 0.2
                    && matches!(seed.tier, SeedTier::OnDemand)
                    && !is_prot(seed)
            })
            .map(|(s, _)| s.id.clone())
            .collect();

        if !to_weaken.is_empty() {
            store.weaken_seeds(&to_weaken, 0.5)?;
            weakened = to_weaken.len();
        }

        // Defensive: Always seeds idle > 30 days → downgrade to OnDemand.
        // Won't trigger in practice since memory_catalog() touches Always seeds
        // every turn, but guards against future catalog behavior changes.
        // UserStatement / Preference seeds are never downgraded.
        let always_to_downgrade: Vec<String> = scored
            .iter()
            .filter(|(seed, _)| matches!(seed.tier, SeedTier::Always))
            .filter(|(seed, _)| now - seed.last_accessed_at > 30 * 24 * 3600)
            .filter(|(seed, _)| !is_prot(seed))
            .map(|(s, _)| s.id.clone())
            .collect();

        if !always_to_downgrade.is_empty() {
            store.set_tier_batch(&always_to_downgrade, "OnDemand")?;
        }

        // ── Score distribution for dashboard ──
        let score_protected = scored
            .iter()
            .filter(|(seed, score)| {
                let actionable = match seed.tier {
                    SeedTier::Always => false,
                    SeedTier::Archive => *score < 0.1,
                    SeedTier::OnDemand => *score < 0.2,
                };
                actionable
                    && (matches!(
                        seed.source,
                        SeedSource::UserStatement | SeedSource::RenSoul | SeedSource::Handoff
                    ) || matches!(seed.nature, SeedNature::Preference))
            })
            .count();
        let score_kept =
            seeds.len() - dissolved - weakened - downgraded - always_to_downgrade.len();

        // Collect up to 5 dissolved seed digests, sorted by lowest score first
        let mut dissolved_sample: Vec<SeedDigest> = scored
            .iter()
            .filter(|(seed, score)| {
                *score < 0.1
                    && !matches!(
                        seed.source,
                        SeedSource::UserStatement | SeedSource::RenSoul | SeedSource::Handoff
                    )
                    && !matches!(seed.nature, SeedNature::Preference)
            })
            .map(|(seed, _score)| {
                let now = crate::utils::unix_now();
                let age_hours = (now - seed.last_accessed_at.max(seed.created_at)) as f32 / 3600.0;
                let primary_dim = if age_hours > 720.0 {
                    "staleness"
                } else if seed.access_count == 0 {
                    "access_decay"
                } else {
                    "staleness"
                };
                SeedDigest {
                    nature: format!("{:?}", seed.nature),
                    source: format!("{:?}", seed.source),
                    primary_dim: primary_dim.into(),
                }
            })
            .collect();
        dissolved_sample.sort_by(|a, b| {
            // Sort by nature+source for stable ordering (scores no longer available)
            a.nature.cmp(&b.nature).then(a.source.cmp(&b.source))
        });
        dissolved_sample.truncate(5);

        // ── Layer 4: VERIFY ────────────────────────────────
        // Re-query the DB to verify mutations were actually applied.
        let remaining_jsons = store.load_all_seeds()?;
        let remaining: Vec<Seed> = remaining_jsons
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        let protected_remaining = remaining
            .iter()
            .filter(|s| {
                matches!(
                    s.source,
                    SeedSource::UserStatement | SeedSource::RenSoul | SeedSource::Handoff
                ) || matches!(s.nature, SeedNature::Preference)
            })
            .count();

        if protected_remaining < original_protected {
            tracing::warn!(
                "Zuowang VERIFY: {}/{} protected seeds were lost! This should not happen.",
                original_protected - protected_remaining,
                original_protected
            );
        }

        // Verify deleted seeds are actually gone
        let deleted_ids_still_present = remaining
            .iter()
            .filter(|s| to_delete.contains(&s.id))
            .count();
        if deleted_ids_still_present > 0 {
            tracing::warn!(
                "Zuowang VERIFY: {} seeds were marked for deletion but still exist",
                deleted_ids_still_present,
            );
        }

        // Post-dissolution entropy check: did dissolution reduce entropy enough?
        let post_entropy = AlayaEntropy::compute(&remaining, now);
        tracing::info!(
            "Zuowang post-dissolution entropy: {:.3} (was {:.3}, delta={:+.3})",
            post_entropy.total,
            entropy.total,
            post_entropy.total - entropy.total,
        );
        if post_entropy.exceeds_threshold(threshold) {
            tracing::warn!(
                "Zuowang: entropy {:.3} still above threshold {:.3} after dissolving {}/{} seeds",
                post_entropy.total,
                threshold,
                dissolved,
                seeds.len(),
            );
        }

        let report = ZuowangReport {
            seeds_examined: seeds.len(),
            seeds_dissolved: dissolved,
            seeds_weakened: weakened,
            seeds_downgraded: downgraded + always_to_downgrade.len(),
            entropy_before: entropy.total,
            entropy_after: post_entropy.total,
            entropy_dimensions: post_entropy,
            timestamp: crate::utils::unix_now(),
            score_kept,
            score_protected,
            dissolved_sample,
        };

        // Record in ring buffer for dashboard
        if let Ok(mut hist) = DISSOLUTION_HISTORY.lock() {
            let mut entries = hist.get_or_insert_with(Vec::new).clone();
            entries.push(report.clone());
            if entries.len() > HISTORY_MAX {
                entries.remove(0);
            }
            *hist = Some(entries);
        }

        // Persist to DB so history survives restarts
        if let Err(e) = store.save_dissolution_report(&report) {
            tracing::warn!(error = %e, "Failed to persist dissolution report");
        }

        Ok(report)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

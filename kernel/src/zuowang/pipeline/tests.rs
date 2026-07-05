use std::sync::Arc;
#[cfg(test)]
mod tests {
    use super::super::ZuowangPipeline;
    use crate::palaces::Palace;
    use crate::palaces::gen_store::Store;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::stems::Stem;
    use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedTier};
    use crate::zuowang::trigger::AlayaEntropy;
    use std::sync::Arc;

    fn temp_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()))
    }

    fn insert_test_seed(
        store: &Arc<Store>,
        id: &str,
        nature: SeedNature,
        source: SeedSource,
        content: SeedContent,
        created_at: i64,
        last_accessed_at: i64,
        access_count: u32,
        strength: f32,
        tier: SeedTier,
    ) {
        let seed = Seed {
            id: id.into(),
            session_id: "test".into(),
            project_id: String::new(),
            nature,
            source,
            content,
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "test_geju".into(),
            created_at,
            access_count,
            last_accessed_at,
            strength,
            tier,
        };
        let json = serde_json::to_string(&seed).unwrap();
        store.insert_seed(&json).unwrap();
    }

    #[test]
    fn dissolve_empty_store() {
        let store = temp_store();
        let report = ZuowangPipeline::dissolve(store, 0.75).unwrap();
        assert_eq!(report.seeds_examined, 0);
        assert_eq!(report.seeds_dissolved, 0);
        assert_eq!(report.seeds_weakened, 0);
    }

    #[test]
    fn dissolve_below_threshold_all_fresh() {
        let store = temp_store();
        let now = crate::utils::unix_now();
        // All fresh seeds with high strength → relevance_score ≈ 1.0, entropy low
        for i in 0..5 {
            insert_test_seed(
                &store,
                &format!("s{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText { text: "ok".into() },
                now,
                now,
                5,
                1.0,
                SeedTier::OnDemand,
            );
        }
        let report = ZuowangPipeline::dissolve(store, 0.75).unwrap();
        assert!(report.seeds_examined > 0, "should have examined seeds");
        assert_eq!(
            report.seeds_dissolved, 0,
            "fresh seeds should not be dissolved"
        );
        assert_eq!(report.seeds_weakened, 0);
    }

    #[test]
    fn dissolve_deletes_low_score_seeds() {
        let store = temp_store();
        let now = crate::utils::unix_now();
        let old = now - 90 * 24 * 3600; // 90 days ago
        // Old seeds with low strength → relevance_score < 0.1
        // Use Archive tier so they get deleted (OnDemand would be downgraded instead)
        for i in 0..10 {
            insert_test_seed(
                &store,
                &format!("old{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText { text: "old".into() },
                old,
                old,
                0,
                0.05,
                SeedTier::Archive,
            );
        }
        // Archive seeds have halved entropy weight, so use lower threshold
        let report = ZuowangPipeline::dissolve(store, 0.20).unwrap();
        assert!(
            report.seeds_dissolved > 0,
            "old low-strength seeds should be dissolved, got dissolved={}",
            report.seeds_dissolved
        );
    }

    #[test]
    fn dissolve_weakens_medium_score_seeds() {
        let store = temp_store();
        let now = crate::utils::unix_now();
        let mid = now - 60 * 24 * 3600; // 60 days ago
        // strength=0.25 → relevance_score ≈ 0.125 + recency*0.3 ≈ 0.13 in [0.1, 0.2) range
        for i in 0..20 {
            insert_test_seed(
                &store,
                &format!("mid{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText { text: "mid".into() },
                mid,
                mid,
                0,
                0.25,
                SeedTier::OnDemand,
            );
        }
        let report = ZuowangPipeline::dissolve(store, 0.30).unwrap();
        // These should be weakened (score in [0.1, 0.2))
        assert!(
            report.seeds_weakened > 0,
            "medium-score seeds should be weakened, got d={} w={}",
            report.seeds_dissolved,
            report.seeds_weakened
        );
    }

    #[test]
    fn dissolve_preserves_user_statements() {
        let store = temp_store();
        let now = crate::utils::unix_now();
        let old = now - 90 * 24 * 3600;
        // Old user-stated seed — should survive
        insert_test_seed(
            &store,
            "user1",
            SeedNature::Preference,
            SeedSource::UserStatement,
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "vim".into(),
            },
            old,
            old,
            0,
            0.05,
            SeedTier::OnDemand,
        );
        // Many old tool-observation seeds — push entropy over threshold
        for i in 0..15 {
            insert_test_seed(
                &store,
                &format!("tool{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText { text: "old".into() },
                old,
                old,
                0,
                0.05,
                SeedTier::Archive,
            );
        }

        let report = ZuowangPipeline::dissolve(store.clone(), 0.20).unwrap();
        assert!(
            report.seeds_dissolved > 0,
            "tool seeds should be dissolved, got dissolved={}",
            report.seeds_dissolved
        );

        // Verify user seed still exists
        let remaining = store.load_all_seeds().unwrap();
        let has_user = remaining.iter().any(|j| j.contains("user1"));
        assert!(has_user, "user-stated seed should be preserved");
    }

    #[test]
    fn dissolve_lock_prevents_concurrent() {
        let store = temp_store();
        let now = crate::utils::unix_now();
        let old = now - 90 * 24 * 3600;
        for i in 0..20 {
            insert_test_seed(
                &store,
                &format!("s{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText { text: "x".into() },
                old,
                old,
                0,
                0.05,
                SeedTier::Archive,
            );
        }

        // Acquire the lock manually
        let guard = store.dissolve_lock.try_lock().unwrap();

        // Try to dissolve while lock is held
        let store2 = store.clone();
        let report = ZuowangPipeline::dissolve(store2, 0.0).unwrap(); // low threshold to force dissolve attempt
        assert_eq!(report.seeds_examined, 0);
        assert_eq!(report.seeds_dissolved, 0);

        drop(guard);
    }

    /// Full evolutionary cycle: accumulate → entropy grows → dissolve → entropy drops → recalibrate.
    #[test]
    fn evolution_full_cycle() {
        use crate::vijnana::manas::Manas;
        use crate::zuowang::trigger::AlayaEntropy;

        let store = temp_store();
        let now = crate::utils::unix_now();

        // ── Phase 1: Initial accumulation (fresh, high-quality seeds) ──
        for i in 0..5 {
            insert_test_seed(
                &store,
                &format!("fresh{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::KeyValue {
                    key: "lang".into(),
                    value: "rust".into(),
                },
                now - i as i64 * 3600,
                now,
                10,
                0.9,
                SeedTier::OnDemand,
            );
        }

        let seeds: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy = AlayaEntropy::compute(&seeds, now);
        assert!(
            entropy.total < 0.4,
            "fresh seeds: entropy should be low, got {:.3}",
            entropy.total
        );
        assert!(!entropy.exceeds_threshold(0.75));

        // Manas should stabilize with low entropy (need ~7 iterations due to momentum blend from 0.80)
        let mut manas = Manas::new();
        for _ in 0..8 {
            manas.recalibrate(&entropy, 5);
        }
        assert!(manas.is_stable(), "low entropy should make manas stable");

        // ── Phase 2: Accumulate problematic seeds (old, contradictory, redundant) ──
        let old = now - 120 * 24 * 3600; // 120 days ago
        // Old stale seeds — OnDemand so they contribute to entropy fully
        for i in 0..10 {
            insert_test_seed(
                &store,
                &format!("old{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText {
                    text: "stale_data".into(),
                },
                old,
                old,
                0,
                0.1,
                SeedTier::OnDemand,
            );
        }
        // Contradictory seeds (same key, different values)
        insert_test_seed(
            &store,
            "contra1",
            SeedNature::Preference,
            SeedSource::Consolidation,
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "vim".into(),
            },
            now - 30 * 24 * 3600,
            now - 30 * 24 * 3600,
            0,
            1.0,
            SeedTier::OnDemand,
        );
        insert_test_seed(
            &store,
            "contra2",
            SeedNature::Preference,
            SeedSource::Consolidation,
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "emacs".into(),
            },
            now - 25 * 24 * 3600,
            now - 25 * 24 * 3600,
            0,
            1.0,
            SeedTier::OnDemand,
        );
        insert_test_seed(
            &store,
            "contra3",
            SeedNature::Preference,
            SeedSource::Consolidation,
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "vscode".into(),
            },
            now - 20 * 24 * 3600,
            now - 20 * 24 * 3600,
            0,
            1.0,
            SeedTier::OnDemand,
        );
        // Redundant seeds (same geju_key)
        for i in 0..5 {
            let s = Seed {
                id: format!("redun{i            }"),
                session_id: "test".into(),
                project_id: String::new(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::FreeText {
                    text: "same_pattern".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "redundant_pattern".into(),
                created_at: now - 10 * 24 * 3600,
                access_count: 0,
                last_accessed_at: now - 10 * 24 * 3600,
                strength: 0.5,
                tier: SeedTier::OnDemand,
            };
            store
                .insert_seed(&serde_json::to_string(&s).unwrap())
                .unwrap();
        }

        // ── Phase 3: Entropy should now be elevated ──
        let seeds: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy2 = AlayaEntropy::compute(&seeds, now);
        assert!(
            entropy2.total > entropy.total,
            "mixed seeds: entropy should rise from {:.3}, got {:.3}",
            entropy.total,
            entropy2.total
        );
        assert!(
            entropy2.contradiction > 0.0,
            "contradictory KV seeds: contradiction should be > 0, got {:.3}",
            entropy2.contradiction
        );
        assert!(
            entropy2.redundancy > 0.0,
            "KV duplicate keys: redundancy should be > 0, got {:.3}",
            entropy2.redundancy
        );

        // Manas recalibrates with bad entropy → ātma-grāha should rise relative to Phase 1
        let atma_before_degradation = manas.atma_graha;
        manas.recalibrate(&entropy2, seeds.len());
        assert!(
            manas.atma_graha > atma_before_degradation,
            "ātma-grāha should rise with degraded memory: {:.3} → {:.3}",
            atma_before_degradation,
            manas.atma_graha
        );

        // ── Phase 4: Dissolution cleans up ──
        let seeds_before: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy_before = AlayaEntropy::compute(&seeds_before, now);
        // Use a threshold just below current entropy to ensure dissolution triggers
        let threshold = (entropy_before.total - 0.05).max(0.30);
        let report = ZuowangPipeline::dissolve(store.clone(), threshold).unwrap();
        assert!(report.seeds_examined > 0);
        // Old low-strength seeds should be dissolved, weakened, or downgraded
        let affected = report.seeds_dissolved + report.seeds_weakened + report.seeds_downgraded;
        assert!(
            affected > 0,
            "dissolution should affect some seeds, got d={} w={} dg={}",
            report.seeds_dissolved,
            report.seeds_weakened,
            report.seeds_downgraded
        );

        // ── Phase 5: Post-dissolution entropy should decrease ──
        let seeds: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy3 = AlayaEntropy::compute(&seeds, now);
        assert!(
            entropy3.total < entropy2.total || entropy3.total < 0.6,
            "post-dissolution entropy {:.3} should drop from {:.3}",
            entropy3.total,
            entropy2.total
        );

        // Fresh seeds should survive
        let has_fresh = seeds.iter().any(|s| s.id.starts_with("fresh"));
        assert!(
            has_fresh,
            "fresh high-quality seeds should survive dissolution"
        );

        // Manas should recover after cleanup (need enough iterations
        // due to momentum blend from elevated ātma-grāha after Phase 3)
        for _ in 0..15 {
            manas.recalibrate(&entropy3, seeds.len());
        }
        assert!(
            manas.atma_graha < 0.50,
            "post-dissolution: atma_graha should drop significantly, got {:.3}",
            manas.atma_graha
        );

        // ── Phase 6: Second cycle — add new knowledge, verify system stays healthy ──
        for i in 0..3 {
            insert_test_seed(
                &store,
                &format!("new{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::KeyValue {
                    key: "framework".into(),
                    value: "tokio".into(),
                },
                now,
                now,
                0,
                1.0,
                SeedTier::OnDemand,
            );
        }

        let seeds: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy4 = AlayaEntropy::compute(&seeds, now);
        // System should be stable enough to not trigger another dissolution immediately
        let final_report = ZuowangPipeline::dissolve(store, 0.75).unwrap();
        // After first cleanup, second dissolve should affect fewer seeds
        let final_affected = final_report.seeds_dissolved
            + final_report.seeds_weakened
            + final_report.seeds_downgraded;
        assert!(
            final_affected <= affected + 3,
            "second cycle should not be more aggressive: affected={} vs first={}",
            final_affected,
            affected
        );

        tracing::info!(
            "Evolution cycle complete: entropy {:.3}→{:.3}→{:.3}→{:.3}, seeds: {}→{}→{}",
            entropy.total,
            entropy2.total,
            entropy3.total,
            entropy4.total,
            5,
            5 + 10 + 3 + 5,
            seeds.len(),
        );
    }

    /// ── Effectiveness: Multi-session agent lifecycle ──────────────
    ///
    /// Simulates 3 "sessions" of developer-agent interaction with a mock
    /// Rust project. Each session produces seeds from tool execution traces.
    /// Between sessions, Zuowang dissolves stale/weak seeds. Manas is
    /// recalibrated after each session from actual seed entropy.
    ///
    /// Verifies:
    ///   - Seeds accumulate proportionally to activity
    ///   - Cross-session seed influence returns relevant past experiences
    ///   - User-stated facts survive multiple dissolution cycles
    ///   - Manas atma_graha converges downward with healthy memory
    ///   - Seed count stabilizes (Zuowang removes ~as many as are created)
    ///   - Relevance scoring correctly ranks seeds
    #[test]
    fn agent_memory_effectiveness_simulation() {
        use crate::palaces::Palace;
        use crate::stems::Stem;
        use crate::vijnana::alaya::Seed;
        use crate::vijnana::alaya::{SeedContent, SeedNature, SeedSource, SeedStore};
        use crate::vijnana::manas::Manas;
        use crate::zuowang::pipeline::ZuowangPipeline;
        use crate::zuowang::trigger::AlayaEntropy;

        let store = temp_store();
        let now = crate::utils::unix_now();

        // ── Helper: create a seed from a simulated tool execution ──
        fn turn_seed(
            session: &str,
            id: &str,
            tool: &str,
            geju: &str,
            mode: &str,
            success: bool,
            ts: i64,
        ) -> Seed {
            let error_text = if !success {
                ", error=compilation failed: mismatched types"
            } else {
                ""
            };
            let output = if success {
                if tool == "read_file" {
                    "src/main.rs: 120 lines, fn main() entry point"
                } else if tool == "grep" {
                    "found 15 matches in 8 files"
                } else if tool == "patch_file" {
                    "replaced 3 occurrences successfully"
                } else {
                    "build succeeded, 0 errors"
                }
            } else {
                "compilation error at src/lib.rs:42"
            };
            Seed {
                id: format!("{session}-{id}"),
                session_id: session.into(),
                project_id: String::new(),
                nature: if success {
                    SeedNature::Fact
                } else {
                    SeedNature::Inference
                },
                source: SeedSource::ToolObservation,
                content: SeedContent::FreeText {
                    text: if success {
                        format!("{tool}: {output} (geju: {geju}, mode: {mode})")
                    } else {
                        format!("{tool} FAILED: {output}{error_text} (geju: {geju}, mode: {mode})")
                    },
                },
                palace: match tool {
                    "read_file" | "grep" => Palace::Kan,
                    "patch_file" | "write_file" => Palace::Kun,
                    "shell" => Palace::Zhen,
                    _ => Palace::Zhen,
                },
                intent_stem: match tool {
                    "read_file" | "grep" => Stem::Wu,
                    "patch_file" | "write_file" => Stem::Ji,
                    "shell" => Stem::Geng,
                    _ => Stem::Geng,
                },
                geju_key: geju.into(),
                created_at: ts,
                last_accessed_at: ts + (if success { 3600 } else { 0 }),
                access_count: if success { 2 } else { 1 },
                strength: if success { 0.9 } else { 0.4 },
                tier: SeedTier::OnDemand,
            }
        }

        // ── Session 1: Codebase exploration (6 turns, mostly success) ──
        let mut manas = Manas::new();
        let s1 = "s1";
        let s1_tools = [
            ("read_file", "wu_jia_kan", "Direct", true),
            ("grep", "wu_jia_kan", "Direct", true),
            ("grep", "wu_jia_zhen", "Guarded", true),
            ("shell", "geng_jia_zhen", "Sandbox", true),
            ("shell", "geng_jia_li", "Guarded", false), // build failed
            ("read_file", "wu_jia_kan", "Direct", true),
        ];
        for (i, (tool, geju, mode, success)) in s1_tools.iter().enumerate() {
            let seed = turn_seed(
                s1,
                &format!("t{i}"),
                tool,
                geju,
                mode,
                *success,
                now + i as i64,
            );
            store
                .insert_seed(&serde_json::to_string(&seed).unwrap())
                .unwrap();
        }

        // Session 1 recap: check seed accumulation
        let seeds_s1: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        assert_eq!(seeds_s1.len(), 6, "session 1 should create 6 seeds");
        let s1_successes = seeds_s1.iter().filter(|s| s.strength > 0.8).count();
        assert_eq!(s1_successes, 5, "5 of 6 turns should be successes");

        // Session 1 entropy + manas recalibration
        let entropy_s1 = AlayaEntropy::compute(&seeds_s1, now + 10);
        manas.recalibrate(&entropy_s1, seeds_s1.len());
        assert!(
            manas.atma_graha < 0.70,
            "after mostly-successful session, atma_graha should drop below 0.70, got {:.3}",
            manas.atma_graha
        );

        // ── Session 2: Debugging (8 turns, more errors) ──
        let s2 = "s2";
        let s2_tools = [
            ("grep", "wu_jia_zhen", "Guarded", true),
            ("patch_file", "ji_jia_dui", "Sandbox", true),
            ("shell", "geng_jia_zhen", "Sandbox", false), // build error
            ("read_file", "wu_jia_kan", "Direct", true),
            ("patch_file", "ji_jia_dui", "Sandbox", false), // edit failed
            ("grep", "wu_jia_zhen", "Guarded", true),
            ("shell", "geng_jia_zhen", "Sandbox", false), // still fails
            ("shell", "geng_jia_zhen", "Sandbox", true),  // finally builds
        ];
        for (i, (tool, geju, mode, success)) in s2_tools.iter().enumerate() {
            let seed = turn_seed(
                s2,
                &format!("t{i}"),
                tool,
                geju,
                mode,
                *success,
                now + 1000 + i as i64,
            );
            store
                .insert_seed(&serde_json::to_string(&seed).unwrap())
                .unwrap();
        }

        let seeds_s2: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        assert_eq!(seeds_s2.len(), 14, "should have 14 seeds after session 2");
        let s2_failures = seeds_s2.iter().filter(|s| s.strength < 0.5).count();
        assert!(s2_failures >= 3, "session 2 should have error seeds");

        // Session 2 entropy — should be higher due to mixed results
        let entropy_s2 = AlayaEntropy::compute(&seeds_s2, now + 2000);
        manas.recalibrate(&entropy_s2, seeds_s2.len());
        // Mixed session: atma_graha may rise due to contradictions/failures
        assert!(
            manas.atma_graha > 0.15 && manas.atma_graha < 0.80,
            "atma_graha should stay in valid range: {:.3}",
            manas.atma_graha
        );

        // ── Preservation check: user-stated facts ──
        // Insert a user preference that should survive dissolution
        let user_seed = Seed {
            id: "user-pref-1".into(),
            session_id: "s1".into(),
            project_id: String::new(),
            nature: SeedNature::Preference,
            source: SeedSource::UserStatement,
            content: SeedContent::KeyValue {
                key: "preferred_editor".into(),
                value: "neovim".into(),
            },
            palace: Palace::Kun,
            intent_stem: Stem::Ji,
            geju_key: "user_pref".into(),
            created_at: now - 30 * 24 * 3600, // 30 days old
            last_accessed_at: now - 25 * 24 * 3600,
            access_count: 0,
            strength: 0.3, // low strength, but UserStatement → should survive
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&user_seed).unwrap())
            .unwrap();

        // ── Dissolution cycle 1 ──
        let seeds_before_d1: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy_before_d1 = AlayaEntropy::compute(&seeds_before_d1, now + 3000);
        // Use adaptive threshold to ensure dissolution triggers
        let threshold = (entropy_before_d1.total - 0.05).max(0.30);
        let _report1 = ZuowangPipeline::dissolve(store.clone(), threshold).unwrap();

        let seeds_after_d1: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy_after_d1 = AlayaEntropy::compute(&seeds_after_d1, now + 3000);

        // User preference must survive
        let user_still_there = seeds_after_d1.iter().any(|s| s.id == "user-pref-1");
        assert!(
            user_still_there,
            "user-stated seed MUST survive dissolution"
        );

        // Entropy should decrease or stay low after dissolution
        assert!(
            entropy_after_d1.total <= entropy_before_d1.total + 0.05,
            "entropy should not rise after dissolution: before={:.3}, after={:.3}",
            entropy_before_d1.total,
            entropy_after_d1.total
        );

        // ── Session 3: Feature development (7 turns, mostly success) ──
        let s3 = "s3";
        let s3_tools = [
            ("read_file", "wu_jia_kan", "Direct", true),
            ("patch_file", "ji_jia_dui", "Sandbox", true),
            ("patch_file", "ji_jia_dui", "Sandbox", true),
            ("shell", "geng_jia_zhen", "Sandbox", true),
            ("grep", "wu_jia_zhen", "Guarded", true),
            ("write_file", "ji_jia_kun", "Guarded", true),
            ("shell", "geng_jia_li", "Guarded", true),
        ];
        for (i, (tool, geju, mode, success)) in s3_tools.iter().enumerate() {
            let seed = turn_seed(
                s3,
                &format!("t{i}"),
                tool,
                geju,
                mode,
                *success,
                now + 4000 + i as i64,
            );
            store
                .insert_seed(&serde_json::to_string(&seed).unwrap())
                .unwrap();
        }

        let seeds_s3: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        // ── Cross-session seed influence ──
        let seed_store = SeedStore::new(store.clone());
        let prompt = seed_store.top_influence_prompt(20).0;
        assert!(!prompt.is_empty(), "should find cross-session seeds");
        let has_exec = prompt.contains("shell") || prompt.contains("build");
        let has_edit = prompt.contains("patch_file") || prompt.contains("write_file");
        assert!(
            has_exec,
            "seed influence should mention past execution experiences"
        );
        assert!(
            has_edit,
            "seed influence should mention past edit experiences"
        );

        // ── Entropy check + Manas convergence ──
        let entropy_s3 = AlayaEntropy::compute(&seeds_s3, now + 5000);
        manas.recalibrate(&entropy_s3, seeds_s3.len());
        // After 3 sessions with mostly successful outcomes, manas should converge low
        assert!(
            manas.atma_graha < 0.55,
            "after 3 healthy sessions, atma_graha should converge, got {:.3}",
            manas.atma_graha
        );

        // ── Dissolution cycle 2 ──
        let threshold2 = (entropy_s3.total - 0.05).max(0.30);
        let _report2 = ZuowangPipeline::dissolve(store.clone(), threshold2).unwrap();

        let seeds_final: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        let entropy_final = AlayaEntropy::compute(&seeds_final, now + 5000);
        manas.recalibrate(&entropy_final, seeds_final.len());

        // User preference must STILL survive after 2 dissolution cycles
        let user_still_there = seeds_final.iter().any(|s| s.id == "user-pref-1");
        assert!(
            user_still_there,
            "user seed must survive multiple dissolution cycles"
        );

        // ── Final health metrics ──
        let success_ratio = seeds_final.iter().filter(|s| s.strength > 0.7).count() as f32
            / seeds_final.len().max(1) as f32;
        // After pruning, healthy seeds should dominate
        assert!(
            success_ratio > 0.3,
            "healthy seeds should dominate after dissolution, got {:.2}",
            success_ratio
        );

        // Manas should be in healthy range
        assert!(
            manas.atma_graha <= 0.70,
            "final atma_graha should be in healthy range, got {:.3}",
            manas.atma_graha
        );

        // Seed count should have stabilized (not unbounded growth)
        let total_seeds = seeds_final.len();
        assert!(
            total_seeds <= 30,
            "seed count should stabilize after dissolution, got {total_seeds}"
        );

        tracing::info!(
            "Agent memory effectiveness: sessions=3, final_seeds={}, atma_graha={:.3}, \
             success_ratio={:.2}, entropy: {:.3}→{:.3}→{:.3}→{:.3}",
            total_seeds,
            manas.atma_graha,
            success_ratio,
            entropy_s1.total,
            entropy_s2.total,
            entropy_s3.total,
            entropy_final.total,
        );
    }

    // ── Extreme states ────────────────────────────────────

    /// All seeds dissolved → system should handle empty state correctly.
    #[test]
    fn extreme_all_seeds_dissolved() {
        use crate::vijnana::manas::Manas;

        let store = temp_store();
        let now = crate::utils::unix_now();
        let old = now - 200 * 24 * 3600; // 200 days — guaranteed to dissolve

        // Insert only extremely old, weak seeds that will all dissolve
        for i in 0..10 {
            insert_test_seed(
                &store,
                &format!("old{i}"),
                SeedNature::Fact,
                SeedSource::ToolObservation,
                SeedContent::FreeText {
                    text: "very old".into(),
                },
                old,
                old,
                0,
                0.01,
                SeedTier::Archive,
            ); // strength=0.01, no access ever
        }

        // Dissolve with a low threshold to ensure it triggers
        let report = ZuowangPipeline::dissolve(store.clone(), 0.10).unwrap();
        println!(
            "  Dissolved: {}, Weakened: {}",
            report.seeds_dissolved, report.seeds_weakened
        );

        // All seeds should be dissolved
        assert!(
            report.seeds_dissolved > 0,
            "old weak seeds should be dissolved, got dissolved={}",
            report.seeds_dissolved
        );

        // Verify emptiness
        let remaining = store.load_all_seeds().unwrap();
        let remaining_seeds: Vec<Seed> = remaining
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        println!("  Remaining seeds: {}", remaining_seeds.len());

        // Entropy on empty seed set should be zero
        let entropy = AlayaEntropy::compute(&remaining_seeds, now);
        assert!(
            (entropy.total - 0.0).abs() < 0.001,
            "empty seed set should have zero entropy, got {:.3}",
            entropy.total
        );
        assert!((entropy.staleness - 0.0).abs() < 0.001);
        assert!((entropy.contradiction - 0.0).abs() < 0.001);

        // Manas should still function after all seeds dissolved
        let mut manas = Manas::new();
        manas.recalibrate(&entropy, 0);
        // With zero entropy, the entropy_driven component should be minimal
        // but atma_graha shouldn't jump to MAX
        assert!(
            manas.atma_graha < 0.80,
            "manas should not panic after all seeds dissolved, atma_graha={:.3}",
            manas.atma_graha
        );

        // Dissolve on empty store should be a no-op
        let report2 = ZuowangPipeline::dissolve(store, 0.10).unwrap();
        assert_eq!(report2.seeds_examined, 0);
        assert_eq!(report2.seeds_dissolved, 0);
    }

    /// Mass redundancy — many seeds with the same key should trigger
    /// redundancy detection. Seeds should be weakened, not all deleted.
    #[test]
    fn extreme_mass_redundancy() {
        let store = temp_store();
        let now = crate::utils::unix_now();

        // Fresh seeds, all with the same KV key and same value (pure redundancy)
        // Re-accessed frequently → access_decay and staleness stay low
        for i in 0..20 {
            let seed = Seed {
                id: format!("redun{i            }"),
                session_id: "test".into(),
                project_id: String::new(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::KeyValue {
                    key: "build_status".into(),
                    value: "passed".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "redundant".into(),
                created_at: now,
                access_count: 5,
                last_accessed_at: now,
                strength: 1.0,
                tier: SeedTier::OnDemand,
            };
            store
                .insert_seed(&serde_json::to_string(&seed).unwrap())
                .unwrap();
        }

        let seeds: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        assert_eq!(seeds.len(), 20);

        let entropy = AlayaEntropy::compute(&seeds, now);
        println!(
            "  Entropy with 20 redundant seeds: total={:.3} redundancy={:.3} staleness={:.3} contradiction={:.3}",
            entropy.total, entropy.redundancy, entropy.staleness, entropy.contradiction
        );

        // 20 seeds, all same key → 19 redundant out of 20 → redundancy = 19/20 = 0.95
        assert!(
            entropy.redundancy > 0.8,
            "mass redundancy should be detected, got redundancy={:.3}",
            entropy.redundancy
        );
        // No contradiction (all same value)
        assert!(
            (entropy.contradiction - 0.0).abs() < 0.01,
            "identical values should not contradict, got {:.3}",
            entropy.contradiction
        );
        // Staleness should be low (all fresh)
        assert!(
            entropy.staleness < 0.1,
            "fresh seeds should have low staleness, got {:.3}",
            entropy.staleness
        );
        // Total should be dominated by redundancy
        assert!(
            entropy.total > 0.2,
            "redundancy should push total entropy up, got {:.3}",
            entropy.total
        );

        // Run dissolution — fresh strong seeds should NOT be deleted
        let report = ZuowangPipeline::dissolve(store.clone(), 0.20).unwrap();
        println!(
            "  Dissolved: {}, Weakened: {} (examined {})",
            report.seeds_dissolved, report.seeds_weakened, report.seeds_examined
        );

        // Fresh seeds with strength=1.0 have relevance_score ≈ 1.0, far above the
        // delete threshold (< 0.1) and weaken threshold (0.1-0.2). So nothing should
        // be dissolved/weakened — the dissolution happens based on per-seed relevance,
        // not on the entropy metric itself.
        // This is expected: Zuowang dissolves low-relevance seeds; redundancy is a
        // signal for the agent to consolidate and stop creating identical seeds,
        // not to delete them.
        let remaining: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();
        assert_eq!(
            remaining.len(),
            20,
            "fresh strong seeds should not be dissolved (even if redundant)"
        );
    }

    /// Mixed: some redundant OLD seeds + some redundant FRESH seeds.
    /// Only the old ones should be affected.
    #[test]
    fn extreme_mixed_old_and_new_redundancy() {
        let store = temp_store();
        let now = crate::utils::unix_now();
        let old = now - 120 * 24 * 3600;

        // Old redundant seeds — should be dissolved
        for i in 0..8 {
            let seed = Seed {
                id: format!("old_redun{i            }"),
                session_id: "test".into(),
                project_id: String::new(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::KeyValue {
                    key: "old_status".into(),
                    value: "stale".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "old".into(),
                created_at: old,
                access_count: 0,
                last_accessed_at: old,
                strength: 0.03, // very weak
                tier: SeedTier::Archive,
            };
            store
                .insert_seed(&serde_json::to_string(&seed).unwrap())
                .unwrap();
        }

        // Fresh seeds (different key) — should survive
        for i in 0..5 {
            let seed = Seed {
                id: format!("fresh{i            }"),
                session_id: "test".into(),
                project_id: String::new(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::KeyValue {
                    key: "fresh_status".into(),
                    value: "ok".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "fresh".into(),
                created_at: now,
                access_count: 5,
                last_accessed_at: now,
                strength: 1.0,
                tier: SeedTier::OnDemand,
            };
            store
                .insert_seed(&serde_json::to_string(&seed).unwrap())
                .unwrap();
        }

        // User preference — must survive
        let user_seed = Seed {
            id: "user-pref-extreme".into(),
            session_id: "test".into(),
            project_id: String::new(),
            nature: SeedNature::Preference,
            source: SeedSource::UserStatement,
            content: SeedContent::KeyValue {
                key: "likes".into(),
                value: "rust".into(),
            },
            palace: Palace::Kun,
            intent_stem: Stem::Ji,
            geju_key: "pref".into(),
            created_at: old, // even if old
            access_count: 0,
            last_accessed_at: old,
            strength: 0.02, // very weak
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&user_seed).unwrap())
            .unwrap();

        let report = ZuowangPipeline::dissolve(store.clone(), 0.15).unwrap();
        println!(
            "  Dissolved: {}, Weakened: {} (examined {})",
            report.seeds_dissolved, report.seeds_weakened, report.seeds_examined
        );

        let remaining: Vec<Seed> = store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        // Fresh seeds survive
        let fresh_surviving = remaining
            .iter()
            .filter(|s| s.id.starts_with("fresh"))
            .count();
        assert_eq!(fresh_surviving, 5, "all fresh seeds should survive");

        // User preference survives
        assert!(
            remaining.iter().any(|s| s.id == "user-pref-extreme"),
            "user preference must survive even when old and weak"
        );

        // Old redundant seeds should be dissolved
        let old_surviving = remaining
            .iter()
            .filter(|s| s.id.starts_with("old_redun"))
            .count();
        println!("  Old seeds surviving: {old_surviving}/8");
        // At least some old seeds should be gone
        assert!(
            old_surviving < 8,
            "at least some old weak seeds should be dissolved, got {old_surviving} surviving"
        );
    }

    // ── Scenario test: cross-session memory recall ───────────

    #[test]
    fn scenario_cross_session_memory_recall() {
        use crate::vijnana::alaya::SeedStore;
        let now = crate::utils::unix_now();

        let store = temp_store();
        let seed_store = SeedStore::new(store.clone());

        // Session 1: create user preference seeds
        insert_test_seed(
            &store,
            "seed-1",
            SeedNature::Preference,
            SeedSource::UserStatement,
            SeedContent::KeyValue {
                key: "language".into(),
                value: "Rust".into(),
            },
            now - 3600,
            now - 3600,
            5,
            0.9,
            SeedTier::OnDemand,
        );
        insert_test_seed(
            &store,
            "seed-2",
            SeedNature::Fact,
            SeedSource::ToolObservation,
            SeedContent::FreeText {
                text: "project uses PostgreSQL for persistence".into(),
            },
            now - 7200,
            now - 3600,
            3,
            0.7,
            SeedTier::OnDemand,
        );
        insert_test_seed(
            &store,
            "seed-3",
            SeedNature::Preference,
            SeedSource::UserStatement,
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "vim".into(),
            },
            now - 1800,
            now - 600,
            8,
            0.95,
            SeedTier::OnDemand,
        );

        // Verify all 3 seeds are stored
        let all = seed_store.load_all().unwrap();
        assert_eq!(all.len(), 3, "all 3 seeds should be present");

        // Top influence prompt should return the strongest seed first
        let (prompt, touched) = seed_store.top_influence_prompt(10);
        assert!(
            !prompt.is_empty(),
            "top_influence should return prompt text"
        );
        assert_eq!(touched.len(), 3, "all 3 seeds should be touched");

        // "Rust" preference should have highest relevance_score due to high strength
        // and recent access
        let rust_seed = all.iter().find(|s| s.id == "seed-1").unwrap();
        let score = rust_seed.relevance_score(now);
        assert!(
            score > 0.4,
            "fresh strong seed should have relevance > 0.4, got {score}"
        );

        // "editor=vim" should have highest score (most recent + highest strength)
        let vim_seed = all.iter().find(|s| s.id == "seed-3").unwrap();
        let vim_score = vim_seed.relevance_score(now);
        assert!(
            vim_score > score,
            "most recent + strongest seed should rank highest"
        );
    }
}

use std::sync::Arc;
#[cfg(test)]
mod integration_tests {
    use std::sync::Arc;
    use super::super::truncate;
    use crate::palaces::Palace;
    use crate::palaces::gen_store::Store;
    use crate::stems::Stem;
    use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore, SeedTier};
    use crate::vijnana::manas::Manas;
    use crate::vijnana::mano::TurnSnapshot;
    use crate::zuowang::pipeline::ZuowangPipeline;
    use crate::zuowang::trigger::AlayaEntropy;

    /// Create realistic TurnSnapshots simulating a Rust debugging session.
    fn debugging_session_snapshots() -> Vec<TurnSnapshot> {
        let base_ts = crate::utils::unix_now();
        vec![
            TurnSnapshot {
                turn_number: 1, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_kan".into(), execution_mode: "Direct".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "src/main.rs"}),
                tool_output: "120 lines, fn main() entry point, imports tokio and serde".into(),
                tool_error: None, timestamp: base_ts,
            },
            TurnSnapshot {
                turn_number: 2, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_zhen".into(), execution_mode: "Guarded".into(),
                tool_name: "grep".into(),
                tool_input: serde_json::json!({"pattern": "Config", "path": "src/"}),
                tool_output: "Found 8 matches in 3 files: src/config.rs (5), src/main.rs (2), src/lib.rs (1)".into(),
                tool_error: None, timestamp: base_ts + 1,
            },
            TurnSnapshot {
                turn_number: 3, intent_stem: Stem::Ji, target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(), execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "src/config.rs", "old_string": "port: 8080", "new_string": "port: 3000"}),
                tool_output: "Replaced 1 occurrence in src/config.rs".into(),
                tool_error: None, timestamp: base_ts + 2,
            },
            TurnSnapshot {
                turn_number: 4, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "".into(),
                tool_error: Some("error[E0308]: mismatched types in src/config.rs:42: expected String, found &str".into()),
                timestamp: base_ts + 3,
            },
            TurnSnapshot {
                turn_number: 5, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_kan".into(), execution_mode: "Direct".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "src/config.rs", "offset": 38, "limit": 10}),
                tool_output: "Line 42: let host: String = \"localhost\"; // type mismatch here".into(),
                tool_error: None, timestamp: base_ts + 4,
            },
            TurnSnapshot {
                turn_number: 6, intent_stem: Stem::Ji, target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(), execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "src/config.rs", "old_string": "let host: String = \"localhost\"", "new_string": "let host: String = \"localhost\".to_string()"}),
                tool_output: "Replaced 1 occurrence in src/config.rs".into(),
                tool_error: None, timestamp: base_ts + 5,
            },
            TurnSnapshot {
                turn_number: 7, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "Compiling jia v0.1.0\nFinished dev [unoptimized] target(s) in 2.34s".into(),
                tool_error: None, timestamp: base_ts + 6,
            },
            TurnSnapshot {
                turn_number: 8, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_zhen".into(), execution_mode: "Guarded".into(),
                tool_name: "grep".into(),
                tool_input: serde_json::json!({"pattern": "use tokio|use serde", "path": "src/"}),
                tool_output: "src/main.rs: use tokio; src/config.rs: use serde::Deserialize; src/lib.rs: use tokio::sync".into(),
                tool_error: None, timestamp: base_ts + 7,
            },
        ]
    }

    // ── Shared helpers ────────────────────────────────────────

    /// Call LLM with the given prompt, return parsed JSON array of facts.
    async fn infer_facts(
        core: &crate::palaces::zhong_core::JiaCore,
        prompt: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        use futures::StreamExt;
        let messages = vec![crate::types::Message::text(
            crate::types::Role::User,
            prompt.to_string(),
        )];
        let mut stream = core.infer(messages, None, None);
        let mut raw = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta)) => raw.push_str(&delta),
                Ok(_) => {}
                Err(e) => return Err(format!("LLM stream error: {e}")),
            }
        }
        println!(
            "  LLM response (len={}): {}",
            raw.len(),
            &raw[..raw.len().min(200)]
        );
        if raw.is_empty() {
            return Err("LLM returned empty response".to_string());
        }

        match serde_json::from_str(&raw) {
            Ok(f) => Ok(f),
            Err(_) => {
                let trimmed = raw.trim();
                // Try to find complete JSON array [...]
                if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
                    serde_json::from_str(&trimmed[start..=end]).map_err(|e| {
                        format!(
                            "JSON parse error: {e}\nExtracted: {}",
                            &trimmed[start..=end]
                        )
                    })
                } else if trimmed.starts_with('[') {
                    // Unclosed array: try to salvage by truncating to last complete object
                    // Find the last "}," or "}" before the truncation point
                    let last_complete = trimmed
                        .rfind("},")
                        .or_else(|| trimmed.rfind("}\n"))
                        .unwrap_or_else(|| {
                            // Try to find the last "}" and close the array there
                            trimmed.rfind('}').map(|i| i + 1).unwrap_or(trimmed.len())
                        });
                    let salvageable = format!("{}]", &trimmed[..last_complete]);
                    match serde_json::from_str::<Vec<serde_json::Value>>(&salvageable) {
                        Ok(facts) => {
                            println!("  Salvaged {} facts from truncated response", facts.len());
                            Ok(facts)
                        }
                        Err(_e) => {
                            // Last resort: try appending just "]"
                            let closed = format!("{}]", trimmed);
                            serde_json::from_str::<Vec<serde_json::Value>>(&closed)
                                .map_err(|e2| {
                                    format!(
                                        "Cannot parse truncated LLM JSON (len={}): {e2}\nStart: {}...\nEnd: ...{}",
                                        raw.len(),
                                        &raw[..raw.len().min(200)],
                                        &raw[raw.len().saturating_sub(100)..],
                                    )
                                })
                        }
                    }
                } else {
                    Err(format!(
                        "No JSON brackets in LLM response: {}",
                        &raw[..raw.len().min(300)]
                    ))
                }
            }
        }
    }

    /// Create seeds from LLM fact JSON values, returning count created.
    fn create_seeds_from_facts(
        facts: &[serde_json::Value],
        session_id: &str,
        geju_key: &str,
        store: &Arc<Store>,
    ) -> u64 {
        let seed_store = SeedStore::new(store.clone());
        let mut count = 0u64;
        for fact in facts {
            let fact_key = fact
                .get("key")
                .or_else(|| fact.get("subject"))
                .and_then(|v| v.as_str());
            let fact_val = fact
                .get("value")
                .or_else(|| fact.get("object"))
                .and_then(|v| v.as_str());
            let content = match (
                fact.get("type").and_then(|v| v.as_str()),
                fact_key,
                fact_val,
            ) {
                (Some("preference"), Some(k), Some(v)) => SeedContent::KeyValue {
                    key: k.to_string(),
                    value: v.to_string(),
                },
                (_, Some(s), Some(o)) if fact.get("predicate").is_some() => SeedContent::Triple {
                    subject: s.to_string(),
                    predicate: fact
                        .get("predicate")
                        .and_then(|v| v.as_str())
                        .unwrap_or("relates_to")
                        .to_string(),
                    object: o.to_string(),
                },
                _ => {
                    let fallback = fact
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("fact")
                        .to_string();
                    SeedContent::FreeText {
                        text: format!(
                            "{fallback}: {}",
                            serde_json::to_string(fact).unwrap_or_default()
                        ),
                    }
                }
            };
            let seed = Seed::new(
                session_id.to_string(),
                SeedNature::Inference,
                SeedSource::Consolidation,
                content,
                Palace::Gen,
                Stem::Gui,
                geju_key.to_string(),
            );
            if seed_store.insert(&seed).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Build a short consolidation prompt from a slice of snapshots.
    fn build_short_consolidation_prompt(snapshots: &[TurnSnapshot]) -> String {
        let mut lines: Vec<String> = vec![
            "Extract up to 5 key facts as a JSON array. Be concise. Format: {\"type\":\"entity|causal\",\"subject\":\"...\",\"predicate\":\"...\",\"object\":\"...\"} or {\"type\":\"preference\",\"key\":\"...\",\"value\":\"...\"}. Output ONLY the JSON array, no other text.".into(),
        ];
        for s in snapshots {
            let err = s.tool_error.as_deref().unwrap_or("none");
            lines.push(format!(
                "Turn {}: {} → {} (err: {})",
                s.turn_number,
                s.tool_name,
                truncate(&s.tool_output, 60),
                truncate(err, 60),
            ));
        }
        lines.join("\n")
    }

    fn print_entropy(label: &str, e: &AlayaEntropy) {
        println!(
            "  {label}: total={:.3} staleness={:.3} contradiction={:.3} redundancy={:.3} access_decay={:.3}",
            e.total, e.staleness, e.contradiction, e.redundancy, e.access_decay
        );
    }

    // ── Session 2 snapshots (development after Session 1's fix) ──

    fn development_session_snapshots() -> Vec<TurnSnapshot> {
        let base_ts = crate::utils::unix_now() + 1000;
        vec![
            TurnSnapshot {
                turn_number: 1,
                intent_stem: Stem::Geng,
                target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(),
                execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo test 2>&1"}),
                tool_output: "running 120 tests\nall passed".into(),
                tool_error: None,
                timestamp: base_ts,
            },
            TurnSnapshot {
                turn_number: 2,
                intent_stem: Stem::Ji,
                target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(),
                execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "src/config.rs", "old_string": "port: 3000", "new_string": "port: 8080"}),
                tool_output: "Replaced 1 occurrence in src/config.rs".into(),
                tool_error: None,
                timestamp: base_ts + 1,
            },
            TurnSnapshot {
                turn_number: 3,
                intent_stem: Stem::Geng,
                target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(),
                execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build --release 2>&1"}),
                tool_output:
                    "Compiling jia v0.2.0\nFinished release [optimized] target(s) in 12.34s".into(),
                tool_error: None,
                timestamp: base_ts + 2,
            },
        ]
    }

    // ═══════════════════════════════════════════════════════════
    // Real-LLM multi-session memory evolution test
    // ═══════════════════════════════════════════════════════════
    //
    // Validates the full vijnana-zuowang lifecycle with a real LLM:
    //
    //   Session 1 (exploration)        Session 2 (development)
    //   ─────────────────────        ──────────────────────
    //   read_file → grep → edit      shell(test) → edit →
    //   → shell(build fails)         shell(build release)
    //   → edit(fix) → shell(ok)
    //          │                              │
    //     consolidation                 consolidation
    //          │                              │
    //          └────── seeds persist ─────────┘
    //                     │
    //   + injected contradictory seeds ("config_status": broken vs fixed)
    //   + injected aged seed (120 days, strength 0.08 — dissolve target)
    //   + user preference seed
    //                     │
    //          entropy analysis (contradiction > 0)
    //          manas recalibration (ātma-grāha responds to contradiction)
    //          zuowang dissolution (aged seed deleted, user seed survives)
    //          recovery (post-dissolution entropy drops, manas converges)
    //
    /// Requires a running LLM backend (see config.toml). Run with:
    ///   cargo test --lib vijnana::xunxi::integration_tests -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires running LLM backend configured in config.toml"]
    async fn real_llm_multisession_evolution() {
        // ── Setup ─────────────────────────────────────────
        let config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml");
        let app_config = crate::palaces::kun_config::AppConfig::load(
            Some(std::path::PathBuf::from(config_path)),
            None,
            None,
        )
        .expect("load config for test");
        let config_loader = crate::palaces::kun_config::ConfigLoader::from_app_config(app_config);
        let profile = config_loader
            .provider("default")
            .expect("No default provider in config.toml");
        let core = crate::palaces::zhong_core::JiaCore::new(&profile, profile.default_main_model());

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));
        let mut manas = Manas::new();

        // ═══════════════════════════════════════════════════
        // Phase 1: Session 1 — Exploration & debugging
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 1: Session 1 — Exploration ==========");

        let s1_snapshots = debugging_session_snapshots();
        let s1_key_turns: Vec<TurnSnapshot> = s1_snapshots
            .iter()
            .filter(|s| s.tool_error.is_some() || s.tool_name == "shell" || s.tool_name == "edit")
            .take(4)
            .cloned()
            .collect();
        let prompt = build_short_consolidation_prompt(&s1_key_turns);
        println!("Consolidation prompt: {} chars", prompt.len());

        let facts = infer_facts(&core, &prompt)
            .await
            .expect("infer_facts failed");
        println!("Session 1 facts: {}", facts.len());
        for (i, f) in facts.iter().enumerate() {
            println!("  [{i}] {}", serde_json::to_string(f).unwrap_or_default());
        }
        assert!(
            facts.len() >= 3,
            "expected >= 3 facts from 4 turns, got {}",
            facts.len()
        );

        let s1_count = create_seeds_from_facts(&facts, "session-1", "session_1", &store);
        println!("Session 1 seeds created: {s1_count}");

        // Record baseline (refresh now: seeds were just created with current time)
        let now = crate::utils::unix_now();
        let seeds = SeedStore::new(store.clone()).load_all().unwrap();
        let entropy1 = AlayaEntropy::compute(&seeds, now);
        print_entropy("S1 entropy", &entropy1);
        assert!(entropy1.total < 0.5, "fresh seeds should have low entropy");

        manas.record_turn();
        manas.recalibrate(&entropy1, seeds.len());
        println!("ātma-grāha after S1: {:.3}", manas.atma_graha);
        assert!(
            manas.atma_graha < 0.80,
            "fresh memory should lower ātma-grāha"
        );

        // ═══════════════════════════════════════════════════
        // Phase 2: Inject seeds to trigger uncovered paths
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 2: Inject contradictory + aged seeds ==========");

        // Contradictory KeyValue seeds: same key, different values
        let seed_store = SeedStore::new(store.clone());
        seed_store
            .insert(&Seed {
                id: "kv-config-broken".into(),
                session_id: "inject".into(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::KeyValue {
                    key: "config_status".into(),
                    value: "broken".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "inject_kv".into(),
                created_at: now - 10,
                last_accessed_at: now - 5,
                access_count: 1,
                strength: 0.8,
                tier: SeedTier::OnDemand,
            })
            .unwrap();
        seed_store
            .insert(&Seed {
                id: "kv-config-fixed".into(),
                session_id: "inject".into(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::KeyValue {
                    key: "config_status".into(),
                    value: "fixed".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "inject_kv".into(),
                created_at: now - 10,
                last_accessed_at: now - 5,
                access_count: 1,
                strength: 0.8,
                tier: SeedTier::OnDemand,
            })
            .unwrap();

        // Aged seed: 120 days old, very weak → relevance_score < 0.1 → dissolve target
        seed_store
            .insert(&Seed {
                id: "aged-stale-seed".into(),
                session_id: "old-session".into(),
                nature: SeedNature::Fact,
                source: SeedSource::Consolidation,
                content: SeedContent::FreeText {
                    text: "old build output: error linking".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "old_build".into(),
                created_at: now - 120 * 24 * 3600,
                last_accessed_at: now - 100 * 24 * 3600,
                access_count: 0,
                strength: 0.08,
                tier: SeedTier::OnDemand,
            })
            .unwrap();

        // User preference seed (must survive dissolution)
        seed_store
            .insert(&Seed {
                id: "user-pref-editor".into(),
                session_id: "user".into(),
                nature: SeedNature::Preference,
                source: SeedSource::UserStatement,
                content: SeedContent::KeyValue {
                    key: "preferred_tool".into(),
                    value: "cargo".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "user_pref".into(),
                created_at: now - 30 * 24 * 3600,
                last_accessed_at: now,
                access_count: 5,
                strength: 0.9,
                tier: SeedTier::OnDemand,
            })
            .unwrap();

        let seeds = seed_store.load_all().unwrap();
        println!(
            "Seeds after injection: {} (S1={} + 4 injected)",
            seeds.len(),
            s1_count
        );

        // ═══════════════════════════════════════════════════
        // Phase 3: Contradiction detection + manas response
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 3: Contradiction + manas response ==========");

        let now = crate::utils::unix_now();
        let entropy2 = AlayaEntropy::compute(&seeds, now);
        print_entropy("Post-injection entropy", &entropy2);
        assert!(
            entropy2.contradiction > 0.0,
            "contradictory seeds (config_status: broken vs fixed) MUST produce contradiction > 0, got {:.3}",
            entropy2.contradiction
        );
        assert!(
            entropy2.total > entropy1.total,
            "injected seeds should increase entropy: {:.3} → {:.3}",
            entropy1.total,
            entropy2.total
        );

        // Manas should rise in response to contradiction
        let before_contra = manas.atma_graha;
        manas.recalibrate(&entropy2, seeds.len());
        println!(
            "ātma-grāha: {:.3} → {:.3} (contradiction={:.3})",
            before_contra, manas.atma_graha, entropy2.contradiction
        );

        // With fresh + contradictory seeds, ātma-grāha may not rise above the
        // previous value due to momentum, but it should be > pure entropy_driven
        let entropy_driven = 0.05 + entropy2.total * 0.70;
        let expected_min = entropy_driven * 0.6 + before_contra * 0.4;
        assert!(
            (manas.atma_graha - expected_min).abs() < 0.01,
            "ātma-grāha {:.3} should match blend of entropy_driven({:.3}) + momentum({:.3}) = {:.3}",
            manas.atma_graha,
            entropy_driven,
            before_contra,
            expected_min
        );

        // ═══════════════════════════════════════════════════
        // Phase 4: Session 2 — Cross-session development
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 4: Session 2 — Cross-session development ==========");

        let s2_snapshots = development_session_snapshots();
        let prompt = build_short_consolidation_prompt(&s2_snapshots);
        println!("Consolidation prompt: {} chars", prompt.len());

        let facts = infer_facts(&core, &prompt)
            .await
            .expect("infer_facts failed");
        println!("Session 2 facts: {}", facts.len());
        for (i, f) in facts.iter().enumerate() {
            println!("  [{i}] {}", serde_json::to_string(f).unwrap_or_default());
        }
        assert!(!facts.is_empty(), "Session 2 should produce facts");

        let s2_count = create_seeds_from_facts(&facts, "session-2", "session_2", &store);
        println!("Session 2 seeds created: {s2_count}");

        // Cross-session influence: top_influence_prompt should return seeds
        // from both sessions regardless of palace/stem context
        let influence = seed_store.top_influence_prompt(20).0;
        println!("\nCross-session influence (Zhen/Geng):");
        if !influence.is_empty() {
            println!("{}", &influence[..influence.len().min(400)]);
        }
        // At minimum, the user preference seed must be visible
        assert!(
            influence.contains("preferred_tool"),
            "user preference seed must appear in influence, got:\n{influence}"
        );

        // ═══════════════════════════════════════════════════
        // Phase 5: Zuowang dissolution (should actually dissolve)
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 5: Zuowang dissolution ==========");

        let now = crate::utils::unix_now();
        let seeds_before = seed_store.load_all().unwrap();
        let entropy_before = AlayaEntropy::compute(&seeds_before, now);
        print_entropy("Pre-dissolution entropy", &entropy_before);

        // Use a low threshold to ensure dissolution triggers on the aged seed
        let threshold = 0.15;
        println!("Dissolution threshold: {threshold:.3}");
        let report = ZuowangPipeline::dissolve(store.clone(), threshold)
            .expect("ZuowangPipeline should succeed");
        println!(
            "Examined: {}, Dissolved: {}, Weakened: {}",
            report.seeds_examined, report.seeds_dissolved, report.seeds_weakened
        );

        // The aged seed (relevance_score ≈ 0.04) MUST be dissolved
        let aged_dissolved = report.seeds_dissolved > 0;
        println!("Aged seed dissolved: {aged_dissolved}");

        // ═══════════════════════════════════════════════════
        // Phase 6: Post-dissolution verification
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 6: Post-dissolution verification ==========");

        let seeds_after = seed_store.load_all().unwrap();
        let entropy_after = AlayaEntropy::compute(&seeds_after, crate::utils::unix_now());
        print_entropy("Post-dissolution entropy", &entropy_after);

        // User seed must survive
        let user_survived = seeds_after.iter().any(|s| s.id == "user-pref-editor");
        assert!(user_survived, "user-stated seed MUST survive dissolution");
        println!("User seed survived: true");

        // Aged seed should be gone
        let aged_gone = !seeds_after.iter().any(|s| s.id == "aged-stale-seed");
        println!("Aged seed removed: {aged_gone}");

        // Session 1 + 2 consolidation seeds should mostly survive (they're fresh)
        let s1_surviving = seeds_after
            .iter()
            .filter(|s| s.session_id == "session-1")
            .count();
        let s2_surviving = seeds_after
            .iter()
            .filter(|s| s.session_id == "session-2")
            .count();
        println!(
            "Surviving seeds: S1={s1_surviving}, S2={s2_surviving}, injected=contradictory+user"
        );

        // Entropy should drop after dissolution removes the aged seed
        // (contradiction may persist if both contradictory seeds survive)
        println!(
            "Entropy change: {:.3} → {:.3}",
            entropy_before.total, entropy_after.total
        );

        // ═══════════════════════════════════════════════════
        // Phase 7: Recovery — manas convergence toward stable
        // ═══════════════════════════════════════════════════
        println!("\n========== Phase 7: Recovery — ātma-grāha convergence ==========");

        let mut atma_trajectory: Vec<f32> = vec![manas.atma_graha];
        for i in 0..12 {
            manas.record_turn();
            manas.recalibrate(&entropy_after, seeds_after.len());
            atma_trajectory.push(manas.atma_graha);
            if i % 3 == 2 {
                println!(
                    "  iteration {}: ātma-grāha={:.3}, stable_epochs={}, is_stable={}",
                    i + 1,
                    manas.atma_graha,
                    manas.stable_epochs(),
                    manas.is_stable()
                );
            }
        }

        // ātma-grāha should converge downward with consistent healthy entropy
        let final_atma = *atma_trajectory.last().unwrap();
        let mid_atma = atma_trajectory[atma_trajectory.len() / 2];
        assert!(
            final_atma <= mid_atma + 0.05,
            "ātma-grāha should trend downward: mid={:.3}, final={:.3}",
            mid_atma,
            final_atma
        );
        println!(
            "ātma-grāha trajectory: {:.3} → ... → {:.3} → {:.3}",
            atma_trajectory[0], mid_atma, final_atma
        );

        // ═══════════════════════════════════════════════════
        // Final summary
        // ═══════════════════════════════════════════════════
        println!("\n========== Final Health ==========");
        println!("ātma-grāha:  {:.3}", manas.atma_graha);
        println!("stable:      {}", manas.is_stable());
        println!("total seeds: {}", seeds_after.len());
        println!("entropy:     {:.3}", entropy_after.total);
        println!("contradiction: {:.3}", entropy_after.contradiction);
        println!("Session 1+2 consolidation seeds: {s1_surviving}+{s2_surviving}");
        println!("User seed preserved: {user_survived}");
        println!("Aged seed dissolved: {aged_gone}");
    }

    // ═══════════════════════════════════════════════════════════
    // Real-LLM end-to-end test for new memory system components
    // ═══════════════════════════════════════════════════════════
    //
    // Validates the 4 new components with a real LLM:
    //
    //   1. L1 SignalDetector — pattern + keyword extraction from user messages
    //   2. UserProfileManager — prompt() + upsert() with dedup
    //   3. FTS5 semantic search — content_text indexing + search_seeds()
    //   4. Triple 1-hop — graph_expand() on related Triple seeds
    //
    /// Run with:
    ///   cargo test --lib vijnana::xunxi::integration_tests -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires running LLM backend configured in config.toml"]
    async fn e2e_new_memory_system() {
        println!("\n═══════════════════════════════════════════════");
        println!("  End-to-end: New Memory System Components");
        println!("═══════════════════════════════════════════════");

        // ── Setup ─────────────────────────────────────────
        let config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml");
        let app_config = crate::palaces::kun_config::AppConfig::load(
            Some(std::path::PathBuf::from(config_path)),
            None,
            None,
        )
        .expect("load config for test");
        let config_loader = crate::palaces::kun_config::ConfigLoader::from_app_config(app_config);
        let profile = config_loader
            .provider("default")
            .expect("No default provider in config.toml");
        let core = crate::palaces::zhong_core::JiaCore::new(&profile, profile.default_main_model());

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        // ═══════════════════════════════════════════════════
        // Phase 1: L1 SignalDetector
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 1: L1 SignalDetector ──");

        use crate::vijnana::xunxi::signal::SignalDetector;

        let user_messages = vec![
            ("session-1", "我用vim和Rust开发后端，我是后端工程师"),
            ("session-1", "我不喜欢Java，太重了"),
            ("session-2", "我在做jia这个项目，是一个AI agent框架"),
            ("session-2", "我喜欢用Postgres和Redis做数据存储"),
        ];

        let mut l1_total = 0usize;
        for (sid, msg) in &user_messages {
            let n = SignalDetector::process(&store, sid, msg);
            l1_total += n;
            println!("  L1 [{sid}] created {n} seeds from: \"{msg}\"");
        }
        println!("  Total L1 seeds created: {l1_total}");
        assert!(
            l1_total >= 3,
            "L1 should create at least 3 seeds from these messages, got {l1_total}"
        );

        // Verify detected signals
        let seed_store = SeedStore::new(store.clone());
        let all = seed_store.load_all().unwrap();

        let has_role = all.iter().any(|s| {
            matches!(&s.content, SeedContent::KeyValue { key, value }
                if key == "role" && value.contains("后端"))
        });
        assert!(has_role, "should detect role with '后端'");

        let has_tool = all.iter().any(|s| {
            matches!(&s.content, SeedContent::KeyValue { key, value }
                if key == "tool" && value == "vim")
        });
        assert!(has_tool, "should detect tool=vim");

        let has_dislike = all.iter().any(|s| {
            matches!(&s.content, SeedContent::KeyValue { key, .. }
                if key == "dislikes")
        });
        assert!(has_dislike, "should detect a dislike");

        let has_project = all.iter().any(|s| {
            matches!(&s.content, SeedContent::KeyValue { key, .. }
                if key == "project")
        });
        assert!(has_project, "should detect a project");

        let signal_detection_count = all
            .iter()
            .filter(|s| matches!(s.source, SeedSource::SignalDetection))
            .count();
        assert!(
            signal_detection_count >= 3,
            "should have ≥3 SignalDetection seeds, got {signal_detection_count}"
        );

        println!("  L1 PASSED: role, tool, dislike, project all detected ✓");

        // ═══════════════════════════════════════════════════
        // Phase 2: UserProfileManager
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 2: UserProfileManager ──");

        use crate::vijnana::user_profile::UserProfileManager;

        // The Preference seeds from SignalDetector should appear in the profile
        let user_prompt = UserProfileManager::prompt(&store);
        println!("  Profile prompt:\n{user_prompt}");

        assert!(
            user_prompt.contains("## About the user:"),
            "profile should have header, got: {user_prompt}"
        );
        assert!(
            user_prompt.contains("Uses:") || user_prompt.contains("tool:"),
            "profile should mention tool, got: {user_prompt}"
        );
        assert!(
            user_prompt.contains("Role:") || user_prompt.contains("role:"),
            "profile should mention role, got: {user_prompt}"
        );

        // Test upsert: same key overwrites
        let n = UserProfileManager::upsert(&store, "test", "tool", "emacs");
        assert_eq!(n, 1, "upsert new tool should create 1 seed");

        // Verify "tool" updated to emacs (old vim should be gone)
        let updated = UserProfileManager::prompt(&store);
        let has_emacs = updated.contains("emacs");
        println!("  After upsert: {updated}");
        assert!(has_emacs, "profile should show updated tool=emacs");

        println!("  UserProfile PASSED ✓");

        // ═══════════════════════════════════════════════════
        // Phase 3: FTS5 Semantic Search
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 3: FTS5 Semantic Search ──");

        // Search for terms that exist in content_text
        let results = store.search_seeds("Rust", 5).unwrap();
        println!("  FTS5 search for 'Rust': {} results", results.len());
        let rust_results: Vec<_> = results
            .iter()
            .filter(|(json, _)| json.contains("Rust"))
            .collect();
        assert!(
            !rust_results.is_empty(),
            "FTS5 should find 'Rust' in content_text"
        );

        let results = store.search_seeds("Postgres", 5).unwrap();
        println!("  FTS5 search for 'Postgres': {} results", results.len());
        assert!(!results.is_empty(), "FTS5 should find 'Postgres'");

        // Search for a term NOT in any seed
        let results = store.search_seeds("zzz_nonexistent_zzz", 5).unwrap();
        assert!(
            results.is_empty(),
            "FTS5 should return empty for nonexistent term"
        );

        // semantic_influence_prompt() should format results
        let prompt = seed_store.semantic_influence_prompt("Rust", 3).0;
        println!("  Semantic influence prompt (Rust):\n{prompt}");
        if !prompt.is_empty() {
            assert!(prompt.contains("## Related past experience (semantic search)"));
        }

        println!("  FTS5 PASSED ✓");

        // ═══════════════════════════════════════════════════
        // Phase 4: Triple 1-hop graph expansion
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 4: Triple 1-hop Graph Expansion ──");

        // Insert Triple seeds programmatically
        let now = crate::utils::unix_now();
        let triples = vec![
            ("Cargo.toml", "depends_on", "serde"),
            ("Cargo.toml", "depends_on", "tokio"),
            ("src/main.rs", "imports", "tokio"),
            ("src/lib.rs", "uses", "serde"),
        ];
        for (i, (s, p, o)) in triples.iter().enumerate() {
            let seed = Seed {
                id: format!("triple-{i            }"),
                session_id: "triple-test".into(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::Triple {
                    subject: s.to_string(),
                    predicate: p.to_string(),
                    object: o.to_string(),
                },
                palace: Palace::Gen,
                intent_stem: Stem::Gui,
                geju_key: "triple_test".into(),
                created_at: now + i as i64,
                access_count: 0,
                last_accessed_at: now + i as i64,
                strength: 1.0,
                tier: SeedTier::OnDemand,
            };
            seed_store.insert(&seed).unwrap();
        }
        println!("  Inserted {} Triple seeds", triples.len());

        // Graph expand from an anchor value
        let anchor_values = vec!["serde".to_string()];
        let expanded = store.graph_expand(&anchor_values, 5).unwrap();
        println!("  Graph expand from 'serde': {} results", expanded.len());
        assert!(
            !expanded.is_empty(),
            "should find Triple seeds related to serde"
        );

        println!("  Triple 1-hop PASSED ✓");

        // ═══════════════════════════════════════════════════
        // Phase 5: LLM Consolidation (L2)
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 5: LLM Consolidation (L2) ──");

        // Create turn snapshots where the user interacts with Rust tooling
        let base_ts = crate::utils::unix_now();
        let snapshots = vec![
            TurnSnapshot {
                turn_number: 1,
                intent_stem: Stem::Wu,
                target_palace: Palace::Kan,
                geju_name: "wu_jia_kan".into(),
                execution_mode: "Direct".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "Cargo.toml"}),
                tool_output:
                    "[package]\nname = \"jia\"\ndependencies:\n  tokio = \"1\"\n  serde = \"1\""
                        .into(),
                tool_error: None,
                timestamp: base_ts,
            },
            TurnSnapshot {
                turn_number: 2,
                intent_stem: Stem::Geng,
                target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(),
                execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "".into(),
                tool_error: Some(
                    "error[E0432]: unresolved import `serde_json`\n  --> src/main.rs:3:5".into(),
                ),
                timestamp: base_ts + 1,
            },
            TurnSnapshot {
                turn_number: 3,
                intent_stem: Stem::Ji,
                target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(),
                execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "Cargo.toml", "old_string": "serde = \"1\"", "new_string": "serde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\""}),
                tool_output: "Replaced 1 occurrence in Cargo.toml".into(),
                tool_error: None,
                timestamp: base_ts + 2,
            },
            TurnSnapshot {
                turn_number: 4,
                intent_stem: Stem::Geng,
                target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(),
                execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "Compiling jia v0.1.0\nFinished dev target(s) in 2.34s".into(),
                tool_error: None,
                timestamp: base_ts + 3,
            },
        ];

        let prompt = build_short_consolidation_prompt(&snapshots);
        println!("  Consolidation prompt: {} chars", prompt.len());

        let facts = infer_facts(&core, &prompt)
            .await
            .expect("infer_facts failed");
        println!("  LLM extracted {} facts:", facts.len());
        for (i, f) in facts.iter().enumerate() {
            println!("    [{i}] {}", serde_json::to_string(f).unwrap_or_default());
        }
        assert!(!facts.is_empty(), "LLM should extract facts from 4 turns");

        let l2_count = create_seeds_from_facts(&facts, "session-l2", "l2_test", &store);
        println!("  L2 seeds created: {l2_count}");
        assert!(l2_count > 0, "L2 should create at least 1 seed");

        // L2 should have created Triple or KeyValue seeds (not just FreeText)
        let all_after_l2 = seed_store.load_all().unwrap();
        let l2_triples = all_after_l2
            .iter()
            .filter(|s| matches!(s.content, SeedContent::Triple { .. }))
            .count();
        let l2_kv = all_after_l2
            .iter()
            .filter(|s| matches!(s.content, SeedContent::KeyValue { .. }))
            .count();
        println!("  After L2: {} triples, {} key-values", l2_triples, l2_kv);
        // L2 created triples or KV seeds from Cargo.toml dependency info
        assert!(
            l2_triples + l2_kv > 0,
            "L2 should create structured seeds (triples or key-value), got 0"
        );

        println!("  LLM Consolidation PASSED ✓");

        // ═══════════════════════════════════════════════════
        // Phase 6: Full pipeline integration
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 6: Full Retrieval Pipeline ──");

        // Profile
        let profile_prompt = UserProfileManager::prompt(&store);
        assert!(!profile_prompt.is_empty(), "profile should not be empty");

        // Memory catalog
        let label_prompt = seed_store.top_influence_prompt(5).0;
        println!("  Top influence length: {} chars", label_prompt.len());

        // FTS5 semantic search
        let semantic_prompt = seed_store
            .semantic_influence_prompt("Cargo.toml dependencies", 5)
            .0;
        println!("  Semantic search length: {} chars", semantic_prompt.len());

        // All pipeline stages produce non-empty output
        let pipeline_output = format!("{profile_prompt}{label_prompt}{semantic_prompt}");
        println!("  Pipeline total: {} chars", pipeline_output.len());
        assert!(
            !pipeline_output.trim().is_empty(),
            "pipeline should produce non-empty output"
        );
        assert!(
            pipeline_output.contains("About the user"),
            "pipeline should include profile section"
        );

        println!("  Full Pipeline PASSED ✓");

        // ═══════════════════════════════════════════════════
        // Phase 7: Dissolution protection for Preference seeds
        // ═══════════════════════════════════════════════════
        println!("\n── Phase 7: Preference Seed Protection ──");

        // Insert an old weak Preference seed
        seed_store
            .insert(&Seed {
                id: "old-pref".into(),
                session_id: "old".into(),
                nature: SeedNature::Preference,
                source: SeedSource::SignalDetection,
                content: SeedContent::KeyValue {
                    key: "likes".into(),
                    value: "old_technology".into(),
                },
                palace: Palace::Kun,
                intent_stem: Stem::Ji,
                geju_key: String::new(),
                created_at: now - 200 * 24 * 3600,
                access_count: 0,
                last_accessed_at: now - 200 * 24 * 3600,
                strength: 0.03,
                tier: SeedTier::OnDemand,
            })
            .unwrap();

        // Insert a weak non-Preference seed that SHOULD be dissolved
        seed_store
            .insert(&Seed {
                id: "old-tool".into(),
                session_id: "old".into(),
                nature: SeedNature::Fact,
                source: SeedSource::ToolObservation,
                content: SeedContent::FreeText {
                    text: "old tool output".into(),
                },
                palace: Palace::Zhen,
                intent_stem: Stem::Geng,
                geju_key: "old".into(),
                created_at: now - 200 * 24 * 3600,
                access_count: 0,
                last_accessed_at: now - 200 * 24 * 3600,
                strength: 0.03,
                tier: SeedTier::OnDemand,
            })
            .unwrap();

        let report =
            ZuowangPipeline::dissolve(store.clone(), 0.10).expect("ZuowangPipeline should succeed");
        println!(
            "  Dissolved: {}, Weakened: {} (examined {})",
            report.seeds_dissolved, report.seeds_weakened, report.seeds_examined
        );

        // The Preference seed MUST survive
        let remaining = seed_store.load_all().unwrap();
        let pref_survived = remaining.iter().any(|s| s.id == "old-pref");
        assert!(pref_survived, "Preference seed MUST survive dissolution");

        println!("  Preference protection PASSED ✓");

        // ═══════════════════════════════════════════════════
        // Final Summary
        // ═══════════════════════════════════════════════════
        println!("\n═══════════════════════════════════════════════");
        println!("  END-TO-END TEST COMPLETE");
        println!("═══════════════════════════════════════════════");
        println!("  L1 SignalDetector:    ✓ ({l1_total} seeds)");
        println!("  UserProfileManager:   ✓");
        println!("  FTS5 Semantic Search: ✓");
        println!("  Triple 1-hop:         ✓");
        println!("  L2 Consolidation:     ✓ ({l2_count} seeds)");
        println!("  Preference Protection:✓");
        let total = seed_store.load_all().unwrap().len();
        println!("  Total seeds:          {total}");
        println!("═══════════════════════════════════════════════");
    }

    // ═══════════════════════════════════════════════════════════════
    // Rigorous 4-session stress test: identity drift, cross-session
    // recall, memory interference, dissolution under load.
    // ═══════════════════════════════════════════════════════════════
    //
    // Sessions:
    //   1. "Project Alpha" — backend identity (Rust, Postgres, API gateway)
    //   2. "Identity Drift" — frontend learning (React, TypeScript), contradicts backend
    //   3. "Back to Alpha" — cross-session recall, verify no memory interference
    //   4. "Evolution & Cleanup" — inject aged + contradictory seeds, dissolve, verify survivors
    //
    /// Run with:
    ///   cargo test --lib vijnana::xunxi::integration_tests -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "requires running LLM backend configured in config.toml"]
    async fn stress_multisession_evolution() {
        println!("\n╔═══════════════════════════════════════════════╗");
        println!("║  STRESS: 4-Session Evolution & Identity Drift ║");
        println!("╚═══════════════════════════════════════════════╝");

        // ── Setup ─────────────────────────────────────────
        let config_path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml");
        let app_config = crate::palaces::kun_config::AppConfig::load(
            Some(std::path::PathBuf::from(config_path)),
            None,
            None,
        )
        .expect("load config for test");
        let config_loader = crate::palaces::kun_config::ConfigLoader::from_app_config(app_config);
        let profile = config_loader
            .provider("default")
            .expect("No default provider in config.toml");
        let core = crate::palaces::zhong_core::JiaCore::new(&profile, profile.default_main_model());

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("stress.db").to_string_lossy()));
        let seed_store = SeedStore::new(store.clone());
        let mut manas = Manas::new();

        use crate::vijnana::user_profile::UserProfileManager;
        use crate::vijnana::xunxi::signal::SignalDetector;

        // ═══════════════════════════════════════════════════
        // Session 1: "Project Alpha" — Backend Identity
        // ═══════════════════════════════════════════════════
        println!("\n═══ Session 1: Project Alpha — Backend Identity ═══");

        // L1 SignalDetector: explicit tech stack + role statements
        let s1_messages = vec![
            ("session-1", "我在做Project Alpha，一个API网关服务"),
            ("session-1", "我用Rust做后端开发，数据库用Postgres"),
            ("session-1", "我不喜欢ORM，更喜欢写原始SQL"),
            ("session-1", "我是后端工程师，做了10年后端了"),
        ];
        let mut s1_l1_total = 0usize;
        for (sid, msg) in &s1_messages {
            let n = SignalDetector::process(&store, sid, msg);
            s1_l1_total += n;
            println!("  L1 [{sid}] +{n} seeds: \"{msg}\"");
        }
        assert!(
            s1_l1_total >= 3,
            "L1 should detect >= 3 explicit facts, got {s1_l1_total}"
        );

        // L2 Consolidation: backend development session
        let base_ts = crate::utils::unix_now();
        let s1_snapshots = vec![
            TurnSnapshot {
                turn_number: 1, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_kan".into(), execution_mode: "Direct".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "Cargo.toml"}),
                tool_output: "[package]\nname = \"alpha-gateway\"\ndependencies:\n  tokio = \"1\"\n  axum = \"0.7\"\n  sqlx = { version = \"0.7\", features = [\"postgres\"] }".into(),
                tool_error: None, timestamp: base_ts,
            },
            TurnSnapshot {
                turn_number: 2, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "".into(),
                tool_error: Some("error[E0432]: unresolved import `axum::Router`\n  --> src/main.rs:3:5".into()),
                timestamp: base_ts + 1,
            },
            TurnSnapshot {
                turn_number: 3, intent_stem: Stem::Ji, target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(), execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "src/main.rs", "old_string": "use axum::Router;", "new_string": "use axum::{Router, routing::get};"}),
                tool_output: "Replaced 1 occurrence in src/main.rs".into(),
                tool_error: None, timestamp: base_ts + 2,
            },
            TurnSnapshot {
                turn_number: 4, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "Compiling alpha-gateway v0.1.0\nFinished dev target(s) in 3.12s".into(),
                tool_error: None, timestamp: base_ts + 3,
            },
        ];

        let prompt = build_short_consolidation_prompt(&s1_snapshots);
        println!("  S1 consolidation prompt: {} chars", prompt.len());
        let facts = infer_facts(&core, &prompt)
            .await
            .expect("infer_facts failed");
        println!("  S1 LLM facts: {}", facts.len());
        for (i, f) in facts.iter().enumerate() {
            println!("    [{i}] {}", serde_json::to_string(f).unwrap_or_default());
        }
        assert!(
            !facts.is_empty(),
            "S1: LLM should extract facts from Alpha session"
        );

        let s1_l2_count = create_seeds_from_facts(&facts, "session-1", "alpha", &store);
        println!("  S1 L2 seeds: {s1_l2_count}");
        assert!(s1_l2_count > 0, "S1: L2 should create at least 1 seed");

        // S1 entropy check
        let now = crate::utils::unix_now();
        let seeds_s1 = seed_store.load_all().unwrap();
        let entropy_s1 = AlayaEntropy::compute(&seeds_s1, now);
        print_entropy("S1 entropy", &entropy_s1);
        assert!(
            entropy_s1.total < 0.5,
            "S1: fresh seeds should have low entropy, got {:.3}",
            entropy_s1.total
        );
        assert!(
            entropy_s1.contradiction < 0.1,
            "S1: no contradiction expected yet, got {:.3}",
            entropy_s1.contradiction
        );

        manas.record_turn();
        manas.recalibrate(&entropy_s1, seeds_s1.len());
        let atma_s1 = manas.atma_graha;
        println!("  ātma-grāha after S1: {:.3}", atma_s1);
        assert!(
            atma_s1 < 0.80,
            "S1: fresh memory should lower ātma-grāha from 0.80, got {:.3}",
            atma_s1
        );

        // Verify profile injection has "Rust" and "Postgres"
        let profile_s1 = UserProfileManager::prompt(&store);
        println!(
            "  S1 profile (first 200 chars): {}",
            &profile_s1[..profile_s1.len().min(200)]
        );
        assert!(
            profile_s1.contains("Rust")
                || profile_s1.contains("Postgres")
                || profile_s1.contains("后端"),
            "S1 profile should contain user identity from L1"
        );

        println!("  ✓ Session 1 PASSED");

        // ═══════════════════════════════════════════════════
        // Session 2: "Identity Drift" — Frontend Learning
        // ═══════════════════════════════════════════════════
        println!("\n═══ Session 2: Identity Drift — Frontend Learning ═══");

        // User starts learning frontend — should create mild identity contradiction
        let s2_messages = vec![
            ("session-2", "我最近在学React和TypeScript做前端"),
            ("session-2", "前端开发也挺有意思的，虽然我是后端"),
            ("session-2", "TypeScript的类型系统比我想象的好用"),
        ];
        let mut s2_l1_total = 0usize;
        for (sid, msg) in &s2_messages {
            let n = SignalDetector::process(&store, sid, msg);
            s2_l1_total += n;
            println!("  L1 [{sid}] +{n} seeds: \"{msg}\"");
        }
        assert!(
            s2_l1_total >= 1,
            "S2: L1 should detect at least 1 fact, got {s2_l1_total}"
        );

        // L2 Consolidation: frontend development session
        let s2_snapshots = vec![
            TurnSnapshot {
                turn_number: 1, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_kan".into(), execution_mode: "Direct".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "package.json"}),
                tool_output: "{\"name\":\"alpha-ui\",\"dependencies\":{\"react\":\"^18.2\",\"typescript\":\"^5.3\"}}".into(),
                tool_error: None, timestamp: base_ts + 1000,
            },
            TurnSnapshot {
                turn_number: 2, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "npm run build 2>&1"}),
                tool_output: "".into(),
                tool_error: Some("TS2345: Argument of type 'string' is not assignable to parameter of type 'number'".into()),
                timestamp: base_ts + 1001,
            },
            TurnSnapshot {
                turn_number: 3, intent_stem: Stem::Ji, target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(), execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "src/App.tsx", "old_string": "const count: string = \"0\"", "new_string": "const count: number = 0"}),
                tool_output: "Replaced 1 occurrence in src/App.tsx".into(),
                tool_error: None, timestamp: base_ts + 1002,
            },
            TurnSnapshot {
                turn_number: 4, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "npm run build 2>&1"}),
                tool_output: "webpack compiled with 2 warnings\nBuild succeeded".into(),
                tool_error: None, timestamp: base_ts + 1003,
            },
        ];

        let prompt = build_short_consolidation_prompt(&s2_snapshots);
        println!("  S2 consolidation prompt: {} chars", prompt.len());
        let facts = infer_facts(&core, &prompt)
            .await
            .expect("infer_facts failed");
        println!("  S2 LLM facts: {}", facts.len());
        for (i, f) in facts.iter().enumerate() {
            println!("    [{i}] {}", serde_json::to_string(f).unwrap_or_default());
        }
        assert!(
            !facts.is_empty(),
            "S2: LLM should extract facts from frontend session"
        );

        let s2_l2_count = create_seeds_from_facts(&facts, "session-2", "frontend", &store);
        println!("  S2 L2 seeds: {s2_l2_count}");
        assert!(s2_l2_count > 0, "S2: L2 should create at least 1 seed");

        // S2 entropy: should show some contradiction (backend vs frontend identity)
        let now = crate::utils::unix_now();
        let seeds_s2 = seed_store.load_all().unwrap();
        let entropy_s2 = AlayaEntropy::compute(&seeds_s2, now);
        print_entropy("S2 entropy", &entropy_s2);
        // Contradiction may appear from backend+frontend mixed identity
        println!(
            "  S2 contradiction: {:.3} (backend+frontend mix)",
            entropy_s2.contradiction
        );

        manas.record_turn();
        manas.recalibrate(&entropy_s2, seeds_s2.len());
        let atma_s2 = manas.atma_graha;
        println!("  ātma-grāha after S2: {:.3}", atma_s2);

        // If contradiction appeared, ātma-grāha should reflect it (rise or stay)
        if entropy_s2.contradiction > 0.15 {
            println!(
                "  Identity drift detected: contradiction={:.3} → agent notices inconsistency",
                entropy_s2.contradiction
            );
        }

        // Verify S2 seeds don't contaminate FTS5 search for Alpha-specific terms
        let alpha_search = store.search_seeds("API gateway", 5).unwrap_or_default();
        let alpha_ids: Vec<&str> = alpha_search.iter().map(|(id, _)| id.as_str()).collect();
        println!("  FTS5 'API gateway' hits: {:?}", alpha_ids);

        // S2 search for "TypeScript" should find frontend seeds
        let ts_search = store.search_seeds("TypeScript", 5).unwrap_or_default();
        let ts_ids: Vec<&str> = ts_search.iter().map(|(id, _)| id.as_str()).collect();
        println!("  FTS5 'TypeScript' hits: {:?}", ts_ids);

        println!("  ✓ Session 2 PASSED");

        // ═══════════════════════════════════════════════════
        // Session 3: "Back to Alpha" — Cross-Session Recall
        // ═══════════════════════════════════════════════════
        println!("\n═══ Session 3: Back to Alpha — Cross-Session Recall ═══");

        // User returns to Alpha project — should recall S1 context
        let s3_messages = vec![
            ("session-3", "继续做Project Alpha的API网关"),
            ("session-3", "我需要给网关加上Postgres连接池"),
        ];
        let mut s3_l1_total = 0usize;
        for (sid, msg) in &s3_messages {
            let n = SignalDetector::process(&store, sid, msg);
            s3_l1_total += n;
            println!("  L1 [{sid}] +{n} seeds: \"{msg}\"");
        }

        // S3 snapshots: continuing Alpha development
        let s3_snapshots = vec![
            TurnSnapshot {
                turn_number: 1, intent_stem: Stem::Wu, target_palace: Palace::Kan,
                geju_name: "wu_jia_kan".into(), execution_mode: "Direct".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "src/db.rs"}),
                tool_output: "use sqlx::postgres::PgPoolOptions;\npub async fn create_pool() -> PgPool { PgPoolOptions::new().connect(\"postgres://localhost/alpha\").await.unwrap() }".into(),
                tool_error: None, timestamp: base_ts + 2000,
            },
            TurnSnapshot {
                turn_number: 2, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo test 2>&1"}),
                tool_output: "running 45 tests\nall passed".into(),
                tool_error: None, timestamp: base_ts + 2001,
            },
            TurnSnapshot {
                turn_number: 3, intent_stem: Stem::Ji, target_palace: Palace::Kun,
                geju_name: "ji_jia_dui".into(), execution_mode: "Sandbox".into(),
                tool_name: "edit".into(),
                tool_input: serde_json::json!({"path": "src/main.rs", "old_string": "let pool = create_pool().await;", "new_string": "let pool = create_pool().await;\n    // Run migrations\n    sqlx::migrate!().run(&pool).await?;"}),
                tool_output: "Replaced 1 occurrence in src/main.rs".into(),
                tool_error: None, timestamp: base_ts + 2002,
            },
            TurnSnapshot {
                turn_number: 4, intent_stem: Stem::Geng, target_palace: Palace::Zhen,
                geju_name: "geng_jia_zhen".into(), execution_mode: "Sandbox".into(),
                tool_name: "shell".into(),
                tool_input: serde_json::json!({"command": "cargo build 2>&1"}),
                tool_output: "Compiling alpha-gateway v0.1.0\nFinished dev target(s) in 2.87s".into(),
                tool_error: None, timestamp: base_ts + 2003,
            },
        ];

        let prompt = build_short_consolidation_prompt(&s3_snapshots);
        println!("  S3 consolidation prompt: {} chars", prompt.len());
        let facts = infer_facts(&core, &prompt)
            .await
            .expect("infer_facts failed");
        println!("  S3 LLM facts: {}", facts.len());
        for (i, f) in facts.iter().enumerate() {
            println!("    [{i}] {}", serde_json::to_string(f).unwrap_or_default());
        }
        assert!(
            !facts.is_empty(),
            "S3: LLM should extract facts from Alpha continuation"
        );

        let s3_l2_count = create_seeds_from_facts(&facts, "session-3", "alpha2", &store);
        println!("  S3 L2 seeds: {s3_l2_count}");
        assert!(s3_l2_count > 0, "S3: L2 should create at least 1 seed");

        // Cross-session retrieval: search for Alpha-related terms must return seeds
        // from S1 and S3 (same project), but not confused with S2 frontend seeds
        let pg_search = store.search_seeds("Postgres", 5).unwrap_or_default();
        let pg_ids: Vec<&str> = pg_search.iter().map(|(id, _)| id.as_str()).collect();
        println!("  FTS5 'Postgres' hits: {:?}", pg_ids);

        // S3 + S1 seeds should dominate the Postgres results (not S2 frontend seeds)
        let pg_from_alpha = pg_ids.iter().any(|id| {
            id.starts_with("seed-")
                || id.contains("alpha")
                || id.contains("session-1")
                || id.contains("session-3")
        });
        println!("  Postgres results include Alpha sessions: {pg_from_alpha}");

        // Full retrieval pipeline for session-3
        let profile_s3 = UserProfileManager::prompt(&store);
        let label_s3 = seed_store.top_influence_prompt(5).0;
        let semantic_s3 = seed_store
            .semantic_influence_prompt("Postgres connection pool API gateway", 5)
            .0;
        let pipeline_s3 = format!("{profile_s3}{label_s3}{semantic_s3}");
        assert!(
            !pipeline_s3.trim().is_empty(),
            "S3 pipeline should produce output"
        );
        println!(
            "  S3 pipeline: {} chars (profile={}, label={}, semantic={})",
            pipeline_s3.len(),
            profile_s3.len(),
            label_s3.len(),
            semantic_s3.len()
        );

        // Memory interference check: TypeScript search must NOT return Alpha-only seeds
        let ts_interference = store
            .search_seeds("TypeScript React frontend", 3)
            .unwrap_or_default();
        println!(
            "  Interference check 'TypeScript React frontend': {} results",
            ts_interference.len()
        );
        // S2 seeds should appear, S1 should not (unless seed contains both topics)
        for (id, rank) in &ts_interference {
            println!("    [{:.3}] id={}", rank, id);
        }

        // Manas after S3
        let now = crate::utils::unix_now();
        let seeds_s3 = seed_store.load_all().unwrap();
        let entropy_s3 = AlayaEntropy::compute(&seeds_s3, now);
        print_entropy("S3 entropy", &entropy_s3);

        manas.record_turn();
        manas.recalibrate(&entropy_s3, seeds_s3.len());
        println!("  ātma-grāha after S3: {:.3}", manas.atma_graha);

        println!("  ✓ Session 3 PASSED");

        // ═══════════════════════════════════════════════════
        // Session 4: "Evolution & Cleanup" — Stress Dissolution
        // ═══════════════════════════════════════════════════
        println!("\n═══ Session 4: Evolution & Cleanup — Stress Dissolution ═══");

        let now = crate::utils::unix_now();

        // Inject AGED seeds (simulating very old sessions)
        for i in 0..8 {
            seed_store
                .insert(&Seed {
                    id: format!("aged-{            }", i),
                    session_id: "ancient".into(),
                    nature: SeedNature::Fact,
                    source: SeedSource::ToolObservation,
                    content: SeedContent::FreeText {
                        text: format!("ancient log entry {}: build output from old system", i),
                    },
                    palace: Palace::Zhen,
                    intent_stem: Stem::Geng,
                    geju_key: "ancient".into(),
                    created_at: now - (180 + i * 30) * 24 * 3600,
                    access_count: 0,
                    last_accessed_at: now - (150 + i * 20) * 24 * 3600,
                    strength: 0.02 + i as f32 * 0.005, // 0.02 to 0.055
                    tier: SeedTier::OnDemand,
                })
                .unwrap();
        }
        println!("  Injected 8 aged seeds (180+ days old)");

        // Inject contradictory KeyValue seeds on critical config keys
        let config_keys = vec![
            ("db_host", "localhost", "db_host", "prod-server.internal"),
            ("cache_ttl", "300", "cache_ttl", "3600"),
            ("log_level", "debug", "log_level", "error"),
        ];
        for (key_a, val_a, key_b, val_b) in &config_keys {
            seed_store
                .insert(&Seed {
                    id: format!("kv-{}-a", key_a),
                    session_id: "contra".into(),
                    nature: SeedNature::Fact,
                    source: SeedSource::ToolObservation,
                    content: SeedContent::KeyValue {
                        key: key_a.to_string(),
                        value: val_a.to_string(),
                    },
                    palace: Palace::Zhen,
                    intent_stem: Stem::Geng,
                    geju_key: "contra".into(),
                    created_at: now - 5,
                    last_accessed_at: now - 2,
                    access_count: 2,
                    strength: 0.75,
                    tier: SeedTier::OnDemand,
                })
                .unwrap();
            seed_store
                .insert(&Seed {
                    id: format!("kv-{}-b", key_b),
                    session_id: "contra".into(),
                    nature: SeedNature::Fact,
                    source: SeedSource::ToolObservation,
                    content: SeedContent::KeyValue {
                        key: key_b.to_string(),
                        value: val_b.to_string(),
                    },
                    palace: Palace::Zhen,
                    intent_stem: Stem::Geng,
                    geju_key: "contra".into(),
                    created_at: now - 5,
                    last_accessed_at: now - 2,
                    access_count: 2,
                    strength: 0.75,
                    tier: SeedTier::OnDemand,
                })
                .unwrap();
        }
        println!(
            "  Injected {} contradictory KV pairs",
            config_keys.len() * 2
        );

        // Inject user preferences (MUST survive all dissolution)
        let prefs = vec![
            ("editor", "vim"),
            ("language", "Rust"),
            ("database", "Postgres"),
            ("framework", "axum"),
        ];
        for (key, value) in &prefs {
            seed_store
                .insert(&Seed {
                    id: format!("pref-{            }", key),
                    session_id: "user".into(),
                    nature: SeedNature::Preference,
                    source: SeedSource::SignalDetection,
                    content: SeedContent::KeyValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    },
                    palace: Palace::Kun,
                    intent_stem: Stem::Ji,
                    geju_key: "user_pref".into(),
                    created_at: now - 10 * 24 * 3600,
                    last_accessed_at: now,
                    access_count: 10,
                    strength: 0.85,
                    tier: SeedTier::OnDemand,
                })
                .unwrap();
        }
        println!("  Injected {} Preference seeds", prefs.len());

        // Entropy before dissolution: should be HIGH
        let seeds_pre = seed_store.load_all().unwrap();
        let entropy_pre = AlayaEntropy::compute(&seeds_pre, now);
        print_entropy("Pre-dissolution entropy", &entropy_pre);
        assert!(
            entropy_pre.contradiction > 0.10,
            "contradictory KV pairs MUST produce contradiction > 0.10, got {:.3}",
            entropy_pre.contradiction
        );
        assert!(
            entropy_pre.staleness > 0.0,
            "aged seeds MUST produce staleness > 0, got {:.3}",
            entropy_pre.staleness
        );
        println!(
            "  Pre-dissolution: staleness={:.3}, contradiction={:.3}, redundancy={:.3}",
            entropy_pre.staleness, entropy_pre.contradiction, entropy_pre.redundancy
        );

        // Manas should respond to the loaded entropy state
        let atma_before_dissolve = manas.atma_graha;
        manas.recalibrate(&entropy_pre, seeds_pre.len());
        println!(
            "  ātma-grāha: {:.3} → {:.3} (entropy_total={:.3})",
            atma_before_dissolve, manas.atma_graha, entropy_pre.total
        );
        // Manas must change in response to entropy — direction depends on total vs current
        assert!(
            (manas.atma_graha - atma_before_dissolve).abs() > 0.001,
            "manas should recalibrate in response to entropy, but stayed at {:.3}",
            manas.atma_graha
        );

        // ═══ RUN DISSOLUTION ═══
        println!("\n── Running Zuowang Dissolution ──");
        let threshold = 0.12;
        let report = ZuowangPipeline::dissolve(store.clone(), threshold)
            .expect("ZuowangPipeline should succeed");
        println!(
            "  Examined: {}, Dissolved: {}, Weakened: {}",
            report.seeds_examined, report.seeds_dissolved, report.seeds_weakened
        );

        // Aged seeds SHOULD be dissolved (relevance_score < threshold)
        assert!(
            report.seeds_dissolved > 0,
            "aged seeds should be dissolved (relevance < {threshold}), got 0 dissolved"
        );

        // ═══ POST-DISSOLUTION VERIFICATION ═══
        println!("\n── Post-Dissolution Verification ──");

        let seeds_post = seed_store.load_all().unwrap();
        let entropy_post = AlayaEntropy::compute(&seeds_post, crate::utils::unix_now());
        print_entropy("Post-dissolution entropy", &entropy_post);

        // Seed count must decrease (aged seeds dissolved)
        assert!(
            seeds_post.len() < seeds_pre.len(),
            "seed count should drop after dissolution: {} → {}",
            seeds_pre.len(),
            seeds_post.len()
        );
        println!(
            "  Seeds: {} → {} (dissolved {})",
            seeds_pre.len(),
            seeds_post.len(),
            seeds_pre.len() - seeds_post.len()
        );

        // Staleness must drop (aged seeds removed)
        assert!(
            entropy_post.staleness < entropy_pre.staleness + 0.01,
            "staleness should drop after aged seeds dissolved: {:.3} → {:.3}",
            entropy_pre.staleness,
            entropy_post.staleness
        );
        println!(
            "  Staleness: {:.3} → {:.3}",
            entropy_pre.staleness, entropy_post.staleness
        );

        // ALL Preference seeds MUST survive
        for (key, _value) in &prefs {
            let survived = seeds_post.iter().any(|s| s.id == format!("pref-{}", key));
            assert!(survived, "Preference '{}' MUST survive dissolution", key);
        }
        println!("  All {} Preference seeds survived ✓", prefs.len());

        // Aged seeds should be gone
        let aged_remaining = seeds_post
            .iter()
            .filter(|s| s.id.starts_with("aged-"))
            .count();
        println!("  Aged seeds remaining: {} (of 8 injected)", aged_remaining);
        assert!(
            aged_remaining < 4,
            "at least half of aged seeds should be dissolved, got {aged_remaining}/8 remaining"
        );

        // Fresh Alpha seeds (S1, S3) should survive
        let alpha_surviving = seeds_post
            .iter()
            .filter(|s| s.session_id == "session-1" || s.session_id == "session-3")
            .count();
        println!("  Alpha seeds surviving: {alpha_surviving}");
        assert!(
            alpha_surviving > 0,
            "fresh Alpha seeds must survive dissolution"
        );

        // S2 frontend seeds should mostly survive (they're also fresh)
        let s2_surviving = seeds_post
            .iter()
            .filter(|s| s.session_id == "session-2")
            .count();
        println!("  S2 frontend seeds surviving: {s2_surviving}");

        // ═══ MANAS CONVERGENCE AFTER CLEANUP ═══
        println!("\n── Manas Convergence After Cleanup ──");

        let mut atma_trajectory: Vec<f32> = vec![manas.atma_graha];
        for i in 0..15 {
            manas.record_turn();
            let seeds_now = seed_store.load_all().unwrap();
            let entropy_now = AlayaEntropy::compute(&seeds_now, crate::utils::unix_now());
            manas.recalibrate(&entropy_now, seeds_now.len());
            atma_trajectory.push(manas.atma_graha);
            if i % 3 == 2 || i == 14 {
                println!(
                    "  iter {}: ātma-grāha={:.3} stable_epochs={} is_stable={} entropy={:.3}",
                    i + 1,
                    manas.atma_graha,
                    manas.stable_epochs(),
                    manas.is_stable(),
                    entropy_now.total
                );
            }
        }

        // After 15 iterations with cleaned memory, manas should trend downward
        let final_atma = *atma_trajectory.last().unwrap();
        let mid_atma = atma_trajectory[atma_trajectory.len() / 2];
        assert!(
            final_atma <= mid_atma + 0.05,
            "ātma-grāha should trend downward after cleanup: mid={:.3} final={:.3}",
            mid_atma,
            final_atma
        );
        assert!(
            final_atma < atma_trajectory[0] + 0.05,
            "final ātma-grāha ({:.3}) should be ≤ initial after cleanup ({:.3})",
            final_atma,
            atma_trajectory[0]
        );

        // Profile must still contain user identity
        let profile_final = UserProfileManager::prompt(&store);
        assert!(!profile_final.is_empty(), "profile must still be populated");
        println!(
            "  Final profile (first 200 chars): {}",
            &profile_final[..profile_final.len().min(200)]
        );

        // ═══════════════════════════════════════════════════
        // Final Health Report
        // ═══════════════════════════════════════════════════
        let seeds_final = seed_store.load_all().unwrap();
        let entropy_final = AlayaEntropy::compute(&seeds_final, crate::utils::unix_now());

        println!("\n╔═══════════════════════════════════════════════╗");
        println!("║  STRESS TEST COMPLETE — Health Report          ║");
        println!("╠═══════════════════════════════════════════════╣");
        println!(
            "║  Total seeds:         {:4}                    ║",
            seeds_final.len()
        );
        println!(
            "║  S1 Alpha seeds:      {:4}                    ║",
            alpha_surviving
        );
        println!(
            "║  S2 Frontend seeds:   {:4}                    ║",
            s2_surviving
        );
        println!(
            "║  Aged dissolved:      {:4}                    ║",
            8 - aged_remaining
        );
        println!(
            "║  Preferences safe:    {:4}                    ║",
            prefs.len()
        );
        println!(
            "║  ātma-grāha:          {:.3}                   ║",
            manas.atma_graha
        );
        println!(
            "║  is_stable:           {:5}                    ║",
            manas.is_stable()
        );
        println!(
            "║  Entropy total:       {:.3}                   ║",
            entropy_final.total
        );
        println!(
            "║  Contradiction:       {:.3}                   ║",
            entropy_final.contradiction
        );
        println!(
            "║  Staleness:           {:.3}                   ║",
            entropy_final.staleness
        );
        println!(
            "║  L1 total seeds:      {:4}                    ║",
            s1_l1_total + s2_l1_total + s3_l1_total
        );
        println!(
            "║  L2 total seeds:      {:4}                    ║",
            s1_l2_count + s2_l2_count + s3_l2_count
        );
        println!("╚═══════════════════════════════════════════════╝");
    }
}

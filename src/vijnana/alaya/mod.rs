use crate::error::JiaError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::palaces::Palace;
use crate::palaces::gen_store::Store;
use crate::stems::Stem;

// ── SeedTier ─────────────────────────────────────────────

/// 种子注入策略——决定系统何时将该种子暴露给 LLM。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum SeedTier {
    /// 每轮注入系统提示或工作记忆（如用户名、当前项目名）。
    Always,
    /// 在 memory_catalog 中列出"存在性提示"，LLM 需要时主动检索。
    /// 新建种子的默认值。
    #[default]
    OnDemand,
    /// 不主动提示，仅 FTS5 / 图遍历可搜。
    Archive,
}

// ── Seed ─────────────────────────────────────────────────

/// 种子 — A single memory seed in the Alaya Store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seed {
    pub id: String,
    pub session_id: String,
    pub nature: SeedNature,
    pub source: SeedSource,
    pub content: SeedContent,
    pub palace: Palace,
    pub intent_stem: Stem,
    pub geju_key: String,
    pub created_at: i64,
    pub access_count: u32,
    pub last_accessed_at: i64,
    pub strength: f32,
    #[serde(default)] // 现有种子 JSON 无 "tier" → 回退到 SeedTier::default() = OnDemand
    pub tier: SeedTier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SeedNature {
    Fact,
    Inference,
    Preference,
    Procedure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SeedSource {
    UserStatement,
    ToolObservation,
    Consolidation,
    SystemInferred,
    /// L1 per-message signal detection (zero LLM, regex/keyword).
    SignalDetection,
    /// Planted from ren_soul.md — the agent's root character seed.
    /// Protected from Zuowang dissolution and tier budget eviction.
    RenSoul,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SeedContent {
    KeyValue {
        key: String,
        value: String,
    },
    Triple {
        subject: String,
        predicate: String,
        object: String,
    },
    FreeText {
        text: String,
    },
}

impl Seed {
    pub fn new(
        session_id: String,
        nature: SeedNature,
        source: SeedSource,
        content: SeedContent,
        palace: Palace,
        intent_stem: Stem,
        geju_key: String,
    ) -> Self {
        let now = crate::utils::unix_now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id,
            nature,
            source,
            content,
            palace,
            intent_stem,
            geju_key,
            created_at: now,
            access_count: 0,
            last_accessed_at: now,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        }
    }

    /// Compute a relevance score for this seed (0.0–1.0).
    pub fn relevance_score(&self, now: i64) -> f32 {
        let age_secs = (now - self.last_accessed_at.max(self.created_at)).max(0) as f32;
        let age_hours = age_secs / 3600.0;
        let recency = 1.0 / (1.0 + age_hours / 24.0);
        let access_bonus = (self.access_count as f32 * 0.05).min(0.3);
        (self.strength * 0.5 + recency * 0.3 + access_bonus).min(1.0)
    }
}

// ── SeedStore ────────────────────────────────────────────

/// Typed Seed CRUD wrapper around the raw Store.
///
/// Keeps the dependency direction clean: memory → palaces, not circular.
pub struct SeedStore {
    store: Arc<Store>,
}

impl SeedStore {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub fn insert(&self, seed: &Seed) -> Result<(), JiaError> {
        let json = serde_json::to_string(seed)?;
        Ok(self.store.insert_seed(&json)?)
    }

    pub fn load_by_session(&self, session_id: &str) -> Result<Vec<Seed>, JiaError> {
        let jsons = self
            .store
            .load_seeds_by_session(session_id)
            ?;
        jsons
            .into_iter()
            .map(|j| serde_json::from_str(&j).map_err(JiaError::from))
            .collect()
    }

    /// Load ALL seeds (agent-wide, no session_id filter).
    pub fn load_all(&self) -> Result<Vec<Seed>, JiaError> {
        let jsons = self.store.load_all_seeds()?;
        jsons
            .into_iter()
            .map(|j| serde_json::from_str(&j).map_err(JiaError::from))
            .collect()
    }

    /// Load top N seeds by strength, no palace/stem filter.
    fn load_top(&self, limit: usize) -> Result<Vec<Seed>, JiaError> {
        let jsons = self
            .store
            .load_top_seeds(limit)
            ?;
        jsons
            .into_iter()
            .map(|j| serde_json::from_str(&j).map_err(JiaError::from))
            .collect()
    }

    /// Format top seeds as a compact prompt injection (no palace/stem filter).
    /// Returns (prompt_text, touched_seed_ids).
    pub fn top_influence_prompt(&self, limit: usize) -> (String, Vec<String>) {
        let seeds = match self.load_top(limit) {
            Ok(s) => s,
            Err(_) => return (String::new(), Vec::new()),
        };
        if seeds.is_empty() {
            return (String::new(), Vec::new());
        }
        let ids: Vec<String> = seeds.iter().map(|s| s.id.clone()).collect();
        let mut lines = vec![
            String::new(),
            "## Relevant past experience (from seed memory):".into(),
        ];
        for seed in &seeds {
            let content = match &seed.content {
                SeedContent::FreeText { text } => text.chars().take(120).collect::<String>(),
                SeedContent::KeyValue { key, value } => {
                    format!("{key}: {value}")
                }
                SeedContent::Triple {
                    subject,
                    predicate,
                    object,
                } => {
                    format!("{subject} {predicate} {object}")
                }
            };
            lines.push(format!("- {content}"));
        }
        (lines.join("\n"), ids)
    }

    /// Semantic search via FTS5. Runs in parallel with label-based search.
    ///
    /// Returns (prompt_text, touched_seed_ids).
    pub fn semantic_influence_prompt(&self, query: &str, limit: usize) -> (String, Vec<String>) {
        let results = match self.store.search_seeds(query, limit) {
            Ok(r) => r,
            Err(_) => return (String::new(), Vec::new()),
        };
        if results.is_empty() {
            return (String::new(), Vec::new());
        }

        let seeds: Vec<Seed> = results
            .iter()
            .filter_map(|(json, _)| serde_json::from_str(json).ok())
            .collect();
        if seeds.is_empty() {
            return (String::new(), Vec::new());
        }

        let ids: Vec<String> = seeds.iter().map(|s| s.id.clone()).collect();

        let mut lines = vec![
            String::new(),
            "## Related past experience (semantic search):".into(),
        ];
        for seed in &seeds {
            let content = match &seed.content {
                SeedContent::FreeText { text } => text.chars().take(120).collect::<String>(),
                SeedContent::KeyValue { key, value } => {
                    format!("{key}: {value}")
                }
                SeedContent::Triple {
                    subject,
                    predicate,
                    object,
                } => {
                    format!("{subject} {predicate} {object}")
                }
            };
            lines.push(format!("- {content}"));
        }
        (lines.join("\n"), ids)
    }

    /// 生成极短的存在性索引文本 + 被全文列出的 Always 种子 ID。
    /// 文本不超过 10 行。Always 种子 ID 用于 touch（全文已传输给 LLM）。
    /// 批量更新访问计数 + 全局分层升级。
    pub fn touch_batch(&self, seed_ids: &[String]) {
        if let Err(e) = self.store.touch_batch(seed_ids) {
            tracing::warn!("touch_batch failed: {e}");
        }
    }

    pub fn memory_catalog(&self) -> (String, Vec<String>) {
        // SQL aggregation — O(1) regardless of total seed count.
        let stats = match self.store.catalog_stats() {
            Ok(s) => s,
            Err(_) => return (String::new(), Vec::new()),
        };

        // Aggregate counts from (tier, nature, count) rows
        let mut fact_count = 0usize;
        let mut pref_count = 0usize;
        let mut proc_count = 0usize;
        let mut inference_count = 0usize;
        let mut ondemand_total = 0usize;
        let mut archive_count = 0usize;

        for (tier, nature, cnt) in &stats {
            match (tier.as_str(), nature.as_str()) {
                ("OnDemand", "Fact") => fact_count += cnt,
                ("OnDemand", "Preference") => pref_count += cnt,
                ("OnDemand", "Procedure") => proc_count += cnt,
                ("OnDemand", "Inference") => inference_count += cnt,
                ("OnDemand", _) => {} // future nature variants
                ("Archive", _) => archive_count += cnt,
                _ => {} // Always handled below via load_always_seeds
            }
            if tier == "OnDemand" {
                ondemand_total += cnt;
            }
        }

        // Always: load full content for inline display (≤10 seeds)
        let always_seeds = self.store.load_always_seeds().unwrap_or_default();
        let always: Vec<Seed> = always_seeds
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        let mut lines: Vec<String> = Vec::new();
        lines.push("\n[Memory]".into());

        // Always: full content inline (expected ≤5)
        if !always.is_empty() {
            let always_parts: Vec<String> = always.iter().map(seed_content_short).collect();
            lines.push(format!("Always: {}", always_parts.join(", ")));
        }

        // OnDemand: grouped by nature (from SQL stats)
        if ondemand_total > 0 {
            let mut parts: Vec<String> = Vec::new();
            if fact_count > 0 {
                parts.push(format!("{} facts", fact_count));
            }
            if pref_count > 0 {
                parts.push(format!("{} preferences", pref_count));
            }
            if proc_count > 0 {
                parts.push(format!("{} procedures", proc_count));
            }
            if inference_count > 0 {
                parts.push(format!("{} inferences", inference_count));
            }
            lines.push(format!("OnDemand: {}", parts.join(", ")));
        }

        // Archive: count only
        if archive_count > 0 {
            lines.push(format!("Archive: {} archived", archive_count));
        }

        // CTA hint: how to retrieve details
        if ondemand_total > 0 || archive_count > 0 {
            lines.push("(Retrieve details with: namarupa query <keyword>)".into());
        }

        let always_ids: Vec<String> = always.iter().map(|s| s.id.clone()).collect();
        (lines.join("\n"), always_ids)
    }

    /// FTS5 search for seeds with text similar to `query`.
    /// Returns up to `limit` content texts of the most similar seeds found.
    pub fn search_similar_texts(&self, query: &str, limit: usize) -> Result<Vec<String>, JiaError> {
        let results = self
            .store
            .search_seeds(query, limit)
            ?;
        Ok(results
            .into_iter()
            .filter_map(|(json, _rank)| {
                serde_json::from_str::<Seed>(&json)
                    .ok()
                    .map(|seed| seed_text_for_dedup(&seed))
            })
            .collect())
    }
}

// ── Catalog helpers ──────────────────────────────────────

/// Extract normalized content text from a Seed for LLM dedup comparison.
fn seed_text_for_dedup(seed: &Seed) -> String {
    match &seed.content {
        SeedContent::FreeText { text } => text.clone(),
        SeedContent::KeyValue { key, value } => format!("{key}: {value}"),
        SeedContent::Triple {
            subject,
            predicate,
            object,
        } => {
            format!("{subject} {predicate} {object}")
        }
    }
}

/// Short human-readable content for Always-tier inline display.
fn seed_content_short(seed: &Seed) -> String {
    match &seed.content {
        SeedContent::KeyValue { key, value } => format!("{key}={value}"),
        SeedContent::Triple {
            subject,
            predicate,
            object,
        } => {
            format!("{subject} {predicate} {object}")
        }
        SeedContent::FreeText { text } => {
            text.chars().take(60).collect::<String>().replace('\n', " ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_seed_store() -> SeedStore {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));
        SeedStore::new(store)
    }

    fn insert_seed(
        ss: &SeedStore,
        id: &str,
        palace: Palace,
        stem: Stem,
        geju_key: &str,
        content: SeedContent,
    ) {
        let seed = Seed {
            id: id.into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content,
            palace,
            intent_stem: stem,
            geju_key: geju_key.into(),
            created_at: crate::utils::unix_now(),
            access_count: 0,
            last_accessed_at: crate::utils::unix_now(),
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        ss.insert(&seed).unwrap();
    }

    #[test]
    fn top_influence_empty_store() {
        let ss = temp_seed_store();
        let prompt = ss.top_influence_prompt(5).0;
        assert!(prompt.is_empty(), "expected empty, got: {prompt:?}");
    }

    #[test]
    fn top_influence_formats_seeds() {
        let ss = temp_seed_store();
        insert_seed(
            &ss,
            "s1",
            Palace::Zhen,
            Stem::Geng,
            "geju_a",
            SeedContent::FreeText {
                text: "found a bug in auth".into(),
            },
        );
        insert_seed(
            &ss,
            "s2",
            Palace::Zhen,
            Stem::Geng,
            "geju_b",
            SeedContent::KeyValue {
                key: "preferred_lang".into(),
                value: "rust".into(),
            },
        );

        let prompt = ss.top_influence_prompt(5).0;
        assert!(prompt.contains("## Relevant past experience (from seed memory):"));
        assert!(prompt.contains("found a bug"));
        assert!(prompt.contains("preferred_lang: rust"));
    }

    #[test]
    fn top_influence_respects_limit() {
        let ss = temp_seed_store();
        for i in 0..8 {
            insert_seed(
                &ss,
                &format!("s{i}"),
                Palace::Zhen,
                Stem::Geng,
                "g",
                SeedContent::FreeText {
                    text: format!("seed {i}"),
                },
            );
        }
        let prompt = ss.top_influence_prompt(5).0;
        let bullet_count = prompt.lines().filter(|l| l.starts_with("- ")).count();
        assert!(bullet_count <= 5, "should cap at 5, got {bullet_count}");
    }

    #[test]
    fn top_influence_formats_keyvalue() {
        let ss = temp_seed_store();
        insert_seed(
            &ss,
            "kv1",
            Palace::Zhen,
            Stem::Geng,
            "g",
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "neovim".into(),
            },
        );
        let prompt = ss.top_influence_prompt(5).0;
        assert!(prompt.contains("editor: neovim"));
    }

    #[test]
    fn top_influence_formats_triple() {
        let ss = temp_seed_store();
        insert_seed(
            &ss,
            "t1",
            Palace::Zhen,
            Stem::Geng,
            "g",
            SeedContent::Triple {
                subject: "Cargo.toml".into(),
                predicate: "depends_on".into(),
                object: "serde".into(),
            },
        );
        let prompt = ss.top_influence_prompt(5).0;
        assert!(prompt.contains("Cargo.toml depends_on serde"));
    }

    #[test]
    fn top_influence_includes_all_palaces() {
        let ss = temp_seed_store();
        // top_influence_prompt has no palace/stem filter — it should return
        // seeds regardless of which context they were stored under.
        insert_seed(
            &ss,
            "s1",
            Palace::Kan,
            Stem::Jia,
            "other",
            SeedContent::FreeText {
                text: "unrelated".into(),
            },
        );
        let prompt = ss.top_influence_prompt(5).0;
        assert!(
            prompt.contains("unrelated"),
            "top_influence should include seeds from any palace, got: {prompt:?}"
        );
    }

    // ── Seed::relevance_score tests ─────────────────

    #[test]
    fn relevance_score_fresh_strong_seed_is_high() {
        let now = crate::utils::unix_now();
        let seed = Seed {
            id: "s1".into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "g".into(),
            created_at: now,
            access_count: 10,
            last_accessed_at: now,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        let score = seed.relevance_score(now);
        // strength*0.5 + recency*0.3 + access_bonus*0.05 (min 0.3) = 0.5 + 0.3 + 0.3 = ~1.0
        assert!(
            score > 0.85,
            "fresh strong seed should be highly relevant, got {score}"
        );
        assert!(score <= 1.0, "score must not exceed 1.0, got {score}");
    }

    #[test]
    fn relevance_score_stale_weak_seed_is_low() {
        let now = crate::utils::unix_now();
        let old = now - 120 * 24 * 3600; // 120 days ago
        let seed = Seed {
            id: "s1".into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "g".into(),
            created_at: old,
            access_count: 0,
            last_accessed_at: old,
            strength: 0.1,
            tier: SeedTier::OnDemand,
        };
        let score = seed.relevance_score(now);
        // strength=0.1→0.05, recency virtually 0, access=0→0
        assert!(
            score < 0.15,
            "stale weak seed should have low relevance, got {score}"
        );
    }

    #[test]
    fn relevance_score_capped_at_one() {
        let now = crate::utils::unix_now();
        let seed = Seed {
            id: "s1".into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "g".into(),
            created_at: now,
            access_count: 100,
            last_accessed_at: now,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        let score = seed.relevance_score(now);
        // access bonus capped at 0.3, so max is 0.5 + 0.3 + 0.3 = 1.1 → min(1.0)
        assert!(score <= 1.0, "score must be capped at 1.0, got {score}");
    }

    #[test]
    fn relevance_score_decays_without_access() {
        let now = crate::utils::unix_now();
        let recent = now - 3600; // 1 hour ago
        let week_old = now - 7 * 24 * 3600; // 7 days ago
        let seed_recent = Seed {
            id: "r".into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "g".into(),
            created_at: recent,
            access_count: 0,
            last_accessed_at: recent,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        let seed_old = Seed {
            id: "o".into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "g".into(),
            created_at: week_old,
            access_count: 0,
            last_accessed_at: week_old,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        let score_recent = seed_recent.relevance_score(now);
        let score_old = seed_old.relevance_score(now);
        assert!(
            score_recent > score_old,
            "recent={score_recent} should exceed old={score_old}"
        );
    }

    // ── SeedStore round-trip tests ──────────────

    #[test]
    fn insert_and_load_all_roundtrip() {
        let ss = temp_seed_store();
        insert_seed(
            &ss,
            "s1",
            Palace::Zhen,
            Stem::Geng,
            "geju_a",
            SeedContent::KeyValue {
                key: "editor".into(),
                value: "vim".into(),
            },
        );
        insert_seed(
            &ss,
            "s2",
            Palace::Kun,
            Stem::Ji,
            "geju_b",
            SeedContent::Triple {
                subject: "tokio".into(),
                predicate: "is_a".into(),
                object: "runtime".into(),
            },
        );

        let all = ss.load_all().unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"s2"));
    }

    #[test]
    fn load_by_session_filters_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));
        let ss = SeedStore::new(store);

        let seed_a = Seed {
            id: "a".into(),
            session_id: "session_1".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText {
                text: "only in session 1".into(),
            },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "g".into(),
            created_at: crate::utils::unix_now(),
            access_count: 0,
            last_accessed_at: crate::utils::unix_now(),
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        let seed_b = Seed {
            id: "b".into(),
            session_id: "session_2".into(),
            nature: SeedNature::Preference,
            source: SeedSource::UserStatement,
            content: SeedContent::KeyValue {
                key: "lang".into(),
                value: "rust".into(),
            },
            palace: Palace::Kun,
            intent_stem: Stem::Wu,
            geju_key: "h".into(),
            created_at: crate::utils::unix_now(),
            access_count: 0,
            last_accessed_at: crate::utils::unix_now(),
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        ss.insert(&seed_a).unwrap();
        ss.insert(&seed_b).unwrap();

        let s1 = ss.load_by_session("session_1").unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].id, "a");

        let s2 = ss.load_by_session("session_2").unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].id, "b");

        let s3 = ss.load_by_session("nonexistent").unwrap();
        assert!(s3.is_empty());
    }
}

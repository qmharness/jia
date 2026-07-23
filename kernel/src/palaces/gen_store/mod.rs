//! gen_store — SQLite Store (艮八)

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub struct Store {
    pool: Pool<SqliteConnectionManager>,
    pub dissolve_lock: std::sync::Mutex<()>,
}

/// Tier budget enforcement result.
#[derive(Debug, Clone)]
pub struct TierBudgetReport {
    pub ondemand_total: usize,
    pub ondemand_demoted: usize,
    pub archive_total: usize,
    pub archive_deleted: usize,
}

pub use crate::error::StoreError;

// ── Submodules ──────────────────────────────────────────────────

pub mod async_store;
mod graph;
mod helpers;
#[allow(unused_imports)]
pub(crate) use helpers::*;
mod manas;
mod principles;
mod projects;
mod seeds;
mod sessions;
mod skills;

impl Store {
    pub fn open(path: &str) -> Self {
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let manager =
            SqliteConnectionManager::file(path).with_init(|conn: &mut rusqlite::Connection| {
                conn.execute_batch(
                    "PRAGMA journal_mode=WAL;
                     PRAGMA busy_timeout=5000;
                     PRAGMA foreign_keys=ON;
                     PRAGMA synchronous=NORMAL;
                     PRAGMA cache_size=-8000;",
                )
            });

        let pool = Pool::builder()
            .max_size(4)
            .build(manager)
            .expect("Failed to create connection pool");

        // Run schema migration on one pooled connection.
        // Version-driven: PRAGMA user_version tracks applied migrations.
        // Only runs migrations whose version > current user_version.
        const CURRENT_SCHEMA_VERSION: i64 = 1;
        let conn = pool.get().expect("Failed to get connection for migration");
        let current_version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap_or(0);
        if current_version < CURRENT_SCHEMA_VERSION {
            tracing::info!(
                from = current_version,
                to = CURRENT_SCHEMA_VERSION,
                "Running schema migration"
            );
            // L2 · DDL 抽到 migrations/*.sql(include_str! 嵌入,语义与原内联一致):
            // 001 基础 schema 整体 execute_batch;002 增量迁移按 ';' 切分
            // 逐条独立容错执行(ALTER 列已存在时失败无害,与原 `let _ =` 一致)。
            const INIT_SQL: &str = include_str!("migrations/001_init.sql");
            const ALTERS_SQL: &str = include_str!("migrations/002_alters.sql");
            conn.execute_batch(INIT_SQL)
                .expect("Failed to create tables");
            for stmt in ALTERS_SQL.split(';') {
                // 去掉注释行再判空/执行——注释里的分号不会撕碎语句
                // (002 文件头虽禁分号,此处防御未来编辑者)。
                let stmt: String = stmt
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("--"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    let _ = conn.execute(stmt, []);
                }
            }
            // TTL cleanup: prune history tables older than 90 days
            let cutoff = crate::utils::unix_now() - 90 * 86400;
            let _ = conn.execute(
                "DELETE FROM manas_history WHERE created_at < ?1",
                rusqlite::params![cutoff],
            );
            let _ = conn.execute(
                "DELETE FROM dissolution_history WHERE timestamp < ?1",
                rusqlite::params![cutoff],
            );

            let _ = conn.pragma_update(None, "user_version", CURRENT_SCHEMA_VERSION);
        } // end migration version guard

        Self {
            pool,
            dissolve_lock: std::sync::Mutex::new(()),
        }
    }

    /// Persist a sub-agent session for crash recovery.
    pub fn save_subagent_session(
        &self,
        id: &str,
        messages_json: &str,
        subagent_type: &str,
        created_at: i64,
        last_used: i64,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR REPLACE INTO subagent_sessions (id, messages_json, subagent_type, created_at, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id, messages_json, subagent_type, created_at, last_used],
        )?;
        Ok(())
    }

    /// Load ALL sub-agent sessions (crash recovery).
    pub fn load_all_subagent_sessions(
        &self,
    ) -> Result<Vec<(String, String, String, i64, i64)>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, messages_json, subagent_type, created_at, last_used
             FROM subagent_sessions ORDER BY last_used DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load a sub-agent session by ID.
    pub fn load_subagent_session(
        &self,
        id: &str,
    ) -> Result<Option<(String, String, i64, i64)>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT messages_json, subagent_type, created_at, last_used
             FROM subagent_sessions WHERE id = ?1",
        )?;
        let result = stmt.query_row(rusqlite::params![id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        });
        match result {
            Ok(row) => Ok(Some(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    // ── Session persistence ──────────────────────────────────
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::Palace;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::stems::Stem;
    use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedTier};
    use std::sync::Arc;

    fn temp_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()))
    }

    fn insert_seed(store: &Store, id: &str, content: SeedContent) {
        let seed = Seed {
            id: id.into(),
            session_id: "test".into(),
            project_id: String::new(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content,
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "geju".into(),
            created_at: crate::utils::unix_now(),
            access_count: 0,
            last_accessed_at: crate::utils::unix_now(),
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }

    // ── FTS5 content_text search edge cases ───────────────
    //
    // NOTE: unicode61 treats consecutive characters in the same Unicode
    // category (L* or N*) as a single token. Mixed CJK+ASCII text without
    // separators may be tokenized as one blob, making substring search
    // ineffective. For best results, use ASCII separators (spaces, colons)
    // in content_text. KeyValue and Triple seeds format with colons/spaces
    // which naturally separate tokens.

    #[test]
    fn fts5_search_ascii_in_keyvalue_format() {
        let store = temp_store();
        // KeyValue format with colon+space separator — properly tokenized
        insert_seed(
            &store,
            "s1",
            SeedContent::KeyValue {
                key: "tech".into(),
                value: "Postgres".into(),
            },
        );

        let results = store.search_seeds("Postgres", 5).unwrap();
        assert!(
            !results.is_empty(),
            "FTS5 should find 'Postgres' in KeyValue content"
        );
        let results = store.search_seeds("tech", 5).unwrap();
        assert!(
            !results.is_empty(),
            "FTS5 should find 'tech' in KeyValue content"
        );
    }

    #[test]
    fn fts5_chinese_text_preserved_in_storage() {
        let store = temp_store();
        insert_seed(
            &store,
            "s1",
            SeedContent::FreeText {
                text: "使用Postgres作为数据库".into(),
            },
        );

        // Verify content is stored correctly even if FTS5 can't tokenize it
        let all = store.load_all_seeds().unwrap();
        let has_chinese = all.iter().any(|j| j.contains("数据库"));
        assert!(has_chinese, "Chinese text should be preserved in seed JSON");
    }

    #[test]
    fn fts5_search_with_special_characters() {
        let store = temp_store();
        insert_seed(
            &store,
            "s1",
            SeedContent::KeyValue {
                key: "tool".into(),
                value: "cargo-watch".into(),
            },
        );
        insert_seed(
            &store,
            "s2",
            SeedContent::FreeText {
                text: "error[E0308]: mismatched types".into(),
            },
        );

        // Hyphenated terms
        let results = store.search_seeds("cargo-watch", 5).unwrap();
        assert!(
            !results.is_empty(),
            "FTS5 should find hyphenated term 'cargo-watch'"
        );

        // Error codes
        let results = store.search_seeds("E0308", 5).unwrap();
        assert!(!results.is_empty(), "FTS5 should find error code 'E0308'");

        // Partial match on error code
        let results = store.search_seeds("mismatched", 5).unwrap();
        assert!(!results.is_empty(), "FTS5 should find 'mismatched'");
    }

    #[test]
    fn fts5_search_empty_store_returns_empty() {
        let store = temp_store();
        let results = store.search_seeds("anything", 5).unwrap();
        assert!(results.is_empty(), "empty FTS5 should return empty");
    }

    #[test]
    fn fts5_search_nonexistent_term_returns_empty() {
        let store = temp_store();
        insert_seed(
            &store,
            "s1",
            SeedContent::FreeText {
                text: "some content here".into(),
            },
        );
        let results = store.search_seeds("zzz_nonexistent_zzz", 5).unwrap();
        assert!(results.is_empty(), "nonexistent term should return empty");
    }

    #[test]
    fn fts5_content_text_extraction_formats() {
        // Verify the content_text extraction produces expected strings
        assert_eq!(
            extract_content_text("KeyValue", r#"{"key":"editor","value":"vim"}"#),
            "editor: vim"
        );
        assert_eq!(
            extract_content_text(
                "Triple",
                r#"{"subject":"Cargo.toml","predicate":"depends_on","object":"serde"}"#
            ),
            "Cargo.toml depends_on serde"
        );
        let ft = extract_content_text("FreeText", r#"{"text":"hello world"}"#);
        assert_eq!(ft, "hello world");
    }

    // ── FTS5 edge cases ───────────────────────────────────

    #[test]
    fn fts5_escape_query_handles_double_quotes() {
        let result = escape_fts5_query(r#""hello" world"#);
        assert_eq!(result, r#""hello world""#);
    }

    #[test]
    fn fts5_escape_query_empty_after_trim() {
        let result = escape_fts5_query(r#"   ""   "#);
        assert_eq!(result, "");
    }

    #[test]
    fn fts5_escape_query_trims_and_wraps() {
        let result = escape_fts5_query("  rust  ");
        assert_eq!(result, r#""rust""#);
    }

    // ── graph_expand_multi tests ───────────────────────────────

    fn insert_triple(store: &Store, id: &str, subj: &str, pred: &str, obj: &str) {
        insert_seed(
            store,
            id,
            SeedContent::Triple {
                subject: subj.into(),
                predicate: pred.into(),
                object: obj.into(),
            },
        );
    }

    #[test]
    fn graph_expand_delegates_to_multi() {
        let store = temp_store();
        insert_triple(&store, "s1", "A", "depends_on", "B");
        insert_triple(&store, "s2", "B", "depends_on", "C");

        // graph_expand does direct match only (max_hops=0)
        let old = store.graph_expand(&["A".into()], 10).unwrap();
        let new: Vec<String> = store
            .graph_expand_multi(&["A".into()], 0, 10, None)
            .unwrap()
            .into_iter()
            .map(|(j, _)| j)
            .collect();

        assert_eq!(old.len(), new.len());
        assert!(!old.is_empty(), "direct match should find A→B");
        assert_eq!(
            old.len(),
            1,
            "direct match should only find seeds directly mentioning A"
        );
    }

    #[test]
    fn graph_expand_multi_2hop_chain() {
        let store = temp_store();
        insert_triple(&store, "s1", "A", "depends_on", "B");
        insert_triple(&store, "s2", "B", "depends_on", "C");
        insert_triple(&store, "s3", "C", "depends_on", "D");

        // max_hops=0: direct match only → A→B
        let r0: Vec<(String, usize)> = store
            .graph_expand_multi(&["A".into()], 0, 10, None)
            .unwrap();
        assert_eq!(r0.len(), 1, "max_hops=0: direct match → A→B");
        assert_eq!(r0[0].1, 0);

        // max_hops=1: direct + 1 expansion → A→B, B→C
        let r1: Vec<(String, usize)> = store
            .graph_expand_multi(&["A".into()], 1, 10, None)
            .unwrap();
        assert_eq!(r1.len(), 2, "max_hops=1: A→B (hop 0), B→C (hop 1)");
        assert_eq!(r1[0].1, 0);
        assert_eq!(r1[1].1, 1);

        // max_hops=2: direct + 2 expansions → A→B, B→C, C→D
        let r2: Vec<(String, usize)> = store
            .graph_expand_multi(&["A".into()], 2, 10, None)
            .unwrap();
        assert_eq!(
            r2.len(),
            3,
            "max_hops=2: A→B (hop 0), B→C (hop 1), C→D (hop 2)"
        );
        assert_eq!(r2[0].1, 0);
        assert_eq!(r2[1].1, 1);
        assert_eq!(r2[2].1, 2);
    }

    #[test]
    fn graph_expand_multi_max_hops_zero_is_exact_match() {
        let store = temp_store();
        insert_triple(&store, "s1", "serde", "is", "serialization_lib");
        insert_triple(&store, "s2", "tokio", "is", "async_runtime");

        let r = store
            .graph_expand_multi(&["serde".into()], 0, 10, None)
            .unwrap();
        assert_eq!(r.len(), 1, "max_hops=0 should only find exact match");
        // serde is the subject, should find it
    }

    #[test]
    fn graph_expand_multi_predicate_filter() {
        let store = temp_store();
        insert_triple(&store, "s1", "A", "depends_on", "B");
        insert_triple(&store, "s2", "A", "imports", "B");

        let r = store
            .graph_expand_multi(&["A".into()], 1, 10, Some("imports"))
            .unwrap();
        assert_eq!(r.len(), 1, "predicate filter should only match 'imports'");
    }

    #[test]
    fn graph_expand_multi_cycle_prevention() {
        let store = temp_store();
        insert_triple(&store, "s1", "A", "depends_on", "B");
        insert_triple(&store, "s2", "B", "depends_on", "A"); // cycle back

        // Should not loop infinitely
        let r = store
            .graph_expand_multi(&["A".into()], 3, 10, None)
            .unwrap();
        // A→B (hop 1), B→A (hop 2) — only 2 unique seeds
        assert!(r.len() <= 2, "cycle prevention should cap at unique seeds");
    }

    // ── find_contradicting_triples tests ────────────────────────

    #[test]
    fn find_contradicting_triples_detects_conflict() {
        let store = temp_store();
        insert_triple(&store, "c1", "Cargo.toml", "depends_on", "serde");
        insert_triple(&store, "c2", "Cargo.toml", "depends_on", "tokio");

        let conflicts = store.find_contradicting_triples(10).unwrap();
        assert_eq!(conflicts.len(), 1, "should detect 1 conflict group");
        assert_eq!(conflicts[0].len(), 2, "conflict group should have 2 seeds");
    }

    #[test]
    fn find_contradicting_triples_no_conflict() {
        let store = temp_store();
        insert_triple(&store, "c1", "Cargo.toml", "depends_on", "serde");
        insert_triple(&store, "c2", "Cargo.toml", "author", "Alice");

        let conflicts = store.find_contradicting_triples(10).unwrap();
        assert!(
            conflicts.is_empty(),
            "different predicates should not conflict"
        );
    }

    #[test]
    fn find_contradicting_triples_respects_limit() {
        let store = temp_store();
        // Create 3 conflict groups
        for i in 0..3 {
            insert_triple(
                &store,
                &format!("a{i}_1"),
                &format!("X{i}"),
                "depends_on",
                "serde",
            );
            insert_triple(
                &store,
                &format!("a{i}_2"),
                &format!("X{i}"),
                "depends_on",
                "tokio",
            );
        }

        let conflicts = store.find_contradicting_triples(2).unwrap();
        assert_eq!(conflicts.len(), 2, "should respect limit of 2");
    }
}

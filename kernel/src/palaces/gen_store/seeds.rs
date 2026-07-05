//! Seed CRUD: insert, load, weaken, delete, tier management, and catalog operations.

use super::helpers::*;
use super::{Store, StoreError, TierBudgetReport};

impl Store {
    pub fn insert_seed(&self, seed_json: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let v: serde_json::Value = serde_json::from_str(seed_json)?;
        let id = v["id"].as_str().unwrap_or("");
        let session_id = v["session_id"].as_str().unwrap_or("");
        let project_id = v["project_id"].as_str().unwrap_or("");
        let nature = v["nature"].as_str().unwrap_or("Fact");
        let source = v["source"].as_str().unwrap_or("SystemInferred");
        let content_type = v["content"]["type"].as_str().unwrap_or("FreeText");
        let content_json = v["content"].to_string();
        let palace = match v["palace"].as_str() {
            Some("Kan") => 0,
            Some("Kun") => 1,
            Some("Zhen") => 2,
            Some("Xun") => 3,
            Some("Zhong") => 4,
            Some("Qian") => 5,
            Some("Dui") => 6,
            Some("Gen") => 7,
            Some("Li") => 8,
            _ => 0,
        };
        let intent_stem = match v["intent_stem"].as_str() {
            Some("Jia") => 0,
            Some("Yi") => 1,
            Some("Bing") => 2,
            Some("Ding") => 3,
            Some("Wu") => 4,
            Some("Ji") => 5,
            Some("Geng") => 6,
            Some("Xin") => 7,
            Some("Ren") => 8,
            Some("Gui") => 9,
            _ => 4,
        };
        let geju_key = v["geju_key"].as_str().unwrap_or("");
        let created_at = v["created_at"].as_i64().unwrap_or(0);
        let access_count = v["access_count"].as_u64().unwrap_or(0) as i64;
        let last_accessed_at = v["last_accessed_at"].as_i64().unwrap_or(0);
        let strength = v["strength"].as_f64().unwrap_or(1.0);
        let tier = v["tier"].as_str().unwrap_or("OnDemand");

        let content_text = extract_content_text(content_type, &content_json);
        conn.execute(
            "INSERT OR IGNORE INTO seeds (id, session_id, project_id, nature, source, content_type, content_json,
             content_text, palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            rusqlite::params![id, session_id, project_id, nature, source, content_type, &content_json,
                &content_text, palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier],
        )?;
        // Keep FTS5 index in sync
        conn.execute(
            "INSERT OR REPLACE INTO seeds_fts(id, content_text) VALUES (?1, ?2)",
            rusqlite::params![id, &content_text],
        )?;
        Ok(())
    }

    pub fn load_seeds_by_session(&self, session_id: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
             FROM seeds WHERE session_id = ?1 ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok(seed_row_to_json(row))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load all seeds affiliated with a project (exact project_id match).
    /// Legacy/global seeds (project_id = '') are excluded.
    pub fn load_seeds_by_project(&self, project_id: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
             FROM seeds WHERE project_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |row| {
            Ok(seed_row_to_json(row))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn seed_count_by_session(&self, session_id: &str) -> Result<u64, StoreError> {
        let conn = self.pool.get()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM seeds WHERE session_id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    pub fn weaken_seeds(&self, ids: &[String], factor: f32) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute("BEGIN IMMEDIATE", [])?;
        for id in ids {
            if let Err(e) = conn.execute(
                "UPDATE seeds SET strength = strength * ?1 WHERE id = ?2",
                rusqlite::params![factor, id],
            ) {
                let _ = conn.execute("ROLLBACK", []);
                return Err(StoreError::Sqlite(e));
            }
        }
        conn.execute("COMMIT", [])?;
        Ok(())
    }

    pub fn delete_seeds(&self, ids: &[String]) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute("BEGIN IMMEDIATE", [])?;
        for id in ids {
            if let Err(e) = conn.execute("DELETE FROM seeds WHERE id = ?1", rusqlite::params![id]) {
                let _ = conn.execute("ROLLBACK", []);
                return Err(StoreError::Sqlite(e));
            }
            // Keep FTS5 in sync
            let _ = conn.execute("DELETE FROM seeds_fts WHERE id = ?1", rusqlite::params![id]);
        }
        conn.execute("COMMIT", [])?;
        Ok(())
    }

    /// Update tier for a batch of seeds (used for OnDemand→Archive downgrade, etc.).
    pub fn set_tier_batch(&self, ids: &[String], new_tier: &str) -> Result<(), StoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let conn = self.pool.get()?;
        conn.execute("BEGIN IMMEDIATE", [])?;
        for id in ids {
            if let Err(e) = conn.execute(
                "UPDATE seeds SET tier = ?1 WHERE id = ?2",
                rusqlite::params![new_tier, id],
            ) {
                let _ = conn.execute("ROLLBACK", []);
                return Err(StoreError::Sqlite(e));
            }
        }
        conn.execute("COMMIT", [])?;
        Ok(())
    }

    /// Batch-update access_count / last_accessed_at for touched seeds.
    /// Also runs global tier promotions (OnDemand→Always, Archive→OnDemand).
    pub fn touch_batch(&self, seed_ids: &[String]) -> Result<(), StoreError> {
        if seed_ids.is_empty() {
            return Ok(());
        }
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();

        // Deduplicate — same seed may be touched multiple times in one turn
        let deduped: Vec<&String> = {
            let mut seen = std::collections::HashSet::new();
            seed_ids.iter().filter(|id| seen.insert(*id)).collect()
        };
        if deduped.is_empty() {
            return Ok(());
        }

        // Build parameterized IN clause
        let placeholders: Vec<String> = deduped
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect();
        let in_clause = placeholders.join(", ");

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
        for id in &deduped {
            params.push(Box::new(id.to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        conn.execute("BEGIN IMMEDIATE", [])?;

        // 1. Update access_count / last_accessed_at for the batch.
        //    Active reinforcement: frequently retrieved seeds gain strength (cap 1.0).
        let update_sql = format!(
            "UPDATE seeds SET access_count = access_count + 1, last_accessed_at = ?1, strength = MIN(strength + 0.01, 1.0) WHERE id IN ({in_clause})"
        );
        if let Err(e) = conn.execute(&update_sql, param_refs.as_slice()) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(StoreError::Sqlite(e));
        }

        // 2. Global promotion: OnDemand → Always (access_count >= 8)
        if let Err(e) = conn.execute(
            "UPDATE seeds SET tier = 'Always' WHERE tier = 'OnDemand' AND access_count >= 8",
            [],
        ) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(StoreError::Sqlite(e));
        }

        // 3. Global promotion: Archive → OnDemand (access_count >= 5)
        if let Err(e) = conn.execute(
            "UPDATE seeds SET tier = 'OnDemand' WHERE tier = 'Archive' AND access_count >= 5",
            [],
        ) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(StoreError::Sqlite(e));
        }

        conn.execute("COMMIT", [])?;
        Ok(())
    }

    /// Load ALL seeds (agent-wide, no session_id filter).
    /// Used for memory recalibration, zuowang, etc. — one agent = one file.
    /// Load top N seeds by strength (descending), no palace/stem filter.
    ///
    /// When `project_bias` is Some(non-empty), same-project seeds get a small
    /// (+0.1) ranking bonus so they surface ahead of equally strong foreign
    /// seeds. Memory stays globally shared — this is a recall nudge, not a filter.
    pub fn load_top_seeds(
        &self,
        limit: usize,
        project_bias: Option<&str>,
    ) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut result = Vec::new();
        match project_bias {
            Some(pid) if !pid.is_empty() => {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, nature, source, content_type, content_json,
                     palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
                     FROM seeds
                     ORDER BY (strength + CASE WHEN project_id = ?1 THEN 0.1 ELSE 0 END) DESC,
                              strength DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![pid, limit as i64], |row| {
                    Ok(seed_row_to_json(row))
                })?;
                for row in rows {
                    result.push(row?);
                }
            }
            _ => {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, nature, source, content_type, content_json,
                     palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
                     FROM seeds
                     ORDER BY strength DESC
                     LIMIT ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
                    Ok(seed_row_to_json(row))
                })?;
                for row in rows {
                    result.push(row?);
                }
            }
        }
        Ok(result)
    }

    pub fn load_all_seeds(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
             FROM seeds ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| Ok(seed_row_to_json(row)))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load only profile seeds: Preference + KeyValue, sorted by last_accessed_at DESC.
    /// Uses composite index on (nature, content_type) for efficient filtering.
    pub fn load_profile_seeds(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
             FROM seeds
             WHERE nature = 'Preference' AND content_type = 'KeyValue'
             ORDER BY last_accessed_at DESC"
        )?;
        let rows = stmt.query_map([], |row| Ok(seed_row_to_json(row)))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn count_seeds(&self) -> Result<usize, StoreError> {
        let conn = self.pool.get()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM seeds", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Tier/nature aggregation for catalog display — single SQL GROUP BY.
    /// Returns (tier, nature, count) tuples.
    pub fn catalog_stats(&self) -> Result<Vec<(String, String, usize)>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt =
            conn.prepare("SELECT tier, nature, COUNT(*) FROM seeds GROUP BY tier, nature")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as usize,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load Always-tier seeds (full JSON, expected ≤10).
    pub fn load_always_seeds(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier, project_id
             FROM seeds WHERE tier = 'Always' ORDER BY strength DESC LIMIT 10"
        )?;
        let rows = stmt.query_map([], |row| Ok(seed_row_to_json(row)))?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Enforce tier budgets: demote excess OnDemand → Archive, delete excess Archive.
    /// Protected seeds (UserStatement/RenSoul/Handoff source OR Preference nature) are never evicted.
    /// Both operations run in a single transaction for atomicity.
    pub fn enforce_tier_budgets(&self) -> Result<TierBudgetReport, StoreError> {
        const ONDEMAND_BUDGET: usize = 200;
        const ARCHIVE_BUDGET: usize = 1000;

        let conn = self.pool.get()?;
        // RAII guard: rollback on any error to prevent transaction pollution
        let result = (|| -> Result<TierBudgetReport, StoreError> {
        conn.execute("BEGIN IMMEDIATE", [])?;

        // ── OnDemand → Archive demotion ──
        let ondemand_total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM seeds WHERE tier = 'OnDemand'",
            [],
            |row| row.get(0),
        )?;
        let ondemand_total = ondemand_total as usize;

        let mut ondemand_demoted = 0usize;
        if ondemand_total > ONDEMAND_BUDGET {
            let excess = ondemand_total - ONDEMAND_BUDGET;
            let mut stmt = conn.prepare(
                "SELECT id FROM seeds
                 WHERE tier = 'OnDemand'
                   AND source != 'UserStatement'
                   AND source != 'RenSoul'
                   AND source != 'Handoff'
                   AND nature != 'Preference'
                 ORDER BY strength ASC
                 LIMIT ?1",
            )?;
            let ids: Vec<String> = stmt
                .query_map(rusqlite::params![excess as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .filter_map(|r| r.ok())
                .collect();
            ondemand_demoted = ids.len();
            if !ids.is_empty() {
                drop(stmt);
                for id in &ids {
                    conn.execute(
                        "UPDATE seeds SET tier = 'Archive' WHERE id = ?1",
                        rusqlite::params![id],
                    )?;
                }
            }
        }

        // ── Archive → delete ──
        let archive_total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM seeds WHERE tier = 'Archive'",
            [],
            |row| row.get(0),
        )?;
        let archive_total = archive_total as usize;

        let mut archive_deleted = 0usize;
        if archive_total > ARCHIVE_BUDGET {
            let excess = archive_total - ARCHIVE_BUDGET;
            let mut stmt = conn.prepare(
                "SELECT id FROM seeds
                 WHERE tier = 'Archive'
                   AND source != 'UserStatement'
                   AND source != 'RenSoul'
                   AND source != 'Handoff'
                   AND nature != 'Preference'
                 ORDER BY strength ASC
                 LIMIT ?1",
            )?;
            let ids: Vec<String> = stmt
                .query_map(rusqlite::params![excess as i64], |row| {
                    row.get::<_, String>(0)
                })?
                .filter_map(|r| r.ok())
                .collect();
            archive_deleted = ids.len();
            if !ids.is_empty() {
                drop(stmt);
                for id in &ids {
                    conn.execute("DELETE FROM seeds WHERE id = ?1", rusqlite::params![id])?;
                    let _ =
                        conn.execute("DELETE FROM seeds_fts WHERE id = ?1", rusqlite::params![id]);
                }
            }
        }

        conn.execute("COMMIT", [])?;

        Ok(TierBudgetReport {
            ondemand_total,
            ondemand_demoted,
            archive_total,
            archive_deleted,
        })
        })(); // end RAII closure
        match result {
            Ok(report) => Ok(report),
            Err(e) => {
                let _ = conn.execute("ROLLBACK", []);
                Err(e)
            }
        }
    }

    /// Load only `source` values for specific seed IDs — avoids full table scan
    /// when callers only need to check source eligibility (e.g., delete protection).
    pub fn load_seed_sources(&self, ids: &[String]) -> Result<Vec<(String, String)>, StoreError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.pool.get()?;
        let placeholders: Vec<&str> = ids.iter().map(|_| "?").collect();
        let sql = format!(
            "SELECT id, source FROM seeds WHERE id IN ({})",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── Manas persistence ─────────────────────────────────
}

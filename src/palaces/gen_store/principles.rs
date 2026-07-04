//! System principles persistence: save/load learned safety constraints.

use super::{Store, StoreError};

impl Store {
    pub fn save_principles(
        &self,
        session_id: &str,
        principles_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let arr: Vec<serde_json::Value> = serde_json::from_str(principles_json)?;
        let now = crate::utils::unix_now();

        conn.execute("BEGIN IMMEDIATE", [])?;

        // Clear existing principles for this session+scope
        if let Err(e) = conn.execute(
            "DELETE FROM principles WHERE session_id = ?1 AND scope = 'agent'",
            rusqlite::params![session_id],
        ) {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.into());
        }

        // Insert new principles
        for p in &arr {
            let id = p["id"].as_str().unwrap_or("");
            let geju_key = p["geju_key"].as_str().unwrap_or("");
            let constraint_type = p["constraint"]["type"].as_str().unwrap_or("");
            let constraint_json = p["constraint"].to_string();
            let confidence = p["confidence"].as_f64().unwrap_or(0.5);
            let source_seed_count = p["source_seed_count"].as_u64().unwrap_or(0) as i64;

            if let Err(e) = conn.execute(
                "INSERT INTO principles (id, session_id, geju_key, scope, constraint_type,
                 constraint_json, confidence, source_seed_count, created_at)
                 VALUES (?1, ?2, ?3, 'agent', ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    id,
                    session_id,
                    geju_key,
                    constraint_type,
                    &constraint_json,
                    confidence,
                    source_seed_count,
                    now
                ],
            ) {
                let _ = conn.execute("ROLLBACK", []);
                return Err(e.into());
            }
        }

        conn.execute("COMMIT", [])?;
        Ok(())
    }

    /// Load ALL principles (agent-wide, no session_id filter).
    pub fn load_principles(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, geju_key, scope, constraint_type, constraint_json,
             confidence, source_seed_count, created_at
             FROM principles ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "session_id": row.get::<_, String>(1)?,
                "geju_key": row.get::<_, String>(2)?,
                "scope": row.get::<_, String>(3)?,
                "constraint": serde_json::from_str::<serde_json::Value>(
                    &row.get::<_, String>(5)?
                ).unwrap_or_default(),
                "confidence": row.get::<_, f64>(6)?,
                "source_seed_count": row.get::<_, i64>(7)? as u64,
            })
            .to_string())
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Archive a single principle (user-initiated, reversible).
    pub fn archive_principle(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE principles SET archived = 1 WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    /// Restore an archived principle.
    pub fn unarchive_principle(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE principles SET archived = 0 WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    /// Load active (non-archived) principles only.
    pub fn load_active_principles(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, geju_key, scope, constraint_type, constraint_json,
             confidence, source_seed_count, created_at
             FROM principles WHERE archived = 0 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "session_id": row.get::<_, String>(1)?,
                "geju_key": row.get::<_, String>(2)?,
                "scope": row.get::<_, String>(3)?,
                "constraint": serde_json::from_str::<serde_json::Value>(
                    &row.get::<_, String>(5)?
                ).unwrap_or_default(),
                "confidence": row.get::<_, f64>(6)?,
                "source_seed_count": row.get::<_, i64>(7)? as u64,
                "archived": row.get::<_, i64>(8)? != 0,
            })
            .to_string())
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load all principles including archived.
    pub fn load_all_principles(&self) -> Result<Vec<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, geju_key, scope, constraint_type, constraint_json,
             confidence, source_seed_count, created_at, archived
             FROM principles ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "session_id": row.get::<_, String>(1)?,
                "geju_key": row.get::<_, String>(2)?,
                "scope": row.get::<_, String>(3)?,
                "constraint": serde_json::from_str::<serde_json::Value>(
                    &row.get::<_, String>(5)?
                ).unwrap_or_default(),
                "confidence": row.get::<_, f64>(6)?,
                "source_seed_count": row.get::<_, i64>(7)? as u64,
                "archived": row.get::<_, i64>(9)? != 0,
            })
            .to_string())
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── Skill evolution (Phase 0) ─────────────────────────────
}

//! Skill evolution persistence: reflections and revisions CRUD.

use super::{Store, StoreError};

impl Store {
    pub fn save_skill_reflection(&self, json: &str) -> Result<(), StoreError> {
        let v: serde_json::Value = serde_json::from_str(json)?;
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR IGNORE INTO skill_reflections (id, skill_name, session_id, reflection_type,
             content_json, confidence, turn_numbers, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                v["id"].as_str().unwrap_or(""),
                v["skill_name"].as_str().unwrap_or(""),
                v["session_id"].as_str().unwrap_or(""),
                v["reflection_type"].as_str().unwrap_or(""),
                v["content_json"].as_str().unwrap_or(""),
                v["confidence"].as_f64().unwrap_or(0.5),
                serde_json::to_string(&v["turn_numbers"]).unwrap_or_default(),
                v["created_at"].as_i64().unwrap_or(0),
            ],
        )?;
        Ok(())
    }

    pub fn load_skill_reflections(
        &self,
        skill_name: &str,
        session_id: &str,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, session_id, reflection_type, content_json, confidence,
             turn_numbers, created_at
             FROM skill_reflections
             WHERE skill_name = ?1 AND session_id = ?2
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![skill_name, session_id], |row| {
            let turn_numbers_str: String = row.get(6)?;
            let turn_numbers: serde_json::Value =
                serde_json::from_str(&turn_numbers_str).unwrap_or(serde_json::Value::Array(vec![]));
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "skill_name": row.get::<_, String>(1)?,
                "session_id": row.get::<_, String>(2)?,
                "reflection_type": row.get::<_, String>(3)?,
                "content_json": row.get::<_, String>(4)?,
                "confidence": row.get::<_, f64>(5)?,
                "turn_numbers": turn_numbers,
                "created_at": row.get::<_, i64>(7)?,
            }))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn save_skill_revision(&self, json: &str) -> Result<(), StoreError> {
        let v: serde_json::Value = serde_json::from_str(json)?;
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR IGNORE INTO skill_revisions (id, skill_name, session_id, old_content,
             new_content, diff_text, avg_confidence, reflection_ids, pre_revision_error_rate,
             post_revision_error_rate, applied, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                v["id"].as_str().unwrap_or(""),
                v["skill_name"].as_str().unwrap_or(""),
                v["session_id"].as_str().unwrap_or(""),
                v["old_content"].as_str().unwrap_or(""),
                v["new_content"].as_str().unwrap_or(""),
                v["diff_text"].as_str().unwrap_or(""),
                v["avg_confidence"].as_f64().unwrap_or(0.0),
                serde_json::to_string(&v["reflection_ids"]).unwrap_or_default(),
                v["pre_revision_error_rate"].as_f64(),
                v["post_revision_error_rate"].as_f64(),
                v["applied"].as_bool().unwrap_or(false) as i32,
                v["created_at"].as_i64().unwrap_or(0),
            ],
        )?;
        Ok(())
    }

    pub fn count_revisions_this_session(
        &self,
        skill_name: &str,
        session_id: &str,
    ) -> Result<u32, StoreError> {
        let conn = self.pool.get()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM skill_revisions WHERE skill_name = ?1 AND session_id = ?2 AND applied = 1",
            rusqlite::params![skill_name, session_id],
            |row| row.get(0),
        )?;
        Ok(count as u32)
    }

    pub fn last_revision_time(&self, skill_name: &str) -> Result<Option<i64>, StoreError> {
        let conn = self.pool.get()?;
        let result: Option<i64> = conn.query_row(
            "SELECT MAX(created_at) FROM skill_revisions WHERE skill_name = ?1 AND applied = 1",
            rusqlite::params![skill_name],
            |row| row.get(0),
        )?;
        Ok(result)
    }

    pub fn backfill_post_revision_error_rate(
        &self,
        skill_name: &str,
        error_rate: f32,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE skill_revisions SET post_revision_error_rate = ?1
             WHERE skill_name = ?2 AND post_revision_error_rate IS NULL AND applied = 1
             AND id = (SELECT id FROM skill_revisions WHERE skill_name = ?2
                       AND post_revision_error_rate IS NULL AND applied = 1
                       ORDER BY created_at DESC LIMIT 1)",
            rusqlite::params![error_rate, skill_name],
        )?;
        Ok(())
    }

    /// Count total revisions across all skills.
    pub fn count_total_revisions(&self) -> Result<usize, StoreError> {
        let conn = self.pool.get()?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM skill_revisions", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Load recent revision diffs across all skills, newest first.
    pub fn load_recent_revisions(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, session_id, diff_text, avg_confidence,
                    pre_revision_error_rate, post_revision_error_rate, applied, created_at
             FROM skill_revisions
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "skill_name": row.get::<_, String>(1)?,
                "session_id": row.get::<_, String>(2)?,
                "diff_text": row.get::<_, String>(3)?,
                "avg_confidence": row.get::<_, f64>(4)?,
                "pre_revision_error_rate": row.get::<_, Option<f64>>(5)?,
                "post_revision_error_rate": row.get::<_, Option<f64>>(6)?,
                "applied": row.get::<_, i32>(7)? != 0,
                "created_at": row.get::<_, i64>(8)?,
            }))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Load aggregate reflection stats for a skill across all sessions.
    pub fn load_reflection_summary(
        &self,
        skill_name: &str,
    ) -> Result<serde_json::Value, StoreError> {
        let conn = self.pool.get()?;
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM skill_reflections WHERE skill_name = ?1",
            rusqlite::params![skill_name],
            |row| row.get(0),
        )?;
        let avg_conf: Option<f64> = conn.query_row(
            "SELECT AVG(confidence) FROM skill_reflections WHERE skill_name = ?1",
            rusqlite::params![skill_name],
            |row| row.get(0),
        )?;
        let by_type: Vec<serde_json::Value> = {
            let mut stmt = conn.prepare(
                "SELECT reflection_type, COUNT(*) as cnt, AVG(confidence) as avg_conf
                 FROM skill_reflections WHERE skill_name = ?1
                 GROUP BY reflection_type ORDER BY cnt DESC",
            )?;
            let rows = stmt.query_map(rusqlite::params![skill_name], |row| {
                Ok(serde_json::json!({
                    "reflection_type": row.get::<_, String>(0)?,
                    "count": row.get::<_, i64>(1)?,
                    "avg_confidence": row.get::<_, f64>(2)?,
                }))
            })?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row?);
            }
            v
        };
        Ok(serde_json::json!({
            "skill_name": skill_name,
            "total_reflections": total,
            "avg_confidence": avg_conf.unwrap_or(0.0),
            "by_type": by_type,
        }))
    }
}

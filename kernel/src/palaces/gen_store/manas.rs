//! Manas and dissolution persistence: save/load self-model and dissolution reports.

use super::{Store, StoreError};

impl Store {
    pub fn save_manas(&self, json: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let v: serde_json::Value = serde_json::from_str(json)?;
        let atma_graha = v["atma_graha"].as_f64().unwrap_or(0.8);
        let total_turns = v["total_turns"].as_u64().unwrap_or(0) as i64;
        let consolidation_count = v["consolidation_count"].as_u64().unwrap_or(0) as i64;
        let stable_pattern_count = v["stable_pattern_count"].as_u64().unwrap_or(0) as i64;
        let last_consolidation_at = v["last_consolidation_at"].as_i64().unwrap_or(0);
        let stable_epochs = v["stable_epochs"].as_u64().unwrap_or(0) as i64;

        conn.execute(
            "INSERT INTO manas (session_id, atma_graha, total_turns, consolidation_count,
             stable_pattern_count, last_consolidation_at, stable_epochs)
             VALUES ('agent', ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
             atma_graha = ?1, total_turns = ?2, consolidation_count = ?3,
             stable_pattern_count = ?4, last_consolidation_at = ?5, stable_epochs = ?6",
            rusqlite::params![
                atma_graha,
                total_turns,
                consolidation_count,
                stable_pattern_count,
                last_consolidation_at,
                stable_epochs
            ],
        )?;
        Ok(())
    }

    /// Load manas for the agent (keyed by "agent").
    pub fn load_manas(&self) -> Result<Option<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, atma_graha, total_turns, consolidation_count,
             stable_pattern_count, last_consolidation_at, stable_epochs
             FROM manas WHERE session_id = 'agent'",
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "atma_graha": row.get::<_, f64>(1)?,
                "total_turns": row.get::<_, i64>(2)? as u64,
                "consolidation_count": row.get::<_, i64>(3)? as u64,
                "stable_pattern_count": row.get::<_, i64>(4)? as u64,
                "last_consolidation_at": row.get::<_, i64>(5)?,
                "stable_epochs": row.get::<_, i64>(6)? as u64,
            })
            .to_string())
        })?;
        Ok(rows.next().transpose()?)
    }

    // ── Dissolution history persistence ────────────────────

    /// Persist a dissolution report for the dashboard history.
    pub fn save_dissolution_report(
        &self,
        report: &crate::zuowang::pipeline::ZuowangReport,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let entropy_json =
            serde_json::to_string(&report.entropy_dimensions).unwrap_or_else(|_| "{}".into());
        let sample_json =
            serde_json::to_string(&report.dissolved_sample).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO dissolution_history (timestamp, seeds_examined, seeds_dissolved,
             seeds_weakened, seeds_downgraded, entropy_before, entropy_after,
             entropy_dimensions_json, score_kept, score_protected, dissolved_sample_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                report.timestamp,
                report.seeds_examined as i64,
                report.seeds_dissolved as i64,
                report.seeds_weakened as i64,
                report.seeds_downgraded as i64,
                report.entropy_before as f64,
                report.entropy_after as f64,
                entropy_json,
                report.score_kept as i64,
                report.score_protected as i64,
                sample_json,
            ],
        )?;
        Ok(())
    }

    /// Load dissolution history from DB (most recent first, then reversed to chronological).
    pub fn load_dissolution_history(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::zuowang::pipeline::ZuowangReport>, StoreError> {
        use crate::zuowang::pipeline::{SeedDigest, ZuowangReport};
        use crate::zuowang::trigger::AlayaEntropy;

        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, seeds_examined, seeds_dissolved, seeds_weakened, seeds_downgraded,
             entropy_before, entropy_after, entropy_dimensions_json, score_kept, score_protected,
             dissolved_sample_json
             FROM dissolution_history ORDER BY timestamp DESC LIMIT ?",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            let entropy_json: String = row.get(7)?;
            let sample_json: String = row.get(10)?;
            Ok(ZuowangReport {
                timestamp: row.get(0)?,
                seeds_examined: row.get::<_, i64>(1)? as usize,
                seeds_dissolved: row.get::<_, i64>(2)? as usize,
                seeds_weakened: row.get::<_, i64>(3)? as usize,
                seeds_downgraded: row.get::<_, i64>(4)? as usize,
                entropy_before: row.get::<_, f64>(5)? as f32,
                entropy_after: row.get::<_, f64>(6)? as f32,
                entropy_dimensions: serde_json::from_str::<AlayaEntropy>(&entropy_json)
                    .unwrap_or_else(|e| {
                        tracing::warn!("Failed to deserialize entropy_dimensions_json: {e}");
                        AlayaEntropy {
                            staleness: 0.0,
                            contradiction: 0.0,
                            redundancy: 0.0,
                            access_decay: 0.0,
                            total: 0.0,
                        }
                    }),
                score_kept: row.get::<_, i64>(8)? as usize,
                score_protected: row.get::<_, i64>(9)? as usize,
                dissolved_sample: serde_json::from_str::<Vec<SeedDigest>>(&sample_json)
                    .unwrap_or_else(|e| {
                        tracing::warn!("Failed to deserialize dissolved_sample_json: {e}");
                        Vec::new()
                    }),
            })
        })?;
        let mut reports: Vec<ZuowangReport> = rows.filter_map(|r| r.ok()).collect();
        reports.reverse(); // chronological order
        Ok(reports)
    }

    // ── Manas history (ātma-grāha time series) ──────────────

    pub fn insert_manas_snapshot(
        &self,
        session_id: &str,
        atma_graha: f32,
        entropy_total: f32,
        seed_count: usize,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = crate::utils::unix_now();
        conn.execute(
            "INSERT INTO manas_history (id, session_id, atma_graha, entropy_total, seed_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, session_id, atma_graha, entropy_total, seed_count as i64, now],
        )?;
        Ok(())
    }

    pub fn load_manas_history(&self, limit: usize) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT atma_graha, entropy_total, seed_count, created_at
             FROM manas_history ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| {
            Ok(serde_json::json!({
                "atma_graha": row.get::<_, f64>(0)?,
                "entropy_total": row.get::<_, f64>(1)?,
                "seed_count": row.get::<_, i64>(2)?,
                "created_at": row.get::<_, i64>(3)?,
            }))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── Principles persistence ────────────────────────────────
}

//! Session persistence: CRUD operations on the sessions table.

use super::helpers::*;
use super::{Store, StoreError};
use crate::types::HistoryEntry;

impl Store {
    pub fn save_session(&self, id: &str, messages_json: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        conn.execute(
            "INSERT INTO sessions (id, messages_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET messages_json = ?2, updated_at = ?3",
            rusqlite::params![id, messages_json, now],
        )?;
        Ok(())
    }

    /// Insert a new session with title, cwd, and project_id.
    /// Uses INSERT OR IGNORE — safe to call even if session already exists.
    pub fn create_session(
        &self,
        id: &str,
        title: &str,
        cwd: &str,
        project_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, messages_json, title, cwd, project_id, updated_at) VALUES (?1, '[]', ?2, ?3, ?4, ?5)",
            rusqlite::params![id, title, cwd, project_id, now],
        )?;
        Ok(())
    }

    pub fn load_session(&self, id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT messages_json FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query_map(rusqlite::params![id], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    /// Load session history deserialized as `Vec<HistoryEntry>`.
    pub fn load_session_history(&self, id: &str) -> Vec<HistoryEntry> {
        self.load_session(id)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// Load already-distilled pair hashes for this session.
    pub fn load_distilled_hashes(&self, id: &str) -> std::collections::HashSet<u64> {
        let conn = match self.pool.get() {
            Ok(c) => c,
            Err(_) => return std::collections::HashSet::new(),
        };
        let mut stmt =
            match conn.prepare("SELECT distilled_hashes_json FROM sessions WHERE id = ?1") {
                Ok(s) => s,
                Err(_) => return std::collections::HashSet::new(),
            };
        let json: Option<String> = stmt
            .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))
            .ok()
            .and_then(|mut rows| rows.next())
            .and_then(|r| r.ok());
        match json {
            Some(j) => serde_json::from_str(&j).unwrap_or_default(),
            None => std::collections::HashSet::new(),
        }
    }

    /// Persist distilled pair hashes for this session.
    pub fn save_distilled_hashes(
        &self,
        id: &str,
        hashes: &std::collections::HashSet<u64>,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let json = serde_json::to_string(hashes)?;
        conn.execute(
            "UPDATE sessions SET distilled_hashes_json = ?1 WHERE id = ?2",
            rusqlite::params![json, id],
        )?;
        Ok(())
    }

    pub fn list_sessions_filtered(
        &self,
        filter: &str,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let sql = match filter {
            "archived" => {
                "SELECT s.id, s.messages_json, s.title, s.updated_at, s.cwd, s.archived, s.project_id, COALESCE(p.name, '') as project_name FROM sessions s LEFT JOIN projects p ON s.project_id = p.id WHERE s.archived = 1 ORDER BY s.updated_at DESC"
            }
            "active" => {
                "SELECT s.id, s.messages_json, s.title, s.updated_at, s.cwd, s.archived, s.project_id, COALESCE(p.name, '') as project_name FROM sessions s LEFT JOIN projects p ON s.project_id = p.id WHERE s.archived = 0 ORDER BY s.updated_at DESC"
            }
            _ => {
                "SELECT s.id, s.messages_json, s.title, s.updated_at, s.cwd, s.archived, s.project_id, COALESCE(p.name, '') as project_name FROM sessions s LEFT JOIN projects p ON s.project_id = p.id ORDER BY s.updated_at DESC"
            }
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let messages_json: String = row.get(1)?;
            let stored_title: Option<String> = row.get(2)?;
            let updated_at: i64 = row.get(3)?;
            let cwd: String = row.get(4)?;
            let archived: i32 = row.get(5)?;
            let project_id: Option<String> = row.get(6)?;
            let project_name: String = row.get(7)?;

            let (derived_title, message_count, has_error) = parse_session_meta(&messages_json);
            let title = stored_title
                .filter(|t| !t.is_empty())
                .or(derived_title)
                .unwrap_or_else(|| id.clone());

            Ok(serde_json::json!({
                "id": id,
                "title": title,
                "cwd": cwd,
                "projectId": project_id,
                "projectName": project_name,
                "messageCount": message_count,
                "updatedAt": updated_at,
                "archived": archived != 0,
                "hasError": has_error,
            }))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn archive_session(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sessions SET archived = 1 WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    pub fn unarchive_session(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sessions SET archived = 0 WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        // Only delete session-scoped data — seeds and manas are agent-wide now
        conn.execute(
            "DELETE FROM principles WHERE session_id = ?1 AND scope = 'session'",
            rusqlite::params![id],
        )?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    pub fn delete_sessions(&self, ids: &[String]) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        for id in ids {
            conn.execute(
                "DELETE FROM principles WHERE session_id = ?1 AND scope = 'session'",
                rusqlite::params![id],
            )?;
            conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])?;
        }
        Ok(())
    }

    pub fn rename_session(&self, id: &str, title: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        conn.execute(
            "UPDATE sessions SET title = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, title, now],
        )?;
        Ok(())
    }

    // ── Projects ──────────────────────────────────────────
}

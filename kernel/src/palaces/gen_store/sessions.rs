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

    /// Insert a new session with title, cwd, and workspace_id.
    /// Uses INSERT OR IGNORE — safe to call even if session already exists.
    /// Empty workspace_id is stored as NULL: the FK on sessions.workspace_id
    /// rejects '' (V3 smoke 发现:'' 触发 FOREIGN KEY constraint failed,
    /// 被调用方 let _ = 静默吞掉,占位行从未建成,title/cwd 全部丢失)。
    pub fn create_session(
        &self,
        id: &str,
        title: &str,
        cwd: &str,
        workspace_id: &str,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        let pid: Option<&str> = if workspace_id.is_empty() {
            None
        } else {
            Some(workspace_id)
        };
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, messages_json, title, cwd, workspace_id, updated_at) VALUES (?1, '[]', ?2, ?3, ?4, ?5)",
            rusqlite::params![id, title, cwd, pid, now],
        )?;
        Ok(())
    }

    pub fn load_session(&self, id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT messages_json FROM sessions WHERE id = ?1")?;
        let mut rows = stmt.query_map(rusqlite::params![id], |row| row.get::<_, String>(0))?;
        Ok(rows.next().transpose()?)
    }

    /// Look up the workspace_id of a session (None if session missing or unaffiliated).
    /// Used at seed-create time to stamp project provenance without threading
    /// workspace_id through the agent/engines.
    pub fn session_workspace_id(&self, session_id: &str) -> Option<String> {
        let conn = self.pool.get().ok()?;
        conn.query_row(
            "SELECT workspace_id FROM sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
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
                "SELECT s.id, s.messages_json, s.title, s.updated_at, s.cwd, s.archived, s.workspace_id, COALESCE(p.name, '') as workspace_name FROM sessions s LEFT JOIN workspaces p ON s.workspace_id = p.id WHERE s.archived = 1 ORDER BY s.updated_at DESC"
            }
            "active" => {
                "SELECT s.id, s.messages_json, s.title, s.updated_at, s.cwd, s.archived, s.workspace_id, COALESCE(p.name, '') as workspace_name FROM sessions s LEFT JOIN workspaces p ON s.workspace_id = p.id WHERE s.archived = 0 ORDER BY s.updated_at DESC"
            }
            _ => {
                "SELECT s.id, s.messages_json, s.title, s.updated_at, s.cwd, s.archived, s.workspace_id, COALESCE(p.name, '') as workspace_name FROM sessions s LEFT JOIN workspaces p ON s.workspace_id = p.id ORDER BY s.updated_at DESC"
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
            let workspace_id: Option<String> = row.get(6)?;
            let workspace_name: String = row.get(7)?;

            let (derived_title, message_count, has_error) = parse_session_meta(&messages_json);
            let title = stored_title
                .filter(|t| !t.is_empty())
                .or(derived_title)
                .unwrap_or_else(|| id.clone());

            Ok(serde_json::json!({
                "id": id,
                "title": title,
                "cwd": cwd,
                "workspaceId": workspace_id,
                "workspaceName": workspace_name,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()))
    }

    /// V3 smoke 发现:sessions.workspace_id 的 FK 拒绝 '' → create_session 静默
    /// 失败、占位行从未建成。空 workspace_id 必须存为 NULL 且插入成功。
    #[test]
    fn create_session_with_empty_workspace_id_succeeds_as_null() {
        let store = temp_store();
        store
            .create_session("sess-fk", "标题", "/tmp/ws", "")
            .expect("empty workspace_id must not violate FK");
        let title: Option<String> = store
            .pool
            .get()
            .unwrap()
            .query_row("SELECT title FROM sessions WHERE id = 'sess-fk'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title.as_deref(), Some("标题"));
        assert_eq!(store.session_workspace_id("sess-fk"), None);
    }

    #[test]
    fn create_session_with_valid_workspace_id_unchanged() {
        let store = temp_store();
        store.ensure_workspace("proj-1", "/tmp/ws", "ws").unwrap();
        store
            .create_session("sess-ok", "t", "/tmp/ws", "proj-1")
            .unwrap();
        assert_eq!(
            store.session_workspace_id("sess-ok").as_deref(),
            Some("proj-1")
        );
    }
}

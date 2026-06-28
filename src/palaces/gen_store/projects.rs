//! Project persistence: CRUD operations on the projects table.

use super::{Store, StoreError};

impl Store {
    pub fn ensure_project(
        &self,
        id: &str,
        cwd: &str,
        name: &str,
        description: &str,
        tags_json: &str,
    ) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        conn.execute(
            "INSERT INTO projects (id, cwd, name, description, tags_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET cwd = ?2, name = ?3, description = ?4, tags_json = ?5, updated_at = ?6",
            rusqlite::params![id, cwd, name, description, tags_json, now],
        )?;
        Ok(())
    }

    pub fn list_projects(
        &self,
        include_archived: bool,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let sql = if include_archived {
            "SELECT p.id, p.cwd, p.name, p.description, p.tags_json, p.archived, p.created_at, p.updated_at,
                    COUNT(s.id) as session_count
             FROM projects p
             LEFT JOIN sessions s ON s.project_id = p.id
             GROUP BY p.id
             ORDER BY p.updated_at DESC"
        } else {
            "SELECT p.id, p.cwd, p.name, p.description, p.tags_json, p.archived, p.created_at, p.updated_at,
                    COUNT(s.id) as session_count
             FROM projects p
             LEFT JOIN sessions s ON s.project_id = p.id
             WHERE p.archived = 0
             GROUP BY p.id
             ORDER BY p.updated_at DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let cwd: String = row.get(1)?;
            let name: String = row.get(2)?;
            let description: String = row.get(3)?;
            let tags_json: String = row.get(4)?;
            let archived: i32 = row.get(5)?;
            let created_at: i64 = row.get(6)?;
            let updated_at: i64 = row.get(7)?;
            let session_count: i64 = row.get(8)?;
            Ok(serde_json::json!({
                "id": id,
                "cwd": cwd,
                "name": name,
                "description": description,
                "tags": serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default(),
                "archived": archived != 0,
                "createdAt": created_at,
                "updatedAt": updated_at,
                "sessionCount": session_count,
            }))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn get_project(&self, id: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, cwd, name, description, tags_json, archived, created_at, updated_at FROM projects WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(rusqlite::params![id], |row| {
            let id: String = row.get(0)?;
            let cwd: String = row.get(1)?;
            let name: String = row.get(2)?;
            let description: String = row.get(3)?;
            let tags_json: String = row.get(4)?;
            let archived: i32 = row.get(5)?;
            let created_at: i64 = row.get(6)?;
            let updated_at: i64 = row.get(7)?;
            Ok(serde_json::json!({
                "id": id, "cwd": cwd,
                "name": name, "description": description,
                "tags": serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default(),
                "archived": archived != 0,
                "createdAt": created_at, "updatedAt": updated_at,
            }))
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn archive_project(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        conn.execute(
            "UPDATE projects SET archived = 1, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        conn.execute(
            "UPDATE sessions SET archived = 1 WHERE project_id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    pub fn unarchive_project(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        conn.execute(
            "UPDATE projects SET archived = 0, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        Ok(())
    }
}

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
        let mut conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        let tx = conn.transaction()?;

        // We will change the project's primary key when the upstream id has
        // drifted (same cwd, new id). sessions.project_id references it, so
        // defer foreign-key checks until commit; we manually cascade below.
        tx.execute("PRAGMA defer_foreign_keys = ON", [])?;

        // Detect an existing project at the same cwd so we can cascade the new id
        // to its sessions when the upstream project id has changed.
        let old_id: Option<String> = match tx.query_row(
            "SELECT id FROM projects WHERE cwd = ?1",
            rusqlite::params![cwd],
            |row| row.get::<_, String>(0),
        ) {
            Ok(old) => Some(old),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(StoreError::Sqlite(e)),
        };

        tx.execute(
            "INSERT INTO projects (id, cwd, name, description, tags_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(cwd) DO UPDATE SET id = ?1, name = ?3, description = ?4, tags_json = ?5, updated_at = ?6",
            rusqlite::params![id, cwd, name, description, tags_json, now],
        )?;

        if let Some(old_id) = old_id {
            if old_id != id {
                tx.execute(
                    "UPDATE sessions SET project_id = ?1 WHERE project_id = ?2",
                    rusqlite::params![id, old_id],
                )?;
            }
        }

        tx.commit()?;
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

#[cfg(test)]
mod tests {
    use super::Store;
    use std::sync::Arc;

    fn temp_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()))
    }

    #[test]
    fn ensure_project_updates_id_and_cascades_sessions_on_cwd_conflict() {
        let store = temp_store();
        let cwd = "/tmp/jia-test-project";
        let id_a = "proj-id-a";
        let id_b = "proj-id-b";

        store.ensure_project(id_a, cwd, "Alpha", "", "[]").unwrap();
        store.create_session("sess-1", "title", cwd, id_a).unwrap();

        // Simulate re-init generating a new id for the same cwd.
        store.ensure_project(id_b, cwd, "Beta", "", "[]").unwrap();

        // New id should be queryable.
        let proj = store
            .get_project(id_b)
            .unwrap()
            .expect("project with new id should exist");
        assert_eq!(proj["cwd"].as_str(), Some(cwd));
        assert_eq!(proj["name"].as_str(), Some("Beta"));

        // Old id should be gone.
        assert!(store.get_project(id_a).unwrap().is_none());

        // Sessions should be cascaded to the new id.
        assert_eq!(store.session_project_id("sess-1").as_deref(), Some(id_b));

        // list_projects should count the session under the new id.
        let projects = store.list_projects(false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"].as_str(), Some(id_b));
        assert_eq!(projects[0]["sessionCount"].as_i64(), Some(1));
    }

    #[test]
    fn ensure_project_same_id_is_idempotent() {
        let store = temp_store();
        let cwd = "/tmp/jia-test-project";
        let id = "proj-id-same";

        store.ensure_project(id, cwd, "First", "desc1", "[]").unwrap();
        store.ensure_project(id, cwd, "First", "desc1", "[]").unwrap();

        let projects = store.list_projects(false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"].as_str(), Some(id));
        assert_eq!(projects[0]["name"].as_str(), Some("First"));
        assert_eq!(projects[0]["sessionCount"].as_i64(), Some(0));
    }
}

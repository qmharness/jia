//! Workspace persistence: CRUD operations on the workspaces table.
//! 一目录=一工作区;"project" 名称留给未来"工作区内创建的项目"。

use super::{Store, StoreError};

impl Store {
    pub fn ensure_workspace(
        &self,
        id: &str,
        cwd: &str,
        name: &str,
        description: &str,
        tags_json: &str,
    ) -> Result<(), StoreError> {
        let mut conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        // BEGIN IMMEDIATE: the SELECT-then-write below must be serialized
        // against concurrent ensure_workspace calls (DEFERRED would allow two
        // callers to both read empty and double-INSERT).
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        // We may change the project's primary key when the upstream id has
        // drifted (same cwd, new id), or change its cwd when the project has
        // moved (same id, new cwd). sessions.workspace_id references it, so
        // defer foreign-key checks until commit; we manually cascade below.
        tx.execute("PRAGMA defer_foreign_keys = ON", [])?;

        let mut stmt = tx.prepare("SELECT id, cwd FROM workspaces WHERE id = ?1 OR cwd = ?2")?;
        let rows: Vec<(String, String)> = stmt
            .query_map(rusqlite::params![id, cwd], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let row_by_id = rows.iter().find(|(rid, _)| rid == id).cloned();
        let row_by_cwd = rows.iter().find(|(_, rcwd)| rcwd == cwd).cloned();

        match (&row_by_id, &row_by_cwd) {
            // No existing project: create it.
            (None, None) => {
                tx.execute(
                    "INSERT INTO workspaces (id, cwd, name, description, tags_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                    rusqlite::params![id, cwd, name, description, tags_json, now],
                )?;
            }
            // Existing project with the same id: follow directory moves.
            (Some(_), _) => {
                if let Some((stale_id, _)) = &row_by_cwd {
                    if stale_id != id {
                        // A different project currently occupies the new cwd.
                        // Merge its sessions (and its memory seeds) into the
                        // canonical project and delete the stale row so the
                        // cwd UNIQUE constraint is not violated when we update
                        // the canonical row.
                        tx.execute(
                            "UPDATE sessions SET workspace_id = ?1 WHERE workspace_id = ?2",
                            rusqlite::params![id, stale_id],
                        )?;
                        tx.execute(
                            "UPDATE seeds SET workspace_id = ?1 WHERE workspace_id = ?2",
                            rusqlite::params![id, stale_id],
                        )?;
                        tx.execute(
                            "DELETE FROM workspaces WHERE id = ?1",
                            rusqlite::params![stale_id],
                        )?;
                    }
                }
                tx.execute(
                    // 只更新身份字段——description/tags 是 PATCH 写入的元数据,
                    // resolve_workspace(rin,每次 hello 都调本函数且只传身份)
                    // 不得抹掉它们(审计 F1 元数据漂移)。
                    "UPDATE workspaces SET cwd = ?2, name = ?3, updated_at = ?6 WHERE id = ?1",
                    rusqlite::params![id, cwd, name, description, tags_json, now],
                )?;
            }
            // Same cwd with a different id: upstream id drifted. Update the
            // existing row's id and cascade its sessions and memory seeds —
            // otherwise load_seeds_by_workspace(new_id) silently sees nothing.
            (None, Some((old_id, _))) => {
                tx.execute(
                    "UPDATE sessions SET workspace_id = ?1 WHERE workspace_id = ?2",
                    rusqlite::params![id, old_id],
                )?;
                tx.execute(
                    "UPDATE seeds SET workspace_id = ?1 WHERE workspace_id = ?2",
                    rusqlite::params![id, old_id],
                )?;
                tx.execute(
                    // 同上:漂移对齐 id 与 name,保留已有 description/tags。
                    "UPDATE workspaces SET id = ?1, name = ?3, updated_at = ?6 WHERE cwd = ?2",
                    rusqlite::params![id, cwd, name, description, tags_json, now],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// PATCH 专用:直接更新 name/description/tags(与 ensure_workspace 的
    /// 身份维护分离——后者在搬家/漂移路径刻意不动元数据)。
    pub fn update_workspace_metadata(
        &self,
        id: &str,
        name: &str,
        description: &str,
        tags_json: &str,
    ) -> Result<usize, StoreError> {
        let conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        let n = conn.execute(
            "UPDATE workspaces SET name = ?2, description = ?3, tags_json = ?4, updated_at = ?5 WHERE id = ?1",
            rusqlite::params![id, name, description, tags_json, now],
        )?;
        Ok(n)
    }

    pub fn list_workspaces(
        &self,
        include_archived: bool,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        // session_count 只计未归档会话(活跃口径;已归档会话不再代表项目活跃度)。
        let sql = if include_archived {
            "SELECT p.id, p.cwd, p.name, p.description, p.tags_json, p.archived, p.created_at, p.updated_at,
                    COUNT(CASE WHEN s.archived = 0 THEN s.id END) as session_count
             FROM workspaces p
             LEFT JOIN sessions s ON s.workspace_id = p.id
             GROUP BY p.id
             ORDER BY p.updated_at DESC"
        } else {
            "SELECT p.id, p.cwd, p.name, p.description, p.tags_json, p.archived, p.created_at, p.updated_at,
                    COUNT(CASE WHEN s.archived = 0 THEN s.id END) as session_count
             FROM workspaces p
             LEFT JOIN sessions s ON s.workspace_id = p.id
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

    pub fn get_workspace(&self, id: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, cwd, name, description, tags_json, archived, created_at, updated_at FROM workspaces WHERE id = ?1"
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

    /// 归档工作区并级联归档其会话(事务包装,返回工作区行数供 404 判定)。
    pub fn archive_workspace(&self, id: &str) -> Result<usize, StoreError> {
        let mut conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE workspaces SET archived = 1, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        tx.execute(
            "UPDATE sessions SET archived = 1 WHERE workspace_id = ?1",
            rusqlite::params![id],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// 取消归档:恢复工作区,并级联恢复其会话(与 archive 对称;
    /// 注意会连带恢复此前被单独归档的会话——当前无单独归档会话的入口)。
    pub fn unarchive_workspace(&self, id: &str) -> Result<usize, StoreError> {
        let mut conn = self.pool.get()?;
        let now = crate::utils::unix_now();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE workspaces SET archived = 0, updated_at = ?2 WHERE id = ?1",
            rusqlite::params![id, now],
        )?;
        tx.execute(
            "UPDATE sessions SET archived = 0 WHERE workspace_id = ?1",
            rusqlite::params![id],
        )?;
        tx.commit()?;
        Ok(n)
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
    fn ensure_workspace_updates_id_and_cascades_sessions_on_cwd_conflict() {
        let store = temp_store();
        let cwd = "/tmp/jia-test-project";
        let id_a = "proj-id-a";
        let id_b = "proj-id-b";

        store
            .ensure_workspace(id_a, cwd, "Alpha", "", "[]")
            .unwrap();
        store.create_session("sess-1", "title", cwd, id_a).unwrap();
        store
            .insert_seed(
                &serde_json::json!({
                    "id": "seed-1", "session_id": "sess-1", "workspace_id": id_a,
                    "content": {"type": "FreeText", "text": "alpha memory"},
                })
                .to_string(),
            )
            .unwrap();

        // Simulate re-init generating a new id for the same cwd.
        store.ensure_workspace(id_b, cwd, "Beta", "", "[]").unwrap();

        // New id should be queryable.
        let proj = store
            .get_workspace(id_b)
            .unwrap()
            .expect("project with new id should exist");
        assert_eq!(proj["cwd"].as_str(), Some(cwd));
        assert_eq!(proj["name"].as_str(), Some("Beta"));

        // Old id should be gone.
        assert!(store.get_workspace(id_a).unwrap().is_none());

        // Sessions should be cascaded to the new id.
        assert_eq!(store.session_workspace_id("sess-1").as_deref(), Some(id_b));

        // Memory seeds follow too (final review I-1): the old id's seeds must
        // stay visible under the new id, not silently detach.
        assert!(store.load_seeds_by_workspace(id_a).unwrap().is_empty());
        assert_eq!(store.load_seeds_by_workspace(id_b).unwrap().len(), 1);

        // list_workspaces should count the session under the new id.
        let projects = store.list_workspaces(false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"].as_str(), Some(id_b));
        assert_eq!(projects[0]["sessionCount"].as_i64(), Some(1));
    }

    #[test]
    fn ensure_workspace_same_id_is_idempotent() {
        let store = temp_store();
        let cwd = "/tmp/jia-test-project";
        let id = "proj-id-same";

        store
            .ensure_workspace(id, cwd, "First", "desc1", "[]")
            .unwrap();
        store.create_session("sess-same", "title", cwd, id).unwrap();
        // Same id + same cwd with changed name: updates identity in place,
        // keeps one row, does not lose the session association — and must NOT
        // touch description/tags (audit F1: resolve paths call ensure on
        // every hello; metadata belongs to update_workspace_metadata).
        store
            .ensure_workspace(id, cwd, "Renamed", "SHOULD-NOT-WIN", "[\"x\"]")
            .unwrap();

        let projects = store.list_workspaces(false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"].as_str(), Some(id));
        assert_eq!(projects[0]["name"].as_str(), Some("Renamed"));
        assert_eq!(projects[0]["sessionCount"].as_i64(), Some(1));
        assert_eq!(store.session_workspace_id("sess-same").as_deref(), Some(id));

        let proj = store.get_workspace(id).unwrap().expect("project exists");
        assert_eq!(
            proj["description"].as_str(),
            Some("desc1"),
            "ensure must preserve metadata"
        );

        // Metadata changes go through the dedicated path.
        store
            .update_workspace_metadata(id, "Renamed", "desc2", "[\"a\"]")
            .unwrap();
        let proj = store.get_workspace(id).unwrap().expect("project exists");
        assert_eq!(proj["description"].as_str(), Some("desc2"));
        assert_eq!(proj["tags"][0].as_str(), Some("a"));
    }

    #[test]
    fn ensure_workspace_follows_directory_move() {
        let store = temp_store();
        let id = "proj-id-move";
        let old_cwd = "/tmp/jia-test-old";
        let new_cwd = "/tmp/jia-test-new";

        store
            .ensure_workspace(id, old_cwd, "Old", "", "[]")
            .unwrap();
        store
            .create_session("sess-1", "title", old_cwd, id)
            .unwrap();

        // Same project id, different cwd (directory moved/renamed).
        store
            .ensure_workspace(id, new_cwd, "New", "desc", "[]")
            .unwrap();

        let proj = store
            .get_workspace(id)
            .unwrap()
            .expect("project should exist after move");
        assert_eq!(proj["cwd"].as_str(), Some(new_cwd));
        assert_eq!(proj["name"].as_str(), Some("New"));
        // Metadata is preserved across moves (audit F1), not overwritten.
        assert_eq!(proj["description"].as_str(), Some(""));

        assert_eq!(store.session_workspace_id("sess-1").as_deref(), Some(id));

        let projects = store.list_workspaces(false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["cwd"].as_str(), Some(new_cwd));
        assert_eq!(projects[0]["sessionCount"].as_i64(), Some(1));
    }

    #[test]
    fn ensure_workspace_move_with_stale_cwd_row() {
        let store = temp_store();
        let canonical_id = "proj-canonical";
        let stale_id = "proj-stale";
        let old_cwd = "/tmp/jia-test-old";
        let new_cwd = "/tmp/jia-test-new";

        store
            .ensure_workspace(canonical_id, old_cwd, "Canonical", "", "[]")
            .unwrap();
        store
            .ensure_workspace(stale_id, new_cwd, "Stale", "", "[]")
            .unwrap();
        store
            .create_session("sess-old", "title", old_cwd, canonical_id)
            .unwrap();
        store
            .create_session("sess-stale", "title", new_cwd, stale_id)
            .unwrap();

        // Directory moved to a cwd already occupied by a stale project row.
        store
            .ensure_workspace(canonical_id, new_cwd, "Canonical", "", "[]")
            .unwrap();

        let proj = store
            .get_workspace(canonical_id)
            .unwrap()
            .expect("canonical project should exist");
        assert_eq!(proj["cwd"].as_str(), Some(new_cwd));

        assert!(store.get_workspace(stale_id).unwrap().is_none());

        assert_eq!(
            store.session_workspace_id("sess-old").as_deref(),
            Some(canonical_id)
        );
        assert_eq!(
            store.session_workspace_id("sess-stale").as_deref(),
            Some(canonical_id)
        );

        let projects = store.list_workspaces(false).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["id"].as_str(), Some(canonical_id));
        assert_eq!(projects[0]["sessionCount"].as_i64(), Some(2));
    }
}

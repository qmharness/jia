use std::sync::Arc;

use super::Store;
use crate::error::JiaError;

/// Async facade over the synchronous Store.
///
/// Delegates all work to `spawn_blocking` so SQLite I/O never blocks
/// a tokio worker thread.  The inner `Arc<Store>` is shared with the
/// sync callers that still need direct access (schema migration,
/// backup, etc.).
pub struct StoreAsync {
    inner: Arc<Store>,
}

impl StoreAsync {
    pub fn new(store: Arc<Store>) -> Self {
        Self { inner: store }
    }

    // ── Wave 1: hot-path methods called from the agent loop ──────

    /// Persist session messages (called 3× per agent turn).
    pub async fn save_session(&self, id: &str, json: &str) -> Result<(), JiaError> {
        let store = self.inner.clone();
        let id = id.to_string();
        let json = json.to_string();
        tokio::task::spawn_blocking(move || store.save_session(&id, &json))
            .await
            .map_err(|e| JiaError::Internal(format!("spawn_blocking join: {e}")))?
            .map_err(JiaError::from)
    }

    /// Load persisted session messages.
    pub async fn load_session(&self, id: &str) -> Result<Option<String>, JiaError> {
        let store = self.inner.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || store.load_session(&id))
            .await
            .map_err(|e| JiaError::Internal(format!("spawn_blocking join: {e}")))?
            .map_err(JiaError::from)
    }

    /// Insert a single seed (JSON-serialised).
    pub async fn insert_seed(&self, json: &str) -> Result<(), JiaError> {
        let store = self.inner.clone();
        let json = json.to_string();
        tokio::task::spawn_blocking(move || store.insert_seed(&json))
            .await
            .map_err(|e| JiaError::Internal(format!("spawn_blocking join: {e}")))?
            .map_err(JiaError::from)
    }

    /// Touch a batch of seed ids (update access timestamps + strength).
    pub async fn touch_batch(&self, ids: &[String]) -> Result<(), JiaError> {
        let store = self.inner.clone();
        let ids = ids.to_vec();
        tokio::task::spawn_blocking(move || store.touch_batch(&ids))
            .await
            .map_err(|e| JiaError::Internal(format!("spawn_blocking join: {e}")))?
            .map_err(JiaError::from)
    }

    /// FTS5 full-text search over seeds.
    pub async fn search_seeds(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, JiaError> {
        let store = self.inner.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || store.search_seeds(&query, limit))
            .await
            .map_err(|e| JiaError::Internal(format!("spawn_blocking join: {e}")))?
            .map_err(JiaError::from)
    }

    /// Count total seeds (used for Manas recalibration).
    pub async fn count_seeds(&self) -> Result<usize, JiaError> {
        let store = self.inner.clone();
        tokio::task::spawn_blocking(move || store.count_seeds())
            .await
            .map_err(|e| JiaError::Internal(format!("spawn_blocking join: {e}")))?
            .map_err(JiaError::from)
    }
}

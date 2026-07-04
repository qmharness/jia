/// Graph and search: triple expansion, entity search, contradiction detection, FTS5 search.
use super::helpers::*;
use super::{Store, StoreError};

impl Store {
    pub fn graph_expand(
        &self,
        anchor_values: &[String],
        limit: usize,
    ) -> Result<Vec<String>, StoreError> {
        let results: Vec<_> = self
            .graph_expand_multi(anchor_values, 0, limit, None)?
            .into_iter()
            .map(|(json, _hop)| json)
            .collect();
        Ok(results)
    }

    /// Multi-hop graph expansion from anchor entity values.
    ///
    /// Breadth-first traversal through Triple seeds: for each hop, finds seeds
    /// where subject or object matches any current anchor value, collects new
    /// entity values as anchors for the next hop.
    ///
    /// - `max_hops=0`: exact entity match (no traversal)
    /// - `max_hops=1`: equivalent to `graph_expand`
    /// - `max_hops>=2`: multi-hop traversal
    ///
    /// Returns (seed_json, hop_distance) tuples ordered by hop distance,
    /// then by strength descending within each hop.
    pub fn graph_expand_multi(
        &self,
        anchor_values: &[String],
        max_hops: usize,
        max_per_hop: usize,
        predicate_filter: Option<&str>,
    ) -> Result<Vec<(String, usize)>, StoreError> {
        if anchor_values.is_empty() || max_per_hop == 0 {
            return Ok(Vec::new());
        }

        let conn = self.pool.get()?;
        let mut results: Vec<(String, usize)> = Vec::new();
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new(); // seed ids
        let mut current_anchors: Vec<String> = anchor_values.to_vec();
        let mut seen_anchors: std::collections::HashSet<String> =
            anchor_values.iter().cloned().collect();

        for hop in 0..=max_hops {
            if current_anchors.is_empty() {
                break;
            }

            let seed_jsons =
                self.search_entities_batch(&conn, &current_anchors, max_per_hop, predicate_filter)?;
            let mut next_anchors: Vec<String> = Vec::new();

            for json in seed_jsons {
                if let Some(id) = extract_seed_id(&json) {
                    if !visited.insert(id) {
                        continue;
                    }
                    // Collect neighbor entity values for the next hop
                    let neighbors = extract_neighbor_values(&json);
                    for n in neighbors {
                        if seen_anchors.insert(n.clone()) {
                            next_anchors.push(n);
                        }
                    }
                    results.push((json, hop));
                }
            }
            current_anchors = next_anchors;
        }

        results.sort_by(|a, b| {
            a.1.cmp(&b.1).then_with(|| {
                let sa = extract_strength(&a.0);
                let sb = extract_strength(&b.0);
                sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        Ok(results)
    }

    /// Find Triple seeds where subject OR object matches any of the given entities.
    fn search_entities_batch(
        &self,
        conn: &rusqlite::Connection,
        entities: &[String],
        limit: usize,
        predicate_filter: Option<&str>,
    ) -> Result<Vec<String>, StoreError> {
        if entities.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<&str> = entities.iter().map(|_| "?").collect();
        let in_clause = placeholders.join(",");

        let pred_clause = match predicate_filter {
            Some(_) => " AND json_extract(content_json, '$.predicate') = ?".to_string(),
            None => String::new(),
        };

        let sql = format!(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier
             FROM seeds
             WHERE content_type = 'Triple'
               AND (json_extract(content_json, '$.subject') IN ({in_clause})
                    OR json_extract(content_json, '$.object') IN ({in_clause}))
               {pred_clause}
             LIMIT ?",
        );
        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        for e in entities {
            all_params.push(Box::new(e.clone()));
        }
        for e in entities {
            all_params.push(Box::new(e.clone()));
        }
        if let Some(p) = predicate_filter {
            all_params.push(Box::new(p.to_string()));
        }
        all_params.push(Box::new(limit as i64));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| Ok(seed_row_to_json(row)))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Find conflicting Triple assertions: same (subject, predicate) → ≥2 divergent objects.
    /// Returns groups of conflicting seed JSONs. Limited to `limit` conflict groups.
    pub fn find_contradicting_triples(&self, limit: usize) -> Result<Vec<Vec<String>>, StoreError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, nature, source, content_type, content_json,
             palace, intent_stem, geju_key, created_at, access_count, last_accessed_at, strength, tier
             FROM seeds
             WHERE content_type = 'Triple'
             ORDER BY json_extract(content_json, '$.subject'),
                      json_extract(content_json, '$.predicate'),
                      created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| Ok(seed_row_to_json(row)))?;

        // Group by (subject, predicate) key
        let mut groups: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        for row in rows {
            let json = row?;
            let key = extract_assertion_key(&json);
            groups.entry(key).or_default().push(json);
        }

        // Return groups with ≥2 distinct objects
        let mut conflicts: Vec<Vec<String>> = Vec::new();
        for group in groups.into_values() {
            if conflicts.len() >= limit {
                break;
            }
            let distinct_objects: std::collections::HashSet<String> = group
                .iter()
                .filter_map(|j| extract_triple_object(j))
                .collect();
            if distinct_objects.len() >= 2 {
                conflicts.push(group);
            }
        }
        Ok(conflicts)
    }

    // ── FTS5 Semantic search ────────────────────────────────

    /// Search seeds by content text using FTS5.
    ///
    /// Returns matching seeds with their BM25 rank scores.
    /// Escapes special FTS5 query characters automatically.
    pub fn search_seeds(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f32)>, StoreError> {
        let conn = self.pool.get()?;
        let safe_query = escape_fts5_query(query);
        if safe_query.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = conn.prepare(
            "SELECT s.id, s.session_id, s.nature, s.source, s.content_type, s.content_json,
             s.palace, s.intent_stem, s.geju_key, s.created_at, s.access_count, s.last_accessed_at, s.strength, s.tier,
             rank
             FROM seeds_fts f
             JOIN seeds s ON s.id = f.id
             WHERE seeds_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(rusqlite::params![&safe_query, limit as i64], |row| {
            let rank: f64 = row.get(14)?;
            Ok((seed_row_to_json(row), rank as f32))
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── Seed CRUD ────────────────────────────────────────────
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::Palace;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::stems::Stem;
    use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource};
    use std::sync::Arc;

    fn temp_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        std::mem::forget(dir);
        Arc::new(Store::open(path.to_str().unwrap()))
    }

    fn insert_triple(store: &Store, subj: &str, pred: &str, obj: &str) {
        let seed = Seed::new(
            "test-sess".into(),
            SeedNature::Fact,
            SeedSource::ToolObservation,
            SeedContent::Triple {
                subject: subj.into(),
                predicate: pred.into(),
                object: obj.into(),
            },
            Palace::Gen,
            Stem::Gui,
            "test_geju".into(),
        );
        let json = serde_json::to_string(&seed).unwrap();
        store.insert_seed(&json).unwrap();
    }

    #[test]
    fn graph_expand_empty_returns_empty() {
        let store = temp_store();
        let result = store.graph_expand(&["nonexistent".into()], 10).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn graph_expand_finds_direct_match() {
        let store = temp_store();
        insert_triple(&store, "A", "depends_on", "B");
        let result = store.graph_expand(&["A".into()], 10).unwrap();
        assert!(!result.is_empty());
    }

    #[test]
    fn search_seeds_fts5_matches() {
        let store = temp_store();
        let seed = Seed::new(
            "test-sess".into(),
            SeedNature::Fact,
            SeedSource::ToolObservation,
            SeedContent::FreeText {
                text: "PostgreSQL database configuration".into(),
            },
            Palace::Zhen,
            Stem::Wu,
            "fts5_test".into(),
        );
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
        let results = store.search_seeds("PostgreSQL", 5).unwrap();
        assert!(!results.is_empty());
    }
}

// ── NamaRupa Tool — Agentic graph memory for nāma-rūpa ──────

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::Palace;
use crate::palaces::gen_store::Store;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::Stem;
use crate::stems::intent::StoreAction;
use crate::utils;
use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedTier};

pub struct NamaRupaTool {
    store: Arc<Store>,
}

impl NamaRupaTool {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    fn handle_query(&self, input: &Value) -> Result<String, String> {
        let anchors: Vec<String> = input["anchors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let keywords = input["keywords"].as_str().map(String::from);
        let max_hops = input["max_hops"].as_u64().unwrap_or(2) as usize;
        let limit = input["limit"].as_u64().unwrap_or(20) as usize;
        let predicate = input["predicate"].as_str();

        if anchors.is_empty() && keywords.is_none() {
            return Ok(serde_json::json!({
                "results": [],
                "count": 0,
                "hint": "Provide 'anchors' (entity names) or 'keywords' to query the graph."
            })
            .to_string());
        }

        let mut seed_jsons: Vec<(String, usize)> = Vec::new();

        if !anchors.is_empty() {
            match self
                .store
                .graph_expand_multi(&anchors, max_hops, limit, predicate)
            {
                Ok(results) => seed_jsons = results,
                Err(e) => return Err(format!("Graph query failed: {e}")),
            }
        }

        // FTS5 fallback: if graph results are sparse and keywords are provided
        if seed_jsons.len() < 3
            && let Some(ref kw) = keywords
            && !kw.is_empty()
        {
            match self.store.search_seeds(kw, limit) {
                Ok(fts_results) => {
                    let existing_ids: std::collections::HashSet<String> = seed_jsons
                        .iter()
                        .filter_map(|(j, _)| {
                            serde_json::from_str::<Value>(j)
                                .ok()
                                .and_then(|v| v["id"].as_str().map(String::from))
                        })
                        .collect();

                    for (json, _rank) in fts_results {
                        let id = serde_json::from_str::<Value>(&json)
                            .ok()
                            .and_then(|v| v["id"].as_str().map(String::from))
                            .unwrap_or_default();
                        if !existing_ids.contains(&id) {
                            // FTS5 results are hop-unknown; mark as hop 99
                            seed_jsons.push((json, 99));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("FTS5 fallback failed: {e}");
                }
            }
        }

        let count = seed_jsons.len();
        let mut formatted: Vec<Value> = Vec::with_capacity(count);
        let mut graph_lines: Vec<String> = Vec::with_capacity(count);
        let mut touched_ids: Vec<String> = Vec::with_capacity(count);

        for (json, hop) in &seed_jsons {
            let v: Value = serde_json::from_str(json).unwrap_or_default();
            let content = &v["content"];
            let is_triple = content["type"].as_str() == Some("Triple");
            let text = if is_triple {
                format!(
                    "{} {} {}",
                    content["subject"].as_str().unwrap_or("?"),
                    content["predicate"].as_str().unwrap_or("?"),
                    content["object"].as_str().unwrap_or("?")
                )
            } else {
                content.to_string().chars().take(200).collect()
            };
            formatted.push(serde_json::json!({
                "id": v["id"],
                "text": text,
                "hop": hop,
                "strength": v["strength"],
            }));
            if let Some(id) = v["id"].as_str() {
                touched_ids.push(id.to_string());
            }
            graph_lines.push(format!("- {text}"));
        }

        Ok(serde_json::json!({
            "results": formatted,
            "graph_text": graph_lines.join("\n"),
            "count": count,
            "touched_ids": touched_ids,
        })
        .to_string())
    }

    fn handle_save(&self, input: &Value) -> Result<String, String> {
        let facts = input["facts"]
            .as_array()
            .ok_or("Missing 'facts' array parameter")?;

        let mut saved = 0u64;
        let mut ids: Vec<String> = Vec::new();
        let now = utils::unix_now();

        for fact in facts {
            let subject = fact["subject"].as_str().unwrap_or("").trim().to_string();
            let predicate = fact["predicate"].as_str().unwrap_or("").trim().to_string();
            let object = fact["object"].as_str().unwrap_or("").trim().to_string();

            if subject.is_empty() || predicate.is_empty() || object.is_empty() {
                continue;
            }

            let id = uuid::Uuid::new_v4().to_string();
            let seed = Seed {
                id: id.clone(),
                session_id: String::new(),
                nature: SeedNature::Fact,
                source: SeedSource::Consolidation,
                content: SeedContent::Triple {
                    subject,
                    predicate,
                    object,
                },
                palace: Palace::Gen,
                intent_stem: Stem::Gui,
                geju_key: "namarupa".into(),
                created_at: now,
                access_count: 0,
                last_accessed_at: now,
                strength: 0.8,
                tier: SeedTier::OnDemand,
            };

            match serde_json::to_string(&seed) {
                Ok(json) => match self.store.insert_seed(&json) {
                    Ok(()) => {
                        saved += 1;
                        ids.push(id);
                    }
                    Err(e) => {
                        tracing::warn!("NamaRupa: failed to save seed: {e}");
                    }
                },
                Err(e) => {
                    tracing::warn!("NamaRupa: failed to serialize seed: {e}");
                }
            }
        }

        Ok(serde_json::json!({
            "saved": saved,
            "touched_ids": ids,
        })
        .to_string())
    }

    fn handle_delete(&self, input: &Value) -> Result<String, String> {
        let ids: Vec<String> = input["ids"]
            .as_array()
            .ok_or("Missing 'ids' array parameter")?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();

        if ids.is_empty() {
            return Ok(serde_json::json!({"deleted": 0}).to_string());
        }

        // Query only sources for the target IDs — avoids loading all seeds.
        // UserStatement seeds are immune to agent deletion.
        let sources = self.store.load_seed_sources(&ids).unwrap_or_default();
        let user_statement_ids: std::collections::HashSet<&str> = sources
            .iter()
            .filter(|(_, source)| source == "UserStatement")
            .map(|(id, _)| id.as_str())
            .collect();

        let eligible: Vec<String> = ids
            .iter()
            .filter(|id| !user_statement_ids.contains(id.as_str()))
            .cloned()
            .collect();

        let count = eligible.len();
        if !eligible.is_empty() {
            let _ = self.store.delete_seeds(&eligible);
        }

        let skipped = ids.len() - count;
        Ok(serde_json::json!({
            "deleted": count,
            "skipped": skipped,
            "note": if skipped > 0 { "Some seeds could not be deleted (UserStatement or not found)" } else { "" },
        }).to_string())
    }

    fn handle_contradictions(&self, input: &Value) -> Result<String, String> {
        let limit = input["limit"].as_u64().unwrap_or(20) as usize;

        let groups = self
            .store
            .find_contradicting_triples(limit)
            .map_err(|e| format!("Contradiction query failed: {e}"))?;

        let conflicts: Vec<Value> = groups.iter().map(|group| {
            let assertions: Vec<Value> = group.iter().map(|json| {
                let v: Value = serde_json::from_str(json).unwrap_or_default();
                let content = &v["content"];
                serde_json::json!({
                    "id": v["id"],
                    "subject": content["subject"],
                    "predicate": content["predicate"],
                    "object": content["object"],
                    "source": v["source"],
                })
            }).collect();

            let distinct_objects: Vec<String> = group.iter()
                .filter_map(|json| {
                    let v: Value = serde_json::from_str(json).ok()?;
                    v["content"]["object"].as_str().map(String::from)
                })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();

            serde_json::json!({
                "subject": assertions.first().and_then(|a| a["subject"].as_str()).unwrap_or("?"),
                "predicate": assertions.first().and_then(|a| a["predicate"].as_str()).unwrap_or("?"),
                "divergent_objects": distinct_objects,
                "assertions": assertions,
            })
        }).collect();

        Ok(serde_json::json!({
            "conflicts": conflicts,
            "count": conflicts.len(),
        })
        .to_string())
    }
}

#[async_trait]
impl BaseTool for NamaRupaTool {
    fn name(&self) -> &str {
        "namarupa"
    }

    fn description(&self) -> String {
        "Query or persist nāma-rūpa (名相) — named facts and their\n\
         connections in the agent's memory graph. The graph stores triples\n\
         (subject, predicate, object) from past interactions and deductions.\n\
         \n\
         Use 'query' to trace connections from concepts (nāma) to related\n\
         phenomena (rūpa) through multi-hop graph traversal. Provide anchors\n\
         (names or entity values) as starting points. Use max_hops=0 for\n\
         exact entity matching without expansion.\n\
         \n\
         Use 'save' when your current vikalpa (discriminative reasoning)\n\
         uncovers a fact worth persisting beyond this conversation.\n\
         \n\
         Use 'delete' to remove false nāma-rūpa — facts your vikalpa has\n\
         determined to be erroneous (hallucinations, corrected mistakes).\n\
         \n\
         Use 'contradictions' to find conflicting assertions — when the same\n\
         subject+predicate maps to divergent objects (heterodox seeds)."
            .to_string()
    }

    fn category(&self) -> &str {
        "memory"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Gui(StoreAction {
            key: "namarupa".into(),
            value: String::new(),
        })
    }

    fn target_palace(&self, input: &Value) -> crate::palaces::Palace {
        match input["action"].as_str() {
            Some("save") | Some("delete") => crate::palaces::Palace::Qian,
            _ => crate::palaces::Palace::Gen,
        }
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["query", "save", "delete", "contradictions"],
                    "description": "What to do: 'query' searches the graph, 'save' persists facts, 'delete' removes erroneous facts, 'contradictions' finds conflicting facts"
                },
                "anchors": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Entity names to use as graph traversal anchors (for 'query' action)"
                },
                "keywords": {
                    "type": "string",
                    "description": "Keyword search terms (for 'query' action, fallback if graph yields few results)"
                },
                "predicate": {
                    "type": "string",
                    "description": "Filter graph edges by predicate type (for 'query' action)"
                },
                "max_hops": {
                    "type": "integer",
                    "description": "Maximum expansion hops for graph traversal. 0 = exact match only (default: 2)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results to return (default: 20)"
                },
                "facts": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "subject": { "type": "string" },
                            "predicate": { "type": "string" },
                            "object": { "type": "string" }
                        },
                        "required": ["subject", "predicate", "object"]
                    },
                    "description": "Array of facts to save (for 'save' action)"
                },
                "ids": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Seed IDs to delete (for 'delete' action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, input: Value) -> Result<String, String> {
        let action = input["action"]
            .as_str()
            .ok_or("Missing 'action' parameter")?;

        match action {
            "query" => self.handle_query(&input),
            "save" => self.handle_save(&input),
            "delete" => self.handle_delete(&input),
            "contradictions" => self.handle_contradictions(&input),
            _ => Err(format!(
                "Unknown action '{}'. Use 'query', 'save', 'delete', or 'contradictions'.",
                action
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::Palace;
    use crate::stems::Stem;
    use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedTier};

    fn temp_store() -> Arc<Store> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()))
    }

    fn insert_triple(store: &Store, id: &str, subj: &str, pred: &str, obj: &str, source: &str) {
        let seed = Seed {
            id: id.into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: match source {
                "UserStatement" => SeedSource::UserStatement,
                "Consolidation" => SeedSource::Consolidation,
                "RenSoul" => SeedSource::RenSoul,
                _ => SeedSource::ToolObservation,
            },
            content: SeedContent::Triple {
                subject: subj.into(),
                predicate: pred.into(),
                object: obj.into(),
            },
            palace: Palace::Gen,
            intent_stem: Stem::Gui,
            geju_key: "namarupa".into(),
            created_at: crate::utils::unix_now(),
            access_count: 0,
            last_accessed_at: crate::utils::unix_now(),
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }

    #[tokio::test]
    async fn query_by_anchors() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());
        insert_triple(
            &store,
            "s1",
            "serde",
            "is",
            "serialization_lib",
            "Consolidation",
        );
        insert_triple(
            &store,
            "s2",
            "tokio",
            "is",
            "async_runtime",
            "Consolidation",
        );

        let input = serde_json::json!({
            "action": "query",
            "anchors": ["serde"],
            "max_hops": 0,
        });
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(v["count"], 1);
        assert!(v["graph_text"].as_str().unwrap().contains("serde"));
    }

    #[tokio::test]
    async fn query_with_keywords_fallback() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());
        insert_triple(
            &store,
            "s1",
            "Cargo.toml",
            "depends_on",
            "serde",
            "Consolidation",
        );

        let input = serde_json::json!({
            "action": "query",
            "keywords": "Cargo",
            "max_hops": 0,
        });
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert!(
            v["count"].as_u64().unwrap() > 0,
            "FTS5 fallback should find 'Cargo'"
        );
    }

    #[tokio::test]
    async fn save_facts() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());

        let input = serde_json::json!({
            "action": "save",
            "facts": [
                {"subject": "node", "predicate": "runs", "object": "tokio"},
                {"subject": "app", "predicate": "uses", "object": "serde"},
            ]
        });
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(v["saved"], 2, "should save 2 facts");
        assert_eq!(v["touched_ids"].as_array().unwrap().len(), 2);

        // Verify persisted
        let all = store.load_all_seeds().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn save_skips_empty_facts() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());

        let input = serde_json::json!({
            "action": "save",
            "facts": [
                {"subject": "", "predicate": "runs", "object": "tokio"},
                {"subject": "app", "predicate": "uses", "object": "serde"},
            ]
        });
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(v["saved"], 1, "should skip empty subject");
    }

    #[tokio::test]
    async fn delete_facts() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());
        insert_triple(&store, "s1", "A", "p", "B", "Consolidation");
        insert_triple(&store, "s2", "C", "q", "D", "Consolidation");

        let input = serde_json::json!({
            "action": "delete",
            "ids": ["s1"],
        });
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(v["deleted"], 1);

        // Verify only s2 remains
        let all = store.load_all_seeds().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn delete_protects_user_statement() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());
        insert_triple(&store, "u1", "user", "likes", "rust", "UserStatement");
        insert_triple(&store, "s1", "system", "knows", "rust", "Consolidation");

        let input = serde_json::json!({
            "action": "delete",
            "ids": ["u1", "s1"],
        });
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(v["deleted"], 1, "should only delete non-UserStatement");
        assert_eq!(v["skipped"], 1, "should skip UserStatement");

        let all = store.load_all_seeds().unwrap();
        assert_eq!(all.len(), 1, "UserStatement should survive");
    }

    #[tokio::test]
    async fn contradictions() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store.clone());
        insert_triple(
            &store,
            "c1",
            "Cargo.toml",
            "depends_on",
            "serde",
            "Consolidation",
        );
        insert_triple(
            &store,
            "c2",
            "Cargo.toml",
            "depends_on",
            "tokio",
            "Consolidation",
        );

        let input = serde_json::json!({"action": "contradictions"});
        let output = tool.execute(input).await.unwrap();
        let v: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(v["count"], 1, "should find 1 conflict group");
        let c = &v["conflicts"][0];
        assert_eq!(c["subject"], "Cargo.toml");
        assert_eq!(c["predicate"], "depends_on");
    }

    #[tokio::test]
    async fn unknown_action_errors() {
        let store = temp_store();
        let tool = NamaRupaTool::new(store);
        let input = serde_json::json!({"action": "foobar"});
        let result = tool.execute(input).await;
        assert!(result.is_err());
    }
}

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Deserialize;

use crate::vijnana::alaya::SeedStore;
use crate::vijnana::manas::Manas;
use crate::zuowang::pipeline::ZuowangPipeline;
use crate::zuowang::trigger::AlayaEntropy;

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct VijnanaQuery {
    session_id: Option<String>,
}

pub async fn handle_vijnana_seeds(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<VijnanaQuery>,
) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };
    let seed_store = SeedStore::new(earth.store.clone());
    let seeds = match &query.session_id {
        Some(sid) => seed_store.load_by_session(sid).unwrap_or_default(),
        None => seed_store.load_all().unwrap_or_default(),
    };
    let list: Vec<_> = seeds.iter().map(|s| {
        let content = match &s.content {
            crate::vijnana::alaya::SeedContent::FreeText { text } => {
                serde_json::json!({"type": "free_text", "text": text})
            }
            crate::vijnana::alaya::SeedContent::KeyValue { key, value } => {
                serde_json::json!({"type": "key_value", "key": key, "value": value})
            }
            crate::vijnana::alaya::SeedContent::Triple { subject, predicate, object } => {
                serde_json::json!({"type": "triple", "subject": subject, "predicate": predicate, "object": object})
            }
        };
        serde_json::json!({
            "id": s.id,
            "nature": format!("{:?}", s.nature),
            "source": format!("{:?}", s.source),
            "content": content,
            "palace": format!("{:?}", s.palace),
            "intent_stem": format!("{:?}", s.intent_stem),
            "geju_key": s.geju_key,
            "strength": s.strength,
            "tier": format!("{:?}", s.tier),
            "access_count": s.access_count,
            "last_accessed_at": s.last_accessed_at,
            "created_at": s.created_at,
        })
    }).collect();
    Json(serde_json::json!({"seeds": list, "count": list.len()}))
}

pub async fn handle_vijnana_state(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let earth = match &state.earth {
        Some(e) => e,
        None => return Json(serde_json::json!({"error": "Agent not initialized"})),
    };

    // ── Manas (self-model) ──
    let model = earth
        .store
        .load_manas()
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str::<Manas>(&json).ok())
        .unwrap_or_default();
    let total_seeds = SeedStore::new(earth.store.clone())
        .load_all()
        .unwrap_or_default()
        .len();
    let manas = serde_json::json!({
        "atma_graha": model.atma_graha,
        "total_turns": model.total_turns,
        "consolidation_count": model.consolidation_count,
        "stable_pattern_count": model.stable_pattern_count,
        "last_consolidation_at": model.last_consolidation_at,
        "stable_epochs": model.stable_epochs(),
        "is_stable": model.is_stable(),
        "total_seeds": total_seeds,
    });

    // ── Entropy ──
    let seed_store = SeedStore::new(earth.store.clone());
    let seeds = seed_store.load_all().unwrap_or_default();
    let now = crate::utils::unix_now();
    let entropy = AlayaEntropy::compute(&seeds, now);
    let current = serde_json::json!({
        "staleness": entropy.staleness,
        "contradiction": entropy.contradiction,
        "redundancy": entropy.redundancy,
        "access_decay": entropy.access_decay,
        "total": entropy.total,
    });
    let history: Vec<_> = ZuowangPipeline::history(earth.store.clone())
        .iter()
        .map(|r| {
            serde_json::json!({
                "timestamp": r.timestamp,
                "examined": r.seeds_examined,
                "dissolved": r.seeds_dissolved,
                "weakened": r.seeds_weakened,
                "entropy_before": r.entropy_before,
                "entropy_after": r.entropy_after,
                "kept": r.score_kept,
                "protected": r.score_protected,
                "dissolved_sample": r.dissolved_sample.iter().map(|d| {
                    serde_json::json!({
                        "nature": d.nature,
                        "source": d.source,
                        "primary_dim": d.primary_dim,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    Json(serde_json::json!({
        "manas": manas,
        "entropy": { "current": current, "dissolution_history": history },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vijnana_query_deserializes() {
        let q: VijnanaQuery = serde_json::from_str(r#"{"session_id": "sess-1"}"#).unwrap();
        assert_eq!(q.session_id, Some("sess-1".into()));
    }

    #[test]
    fn vijnana_query_defaults() {
        let q: VijnanaQuery = serde_json::from_str(r#"{}"#).unwrap();
        assert!(q.session_id.is_none());
    }
}

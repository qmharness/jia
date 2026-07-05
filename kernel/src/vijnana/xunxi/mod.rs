//! xunxi — Consolidation / Habituation (熏习)

use std::sync::Arc;
pub mod coactivation;
pub mod distillation;
pub mod signal;

use crate::palaces::gen_store::Store;
use crate::palaces::zhong_core::JiaCore;
use crate::stems::Stem;
use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore};
use crate::vijnana::mano::TurnSnapshot;

/// Slow-path consolidation engine.
///
/// Runs after the agent loop, using the LLM to extract causal relationships,
/// entity edges, and user preferences from turn snapshots. Each extracted fact
/// becomes a `Seed` stored in the Alaya Store.
pub struct ConsolidationEngine;

impl ConsolidationEngine {
    /// Run consolidation asynchronously. Returns the number of new seeds created.
    ///
    /// Only triggers when at least 3 snapshots are available.
    pub async fn run(
        session_id: String,
        snapshots: Vec<TurnSnapshot>,
        store: Arc<Store>,
        core: &JiaCore,
    ) -> Result<u64, String> {
        if snapshots.is_empty() {
            return Ok(0);
        }

        let prompt = build_consolidation_prompt(&snapshots);
        let system_msg = crate::types::Message::text(
            crate::types::Role::System,
            "You are a memory consolidation engine. Extract structured facts from agent execution traces. Output ONLY valid JSON — an array of fact objects. No other text.",
        );

        use futures::StreamExt;
        const MAX_RETRIES: u32 = 3;
        let mut response = String::new();
        let mut last_error = String::new();
        let mut facts: Vec<ConsolidatedFact> = Vec::new();

        for attempt in 0..MAX_RETRIES {
            let mut msgs = vec![system_msg.clone()];
            msgs.push(crate::types::Message::text(
                crate::types::Role::User,
                prompt.clone(),
            ));
            if attempt > 0 {
                msgs.push(crate::types::Message::text(
                    crate::types::Role::User,
                    format!(
                        "Previous attempt failed: {last_error}. Output ONLY a valid JSON array."
                    ),
                ));
            }

            response.clear();
            let mut stream_error = false;
            let mut stream = core.infer(msgs, None, None);
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta)) => {
                        response.push_str(&delta)
                    }
                    Ok(_) => {}
                    Err(e) => {
                        last_error = format!("stream error: {e}");
                        stream_error = true;
                        break;
                    }
                }
            }

            if stream_error {
                if attempt < MAX_RETRIES - 1 {
                    tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
                    continue;
                }
                return Err(last_error);
            }

            match serde_json::from_str::<Vec<ConsolidatedFact>>(&response) {
                Ok(f) => {
                    facts = f;
                    break;
                }
                Err(e) => {
                    // Fallback: try to extract JSON array from surrounding text
                    let trimmed = response.trim();
                    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']'))
                        && let Ok(f) = serde_json::from_str(&trimmed[start..=end])
                    {
                        facts = f;
                        break;
                    }
                    last_error = format!("JSON parse error: {e}");
                    if attempt < MAX_RETRIES - 1 {
                        tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
                    }
                }
            }
        }

        if facts.is_empty() && !last_error.is_empty() {
            tracing::warn!(
                "Consolidation: failed after {MAX_RETRIES} attempts ({} chars): {}...",
                response.len(),
                &response[..response.len().min(200)],
            );
            return Err(last_error);
        }

        // Derive geju_key from the source snapshots (use most common geju name)
        let geju_key = snapshots
            .first()
            .map(|s| s.geju_name.as_str())
            .filter(|n| !n.is_empty())
            .unwrap_or("consolidation");

        let project_id = store.session_project_id(&session_id).unwrap_or_default();
        let seed_store = SeedStore::new(store.clone());
        let mut count = 0u64;
        for fact in facts {
            let key = fact.key.as_deref().or(fact.subject.as_deref());
            let value = fact.value.as_deref().or(fact.object.as_deref());
            let content = match (fact.fact_type.as_deref(), key, value) {
                (Some("preference"), Some(k), Some(v)) => SeedContent::KeyValue {
                    key: k.to_string(),
                    value: v.to_string(),
                },
                (_, Some(s), Some(o)) if fact.predicate.is_some() => SeedContent::Triple {
                    subject: s.to_string(),
                    predicate: fact
                        .predicate
                        .as_deref()
                        .unwrap_or("relates_to")
                        .to_string(),
                    object: o.to_string(),
                },
                _ => {
                    let fallback = fact.text.unwrap_or_else(|| format!("{:?}", key));
                    SeedContent::FreeText { text: fallback }
                }
            };

            let seed = Seed::new(
                session_id.clone(),
                project_id.clone(),
                SeedNature::Inference,
                SeedSource::Consolidation,
                content,
                crate::palaces::Palace::Gen, // Store palace
                Stem::Gui,                   // Store intent
                geju_key.to_string(),
            );

            if seed_store.insert(&seed).is_ok() {
                count += 1;
            }
        }

        tracing::info!(
            "Consolidation: created {count} seeds from {} snapshots",
            snapshots.len()
        );
        Ok(count)
    }
}

#[derive(Debug, serde::Deserialize)]
#[allow(dead_code)]
#[derive(Default)]
struct ConsolidatedFact {
    #[serde(rename = "type")]
    fact_type: Option<String>,
    key: Option<String>,
    value: Option<String>,
    subject: Option<String>,
    predicate: Option<String>,
    object: Option<String>,
    text: Option<String>,
}

fn build_consolidation_prompt(snapshots: &[TurnSnapshot]) -> String {
    let mut lines = vec![
        "Extract structured facts from these agent execution traces. Output a JSON array.".into(),
        String::new(),
    ];

    for snap in snapshots {
        lines.push(format!(
            "Turn {}: tool={}, geju={}, mode={}, output={}",
            snap.turn_number,
            snap.tool_name,
            snap.geju_name,
            snap.execution_mode,
            truncate(&snap.tool_output, 200),
        ));
    }

    lines.push(String::new());
    lines.push(
        r#"Output format: [{"type":"causal", "subject":"...", "predicate":"...", "object":"..."}, {"type":"preference", "key":"...", "value":"..."}, {"type":"entity", "subject":"...", "predicate":"is_a", "object":"..."}]

Focus on:
- Causal relationships (what caused what)
- Entity facts (X is a Y)
- User preferences (user likes/wants X)
- Procedures that succeeded or failed

Output ONLY the JSON array, no other text."#.into(),
    );

    lines.join("\n")
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let end = s.floor_char_boundary(max_len);
        &s[..end]
    }
}

#[cfg(test)]
#[path = "tests/consolidation_stress.rs"]
mod consolidation_stress;

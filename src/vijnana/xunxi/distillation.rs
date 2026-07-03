use std::sync::Arc;
use std::collections::HashSet;
use std::time::Duration;

use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::palaces::Palace;
use crate::palaces::gen_store::Store;
use crate::palaces::zhong_core::JiaCore;
use crate::stems::Stem;
use crate::types::{HistoryEntry, Message, Role};
use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore};

/// DistillationEngine — extracts reusable insights from completed exchanges.
///
/// Runs after the agent loop, feeding (user query, assistant response) pairs
/// to a lightweight LLM to produce compact knowledge seeds.
pub struct DistillationEngine;

impl DistillationEngine {
    /// Run distillation over session history.
    ///
    /// Returns the set of new content-hashes and the count of seeds created.
    /// The caller is responsible for merging hashes and persisting them.
    #[tracing::instrument(skip(history, distilled_hashes, store, core), fields(session = %session_id))]
    pub async fn run(
        session_id: &str,
        history: &[HistoryEntry],
        distilled_hashes: &HashSet<u64>,
        store: &Arc<Store>,
        core: &JiaCore,
    ) -> (HashSet<u64>, usize) {
        let seed_store = SeedStore::new(store.clone());
        let mut new_hashes = HashSet::new();
        let mut seeds_created = 0usize;
        let mut last_user: Option<&str> = None;

        for entry in history {
            match entry {
                HistoryEntry::User { content, .. } => last_user = Some(content.as_str()),
                HistoryEntry::Assistant { content } => {
                    if let Some(query) = last_user.take() {
                        let pair_hash = fnv1a_hash(&format!("{}|{}", query, content));
                        if distilled_hashes.contains(&pair_hash) || new_hashes.contains(&pair_hash)
                        {
                            continue;
                        }
                        if let Some(thought) = distill_thought(core, query, content, None).await
                            && !is_redundant(&seed_store, &thought, core, None).await
                        {
                            let seed = Seed::new(
                                session_id.to_string(),
                                SeedNature::Inference,
                                SeedSource::Consolidation,
                                SeedContent::FreeText { text: thought },
                                Palace::Gen,
                                Stem::Gui,
                                "thought_distillation".into(),
                            );
                            if let Err(e) = seed_store.insert(&seed) {
                                tracing::warn!(error = %e, "Failed to store distilled thought");
                            } else {
                                seeds_created += 1;
                            }
                        }
                        new_hashes.insert(pair_hash);
                    }
                }
                HistoryEntry::System { .. } | HistoryEntry::ToolCall { .. } => {}
            }
        }

        (new_hashes, seeds_created)
    }
}

/// Extract one reusable insight from a (query, response) exchange.
/// Returns `None` if the response is too short or the LLM outputs SKIP.
async fn distill_thought(
    core: &JiaCore,
    query: &str,
    response: &str,
    cancel_token: Option<CancellationToken>,
) -> Option<String> {
    if response.len() < 50 {
        return None;
    }

    let prompt = format!(
        "Based on this interaction:\n  User: {query}\n  Assistant: {response}\n\n\
         Distill one reusable insight from this exchange.\n\
         If the response is merely \"I don't know\" or cannot be answered, output only: SKIP\n\
         Otherwise output 1-2 sentences capturing the key knowledge."
    );

    let messages = vec![Message::text(Role::User, prompt)];
    let inference = async {
        let mut stream = core.infer(messages, None, cancel_token);
        let mut thought = String::new();
        while let Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) =
            stream.next().await
        {
            thought.push_str(&delta);
        }
        thought
    };
    let thought = match tokio::time::timeout(Duration::from_secs(30), inference).await {
        Ok(t) => t,
        Err(_) => {
            tracing::warn!("DistillationEngine: distill_thought timed out after 30s");
            return None;
        }
    };

    let thought = thought.trim().to_string();
    if thought.is_empty() || thought == "SKIP" {
        return None;
    }
    Some(thought)
}

/// FNV-1a 64-bit deterministic hash (same input → same hash across process restarts).
pub(crate) fn fnv1a_hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Check if `new_text` is semantically redundant with existing seeds.
/// Layer 1: FNV-1a hash exact-match (0 LLM calls).
/// Layer 2: FTS5 candidate retrieval → LLM pairwise comparison.
/// Early-exits on first YES match.
async fn is_redundant(
    seed_store: &SeedStore,
    new_text: &str,
    core: &JiaCore,
    cancel_token: Option<CancellationToken>,
) -> bool {
    let candidates = match seed_store.search_similar_texts(new_text, 5) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let new_hash = fnv1a_hash(new_text);

    for c in &candidates {
        if c.is_empty() {
            continue;
        }
        if fnv1a_hash(c) == new_hash {
            return true;
        }
        let prompt = format!(
            "Do these two texts express the same information?\nA: {new_text}\nB: {c}\nAnswer only YES or NO."
        );
        let messages = vec![Message::text(Role::User, prompt)];
        let inference = async {
            let mut stream = core.infer(messages, None, cancel_token.clone());
            let mut answer = String::new();
            while let Some(Ok(crate::palaces::zhong_core::StreamChunk::Delta(delta))) =
                stream.next().await
            {
                answer.push_str(&delta);
            }
            answer
        };
        let answer = match tokio::time::timeout(Duration::from_secs(10), inference).await {
            Ok(a) => a,
            Err(_) => {
                tracing::warn!("DistillationEngine: redundancy check timed out after 10s");
                continue; // conservative: don't mark as redundant on timeout
            }
        };
        if answer.trim().to_uppercase().contains("YES") {
            return true;
        }
    }
    false
}

// ── 艮藏 RetrieveCompacted Tool — 巽影艮藏 CCR retrieval ──────

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::palaces::gen_store::Store;
use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::CeremoniesIntent;
use crate::stems::action::ExecContext;
use crate::stems::intent::StoreAction;

/// 巽影艮藏: retrieve original messages that were archived during
/// 丙奇 compaction. The hash is injected into the checkpoint marker
/// as `[艮藏: hash]` — the LLM calls this tool to recover full context.
pub struct RetrieveCompactedTool {
    store: Arc<Store>,
}

impl RetrieveCompactedTool {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl BaseTool for RetrieveCompactedTool {
    fn name(&self) -> &str {
        "retrieve_compacted"
    }

    fn description(&self) -> String {
        "Retrieve original conversation messages that were archived during context compaction. \
         Use this when the compaction checkpoint lacks details you need. \
         The hash can be found in the checkpoint marker as [艮藏: hash]."
            .to_string()
    }

    fn category(&self) -> &str {
        "记忆"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Gui(StoreAction {
            key: "retrieve_compacted".into(),
            value: String::new(),
        })
    }

    /// Read-only retrieval — never blocks on ShangMen gate.
    fn is_destructive(&self) -> bool {
        false
    }

    /// Pure retrieval, safe to call concurrently.
    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "hash": {
                    "type": "string",
                    "description": "The hash key from the [艮藏: hash] marker in a compaction checkpoint."
                }
            },
            "required": ["hash"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, String> {
        let hash = input["hash"]
            .as_str()
            .ok_or("Missing required parameter: hash")?;

        if hash.len() != 16 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid hash format: expected 16 hex characters".into());
        }

        match self.store.get_compaction_backup(hash) {
            Ok(Some(content)) => Ok(content),
            Ok(None) => Err(format!(
                "No compaction backup found for hash: {hash}. \
                 It may have expired (TTL: 30 minutes) or never existed."
            )),
            Err(e) => Err(format!("Failed to retrieve compaction backup: {e}")),
        }
    }
}

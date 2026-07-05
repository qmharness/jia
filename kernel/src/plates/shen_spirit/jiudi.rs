//! 九地 (JiuDi) — 坚牢稳固，深藏地基，上下文压缩与系统稳定性观测。

use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;

pub struct JiudiHook;

#[async_trait]
impl Hook for JiudiHook {
    fn name(&self) -> &str {
        "jiudi"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::JiuDi]
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        if let HookEvent::CompactionTriggered {
            messages_before,
            messages_after,
            tokens_before,
            tokens_after,
            method,
        } = &event
        {
            tracing::info!(
                messages_before = messages_before,
                messages_after = messages_after,
                tokens_before = tokens_before,
                tokens_after = tokens_after,
                method = method.as_str(),
                "九地: compaction triggered"
            );
        }
        HookResult::Ok
    }
}

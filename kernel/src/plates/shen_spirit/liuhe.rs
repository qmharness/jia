//! 六合 (LiuHe) — 护卫和合，聚合统一，轮次整合基线观测。

use async_trait::async_trait;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};

pub struct LiuheHook;

#[async_trait]
impl Hook for LiuheHook {
    fn name(&self) -> &str { "liuhe" }
    fn spirit_types(&self) -> Vec<SpiritType> { vec![SpiritType::LiuHe] }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        if let HookEvent::BatchEnded { tool_count, turn, .. } = &event {
            tracing::info!(tool_count = tool_count, turn = turn, "六合: batch ended");
        }
        HookResult::Ok
    }
}

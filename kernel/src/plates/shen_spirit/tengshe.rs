//! 螣蛇 (TengShe) — 虚妄虚诈，变幻非实，LLM 响应观测。

use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;

pub struct TengsheHook;

#[async_trait]
impl Hook for TengsheHook {
    fn name(&self) -> &str {
        "tengshe"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::TengShe]
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        if let HookEvent::LlmResponse {
            response_len,
            tool_call_count,
            certainty: _,
        } = &event
        {
            tracing::info!(
                response_len = response_len,
                tool_call_count = tool_call_count,
                "螣蛇: LLM response"
            );
        }
        HookResult::Ok
    }
}

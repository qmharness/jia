//! 值符 (ZhiFu) — 百神之首，统率全局，工具生命周期守卫与观测。

use async_trait::async_trait;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};

pub struct ZhifuHook;

#[async_trait]
impl Hook for ZhifuHook {
    fn name(&self) -> &str { "zhifu" }
    fn spirit_types(&self) -> Vec<SpiritType> { vec![SpiritType::ZhiFu] }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        match &event {
            HookEvent::ToolPreExecute { tool_name, input } => {
                tracing::info!(tool = %tool_name, input = %input, "值符: tool pre-execute");
            }
            HookEvent::ToolPostExecute { tool_name, output, error, duration_ms } => {
                if let Some(err) = error {
                    tracing::warn!(tool = %tool_name, error = %err, duration_ms = duration_ms, "值符: tool post-execute (error)");
                } else {
                    tracing::info!(tool = %tool_name, output_len = output.len(), duration_ms = duration_ms, "值符: tool post-execute");
                }
            }
            _ => {}
        }
        HookResult::Ok
    }
}

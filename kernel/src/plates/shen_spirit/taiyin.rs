//! 太阴 (TaiYin) — 确定度轨迹 + 种子激活迹观测。
//!
//! 经典含义：荫佑护持，隐秘，内在，不显。
//! Jia 观测维度：确定度轨迹 + 种子激活迹——内部隐藏的动态。

use crate::plates::shen_spirit::EventBus;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;

pub struct TaiYinHook {
    event_bus: EventBus,
}

impl TaiYinHook {
    pub fn new(event_bus: EventBus) -> Self {
        Self { event_bus }
    }
}

#[async_trait]
impl Hook for TaiYinHook {
    fn name(&self) -> &str {
        "tai_yin"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::TaiYin]
    }
    fn matcher(&self) -> Option<&str> {
        None
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        match event {
            HookEvent::BatchEnded {
                tool_count, turn, ..
            } => {
                self.event_bus.emit(RuntimeEvent::SeedDynamicsSnapshot {
                    turn,
                    activated_count: tool_count,
                    top_seeds: vec![],
                });
            }
            HookEvent::LlmResponse { certainty, .. } => {
                if let Some(c) = certainty {
                    self.event_bus.emit(RuntimeEvent::CertaintyTrace {
                        turn: 0,
                        c_task: 0.0,
                        c_open: 0.0,
                        composite: c,
                    });
                }
            }
            _ => {}
        }
        HookResult::Ok
    }
}

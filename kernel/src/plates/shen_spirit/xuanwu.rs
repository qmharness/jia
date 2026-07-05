//! 玄武 (XuanWu) — 记忆损失观测。
//!
//! 经典含义：贼盗阴私，暗流，水属性隐没。
//! Jia 观测维度：不可恢复之失——压缩丢弃、坐忘删除、蒸馏去重。

use crate::plates::shen_spirit::EventBus;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;

pub struct XuanWuHook {
    event_bus: EventBus,
    compaction_tokens_lost: std::sync::atomic::AtomicU64,
}

impl XuanWuHook {
    pub fn new(event_bus: EventBus) -> Self {
        Self {
            event_bus,
            compaction_tokens_lost: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl Hook for XuanWuHook {
    fn name(&self) -> &str {
        "xuan_wu"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::XuanWu]
    }
    fn matcher(&self) -> Option<&str> {
        None
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        match event {
            HookEvent::CompactionTriggered {
                tokens_before,
                tokens_after,
                ..
            } => {
                let lost = (tokens_before.saturating_sub(tokens_after)) as u64;
                self.compaction_tokens_lost
                    .fetch_add(lost, std::sync::atomic::Ordering::Relaxed);
            }
            HookEvent::BatchEnded {
                tool_count: _,
                turn: _,
                ..
            } => {
                let lost = self
                    .compaction_tokens_lost
                    .swap(0, std::sync::atomic::Ordering::Relaxed);
                if lost > 0 {
                    self.event_bus.emit(RuntimeEvent::MemoryLossRecord {
                        compaction_tokens_lost: lost,
                        seeds_deleted: 0,
                        seeds_downgraded: 0,
                        distillation_dedup_pairs: 0,
                    });
                }
            }
            _ => {}
        }
        HookResult::Ok
    }
}

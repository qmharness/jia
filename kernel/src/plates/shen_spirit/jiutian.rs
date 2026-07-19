//! 九天 (JiuTian) — 策略涌现观测。
//!
//! 经典含义：威悍扬兵，高远，战略俯瞰。
//! Jia 观测维度：跨轮次轨迹序列、GeJu 模式识别。
//! 纯观测 hook，不托管 JiaCore。LLM 策略分析通过 aux_core 路由到 post_loop。

use crate::plates::shen_spirit::EventBus;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;
use std::sync::Mutex;

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct TrajectoryPoint {
    turn: u64,
    geju_name: String,
    execution_mode: String,
    #[allow(dead_code)]
    composite_certainty: Option<f32>,
}

pub struct JiuTianHook {
    event_bus: EventBus,
    trajectory: Mutex<Vec<TrajectoryPoint>>,
    enabled: bool,
}

impl JiuTianHook {
    pub fn new(event_bus: EventBus, enabled: bool) -> Self {
        Self {
            event_bus,
            trajectory: Mutex::new(Vec::new()),
            enabled,
        }
    }
}

#[async_trait]
impl Hook for JiuTianHook {
    fn name(&self) -> &str {
        "jiu_tian"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::JiuTian]
    }
    fn matcher(&self) -> Option<&str> {
        None
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        if !self.enabled {
            return HookResult::Ok;
        }
        match event {
            HookEvent::BatchEnded {
                tool_count: _,
                turn,
                ..
            } => {
                let traj = self.trajectory.lock().unwrap_or_else(|e| e.into_inner());
                if traj.len() >= 5 {
                    let summary = traj
                        .iter()
                        .map(|p| format!("t{}:{}", p.turn, p.geju_name))
                        .collect::<Vec<_>>()
                        .join(" → ");
                    self.event_bus.emit(RuntimeEvent::StrategyInsight {
                        turn,
                        trajectory_summary: summary,
                    });
                }
            }
            _ => {}
        }
        HookResult::Ok
    }
}

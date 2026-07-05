//! 白虎 (BaiHu) — 异常/认知病理检测。
//!
//! 经典含义：凶杀威权，危险，金属肃杀。
//! Jia 观测维度：连续失败、检索环、确定度骤降——四级 gate 门控。

use crate::plates::shen_spirit::EventBus;
use crate::plates::shen_spirit::RuntimeEvent;
use crate::plates::shen_spirit::hook::{Hook, HookEvent, HookResult, SpiritType};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct BaiHuConfig {
    pub blocking_enabled: bool,
    pub severity_threshold: f32,
    pub consecutive_failure_window: usize,
    pub consecutive_failure_rate: f32,
    pub retrieval_loop_window: usize,
    pub retrieval_loop_repeat: usize,
    pub certainty_crash_delta: f32,
}

impl Default for BaiHuConfig {
    fn default() -> Self {
        Self {
            blocking_enabled: false,
            severity_threshold: 0.9,
            consecutive_failure_window: 10,
            consecutive_failure_rate: 0.6,
            retrieval_loop_window: 5,
            retrieval_loop_repeat: 3,
            certainty_crash_delta: 0.3,
        }
    }
}

pub struct BaiHuHook {
    config: BaiHuConfig,
    event_bus: EventBus,
    failure_history: Mutex<VecDeque<bool>>,
    call_history: Mutex<VecDeque<(String, u64)>>,
    certainty_history: Mutex<VecDeque<f32>>,
}

impl BaiHuHook {
    pub fn new(config: BaiHuConfig, event_bus: EventBus) -> Self {
        Self {
            config,
            event_bus,
            failure_history: Mutex::new(VecDeque::new()),
            call_history: Mutex::new(VecDeque::new()),
            certainty_history: Mutex::new(VecDeque::new()),
        }
    }

    fn compute_severity(&self) -> f32 {
        let failures = self.failure_history.lock().unwrap();
        let failure_rate = if failures.len() >= self.config.consecutive_failure_window {
            let recent: Vec<_> = failures
                .iter()
                .rev()
                .take(self.config.consecutive_failure_window)
                .collect();
            recent.iter().filter(|&&f| !f).count() as f32 / recent.len() as f32
        } else {
            0.0
        };

        let calls = self.call_history.lock().unwrap();
        let loop_detected = calls.len() >= self.config.retrieval_loop_repeat as f32 as usize;

        let certainty = self.certainty_history.lock().unwrap();
        let crash = if certainty.len() >= 2 {
            let last = certainty.back().copied().unwrap_or(0.0);
            let prev = certainty.iter().rev().nth(1).copied().unwrap_or(0.0);
            if prev - last > self.config.certainty_crash_delta {
                1.0
            } else {
                0.0
            }
        } else {
            0.0
        };

        failure_rate
            .max(if loop_detected { 1.0 } else { 0.0 })
            .max(crash)
    }
}

#[async_trait]
impl Hook for BaiHuHook {
    fn name(&self) -> &str {
        "bai_hu"
    }
    fn spirit_types(&self) -> Vec<SpiritType> {
        vec![SpiritType::BaiHu]
    }
    fn block_on_failure(&self) -> bool {
        self.config.blocking_enabled
    }

    async fn on_event(&self, event: HookEvent) -> HookResult {
        match event {
            HookEvent::ToolPostExecute {
                tool_name: _,
                output: _,
                error,
                duration_ms: _,
            } => {
                let mut h = self.failure_history.lock().unwrap();
                h.push_back(error.is_none());
                if h.len() > 20 {
                    h.pop_front();
                }
            }
            HookEvent::ToolPreExecute { tool_name, input } => {
                let hash = crate::vijnana::vasana::distillation::fnv1a_hash(&format!(
                    "{}|{}",
                    tool_name, input
                ));
                let mut h = self.call_history.lock().unwrap();
                h.push_back((tool_name, hash));
                if h.len() > 20 {
                    h.pop_front();
                }
            }
            HookEvent::LlmResponse { certainty, .. } => {
                if let Some(c) = certainty {
                    let mut h = self.certainty_history.lock().unwrap();
                    h.push_back(c);
                    if h.len() > self.config.consecutive_failure_window {
                        h.pop_front();
                    }
                }
            }
            HookEvent::BatchEnded {
                tool_count: _,
                turn,
                ..
            } => {
                let severity = self.compute_severity();
                if severity > 0.0 {
                    let anomaly_type = if severity > self.config.severity_threshold {
                        "critical"
                    } else {
                        "warning"
                    };
                    self.event_bus.emit(RuntimeEvent::BehavioralAlert {
                        severity,
                        anomaly_type: anomaly_type.to_string(),
                        turn,
                    });
                }
                if severity > self.config.severity_threshold && self.config.blocking_enabled {
                    return HookResult::Cancel("BaiHu: critical anomaly detected".into());
                }
            }
            _ => {}
        }
        HookResult::Ok
    }
}

//! 行为 TurnCertainty — 从可观测行为合成确定度，驱动自适应终止。
//!
//! 不读模型内部概率，全 provider 通用、无外部 API 依赖。
//!
//! 哲学锚点：儒家的"信"——知之为知之。确定度是 agent 对自身的诚实。

use crate::vijnana::mano::TurnSnapshot;

/// 确定度评估结果与循环决策。
#[derive(Debug, Clone)]
pub struct TurnCertainty {
    /// 任务侧确定度（行为信号合成）
    pub c_task: f32,
    /// 开放度（1 − atma_graha，末那识自我模型）
    pub c_open: f32,
    /// 复合确定度
    pub composite: f32,
    /// 循环决策
    pub decision: LoopDecision,
}

/// 确定度驱动的循环决策。
#[derive(Debug, Clone, PartialEq)]
pub enum LoopDecision {
    /// 继续下一轮
    Continue,
    /// 确信完成——自适应终止
    ConfidentStop,
    /// 低确定度——升级为人工介入
    EscalateToHuman,
    /// 达到硬上限——安全网终止
    HardLimitReached,
}

/// 确定度计算的可调参数。
#[derive(Debug, Clone)]
pub struct CertaintyParams {
    /// 行为信号滑动窗口大小（轮数）
    pub window_size: usize,
    /// ConfidentStop 的 composite 阈值
    pub theta_high: f32,
    /// EscalateToHuman 的 composite 阈值
    pub theta_low: f32,
    /// c_task 在 composite 中的权重
    pub alpha: f32,
    /// c_open 在 composite 中的权重
    pub beta: f32,
    /// tool_success_rate 的权重
    pub w1: f32,
    /// no_tool_run 的权重
    pub w2: f32,
    /// output_stability 的权重
    pub w3: f32,
}

impl Default for CertaintyParams {
    fn default() -> Self {
        Self {
            window_size: 10,
            theta_high: 0.80,
            theta_low: 0.30,
            alpha: 0.70,
            beta: 0.30,
            w1: 0.20,
            w2: 0.50,
            w3: 0.30,
        }
    }
}

impl TurnCertainty {
    /// 从 WorkingMemory 最近 K 轮 + 当前轮导出信号，计算确定度与循环决策。
    ///
    /// # Arguments
    /// * `snapshots` — WorkingMemory 中的全部 TurnSnapshot（最近 N 轮）。
    /// * `atma_graha` — 末那识我执值。
    /// * `turn_count` — 当前轮次编号。
    /// * `max_turns` — 最大轮次上限。
    /// * `params` — 可调参数（权重与阈值）。
    pub fn evaluate(
        snapshots: &[TurnSnapshot],
        atma_graha: f32,
        turn_count: u32,
        max_turns: u32,
        params: &CertaintyParams,
    ) -> Self {
        let window: Vec<&TurnSnapshot> = snapshots.iter().rev().take(params.window_size).collect();

        // ── 信号 1：工具成功率 ──
        let tool_success_rate = if window.is_empty() {
            1.0
        } else {
            let ok_count = window
                .iter()
                .filter(|s| s.tool_error.is_none() && !s.tool_name.is_empty())
                .count();
            let total = window.iter().filter(|s| !s.tool_name.is_empty()).count();
            if total == 0 {
                1.0
            } else {
                ok_count as f32 / total as f32
            }
        };

        // ── 信号 2：连续无工具调用轮数 ──
        let no_tool_run = {
            let mut count = 0u32;
            for s in snapshots.iter().rev() {
                if s.tool_name.is_empty() {
                    count += 1;
                } else {
                    break;
                }
            }
            // 归一化：连续 3 轮无工具调用 → 1.0
            (count as f32 / 3.0).min(1.0)
        };

        // ── 信号 3：输出长度稳定性 ──
        let output_stability = {
            let lens: Vec<f32> = window
                .iter()
                .filter(|s| !s.tool_output.is_empty())
                .map(|s| s.tool_output.len() as f32)
                .collect();
            if lens.len() < 2 {
                1.0
            } else {
                let mean = lens.iter().sum::<f32>() / lens.len() as f32;
                let variance =
                    lens.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / lens.len() as f32;
                let normalized = variance / (mean.powi(2) + 1.0);
                (1.0 - normalized).max(0.0)
            }
        };

        // ── 复合 ──
        let c_task =
            params.w1 * tool_success_rate + params.w2 * no_tool_run + params.w3 * output_stability;

        // c_open = 1 − atma_graha
        // 低我执=信任积累的记忆和工具结果=行为趋向完成
        // 高我执=固执自我=行为趋向继续
        // 标为"开放度"而非"自信"：释放我执不等于构建自信
        let c_open = 1.0 - atma_graha;

        let composite = params.alpha * c_task + params.beta * c_open;

        // ── 决策 ──
        let decision = if turn_count > max_turns {
            LoopDecision::HardLimitReached
        } else if composite > params.theta_high && no_tool_run > 0.0 {
            LoopDecision::ConfidentStop
        } else if composite < params.theta_low && turn_count > 3 {
            LoopDecision::EscalateToHuman
        } else {
            LoopDecision::Continue
        };

        Self {
            c_task,
            c_open,
            composite,
            decision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::Palace;
    use crate::stems::Stem;

    fn make_snapshot(tool_name: &str, tool_error: Option<&str>, tool_output: &str) -> TurnSnapshot {
        TurnSnapshot {
            turn_number: 0,
            intent_stem: Stem::Wu,
            target_palace: Palace::Zhen,
            geju_name: String::new(),
            execution_mode: String::new(),
            tool_name: tool_name.to_string(),
            tool_input: serde_json::Value::Null,
            tool_output: tool_output.to_string(),
            tool_error: tool_error.map(|s| s.to_string()),
            timestamp: 0,
            certainty: None,
            active_seed_ids: vec![],
            tool_count: if tool_name.is_empty() { 0 } else { 1 },
        }
    }

    #[test]
    fn confident_stop_after_clean_tool_runs() {
        let snapshots: Vec<TurnSnapshot> = (0..5)
            .map(|i| make_snapshot("shell", None, &format!("output-{}", i)))
            .collect();
        let result = TurnCertainty::evaluate(
            &snapshots,
            0.20, // low atma-graha = open
            6,
            25,
            &CertaintyParams::default(),
        );
        assert!(
            result.composite > 0.5,
            "clean runs should yield reasonable certainty"
        );
    }

    #[test]
    fn escalate_when_tools_fail() {
        let mut snapshots: Vec<TurnSnapshot> = (0..8)
            .map(|i| make_snapshot("shell", Some("command not found"), ""))
            .collect();
        // Add a few no-tool turns at the end to trigger the no_tool_run signal
        snapshots.push(make_snapshot("", None, ""));
        snapshots.push(make_snapshot("", None, ""));
        let result = TurnCertainty::evaluate(
            &snapshots,
            0.70, // high atma-graha = closed
            10,
            25,
            &CertaintyParams::default(),
        );
        assert!(
            result.composite < 0.5,
            "failing tools + high atma-graha should yield low certainty"
        );
    }

    #[test]
    fn hard_limit_when_exceeding_max_turns() {
        let snapshots: Vec<TurnSnapshot> = vec![];
        let result = TurnCertainty::evaluate(&snapshots, 0.20, 30, 25, &CertaintyParams::default());
        assert_eq!(result.decision, LoopDecision::HardLimitReached);
    }

    #[test]
    fn no_tool_run_sequence_drives_confident_stop() {
        let mut snapshots: Vec<TurnSnapshot> = vec![];
        // 3 clean tool runs, then 3 no-tool turns
        for i in 0..3 {
            snapshots.push(make_snapshot("shell", None, &format!("ok-{}", i)));
        }
        for _ in 0..3 {
            snapshots.push(make_snapshot("", None, ""));
        }
        let result = TurnCertainty::evaluate(
            &snapshots,
            0.15, // very open
            7,
            25,
            &CertaintyParams::default(),
        );
        assert_eq!(result.decision, LoopDecision::ConfidentStop);
    }
}

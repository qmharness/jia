//! 上下文重置 Context Reset — 巽四·己，长会话续命。
//!
//! 补既有压缩（艮藏 compaction）的第二条续命路。
//! 压缩就地保连续，重置给干净起点。

use serde::{Deserialize, Serialize};

/// 上下文重置协调器。
#[derive(Debug, Clone)]
pub struct ContextReset {
    /// Reset 后 N 轮内不触发压缩
    pub cooldown_turns: usize,
    /// 上次 reset 的轮次编号
    pub last_reset_at: u64,
    /// 上次 compaction 的轮次编号
    pub last_compaction_at: u64,
}

impl Default for ContextReset {
    fn default() -> Self {
        Self {
            cooldown_turns: 5,
            last_reset_at: 0,
            last_compaction_at: 0,
        }
    }
}

/// 结构化交接产物。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffArtifact {
    pub goals: String,
    pub done: Vec<String>,
    pub todo: Vec<String>,
    pub key_decisions: Vec<String>,
    pub obstacles: Vec<String>,
}

impl ContextReset {
    pub fn new(cooldown_turns: usize) -> Self {
        Self {
            cooldown_turns,
            last_reset_at: 0,
            last_compaction_at: 0,
        }
    }

    /// 判断是否应触发 reset。
    pub fn should_reset(&self, tokens_used: usize, max_tokens: usize, current_turn: u64) -> bool {
        if current_turn.saturating_sub(self.last_reset_at) < self.cooldown_turns as u64 {
            return false;
        }
        if current_turn.saturating_sub(self.last_compaction_at) < 3 {
            return false;
        }
        let threshold = (max_tokens as f64 * 0.85) as usize;
        tokens_used > threshold
    }

    /// 生成结构化 handoff（简单版——从历史提取摘要，不使用 LLM）。
    pub fn generate_handoff_stub(turn_count: u64, task_description: &str) -> HandoffArtifact {
        HandoffArtifact {
            goals: task_description.to_string(),
            done: vec![format!("Completed {} turns of work", turn_count)],
            todo: vec!["Continue from handoff".to_string()],
            key_decisions: vec![],
            obstacles: vec![],
        }
    }

    /// 标记 reset 已发生。
    pub fn mark_reset(&mut self, turn: u64) {
        self.last_reset_at = turn;
    }

    /// 标记 compaction 已发生。
    pub fn mark_compaction(&mut self, turn: u64) {
        self.last_compaction_at = turn;
    }
}

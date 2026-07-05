//! mano — Working Memory (意识)

use serde::{Deserialize, Serialize};

use crate::palaces::Palace;
use crate::stems::Stem;

/// Working memory — per-turn snapshots from the current agent session.
///
/// Fixed-capacity ring buffer: new snapshots evict the oldest when at capacity.
/// Uses Vec with remove(0) — O(1) amortized push, O(n) eviction. Negligible for
/// the typical capacity of 20.
#[derive(Debug, Clone)]
pub struct WorkingMemory {
    pub snapshots: Vec<TurnSnapshot>,
    max_snapshots: usize,
}

/// A single turn's execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub turn_number: u64,
    pub intent_stem: Stem,
    pub target_palace: Palace,
    pub geju_name: String,
    pub execution_mode: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub tool_output: String,
    pub tool_error: Option<String>,
    pub timestamp: i64,
    /// TurnCertainty composite score (set when certainty evaluation runs).
    #[serde(default)]
    pub certainty: Option<f32>,
    /// IDs of seeds touched during prompt construction this turn.
    #[serde(default)]
    pub active_seed_ids: Vec<String>,
    /// Number of tool calls in this turn.
    #[serde(default)]
    pub tool_count: u32,
}

impl WorkingMemory {
    pub fn new(max_snapshots: usize) -> Self {
        Self {
            snapshots: Vec::with_capacity(max_snapshots),
            max_snapshots,
        }
    }

    pub fn record(&mut self, snapshot: TurnSnapshot) {
        if self.snapshots.len() >= self.max_snapshots {
            self.snapshots.remove(0);
        }
        self.snapshots.push(snapshot);
    }

    pub fn len(&self) -> usize {
        self.snapshots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snap(turn: u64, tool: &str, geju: &str) -> TurnSnapshot {
        TurnSnapshot {
            turn_number: turn,
            intent_stem: Stem::Geng,
            target_palace: Palace::Zhen,
            geju_name: geju.into(),
            execution_mode: "Direct".into(),
            tool_name: tool.into(),
            tool_input: serde_json::Value::Null,
            tool_output: "ok".into(),
            tool_error: None,
            timestamp: turn as i64,
        }
    }

    #[test]
    fn record_and_retrieve_snapshots() {
        let mut wm = WorkingMemory::new(20);
        wm.record(make_snap(1, "read_file", "feiniao"));
        wm.record(make_snap(2, "shell", "taibai"));
        assert_eq!(wm.len(), 2);
        assert_eq!(wm.snapshots[0].turn_number, 1);
        assert_eq!(wm.snapshots[1].turn_number, 2);
    }

    #[test]
    fn ring_buffer_evicts_oldest_on_overflow() {
        let mut wm = WorkingMemory::new(3);
        for i in 0..5 {
            wm.record(make_snap(i, "tool", "geju"));
        }
        assert_eq!(wm.len(), 3);
        // Turns 0 and 1 should be evicted; remaining: 2, 3, 4
        assert_eq!(wm.snapshots[0].turn_number, 2);
        assert_eq!(wm.snapshots[1].turn_number, 3);
        assert_eq!(wm.snapshots[2].turn_number, 4);
    }

    #[test]
    fn ring_buffer_at_exact_capacity() {
        let mut wm = WorkingMemory::new(3);
        for i in 0..3 {
            wm.record(make_snap(i, "t", "g"));
        }
        assert_eq!(wm.len(), 3);
        assert_eq!(wm.snapshots[0].turn_number, 0);
        assert_eq!(wm.snapshots[2].turn_number, 2);
    }

    #[test]
    fn empty_working_memory_is_empty() {
        let wm = WorkingMemory::new(10);
        assert_eq!(wm.len(), 0);
        assert!(wm.snapshots.is_empty());
    }

    #[test]
    fn preserves_all_snapshot_fields() {
        let mut wm = WorkingMemory::new(5);
        let snap = TurnSnapshot {
            turn_number: 42,
            intent_stem: Stem::Xin,
            target_palace: Palace::Xun,
            geju_name: "xin_jia_xun".into(),
            execution_mode: "Guarded".into(),
            tool_name: "edit".into(),
            tool_input: serde_json::json!({"path": "src/main.rs"}),
            tool_output: "edited".into(),
            tool_error: Some("permission denied".into()),
            timestamp: 999,
        };
        wm.record(snap);
        assert_eq!(wm.len(), 1);
        let s = &wm.snapshots[0];
        assert_eq!(s.turn_number, 42);
        assert_eq!(s.intent_stem, Stem::Xin);
        assert_eq!(s.target_palace, Palace::Xun);
        assert_eq!(s.geju_name, "xin_jia_xun");
        assert_eq!(s.execution_mode, "Guarded");
        assert_eq!(s.tool_name, "edit");
        assert_eq!(s.tool_error.as_deref(), Some("permission denied"));
        assert_eq!(s.timestamp, 999);
    }
}

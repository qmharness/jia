//! At-least-once 去重窗口。
//!
//! iLink / Telegram 均为 at-least-once 投递:崩溃恢复、offset 未确认、
//! 长轮询重试都可能让同一条消息重复到达。`DedupWindow` 按消息 ID 在
//! TTL 窗口内去重,与 wechat 原 `seen_msg_ids`(300 s)语义一致。

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

pub(crate) struct DedupWindow<K> {
    seen: HashMap<K, Instant>,
    ttl: Duration,
}

impl<K: Hash + Eq> DedupWindow<K> {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            ttl,
        }
    }

    /// 记录 `id`;若 TTL 窗口内已见过则返回 true(重复,应跳过)。
    /// 每次调用顺手清理过期条目,避免无界增长。
    pub(crate) fn is_duplicate(&mut self, id: K, now: Instant) -> bool {
        self.seen
            .retain(|_, ts| now.saturating_duration_since(*ts) < self.ttl);
        if self.seen.contains_key(&id) {
            return true;
        }
        self.seen.insert(id, now);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_seen_is_not_duplicate() {
        let mut w = DedupWindow::new(Duration::from_secs(300));
        let now = Instant::now();
        assert!(!w.is_duplicate("a", now));
    }

    #[test]
    fn resend_within_ttl_is_duplicate() {
        let mut w = DedupWindow::new(Duration::from_secs(300));
        let now = Instant::now();
        assert!(!w.is_duplicate("a", now));
        assert!(w.is_duplicate("a", now + Duration::from_secs(10)));
    }

    #[test]
    fn resend_after_ttl_is_not_duplicate() {
        let mut w = DedupWindow::new(Duration::from_secs(300));
        let now = Instant::now();
        assert!(!w.is_duplicate("a", now));
        assert!(!w.is_duplicate("a", now + Duration::from_secs(301)));
    }

    #[test]
    fn distinct_ids_do_not_collide() {
        let mut w = DedupWindow::new(Duration::from_secs(300));
        let now = Instant::now();
        assert!(!w.is_duplicate("a", now));
        assert!(!w.is_duplicate("b", now));
        assert!(w.is_duplicate("a", now));
    }

    #[test]
    fn expired_entries_are_pruned() {
        let mut w = DedupWindow::new(Duration::from_secs(300));
        let now = Instant::now();
        w.is_duplicate("old", now);
        // 触发一次带新时间的调用,旧条目应被清理
        w.is_duplicate("new", now + Duration::from_secs(400));
        assert_eq!(w.seen.len(), 1);
        assert!(w.seen.contains_key(&"new"));
    }

    #[test]
    fn works_with_integer_keys() {
        let mut w: DedupWindow<u64> = DedupWindow::new(Duration::from_secs(300));
        let now = Instant::now();
        assert!(!w.is_duplicate(42, now));
        assert!(w.is_duplicate(42, now));
    }
}

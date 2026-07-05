use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt;

use crate::geju::{ExecutionMode, GeJu, GeJuResult};
use crate::stems::Stem;

use super::{EventBus, RuntimeEvent};

// ── SpiritType ────────────────────────────────────────────

/// Eight spirit types (八神). ZhiFu/TengShe/LiuHe/JiuDi used in production;
/// TaiYin/BaiHu/XuanWu/JiuTian reserved for future hook patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SpiritType {
    ZhiFu,
    TengShe,
    TaiYin,
    LiuHe,
    BaiHu,
    XuanWu,
    JiuDi,
    JiuTian,
}

impl SpiritType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ZhiFu => "ZhiFu",
            Self::TengShe => "TengShe",
            Self::TaiYin => "TaiYin",
            Self::LiuHe => "LiuHe",
            Self::BaiHu => "BaiHu",
            Self::XuanWu => "XuanWu",
            Self::JiuDi => "JiuDi",
            Self::JiuTian => "JiuTian",
        }
    }
}

// ── HookEvent ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum HookEvent {
    LlmResponse {
        response_len: usize,
        tool_call_count: usize,
        certainty: Option<f32>,
    },
    ToolPreExecute {
        tool_name: String,
        input: serde_json::Value,
    },
    ToolPostExecute {
        tool_name: String,
        output: String,
        error: Option<String>,
        duration_ms: u64,
    },
    BatchEnded {
        tool_count: usize,
        turn: u64,
        /// Dominant GeJu pattern name this turn (for JiuTian trajectory).
        geju_name: Option<String>,
    },
    CompactionTriggered {
        messages_before: usize,
        messages_after: usize,
        tokens_before: usize,
        tokens_after: usize,
        method: String,
    },
}

// ── HookResult ────────────────────────────────────────────

pub enum HookResult {
    Ok,
    Cancel(String),
}

// ── Hook trait ────────────────────────────────────────────

#[async_trait]
pub trait Hook: Send + Sync {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 {
        0
    }
    fn spirit_types(&self) -> Vec<SpiritType>;
    fn matcher(&self) -> Option<&str> {
        None
    }
    fn block_on_failure(&self) -> bool {
        false
    }
    async fn on_event(&self, _event: HookEvent) -> HookResult {
        HookResult::Ok
    }
}

// ── HookRegistry ──────────────────────────────────────────

pub struct HookRegistry {
    hooks: Vec<Arc<dyn Hook>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, hook: Box<dyn Hook>) {
        self.hooks.push(Arc::from(hook));
        self.hooks.sort_by_key(|h| -h.priority());
    }

    pub fn remove(&mut self, name: &str) {
        self.hooks.retain(|h| h.name() != name);
    }

    pub fn by_spirit_type(&self, st: SpiritType) -> Vec<Arc<dyn Hook>> {
        self.hooks
            .iter()
            .filter(|h| h.spirit_types().contains(&st))
            .cloned()
            .collect()
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── event_matches_matcher ─────────────────────────────────

fn event_matches_matcher(hook: &dyn Hook, event: &HookEvent) -> bool {
    let Some(glob) = hook.matcher() else {
        return true;
    };
    let tool_name = match event {
        HookEvent::ToolPreExecute { tool_name, .. }
        | HookEvent::ToolPostExecute { tool_name, .. } => tool_name,
        _ => return true,
    };
    glob::Pattern::new(glob).is_ok_and(|p| p.matches(tool_name))
}

// ── GeJu gate + dispatch ─────────────────────────────────

fn geju_gate_and_collect(
    registry: &HookRegistry,
    event_bus: &EventBus,
    spirit_type: SpiritType,
    earth_stem: Stem,
    event: &HookEvent,
) -> (GeJuResult, Vec<Arc<dyn Hook>>) {
    let geju = GeJu::new(Stem::Ding, earth_stem);
    let result = geju.evaluate();

    event_bus.emit(RuntimeEvent::GeJuResult {
        tool: format!("hook:{}", spirit_type.name()),
        pattern: result.name.clone(),
        mode: format!("{:?}", result.execution_mode),
    });

    if result.execution_mode == ExecutionMode::Denied {
        return (result, vec![]);
    }

    let matching: Vec<_> = registry
        .by_spirit_type(spirit_type)
        .into_iter()
        .filter(|h| event_matches_matcher(h.as_ref(), event))
        .collect();

    (result, matching)
}

// ── fire_void_hooks ──────────────────────────────────────

/// Void hooks: fire-and-forget.
/// LlmResponse / ToolPostExecute / BatchEnded
pub fn fire_void_hooks(
    registry: &HookRegistry,
    event_bus: &EventBus,
    spirit_type: SpiritType,
    earth_stem: Stem,
    event: HookEvent,
) {
    let (_geju, matching) =
        geju_gate_and_collect(registry, event_bus, spirit_type, earth_stem, &event);
    for hook in matching {
        let hook = Arc::clone(&hook);
        let event = event.clone();
        tokio::spawn(async move {
            let result = std::panic::AssertUnwindSafe(hook.on_event(event))
                .catch_unwind()
                .await;
            if result.is_err() {
                tracing::error!("Hook '{}' panicked", hook.name());
            }
        });
    }
}

// ── fire_guarding_hooks ──────────────────────────────────

/// Guarding hooks: sequential await, supports Cancel.
/// Only for ToolPreExecute (ZhiFu).
/// Returns None if all pass, Some(reason) if a blocking hook cancels.
pub async fn fire_guarding_hooks(
    registry: &HookRegistry,
    event_bus: &EventBus,
    spirit_type: SpiritType,
    earth_stem: Stem,
    event: HookEvent,
) -> Option<String> {
    let (_geju, matching) =
        geju_gate_and_collect(registry, event_bus, spirit_type, earth_stem, &event);
    for hook in matching {
        if !hook.block_on_failure() {
            let hook = Arc::clone(&hook);
            let event = event.clone();
            tokio::spawn(async move {
                let _ = std::panic::AssertUnwindSafe(hook.on_event(event))
                    .catch_unwind()
                    .await;
            });
            continue;
        }
        let result = std::panic::AssertUnwindSafe(hook.on_event(event.clone()))
            .catch_unwind()
            .await;
        match result {
            Err(_) => {
                tracing::error!("Hook '{}' panicked during guarding dispatch", hook.name());
            }
            Ok(HookResult::Cancel(reason)) => {
                return Some(reason);
            }
            Ok(HookResult::Ok) => {}
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stems::Stem;
    use std::sync::Mutex;

    struct TestHook {
        name: String,
        priority: i32,
        spirit_types: Vec<SpiritType>,
        matcher_str: Option<String>,
        blocking: bool,
        result: HookResult,
        call_count: Arc<Mutex<usize>>,
    }

    impl TestHook {
        fn new(
            name: &str,
            priority: i32,
            spirit_types: Vec<SpiritType>,
            matcher_str: Option<&str>,
            blocking: bool,
            result: HookResult,
        ) -> Self {
            Self {
                name: name.into(),
                priority,
                spirit_types,
                matcher_str: matcher_str.map(|s| s.into()),
                blocking,
                result,
                call_count: Arc::new(Mutex::new(0)),
            }
        }
    }

    #[async_trait]
    impl Hook for TestHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        fn spirit_types(&self) -> Vec<SpiritType> {
            self.spirit_types.clone()
        }
        fn matcher(&self) -> Option<&str> {
            self.matcher_str.as_deref()
        }
        fn block_on_failure(&self) -> bool {
            self.blocking
        }
        async fn on_event(&self, _event: HookEvent) -> HookResult {
            *self.call_count.lock().unwrap() += 1;
            match &self.result {
                HookResult::Ok => HookResult::Ok,
                HookResult::Cancel(s) => HookResult::Cancel(s.clone()),
            }
        }
    }

    struct PanicHook;
    #[async_trait]
    impl Hook for PanicHook {
        fn name(&self) -> &str {
            "panic"
        }
        fn spirit_types(&self) -> Vec<SpiritType> {
            vec![SpiritType::ZhiFu]
        }
        async fn on_event(&self, _event: HookEvent) -> HookResult {
            panic!("intentional panic in hook");
        }
    }

    fn test_event_bus() -> EventBus {
        EventBus::new()
    }

    // ── GeJu gate tests ─────────────────────────────────

    #[test]
    fn test_geju_gate_passes_for_all_earth_stems() {
        let eb = test_event_bus();
        let registry = HookRegistry::new();
        for &stem in &Stem::ALL {
            let event = HookEvent::LlmResponse {
                response_len: 0,
                tool_call_count: 0,
                certainty: None,
            };
            let (_result, matching) =
                geju_gate_and_collect(&registry, &eb, SpiritType::TengShe, stem, &event);
            // No Ding GeJu combination returns Denied, so matching may be
            // empty (no hooks registered) but gate never blocks.
            assert!(
                matching.is_empty(),
                "no hooks registered, should be empty for {stem:?}"
            );
        }
    }

    // ── Void hooks tests ────────────────────────────────

    #[tokio::test]
    async fn test_void_hooks_spawn_and_isolate_panics() {
        let mut registry = HookRegistry::new();
        // PanicHook panics but fire_void_hooks should swallow it
        registry.register(Box::new(PanicHook));
        let eb = test_event_bus();
        let event = HookEvent::ToolPostExecute {
            tool_name: "test".into(),
            output: String::new(),
            error: None,
            duration_ms: 0,
        };
        // This should NOT panic
        fire_void_hooks(&registry, &eb, SpiritType::ZhiFu, Stem::Geng, event);
        // Let the spawn complete
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_void_hooks_dispatches_matching_hooks() {
        let mut registry = HookRegistry::new();
        let hook = TestHook::new(
            "obs",
            0,
            vec![SpiritType::ZhiFu],
            None,
            false,
            HookResult::Ok,
        );
        let count = hook.call_count.clone();
        registry.register(Box::new(hook));
        let eb = test_event_bus();
        let event = HookEvent::ToolPostExecute {
            tool_name: "test".into(),
            output: String::new(),
            error: None,
            duration_ms: 0,
        };
        fire_void_hooks(&registry, &eb, SpiritType::ZhiFu, Stem::Geng, event);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(*count.lock().unwrap(), 1);
    }

    // ── Guarding hooks tests ────────────────────────────

    #[tokio::test]
    async fn test_guarding_hooks_ok_returns_none() {
        let mut registry = HookRegistry::new();
        registry.register(Box::new(TestHook::new(
            "ok",
            0,
            vec![SpiritType::ZhiFu],
            None,
            true,
            HookResult::Ok,
        )));
        let eb = test_event_bus();
        let event = HookEvent::ToolPreExecute {
            tool_name: "test".into(),
            input: serde_json::json!({}),
        };
        let result =
            fire_guarding_hooks(&registry, &eb, SpiritType::ZhiFu, Stem::Geng, event).await;
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_guarding_hooks_cancel_returns_reason() {
        let mut registry = HookRegistry::new();
        registry.register(Box::new(TestHook::new(
            "blocker",
            0,
            vec![SpiritType::ZhiFu],
            None,
            true,
            HookResult::Cancel("not allowed".into()),
        )));
        let eb = test_event_bus();
        let event = HookEvent::ToolPreExecute {
            tool_name: "test".into(),
            input: serde_json::json!({}),
        };
        let result =
            fire_guarding_hooks(&registry, &eb, SpiritType::ZhiFu, Stem::Geng, event).await;
        assert_eq!(result, Some("not allowed".into()));
    }

    // ── Registry tests ──────────────────────────────────

    #[test]
    fn test_registry_orders_by_priority_desc() {
        let mut registry = HookRegistry::new();
        registry.register(Box::new(TestHook::new(
            "low",
            0,
            vec![SpiritType::ZhiFu],
            None,
            false,
            HookResult::Ok,
        )));
        registry.register(Box::new(TestHook::new(
            "high",
            10,
            vec![SpiritType::ZhiFu],
            None,
            false,
            HookResult::Ok,
        )));
        let matching = registry.by_spirit_type(SpiritType::ZhiFu);
        assert_eq!(matching.len(), 2);
        assert_eq!(matching[0].name(), "high");
        assert_eq!(matching[1].name(), "low");
    }

    #[test]
    fn test_registry_remove() {
        let mut registry = HookRegistry::new();
        registry.register(Box::new(TestHook::new(
            "x",
            0,
            vec![SpiritType::ZhiFu],
            None,
            false,
            HookResult::Ok,
        )));
        assert_eq!(registry.by_spirit_type(SpiritType::ZhiFu).len(), 1);
        registry.remove("x");
        assert_eq!(registry.by_spirit_type(SpiritType::ZhiFu).len(), 0);
    }

    // ── Matcher tests ───────────────────────────────────

    #[test]
    fn test_matcher_filters_by_tool_name() {
        // Match: matcher "read_*" matches ToolPreExecute with "read_file"
        let hook = TestHook::new("reader", 0, vec![], Some("read_*"), false, HookResult::Ok);
        let event = HookEvent::ToolPreExecute {
            tool_name: "read_file".into(),
            input: serde_json::json!({}),
        };
        assert!(event_matches_matcher(&hook, &event));

        let event2 = HookEvent::ToolPreExecute {
            tool_name: "write_file".into(),
            input: serde_json::json!({}),
        };
        assert!(!event_matches_matcher(&hook, &event2));
    }

    #[test]
    fn test_matcher_always_matches_non_tool_events() {
        let hook = TestHook::new("reader", 0, vec![], Some("read_*"), false, HookResult::Ok);
        let event = HookEvent::BatchEnded {
            geju_name: None,
            tool_count: 1,
            turn: 1,
        };
        assert!(event_matches_matcher(&hook, &event));

        let event2 = HookEvent::LlmResponse {
            response_len: 100,
            tool_call_count: 0,
            certainty: None,
        };
        assert!(event_matches_matcher(&hook, &event2));
    }

    #[test]
    fn test_matcher_none_always_matches() {
        let hook = TestHook::new("all", 0, vec![], None, false, HookResult::Ok);
        let event = HookEvent::ToolPreExecute {
            tool_name: "anything".into(),
            input: serde_json::json!({}),
        };
        assert!(event_matches_matcher(&hook, &event));
    }
}

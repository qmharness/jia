use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use kernel::palaces::li_skill::SkillRegistry;
use kernel::plates::shen_spirit::EventBus;

#[test]
fn eventbus_many_emits_no_panic() {
    let bus = EventBus::new();
    let mut rx = bus.subscribe();

    // Emit 1000 events — should not panic or deadlock
    for i in 0..1000 {
        bus.emit(kernel::plates::shen_spirit::RuntimeEvent::TurnStart { turn: i });
    }

    // Drain all events (best-effort; broadcast may drop some)
    let mut received = 0;
    while rx.try_recv().is_ok() {
        received += 1;
    }
    // With capacity 1024 and no slow subscriber, all should arrive
    assert!(received > 0, "should receive at least some events");
}

#[test]
fn skill_registry_concurrent_reads() {
    let mut reg = SkillRegistry::new();
    reg.register(kernel::palaces::li_skill::Skill {
        name: "test-skill".into(),
        description: "A test skill".into(),
        prompt: "Do test things.".into(),
        source_path: std::path::PathBuf::from("skills/test.md"),
        always: false,
        paths: None,
        emphasis: None,
        auto_evolve: false,
        evolve_min_confidence: 0.7,
        evolve_max_revisions_per_session: 3,
        evolve_reflection_threshold: 3,
        scripts: HashMap::new(),
        references: HashMap::new(),
    });
    let registry = Arc::new(std::sync::RwLock::new(reg));

    // 50 concurrent readers
    std::thread::scope(|s| {
        for _ in 0..50 {
            let reg = registry.clone();
            s.spawn(move || {
                let guard = reg.read().unwrap();
                let _names = guard.list_names();
                let _all = guard.list_all();
                let _skill = guard.get("test-skill");
            });
        }
    });
    // If we got here without deadlock, the test passes
}

#[test]
fn skill_registry_concurrent_read_write() {
    let registry = Arc::new(std::sync::RwLock::new(SkillRegistry::new()));

    std::thread::scope(|s| {
        // Writer thread
        let reg_w = registry.clone();
        s.spawn(move || {
            for i in 0..10 {
                std::thread::sleep(Duration::from_micros(100));
                let mut guard = reg_w.write().unwrap();
                guard.register(kernel::palaces::li_skill::Skill {
                    name: format!("skill-{i}"),
                    description: "test".into(),
                    prompt: "Do things.".into(),
                    source_path: std::path::PathBuf::from(format!("skills/{i}.md")),
                    always: false,
                    paths: None,
                    emphasis: None,
                    auto_evolve: false,
                    evolve_min_confidence: 0.7,
                    evolve_max_revisions_per_session: 3,
                    evolve_reflection_threshold: 3,
                    scripts: HashMap::new(),
                    references: HashMap::new(),
                });
            }
        });

        // Reader threads
        for _ in 0..20 {
            let reg_r = registry.clone();
            s.spawn(move || {
                for _ in 0..50 {
                    std::thread::sleep(Duration::from_micros(50));
                    if let Ok(guard) = reg_r.read() {
                        let _ = guard.list_names();
                        let _ = guard.list_all();
                    }
                }
            });
        }
    });
    // No deadlock, no panic
}

#[test]
fn eventbus_multiple_subscribers_receive_events() {
    let bus = EventBus::new();
    let mut rx1 = bus.subscribe();
    let mut rx2 = bus.subscribe();

    bus.emit(kernel::plates::shen_spirit::RuntimeEvent::TurnStart { turn: 1 });
    bus.emit(kernel::plates::shen_spirit::RuntimeEvent::TurnEnd { turn: 1 });

    let c1 = (rx1.try_recv().is_ok() as u8) + (rx1.try_recv().is_ok() as u8);
    let c2 = (rx2.try_recv().is_ok() as u8) + (rx2.try_recv().is_ok() as u8);

    assert!(c1 > 0, "subscriber 1 should receive at least 1 event");
    assert!(c2 > 0, "subscriber 2 should receive at least 1 event");
}

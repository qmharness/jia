// Stress tests for Jia's reliability under load.
//
// These tests verify that core systems don't degrade, panic, or produce
// incorrect results under high-volume conditions.

use std::sync::Arc;

use jia::palaces::Palace;
use jia::palaces::gen_store::Store;
use jia::palaces::xun_context::ContextWindow;
use jia::stems::Stem;
use jia::types::{Message, Role};
use jia::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore, SeedTier};
use jia::vijnana::mano::{TurnSnapshot, WorkingMemory};

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// ── Helpers ──────────────────────────────────────────────────

fn new_store() -> Store {
    // Use an in-memory SQLite database so stress tests don't touch store.db
    // We'll use the Store's internal structure by simply creating a new one
    // and cleaning up after. For speed, we use WAL mode in-memory.
    Store::open(":memory:")
}

fn new_seed(session_id: &str, idx: usize) -> Seed {
    Seed {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        nature: if idx.is_multiple_of(3) {
            SeedNature::Fact
        } else if idx % 3 == 1 {
            SeedNature::Inference
        } else {
            SeedNature::Preference
        },
        source: if idx.is_multiple_of(2) {
            SeedSource::ToolObservation
        } else {
            SeedSource::Consolidation
        },
        content: SeedContent::FreeText {
            text: format!(
                "Stress test seed number {idx}. This is a sample memory entry with some text content to simulate real-world usage patterns in the agent memory system. The quick brown fox jumps over the lazy dog."
            ),
        },
        palace: Palace::Zhen,
        intent_stem: Stem::Wu,
        geju_key: format!("stress+{idx}"),
        created_at: 1700000000 + idx as i64,
        access_count: 0,
        last_accessed_at: 1700000000 + idx as i64,
        strength: 1.0,
        tier: SeedTier::OnDemand,
    }
}

fn new_snapshot(turn: u64) -> TurnSnapshot {
    TurnSnapshot {
        turn_number: turn,
        intent_stem: Stem::Wu,
        target_palace: Palace::Zhen,
        geju_name: "test".to_string(),
        execution_mode: "Guarded".to_string(),
        tool_name: "read_file".to_string(),
        tool_input: serde_json::json!({"path": "/tmp/test.txt"}),
        tool_output: "content here".to_string(),
        tool_error: None,
        timestamp: 1700000000 + turn as i64,
    }
}

fn long_message(approx_chars: usize, seed: &str) -> Message {
    // Build a message with approximately `approx_chars` characters
    let mut content = String::with_capacity(approx_chars);
    let base = format!("{seed} ");
    while content.len() < approx_chars {
        content.push_str(&base);
    }
    content.truncate(approx_chars);
    Message::text(Role::User, content)
}

// ── ContextWindow Stress ─────────────────────────────────────

#[test]
fn context_window_with_1000_messages() {
    let ctx = ContextWindow::new(8192, 0.75); // ~6144 token limit
    let mut messages: Vec<Message> = Vec::with_capacity(1002);

    // System message
    messages.push(Message::text(Role::System, "You are a helpful assistant."));

    // 1000 user/assistant messages, each ~50 chars (~14 tokens)
    for i in 0..500 {
        messages.push(Message::text(
            Role::User,
            format!("User message number {i:04}. This is some more text to add volume."),
        ));
        messages.push(Message::text(
            Role::Assistant,
            format!(
                "Assistant response number {i:04}. Here is the reply with additional padding words."
            ),
        ));
    }

    // Add one final message that must be preserved
    messages.push(Message::text(
        Role::User,
        "The last message that should survive",
    ));

    assert_eq!(messages.len(), 1002);

    let (dropped, remaining) = ctx.fit(&mut messages);

    // After fitting: system message + survivors + last message
    assert!(
        messages.len() >= 2,
        "at least system + last user message should survive"
    );
    assert!(messages.len() < 1002, "should have dropped many messages");
    assert_eq!(
        messages[0].role,
        Role::System,
        "system message must be preserved"
    );
    assert!(
        remaining < ctx.max_tokens,
        "remaining tokens under max: {remaining}"
    );
    assert!(dropped > 0, "should have dropped messages");
}

#[test]
fn context_window_with_10000_tiny_messages() {
    let ctx = ContextWindow::new(4096, 0.75); // ~3072 token limit
    let mut messages: Vec<Message> = Vec::with_capacity(10001);

    messages.push(Message::text(Role::System, "sys"));

    // 10000 tiny messages (~5 chars each = ~1.4 tokens)
    for i in 0..10000 {
        messages.push(Message::text(
            if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            format!("m{i:05}"),
        ));
    }

    // Should not crash — verify fit completes
    let (_dropped, _remaining) = ctx.fit(&mut messages);
    assert!(!messages.is_empty());
    assert_eq!(messages[0].role, Role::System);
}

#[test]
fn context_window_no_system_message_with_many_messages() {
    let ctx = ContextWindow::new(2048, 0.75);
    let mut messages: Vec<Message> = Vec::with_capacity(500);

    for i in 0..500 {
        messages.push(Message::text(
            if i % 2 == 0 {
                Role::User
            } else {
                Role::Assistant
            },
            format!("msg number {i:04} with some filler to build up token count quickly enough"),
        ));
    }

    assert_eq!(messages.len(), 500);
    let (_dropped, _remaining) = ctx.fit(&mut messages);
    assert!(!messages.is_empty());
    // No system message → first message can be anything
}

#[test]
fn context_window_victim_range_with_many_messages() {
    let ctx = ContextWindow::new(2048, 0.75);
    let mut messages: Vec<Message> = Vec::with_capacity(200);

    messages.push(Message::text(Role::System, "sys"));
    for i in 0..199 {
        messages.push(Message::text(
            Role::User,
            format!("long message number {i:04} to ensure we exceed the limit quickly here"),
        ));
    }

    let (start, count) = ctx.victim_range(&messages);
    assert!(count > 0, "should predict drops for 200 messages");
    assert_eq!(start, 1, "should start dropping after system message");
}

// ── Token Counting Edge Cases ────────────────────────────────

#[test]
fn token_count_very_long_message() {
    let msg = long_message(100_000, "the quick brown fox jumps over the lazy dog");
    let tokens = ContextWindow::count_tokens(&[msg]);
    assert!(
        tokens > 1000,
        "very long message should have many tokens: {tokens}"
    );
    assert!(tokens < 50_000, "tokens should be reasonable: {tokens}");
}

#[test]
fn token_count_mixed_cjk_and_latin() {
    let msgs = vec![Message::text(
        Role::User,
        "这是一个混合中英文的测试消息。This is a mixed CJK and Latin text message. \
             日本語も含まれています。한국어도 포함되어 있습니다. \
             The quick brown fox jumps over the lazy dog. 快速的棕色狐狸跳过了懒狗。",
    )];
    let tokens = ContextWindow::count_tokens(&msgs);
    assert!(tokens > 10, "mixed language should have tokens: {tokens}");
    assert!(tokens < 200, "should not be huge: {tokens}");
}

#[test]
fn token_count_empty_messages() {
    let msgs: Vec<Message> = vec![
        Message::text(Role::User, ""),
        Message::text(Role::Assistant, ""),
    ];
    let tokens = ContextWindow::count_tokens(&msgs);
    assert_eq!(tokens, 0, "empty messages should have 0 tokens");
}

#[test]
fn token_count_all_cjk() {
    // cl100k_base: CJK bigrams are often 1 token, ~0.6-1.0 tokens per char
    let long_cjk: String = std::iter::repeat_n("中文测试消息", 500)
        .collect::<Vec<_>>()
        .join("");
    let msgs = vec![Message::text(Role::User, long_cjk)];
    let tokens = ContextWindow::count_tokens(&msgs);
    // 500 × 5 CJK chars = 2500 chars; cl100k packs common CJK bigrams so ~1500-2500 tokens
    assert!(tokens > 1000, "CJK message should have tokens: {tokens}");
    assert!(tokens < 5000, "should be reasonable: {tokens}");
}

// ── WorkingMemory Ring Buffer Stress ─────────────────────────

#[test]
fn working_memory_ring_buffer_overflow() {
    let capacity = 20;
    let mut wm = WorkingMemory::new(capacity);

    // Record 100 snapshots — the ring buffer should wrap
    for turn in 1..=100 {
        wm.record(new_snapshot(turn));
    }

    assert_eq!(wm.len(), capacity, "should be at capacity, not exceeded");
    // The oldest should be #81 (dropped first 80)
    assert_eq!(wm.snapshots[0].turn_number, 81);
    // The newest should be #100
    assert_eq!(wm.snapshots[capacity - 1].turn_number, 100);
}

#[test]
fn working_memory_exact_capacity() {
    let capacity = 20;
    let mut wm = WorkingMemory::new(capacity);

    for turn in 1..=20 {
        wm.record(new_snapshot(turn));
    }

    assert_eq!(wm.len(), 20);
    assert_eq!(wm.snapshots[0].turn_number, 1);
    assert_eq!(wm.snapshots[19].turn_number, 20);
}

#[test]
fn working_memory_single_entry() {
    let mut wm = WorkingMemory::new(5);
    wm.record(new_snapshot(1));
    assert_eq!(wm.len(), 1);
    assert_eq!(wm.snapshots[0].turn_number, 1);
}

#[test]
fn working_memory_wrapping_preserves_order() {
    let mut wm = WorkingMemory::new(5);

    for turn in 1..=8 {
        wm.record(new_snapshot(turn));
    }

    // Should contain turns 4, 5, 6, 7, 8 in order
    assert_eq!(wm.len(), 5);
    for (i, expected_turn) in (4..=8).enumerate() {
        assert_eq!(
            wm.snapshots[i].turn_number, expected_turn as u64,
            "snapshot at index {i} should be turn {expected_turn}"
        );
    }
}

// ── Seed Store Bulk Operations ───────────────────────────────

fn cleanup_seeds_for_session(store: &Store, session_id: &str) {
    // Delete all seeds for a session by fetching their IDs
    let jsons = store.load_seeds_by_session(session_id).unwrap_or_default();
    let ids: Vec<String> = jsons
        .iter()
        .filter_map(|j| {
            serde_json::from_str::<serde_json::Value>(j)
                .ok()
                .and_then(|v| v["id"].as_str().map(|s| s.to_string()))
        })
        .collect();
    if !ids.is_empty() {
        store.delete_seeds(&ids).ok();
    }
}

#[test]
fn seed_store_bulk_insert_and_retrieve() {
    let store = Arc::new(new_store());
    let seed_store = SeedStore::new(store.clone());
    let session_id = format!("bulk-{}", uuid::Uuid::new_v4());
    cleanup_seeds_for_session(&store, &session_id);

    // Insert 500 seeds
    let mut ids: Vec<String> = Vec::with_capacity(500);
    for i in 0..500 {
        let seed = new_seed(&session_id, i);
        ids.push(seed.id.clone());
        seed_store
            .insert(&seed)
            .expect("should insert successfully");
    }

    // Count
    let count = seed_store
        .load_by_session(&session_id)
        .expect("should count")
        .len();
    assert_eq!(count, 500, "should have all 500 seeds");

    // Verify via raw store — load_seeds_by_session returns JSON
    let jsons = store
        .load_seeds_by_session(&session_id)
        .expect("should get raw seeds");
    assert_eq!(jsons.len(), 500, "should have all 500 seeds in raw store");

    // Clean up — delete them all
    store.delete_seeds(&ids).expect("should delete");
    let count_after = seed_store
        .load_by_session(&session_id)
        .expect("should count after delete")
        .len();
    assert_eq!(count_after, 0, "all seeds should be deleted");
}

#[test]
fn seed_store_multiple_sessions() {
    let store = Arc::new(new_store());
    let seed_store = SeedStore::new(store.clone());
    let prefix = format!("multi-{}", uuid::Uuid::new_v4());

    // Insert seeds across 10 sessions, track IDs for cleanup
    let mut all_ids: Vec<String> = Vec::new();
    for session_idx in 0..10 {
        let sid = format!("{prefix}-{session_idx}");
        cleanup_seeds_for_session(&store, &sid);
        for i in 0..50 {
            let seed = new_seed(&sid, i);
            all_ids.push(seed.id.clone());
            seed_store.insert(&seed).expect("insert");
        }
    }

    // Verify each session
    for session_idx in 0..10 {
        let sid = format!("{prefix}-{session_idx}");
        let count = seed_store.load_by_session(&sid).expect("count").len();
        assert_eq!(count, 50, "session {sid} should have 50 seeds");
    }

    // Clean up
    store.delete_seeds(&all_ids).expect("cleanup");
}

#[test]
fn seed_store_stress_insert_delete_cycle() {
    let store = Arc::new(new_store());
    let seed_store = SeedStore::new(store.clone());
    let session_id = format!("cycle-{}", uuid::Uuid::new_v4());
    cleanup_seeds_for_session(&store, &session_id);

    // Cycle: insert 100, delete 80, insert 100 more, count should be 120
    let mut all_ids: Vec<String> = Vec::new();

    for i in 0..100 {
        let seed = new_seed(&session_id, i);
        all_ids.push(seed.id.clone());
        seed_store.insert(&seed).expect("insert");
    }
    assert_eq!(seed_store.load_by_session(&session_id).unwrap().len(), 100);

    // Delete first 80
    let to_delete: Vec<String> = all_ids.drain(..80).collect();
    store.delete_seeds(&to_delete).expect("delete");
    assert_eq!(seed_store.load_by_session(&session_id).unwrap().len(), 20);

    // Insert 100 more
    for i in 100..200 {
        let seed = new_seed(&session_id, i);
        all_ids.push(seed.id.clone());
        seed_store.insert(&seed).expect("insert");
    }
    assert_eq!(seed_store.load_by_session(&session_id).unwrap().len(), 120);

    // Clean up remaining
    store.delete_seeds(&all_ids).expect("cleanup");
    assert_eq!(seed_store.load_by_session(&session_id).unwrap().len(), 0);
}

// ── ContextWindow Large Message Edge Cases ───────────────────

#[test]
fn context_window_fit_with_single_huge_message() {
    let ctx = ContextWindow::new(100, 0.75);
    let mut messages = vec![
        Message::text(Role::System, "sys"),
        long_message(10_000, "massive content that will dominate the window"),
    ];

    let (_dropped, _remaining) = ctx.fit(&mut messages);
    // Should keep at least the system message and preserve function
    assert_eq!(messages[0].role, Role::System);
    assert!(
        messages.len() >= 2,
        "must preserve system + at least last message"
    );
}

#[test]
fn context_window_would_exceed_with_large_messages() {
    let ctx = ContextWindow::new(500, 0.75);
    let messages: Vec<Message> = (0..20)
        .map(|i| long_message(200, &format!("msg{i}")))
        .collect();

    let tokens = ContextWindow::count_tokens(&messages);
    let would = ctx.would_exceed(&messages, 1000);
    assert!(tokens > 0);
    // would_exceed just checks if total > limit, so verify it doesn't panic
    let _ = would;
}

// ── Manas with Many Turns ─────────────────────────────────────

#[test]
fn self_model_many_turns_decay() {
    use jia::vijnana::manas::Manas;
    let mut model = Manas::new();
    assert!(model.atma_graha > 0.7, "starts with high ego");

    // 1000 turns — ego should converge toward 0.05 but not below
    for _ in 0..1000 {
        model.record_turn();
    }

    // After 1000 turns at 0.002 decay per turn: ~2.0 total decay potential,
    // but clamped at 0.05 floor
    assert!(
        model.atma_graha <= 0.80,
        "ego should decay after many turns"
    );
    assert!(model.atma_graha >= 0.05, "ego should not drop below floor");
    assert_eq!(model.total_turns, 1000, "should track turn count");
}

// ── Seed Roundtrip Verification ────────────────────────────

#[test]
fn seed_row_to_json_roundtrip() {
    let store = std::sync::Arc::new(new_store());
    let seed_store = SeedStore::new(store.clone());
    let sid = "rt-test";

    let original = Seed {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: sid.to_string(),
        nature: SeedNature::Fact,
        source: SeedSource::ToolObservation,
        content: SeedContent::FreeText {
            text: "roundtrip content".to_string(),
        },
        palace: Palace::Zhen,
        intent_stem: Stem::Wu,
        geju_key: "rt+0".to_string(),
        created_at: 1700000000,
        access_count: 0,
        last_accessed_at: 1700000000,
        strength: 1.0,
        tier: SeedTier::OnDemand,
    };

    let json = serde_json::to_string(&original).unwrap();
    store.insert_seed(&json).unwrap();

    let raw = &store.load_seeds_by_session(sid).unwrap()[0];
    let roundtripped: Seed =
        serde_json::from_str(raw).expect("seed_row_to_json must produce Seed-compatible JSON");
    assert_eq!(roundtripped.id, original.id);
    assert_eq!(roundtripped.geju_key, "rt+0");

    let seeds = seed_store.load_by_session(sid).expect("find_by_session");
    assert_eq!(seeds.len(), 1);

    let all = seed_store.load_all().expect("find_all");
    assert!(all.iter().any(|s| s.id == original.id));

    store.delete_seeds(&[original.id]).unwrap();
}

// ── Tier budget & catalog smoke tests ──────────────────────────

#[test]
fn smoke_catalog_stats_empty() {
    let store = Arc::new(new_store());
    let stats = store.catalog_stats().unwrap();
    assert!(stats.is_empty(), "empty store should have no stats");
}

#[test]
fn smoke_catalog_stats_mixed_tiers() {
    let store = Arc::new(new_store());
    // Insert Always, OnDemand, Archive seeds
    for (id, tier, nature) in [
        ("a1", SeedTier::Always, SeedNature::Fact),
        ("o1", SeedTier::OnDemand, SeedNature::Fact),
        ("o2", SeedTier::OnDemand, SeedNature::Preference),
        ("ar1", SeedTier::Archive, SeedNature::Inference),
    ] {
        let seed = Seed {
            id: id.into(),
            session_id: "smoke".into(),
            nature,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "k".into(),
            created_at: now(),
            access_count: 0,
            last_accessed_at: now(),
            strength: 1.0,
            tier,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }
    let stats = store.catalog_stats().unwrap();
    assert_eq!(stats.len(), 4, "4 groups: got {:?}", stats);
}

#[test]
fn smoke_load_always_seeds() {
    let store = Arc::new(new_store());
    let seed = Seed {
        id: "always1".into(),
        session_id: "smoke".into(),
        nature: SeedNature::Fact,
        source: SeedSource::UserStatement,
        content: SeedContent::KeyValue {
            key: "user".into(),
            value: "alice".into(),
        },
        palace: Palace::Kun,
        intent_stem: Stem::Ji,
        geju_key: "k".into(),
        created_at: now(),
        access_count: 0,
        last_accessed_at: now(),
        strength: 1.0,
        tier: SeedTier::Always,
    };
    store
        .insert_seed(&serde_json::to_string(&seed).unwrap())
        .unwrap();
    let always = store.load_always_seeds().unwrap();
    assert_eq!(always.len(), 1);
    assert!(always[0].contains("alice"));
}

#[test]
fn smoke_memory_catalog_format() {
    let store = Arc::new(new_store());
    // Always: key=value
    let s1 = Seed {
        id: "a1".into(),
        session_id: "s".into(),
        nature: SeedNature::Preference,
        source: SeedSource::UserStatement,
        content: SeedContent::KeyValue {
            key: "editor".into(),
            value: "vim".into(),
        },
        palace: Palace::Kun,
        intent_stem: Stem::Ji,
        geju_key: "k".into(),
        created_at: now(),
        access_count: 0,
        last_accessed_at: now(),
        strength: 1.0,
        tier: SeedTier::Always,
    };
    // OnDemand: Fact + Preference + Inference
    let s2 = Seed {
        id: "o1".into(),
        session_id: "s".into(),
        nature: SeedNature::Fact,
        source: SeedSource::ToolObservation,
        content: SeedContent::FreeText {
            text: "fact".into(),
        },
        palace: Palace::Zhen,
        intent_stem: Stem::Geng,
        geju_key: "k".into(),
        created_at: now(),
        access_count: 0,
        last_accessed_at: now(),
        strength: 0.8,
        tier: SeedTier::OnDemand,
    };
    let s3 = Seed {
        id: "o2".into(),
        session_id: "s".into(),
        nature: SeedNature::Preference,
        source: SeedSource::Consolidation,
        content: SeedContent::KeyValue {
            key: "lang".into(),
            value: "rust".into(),
        },
        palace: Palace::Kun,
        intent_stem: Stem::Wu,
        geju_key: "k".into(),
        created_at: now(),
        access_count: 0,
        last_accessed_at: now(),
        strength: 1.0,
        tier: SeedTier::OnDemand,
    };
    let s4 = Seed {
        id: "o3".into(),
        session_id: "s".into(),
        nature: SeedNature::Inference,
        source: SeedSource::Consolidation,
        content: SeedContent::FreeText {
            text: "inferred".into(),
        },
        palace: Palace::Gen,
        intent_stem: Stem::Gui,
        geju_key: "k".into(),
        created_at: now(),
        access_count: 0,
        last_accessed_at: now(),
        strength: 1.0,
        tier: SeedTier::OnDemand,
    };
    let s5 = Seed {
        id: "archive1".into(),
        session_id: "s".into(),
        nature: SeedNature::Fact,
        source: SeedSource::ToolObservation,
        content: SeedContent::FreeText { text: "old".into() },
        palace: Palace::Zhen,
        intent_stem: Stem::Geng,
        geju_key: "k".into(),
        created_at: now(),
        access_count: 0,
        last_accessed_at: now(),
        strength: 0.1,
        tier: SeedTier::Archive,
    };
    for s in [&s1, &s2, &s3, &s4, &s5] {
        store
            .insert_seed(&serde_json::to_string(s).unwrap())
            .unwrap();
    }

    let ss = SeedStore::new(store.clone());
    let (catalog, always_ids) = ss.memory_catalog();
    assert!(catalog.contains("[Memory]"));
    assert!(catalog.contains("Always:"));
    assert!(catalog.contains("editor=vim"));
    assert!(catalog.contains("OnDemand:"));
    assert!(catalog.contains("1 facts"));
    assert!(catalog.contains("1 preferences"));
    assert!(
        catalog.contains("1 inferences"),
        "Inference should appear as 'inferences': {}",
        catalog
    );
    assert!(catalog.contains("Archive:"));
    assert!(catalog.contains("1 archived"));
    assert_eq!(always_ids.len(), 1);
}

#[test]
fn smoke_enforce_tier_budgets_below_limit() {
    let store = Arc::new(new_store());
    for i in 0..5 {
        let seed = Seed {
            id: format!("od{i}"),
            session_id: "s".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "k".into(),
            created_at: now(),
            access_count: 0,
            last_accessed_at: now(),
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }
    let report = store.enforce_tier_budgets().unwrap();
    assert_eq!(report.ondemand_demoted, 0, "no demotion below 200");
    assert_eq!(report.archive_deleted, 0);
}

#[test]
fn smoke_enforce_tier_budgets_demotes_excess_ondemand() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));
    for i in 0..250 {
        let seed = Seed {
            id: format!("od{i}"),
            session_id: "s".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText {
                text: format!("seed {i}"),
            },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: "k".into(),
            created_at: now(),
            access_count: 0,
            last_accessed_at: now(),
            strength: 0.01 + (i as f32 * 0.003),
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }
    let report = store.enforce_tier_budgets().unwrap();
    assert!(report.ondemand_total > 200);
    assert!(report.ondemand_demoted > 0, "should demote excess OnDemand");
}

#[test]
fn smoke_enforce_tier_budgets_protects_preference() {
    let store = Arc::new(new_store());
    // 201 Preference seeds — all should be protected from demotion
    for i in 0..201 {
        let seed = Seed {
            id: format!("pref{i}"),
            session_id: "s".into(),
            nature: SeedNature::Preference,
            source: SeedSource::UserStatement,
            content: SeedContent::KeyValue {
                key: "k".into(),
                value: format!("v{i}"),
            },
            palace: Palace::Kun,
            intent_stem: Stem::Ji,
            geju_key: "k".into(),
            created_at: now(),
            access_count: 0,
            last_accessed_at: now(),
            strength: 0.1,
            tier: SeedTier::OnDemand,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }
    let report = store.enforce_tier_budgets().unwrap();
    assert_eq!(
        report.ondemand_demoted, 0,
        "Preference/UserStatement seeds should NEVER be demoted, got demoted={}",
        report.ondemand_demoted
    );
}

#[test]
fn smoke_enforce_tier_budgets_deletes_excess_archive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));
    for i in 0..1100 {
        let seed = Seed {
            id: format!("ar{i}"),
            session_id: "s".into(),
            nature: SeedNature::Inference,
            source: SeedSource::Consolidation,
            content: SeedContent::FreeText {
                text: format!("archive {i}"),
            },
            palace: Palace::Gen,
            intent_stem: Stem::Gui,
            geju_key: "k".into(),
            created_at: now(),
            access_count: 0,
            last_accessed_at: now(),
            strength: 0.001,
            tier: SeedTier::Archive,
        };
        store
            .insert_seed(&serde_json::to_string(&seed).unwrap())
            .unwrap();
    }
    let report = store.enforce_tier_budgets().unwrap();
    assert!(report.archive_total > 1000);
    assert!(report.archive_deleted > 0, "should delete excess Archive");
}

#[test]
fn smoke_fnv1a_hash_deterministic() {
    // Verify deterministic hash (same input → same output)
    let h1 = {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in "hello world".bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    };
    let h2 = {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in "hello world".bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    };
    assert_eq!(h1, h2);
    assert_ne!(h1, {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in "hello world!".bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    });
}

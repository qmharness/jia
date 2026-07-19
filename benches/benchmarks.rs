use criterion::{Criterion, black_box, criterion_group, criterion_main};
use jia::palaces::Palace;
use jia::palaces::gen_store::Store;
use jia::palaces::xun_context::ContextWindow;
use jia::stems::parse_tool_calls;
use jia::stems::Stem;
use jia::types::{HistoryEntry, Message, Role, to_llm_messages};
use jia::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore, SeedTier};
use jia::zuowang::trigger::AlayaEntropy;

fn bench_token_counting(c: &mut Criterion) {
    let messages: Vec<Message> = (0..50)
        .map(|i| Message::text(
            Role::User,
            format!("Message number {i} with some content that is reasonably long enough to produce several tokens per message."),
        ))
        .collect();

    c.bench_function("token_counting_50_messages", |b| {
        b.iter(|| {
            let count = ContextWindow::count_tokens(black_box(&messages));
            black_box(count)
        })
    });
}

fn bench_parse_tool_calls(c: &mut Criterion) {
    c.bench_function("parse_tool_calls_single", |b| {
        let input = r#"Some text before.
<tool_call>{"name": "read_file", "parameters": {"path": "/tmp/test.txt"}}</tool_call>"#;
        b.iter(|| {
            let result = parse_tool_calls(black_box(input));
            black_box(result)
        })
    });

    c.bench_function("parse_tool_calls_none", |b| {
        let input = "Just a regular response with no tool calls at all.";
        b.iter(|| {
            let result = parse_tool_calls(black_box(input));
            black_box(result)
        })
    });
}

fn bench_seed_store_ops(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.db");
    let store = std::sync::Arc::new(Store::open(path.to_str().unwrap()));

    c.bench_function("seed_store_insert", |b| {
        b.iter(|| {
            let seed = Seed::new(
                "bench-session".into(),
                SeedNature::Fact,
                SeedSource::UserStatement,
                SeedContent::KeyValue {
                    key: "bench_key".into(),
                    value: "bench_value".into(),
                },
                Palace::Gen,
                Stem::Geng,
                "bench_geju".into(),
            );
            let seed_store = SeedStore::new(store.clone());
            let _ = seed_store.insert(black_box(&seed));
        })
    });
}

fn bench_history_to_llm(c: &mut Criterion) {
    let entries: Vec<HistoryEntry> = (0..100)
        .flat_map(|i| {
            vec![
                HistoryEntry::User {
                    content: format!("User message {i}"),
                    images: vec![],
                },
                HistoryEntry::Assistant {
                    content: format!(
                        "Assistant response {i} with enough text to simulate a real interaction"
                    ),
                },
            ]
        })
        .collect();

    c.bench_function("history_to_llm_messages_200", |b| {
        b.iter(|| {
            let msgs = to_llm_messages(black_box(&entries));
            black_box(msgs)
        })
    });
}

fn bench_fts5_search(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_fts5.db");
    let store = std::sync::Arc::new(Store::open(path.to_str().unwrap()));
    let seed_store = SeedStore::new(store.clone());

    // Populate with seeds for FTS5 indexing
    for i in 0..500 {
        let seed = Seed::new(
            format!("session-{}", i % 10),
            SeedNature::Fact,
            SeedSource::ToolObservation,
            SeedContent::FreeText {
                text: format!("benchmark seed content number {}", i),
            },
            Palace::Zhen,
            Stem::Wu,
            "fts5_bench".into(),
        );
        let _ = seed_store.insert(&seed);
    }

    c.bench_function("fts5_search_500_seeds", |b| {
        b.iter(|| {
            let result = store.search_seeds(black_box("benchmark seed"), 20);
            black_box(result)
        })
    });
}

fn bench_entropy_compute(c: &mut Criterion) {
    // Pre-build seeds with varied ages and strengths
    let now = jia::utils::unix_now();
    let mut seeds = Vec::new();
    for i in 0..1000u64 {
        let seed = Seed {
            id: format!("seed-{}", i),
            session_id: format!("sess-{}", i % 5),
            nature: if i % 3 == 0 {
                SeedNature::Fact
            } else if i % 3 == 1 {
                SeedNature::Inference
            } else {
                SeedNature::Preference
            },
            source: SeedSource::ToolObservation,
            content: SeedContent::KeyValue {
                key: format!("key-{}", i % 20),
                value: format!("value-{}", i),
            },
            palace: Palace::Zhen,
            intent_stem: Stem::Wu,
            geju_key: "bench".into(),
            created_at: now - (i as i64 * 3600),
            access_count: (i % 10) as u32,
            last_accessed_at: now - (i as i64 * 600),
            strength: 0.1 + (i % 10) as f32 * 0.1,
            tier: if i % 4 == 0 {
                SeedTier::Always
            } else if i % 4 == 1 {
                SeedTier::OnDemand
            } else {
                SeedTier::Archive
            },
        };
        seeds.push(seed);
    }

    c.bench_function("entropy_compute_1000_seeds", |b| {
        b.iter(|| {
            let entropy = AlayaEntropy::compute(black_box(&seeds), now);
            black_box(entropy)
        })
    });
}

criterion_group!(
    benches,
    bench_token_counting,
    bench_parse_tool_calls,
    bench_seed_store_ops,
    bench_history_to_llm,
    bench_fts5_search,
    bench_entropy_compute,
);
criterion_main!(benches);

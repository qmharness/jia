use std::sync::Arc;

use crate::palaces::Palace;
use crate::palaces::gen_store::Store;
use crate::stems::Stem;
use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore};

/// Detected signal from a user message — ready to become a seed.
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedSignal {
    pub key: String,
    pub value: String,
    pub nature: SeedNature,
}

/// L1 per-message signal detector — zero LLM, pattern/keyword only.
///
/// Catches explicit statements. Leaves implicit/inferred meaning to L2 consolidation.
/// Philosophy: 宁漏勿错 (rather miss than get wrong).
pub struct SignalDetector;

impl SignalDetector {
    /// Detect signals from a single user message.
    pub fn detect(message: &str) -> Vec<DetectedSignal> {
        let mut signals = Vec::new();

        // ── Pattern-based detection ──────────────────────────
        for (idx, _) in message.char_indices() {
            let tail = &message[idx..];

            // "我使用的是X" / "我用的是X" / "我使用X" / "我用X" → tool preference
            // Tool/tech names are ASCII identifiers; use take_ident to stop at non-ASCII.
            for prefix in &["我使用的是", "我用的是", "我使用", "我用"] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_ident(val)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "tool".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }
            // English tool patterns
            for prefix in &[
                "I use ",
                "I'm using ",
                "I am using ",
                "my setup is ",
                "I switched to ",
            ] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_ident(val)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "tool".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }

            // "我是做X的" / "我是X" → role (may contain Chinese)
            for prefix in &["我是做", "我是搞", "我是"] {
                if let Some(rest) = tail.strip_prefix(prefix) {
                    let val = rest.trim_end_matches('的');
                    if let Some(word) = take_cn_word(val)
                        && !word.is_empty()
                    {
                        signals.push(DetectedSignal {
                            key: "role".into(),
                            value: word.to_string(),
                            nature: SeedNature::Preference,
                        });
                    }
                }
            }
            // English role patterns
            for prefix in &["I am a ", "I'm a ", "I work as a ", "I work as an "] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_en_phrase(val, 4)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "role".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }

            // "不喜欢X" / "讨厌X" / "不用X" → dislikes (X may be Chinese or English)
            for prefix in &["不喜欢", "讨厌", "不想用", "不用"] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_cn_word(val)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "dislikes".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }
            // English dislike patterns
            for prefix in &["I don't like ", "I dislike ", "I hate ", "I avoid "] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_en_phrase(val, 4)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "dislikes".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }

            // "爱用X" / "偏爱X" / "常用X" / "喜欢X" → likes
            for prefix in &["偏爱", "爱用", "常用", "喜欢"] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_cn_word(val)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "likes".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }
            // English like patterns
            for prefix in &["I like ", "I love ", "I prefer ", "I enjoy "] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_en_phrase(val, 4)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "likes".into(),
                        value: word.to_string(),
                        nature: SeedNature::Preference,
                    });
                }
            }

            // "在开发X" / "在做X" / "在写X" / "在搞X" / "在维护X" → project
            for prefix in &["在开发", "在维护", "在做", "在写", "在搞", "在弄"] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_cn_word(val)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "project".into(),
                        value: word.to_string(),
                        nature: SeedNature::Fact,
                    });
                }
            }
            // English project patterns
            for prefix in &[
                "I'm building ",
                "I am building ",
                "I'm working on ",
                "I maintain ",
            ] {
                if let Some(val) = tail.strip_prefix(prefix)
                    && let Some(word) = take_en_phrase(val, 4)
                    && !word.is_empty()
                {
                    signals.push(DetectedSignal {
                        key: "project".into(),
                        value: word.to_string(),
                        nature: SeedNature::Fact,
                    });
                }
            }
        }

        // ── Tech keyword detection ───────────────────────────
        let techs = detect_tech_keywords(message);
        for tech in techs {
            // Don't duplicate if already caught by "我用X" pattern
            if !signals.iter().any(|s| s.key == "tool" && s.value == tech) {
                signals.push(DetectedSignal {
                    key: "tech_stack".into(),
                    value: tech,
                    nature: SeedNature::Fact,
                });
            }
        }

        signals
    }

    /// Process a user message: detect signals, deduplicate, and insert as seeds.
    ///
    /// Returns the number of new seeds created.
    pub fn process(store: &Arc<Store>, session_id: &str, message: &str) -> usize {
        let signals = Self::detect(message);
        if signals.is_empty() {
            return 0;
        }

        let seed_store = SeedStore::new(store.clone());
        let project_id = store.session_project_id(session_id).unwrap_or_default();

        // Load existing seeds to check for duplicates
        let existing = seed_store.load_all().unwrap_or_default();

        let mut created = 0usize;
        for sig in &signals {
            // Dedup: same key + same value → skip
            let is_dup = existing.iter().any(|s| match &s.content {
                SeedContent::KeyValue { key, value } => key == &sig.key && value == &sig.value,
                _ => false,
            });
            if is_dup {
                continue;
            }

            // Preference signals: delete old same-key seeds (upsert semantics)
            if matches!(sig.nature, SeedNature::Preference) {
                let stale: Vec<String> = existing
                    .iter()
                    .filter(|s| matches!(s.nature, SeedNature::Preference))
                    .filter_map(|s| match &s.content {
                        SeedContent::KeyValue { key, .. } if key == &sig.key => Some(s.id.clone()),
                        _ => None,
                    })
                    .collect();
                let _ = store.delete_seeds(&stale);
            }

            let seed = Seed::new(
                session_id.to_string(),
                project_id.clone(),
                sig.nature.clone(),
                SeedSource::SignalDetection,
                SeedContent::KeyValue {
                    key: sig.key.clone(),
                    value: sig.value.clone(),
                },
                Palace::Kun,
                Stem::Ji,
                String::new(),
            );

            if seed_store.insert(&seed).is_ok() {
                created += 1;
            }
        }

        if created > 0 {
            tracing::info!(
                "SignalDetector: created {created} seeds from {} signals",
                signals.len(),
            );
        }

        created
    }
}

// ── Helpers ──────────────────────────────────────────────────

/// Extract the first ASCII identifier (alphanumeric + common symbols).
/// Stops at whitespace, punctuation, or non-ASCII characters.
fn take_ident(s: &str) -> Option<&str> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s
        .find(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-' | '#' | '_' | '/'))
        })
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some(&s[..end])
}

/// Extract the first meaningful phrase. Grabs contiguous non-delimiter
/// characters. Handles both Chinese and ASCII content.
fn take_cn_word(s: &str) -> Option<&str> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let end = s
        .find(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '，' | ','
                        | '。'
                        | '.'
                        | '、'
                        | '；'
                        | ';'
                        | '：'
                        | ':'
                        | '？'
                        | '?'
                        | '！'
                        | '!'
                        | '）'
                        | ')'
                        | '】'
                        | ']'
                        | '」'
                        | '（'
                        | '('
                        | '【'
                        | '['
                        | '「'
                        | '"'
                        | '\''
                        | '“'
                        | '”'
                )
        })
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some(&s[..end])
}

/// Extract an English phrase up to `max_words` words or first sentence delimiter.
/// Stops at `.`, `!`, `?`, `;`, `\n`. Limits word count to avoid over-capture.
fn take_en_phrase(s: &str, max_words: usize) -> Option<&str> {
    let s = s.trim_start();
    if s.is_empty() {
        return None;
    }
    let mut word_count = 0usize;
    let mut end = s.len();
    for (i, ch) in s.char_indices() {
        if matches!(ch, '.' | '!' | '?' | ';' | '\n') {
            end = i;
            break;
        }
        if ch.is_whitespace() {
            word_count += 1;
            if word_count >= max_words {
                end = i;
                break;
            }
        }
    }
    if end == 0 {
        return None;
    }
    Some(&s[..end])
}

// ── Tech keyword dictionary ─────────────────────────────────

/// Known technology keywords to detect in user messages.
const TECH_KEYWORDS: &[&str] = &[
    // Languages
    "Rust",
    "Python",
    "Go",
    "TypeScript",
    "JavaScript",
    "C++",
    "C",
    "Java",
    "Kotlin",
    "Swift",
    "Zig",
    "Elixir",
    "Haskell",
    "OCaml",
    "Scala",
    "Lua",
    // Editors / IDEs
    "vim",
    "neovim",
    "emacs",
    "VSCode",
    "IntelliJ",
    "Cursor",
    "Helix",
    // Rust ecosystem
    "sqlx",
    "tokio",
    "serde",
    "diesel",
    "axum",
    "actix",
    "rocket",
    "tauri",
    "bevy",
    "egui",
    "ratatui",
    "clap",
    "rayon",
    "nom",
    "pest",
    // Databases
    "Postgres",
    "MySQL",
    "SQLite",
    "Redis",
    "MongoDB",
    "DuckDB",
    // Infrastructure
    "Docker",
    "Kubernetes",
    "AWS",
    "GCP",
    "Fly.io",
    "Cloudflare",
    // Frameworks
    "React",
    "Vue",
    "Svelte",
    "Next.js",
    "Tailwind",
];

/// Detect tech keywords in a user message. Returns deduplicated list.
fn detect_tech_keywords(message: &str) -> Vec<String> {
    let mut found = Vec::new();
    for kw in TECH_KEYWORDS {
        if message.contains(kw) {
            found.push(kw.to_string());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_tool_usage() {
        let sigs = SignalDetector::detect("我用vim写代码");
        let tool: Vec<_> = sigs.iter().filter(|s| s.key == "tool").collect();
        assert!(!tool.is_empty(), "should detect tool usage, got: {sigs:?}");
        assert!(
            tool.iter().any(|s| s.value == "vim"),
            "should find vim as tool"
        );
    }

    #[test]
    fn detect_tool_usage_with_de() {
        let sigs = SignalDetector::detect("我用的是neovim");
        let tool: Vec<_> = sigs.iter().filter(|s| s.key == "tool").collect();
        assert!(!tool.is_empty(), "should detect '我用的是X': {sigs:?}");
        assert!(tool.iter().any(|s| s.value == "neovim"));
    }

    #[test]
    fn detect_tool_usage_shiyong() {
        let sigs = SignalDetector::detect("我使用cursor写代码");
        let tool: Vec<_> = sigs.iter().filter(|s| s.key == "tool").collect();
        assert!(!tool.is_empty(), "should detect '我使用': {sigs:?}");
    }

    #[test]
    fn detect_role() {
        let sigs = SignalDetector::detect("我是后端工程师");
        let role: Vec<_> = sigs.iter().filter(|s| s.key == "role").collect();
        assert!(!role.is_empty(), "should detect role: {sigs:?}");
    }

    #[test]
    fn detect_role_with_zuo() {
        let sigs = SignalDetector::detect("我是做infra的");
        let role: Vec<_> = sigs.iter().filter(|s| s.key == "role").collect();
        assert!(!role.is_empty(), "should detect '我是做X的': {sigs:?}");
    }

    #[test]
    fn detect_dislike() {
        let sigs = SignalDetector::detect("我不喜欢ORM，太重了");
        let dislike: Vec<_> = sigs.iter().filter(|s| s.key == "dislikes").collect();
        assert!(!dislike.is_empty(), "should detect dislike: {sigs:?}");
    }

    #[test]
    fn detect_dislike_buyong() {
        let sigs = SignalDetector::detect("我不用Docker");
        let dislike: Vec<_> = sigs.iter().filter(|s| s.key == "dislikes").collect();
        assert!(!dislike.is_empty(), "should detect '不用X': {sigs:?}");
    }

    #[test]
    fn detect_like() {
        let sigs = SignalDetector::detect("我喜欢Rust的错误处理");
        let like: Vec<_> = sigs.iter().filter(|s| s.key == "likes").collect();
        assert!(!like.is_empty(), "should detect likes: {sigs:?}");
    }

    #[test]
    fn detect_project() {
        let sigs = SignalDetector::detect("我在做jia项目");
        let proj: Vec<_> = sigs.iter().filter(|s| s.key == "project").collect();
        assert!(!proj.is_empty(), "should detect project: {sigs:?}");
    }

    #[test]
    fn detect_project_kaifa() {
        let sigs = SignalDetector::detect("我在开发一个CLI工具");
        let proj: Vec<_> = sigs.iter().filter(|s| s.key == "project").collect();
        assert!(!proj.is_empty(), "should detect '在开发X': {sigs:?}");
    }

    #[test]
    fn detect_tech_keywords() {
        let sigs = SignalDetector::detect("我用Rust和Postgres开发后端");
        let has_tech = sigs.iter().any(|s| s.key == "tech_stack");
        assert!(has_tech, "should detect tech keywords: {sigs:?}");
    }

    #[test]
    fn tech_dedup_with_tool() {
        let sigs = SignalDetector::detect("我用vim");
        let tool = sigs.iter().any(|s| s.key == "tool" && s.value == "vim");
        let tech = sigs
            .iter()
            .any(|s| s.key == "tech_stack" && s.value == "vim");
        assert!(tool, "should have tool=vim");
        assert!(!tech, "should not duplicate as tech_stack");
    }

    #[test]
    fn no_detect_implicit_statement() {
        // "ORM太麻烦了" — implicit, L1 should skip
        let sigs = SignalDetector::detect("ORM太麻烦了");
        let dislike = sigs
            .iter()
            .filter(|s| s.key == "dislikes")
            .collect::<Vec<_>>();
        assert!(
            dislike.is_empty(),
            "implicit statements should be left to L2: {sigs:?}"
        );
    }

    #[test]
    fn empty_message_no_signals() {
        let sigs = SignalDetector::detect("");
        assert!(sigs.is_empty());
    }

    #[test]
    fn dedup_same_key_value() {
        use crate::palaces::gen_store::Store;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        let n1 = SignalDetector::process(&store, "test", "我用vim开发");
        assert_eq!(n1, 1, "first detection should create 1 seed, got {n1}");

        let n2 = SignalDetector::process(&store, "test", "我用vim和neovim");
        assert_eq!(
            n2, 1,
            "dedup: vim should be skipped, only neovim is new, got {n2}"
        );

        let all = SeedStore::new(store).load_all().unwrap();
        let kv_seeds: Vec<_> = all
            .iter()
            .filter(|s| matches!(s.content, SeedContent::KeyValue { .. }))
            .collect();
        assert_eq!(
            kv_seeds.len(),
            2,
            "should have 2 unique KV seeds, got {}",
            kv_seeds.len()
        );
    }

    #[test]
    fn nature_tagging() {
        let sigs = SignalDetector::detect("我是后端，我用vim，不喜欢Java");
        for s in &sigs {
            match s.key.as_str() {
                "role" | "tool" | "dislikes" | "likes" => {
                    assert!(
                        matches!(s.nature, SeedNature::Preference),
                        "{} should be Preference, got {:?}",
                        s.key,
                        s.nature
                    );
                }
                "project" | "tech_stack" => {
                    assert!(
                        matches!(s.nature, SeedNature::Fact),
                        "{} should be Fact, got {:?}",
                        s.key,
                        s.nature
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn signal_source_is_signal_detection() {
        use crate::palaces::gen_store::Store;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        SignalDetector::process(&store, "test", "我用vim");
        let all = SeedStore::new(store).load_all().unwrap();
        let signal_seed = all
            .iter()
            .find(|s| matches!(s.content, SeedContent::KeyValue { ref key, .. } if key == "tool"));
        assert!(
            signal_seed.is_some(),
            "should have created a tool preference seed"
        );
        let seed = signal_seed.unwrap();
        assert!(
            matches!(seed.source, SeedSource::SignalDetection),
            "source should be SignalDetection, got {:?}",
            seed.source
        );
    }
}

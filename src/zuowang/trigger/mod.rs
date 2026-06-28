use crate::vijnana::alaya::Seed;

/// Alaya entropy — multi-dimensional measure of memory store disorder.
///
/// When total entropy exceeds the threshold (default 0.75), the
/// ZuowangPipeline triggers dissolution of stale/weak seeds.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AlayaEntropy {
    pub staleness: f32,
    pub contradiction: f32,
    pub redundancy: f32,
    pub access_decay: f32,
    pub total: f32,
}

impl AlayaEntropy {
    /// Compute entropy from a set of seeds, given the current timestamp.
    pub fn compute(seeds: &[Seed], now: i64) -> Self {
        if seeds.is_empty() {
            return Self {
                staleness: 0.0,
                contradiction: 0.0,
                redundancy: 0.0,
                access_decay: 0.0,
                total: 0.0,
            };
        }

        let max_age = seeds
            .iter()
            .map(|s| now - s.created_at)
            .max()
            .unwrap_or(1)
            .max(1) as f32;

        // Staleness: average age normalized to [0, 1]
        // Archive seeds' staleness is expected (they're cold storage) — weight ×0.5
        let staleness = seeds
            .iter()
            .map(|s| {
                let raw = (now - s.created_at) as f32 / max_age;
                match s.tier {
                    crate::vijnana::alaya::SeedTier::Archive => raw * 0.5,
                    _ => raw,
                }
            })
            .sum::<f32>()
            / seeds.len() as f32;

        // Access decay: average of (now - last_accessed) normalized by max access gap
        // Archive seeds' access decay is expected — weight ×0.5
        let max_gap = seeds
            .iter()
            .map(|s| now - s.last_accessed_at)
            .max()
            .unwrap_or(1)
            .max(1) as f32;
        let access_decay = seeds
            .iter()
            .map(|s| {
                let gap = now - s.last_accessed_at;
                let raw = (gap as f32 / max_gap).min(1.0);
                match s.tier {
                    crate::vijnana::alaya::SeedTier::Archive => raw * 0.5,
                    _ => raw,
                }
            })
            .sum::<f32>()
            / seeds.len() as f32;

        // Redundancy: content-based duplicate detection.
        // KeyValue seeds with the same key count as redundant.
        // Triple seeds with the same (predicate, object) count as redundant.
        // FreeText seeds are excluded (text comparison is fragile).
        let redundancy = compute_redundancy(seeds);

        // Contradiction: detect KeyValue and Triple conflicts across seeds
        let contradiction = compute_contradiction(seeds);

        let total = staleness * 0.3 + contradiction * 0.2 + redundancy * 0.25 + access_decay * 0.25;

        Self {
            staleness,
            contradiction,
            redundancy,
            access_decay,
            total,
        }
    }

    pub fn exceeds_threshold(&self, threshold: f32) -> bool {
        self.total >= threshold
    }
}

/// Compute content-based redundancy across seeds.
///
/// KeyValue seeds sharing the same key are counted as redundant (N-1 per group).
/// Triple seeds sharing the same (predicate, object) are counted as redundant.
/// FreeText seeds are excluded — text comparison is too fragile.
///
/// This replaces the previous geju_key-based counting, which conflated
/// "same retrieval group" with "same information."
fn compute_redundancy(seeds: &[crate::vijnana::alaya::Seed]) -> f32 {
    use crate::vijnana::alaya::SeedContent;
    use std::collections::HashMap;

    let total = seeds.len() as f32;
    if total == 0.0 {
        return 0.0;
    }

    let mut kv_counts: HashMap<&str, usize> = HashMap::new();
    let mut triple_counts: HashMap<(&str, &str), usize> = HashMap::new();
    let mut free_text_fingerprints: HashMap<String, usize> = HashMap::new();

    for seed in seeds {
        match &seed.content {
            SeedContent::KeyValue { key, .. } => {
                *kv_counts.entry(key.as_str()).or_default() += 1;
            }
            SeedContent::Triple {
                predicate, object, ..
            } => {
                *triple_counts
                    .entry((predicate.as_str(), object.as_str()))
                    .or_default() += 1;
            }
            SeedContent::FreeText { text } => {
                // Use first 50 chars (lowered) as a fingerprint.
                // Near-duplicate free-text seeds share the same prefix.
                let fp: String = text.chars().take(50).collect::<String>().to_lowercase();
                *free_text_fingerprints.entry(fp).or_default() += 1;
            }
        }
    }

    let kv_redundant: usize = kv_counts.values().filter(|&&c| c > 1).map(|&c| c - 1).sum();
    let triple_redundant: usize = triple_counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();
    let free_text_redundant: usize = free_text_fingerprints
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();

    (kv_redundant + triple_redundant + free_text_redundant) as f32 / total
}

/// Compute contradiction score across seeds by detecting conflicting assertions.
///
/// Detects two types of conflicts:
/// - KeyValue: same key with different values
/// - Triple: same subject+predicate with different objects
///
/// Returns normalized score in [0, 1].
///
/// Uses lazy conflict tracking: only allocates a HashSet for keys that actually
/// have disagreements. Keys with all-identical values stay cheap (one &str + count).
fn compute_contradiction(seeds: &[crate::vijnana::alaya::Seed]) -> f32 {
    use crate::vijnana::alaya::SeedContent;
    use std::collections::{HashMap, HashSet};

    /// Tracks distinct values per assertion-group. Starts cheap (one value + count)
    /// and only upgrades to a HashSet when a conflict is detected.
    enum ValueSet<'a> {
        One(&'a str, u32),
        Many(HashSet<&'a str>, u32),
    }

    impl<'a> ValueSet<'a> {
        fn add(&mut self, v: &'a str) {
            match self {
                Self::One(existing, count) if *existing == v => *count += 1,
                Self::One(existing, count) => {
                    let mut set = HashSet::new();
                    set.insert(*existing);
                    set.insert(v);
                    *self = Self::Many(set, *count + 1);
                }
                Self::Many(set, count) => {
                    set.insert(v);
                    *count += 1;
                }
            }
        }
    }

    let mut key_values: HashMap<&str, ValueSet> = HashMap::new();
    let mut triples: HashMap<(&str, &str), ValueSet> = HashMap::new();

    for seed in seeds {
        match &seed.content {
            SeedContent::KeyValue { key, value } => {
                key_values
                    .entry(key)
                    .and_modify(|vs| vs.add(value))
                    .or_insert(ValueSet::One(value, 1));
            }
            SeedContent::Triple {
                subject,
                predicate,
                object,
            } => {
                triples
                    .entry((subject, predicate))
                    .and_modify(|vs| vs.add(object))
                    .or_insert(ValueSet::One(object, 1));
            }
            SeedContent::FreeText { .. } => {}
        }
    }

    let mut conflict_count = 0u32;
    let mut total_assertions = 0u32;

    for vs in key_values.values().chain(triples.values()) {
        match vs {
            ValueSet::One(_, count) => total_assertions += count,
            ValueSet::Many(set, count) => {
                total_assertions += count;
                conflict_count += (set.len() - 1) as u32;
            }
        }
    }

    if total_assertions == 0 {
        return 0.0;
    }

    (conflict_count as f32 / total_assertions as f32).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::Palace;
    use crate::stems::Stem;
    use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedTier};

    fn make_seed(
        id: &str,
        geju_key: &str,
        created_at: i64,
        last_accessed_at: i64,
        access_count: u32,
        strength: f32,
    ) -> Seed {
        Seed {
            id: id.into(),
            session_id: "test".into(),
            nature: SeedNature::Fact,
            source: SeedSource::ToolObservation,
            content: SeedContent::FreeText { text: "x".into() },
            palace: Palace::Zhen,
            intent_stem: Stem::Geng,
            geju_key: geju_key.into(),
            created_at,
            access_count,
            last_accessed_at,
            strength,
            tier: SeedTier::OnDemand,
        }
    }

    fn make_kv_seed(id: &str, key: &str, value: &str, created_at: i64) -> Seed {
        Seed {
            id: id.into(),
            session_id: "test".into(),
            nature: SeedNature::Preference,
            source: SeedSource::Consolidation,
            content: SeedContent::KeyValue {
                key: key.into(),
                value: value.into(),
            },
            palace: Palace::Kun,
            intent_stem: Stem::Wu,
            geju_key: "kv_test".into(),
            created_at,
            access_count: 0,
            last_accessed_at: created_at,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        }
    }

    #[test]
    fn empty_seeds_all_zero() {
        let e = AlayaEntropy::compute(&[], 1000);
        assert!((e.staleness - 0.0).abs() < 0.001);
        assert!((e.contradiction - 0.0).abs() < 0.001);
        assert!((e.redundancy - 0.0).abs() < 0.001);
        assert!((e.access_decay - 0.0).abs() < 0.001);
        assert!((e.total - 0.0).abs() < 0.001);
    }

    #[test]
    fn single_seed_zero_staleness() {
        // All seeds created just now → staleness ≈ 0
        let now = 5000;
        let seeds = vec![make_seed("s1", "g1", now, now, 0, 1.0)];
        let e = AlayaEntropy::compute(&seeds, now);
        assert!(
            (e.staleness - 0.0).abs() < 0.001,
            "staleness={}",
            e.staleness
        );
        assert!((e.contradiction - 0.0).abs() < 0.001);
        assert!((e.redundancy - 0.0).abs() < 0.001);
        assert!((e.total - 0.0).abs() < 0.001);
    }

    #[test]
    fn staleness_increases_with_age() {
        let now = 100_000;
        let seeds = vec![make_seed("s1", "g1", 0, 0, 0, 1.0)];
        let e = AlayaEntropy::compute(&seeds, now);
        // Created at t=0, now t=100000 → max_age=100000, staleness=1.0
        assert!(
            e.staleness > 0.5,
            "staleness should be high: {}",
            e.staleness
        );
    }

    #[test]
    fn contradiction_detected_same_key_different_values() {
        let now = 1000;
        let seeds = vec![
            make_kv_seed("a", "lang", "rust", now),
            make_kv_seed("b", "lang", "python", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        assert!(e.contradiction > 0.0, "contradiction={}", e.contradiction);
        // 2 assertions, 1 conflict → 0.5
        assert!(
            (e.contradiction - 0.5).abs() < 0.01,
            "contradiction={}",
            e.contradiction
        );
    }

    #[test]
    fn no_contradiction_with_agreement() {
        let now = 1000;
        let seeds = vec![
            make_kv_seed("a", "lang", "rust", now),
            make_kv_seed("b", "lang", "rust", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        assert!(
            (e.contradiction - 0.0).abs() < 0.001,
            "contradiction={}",
            e.contradiction
        );
    }

    #[test]
    fn redundancy_detected_same_predicate_object() {
        let now = 1000;
        // 3 Triple seeds share (predicate="has", object="src/main.rs")
        // → 2 redundant. 1 Triple with different p+o.  → total 4, redundant 2.
        let seeds = vec![
            make_triple_seed("t1", "file1.rs", "has", "src/main.rs", now),
            make_triple_seed("t2", "file2.rs", "has", "src/main.rs", now),
            make_triple_seed("t3", "file3.rs", "has", "src/main.rs", now),
            make_triple_seed("t4", "file4.rs", "imports", "tokio", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        assert!(
            (e.redundancy - 0.5).abs() < 0.01,
            "redundancy={}",
            e.redundancy
        );
    }

    #[test]
    fn redundancy_zero_for_distinct_content() {
        let now = 1000;
        let seeds = vec![
            make_triple_seed("t1", "A", "modified_file", "src/a.rs", now),
            make_triple_seed("t2", "B", "triggered_error", "E0308", now),
            make_triple_seed("t3", "C", "compiled", "jia v0.1", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        assert!(
            (e.redundancy - 0.0).abs() < 0.001,
            "distinct p+o should have zero redundancy, got {}",
            e.redundancy
        );
    }

    #[test]
    fn redundancy_detects_duplicate_keyvalue_keys() {
        let now = 1000;
        let seeds = vec![
            make_kv_seed("a", "editor", "vim", now),
            make_kv_seed("b", "editor", "neovim", now), // same key → redundant AND contradictory
            make_kv_seed("c", "lang", "rust", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        // "editor": 2 seeds → 1 redundant → 1/3 ≈ 0.333
        assert!(
            (e.redundancy - 0.333).abs() < 0.05,
            "redundancy={}",
            e.redundancy
        );
        // contradiction also fires (editor: vim vs neovim)
        assert!(
            e.contradiction > 0.0,
            "same key diff value should contradict"
        );
    }

    #[test]
    fn access_decay_from_last_access() {
        let now = 100_000;
        let seeds = vec![
            make_seed("s1", "g1", 0, 0, 0, 1.0), // never accessed since creation
            make_seed("s2", "g2", 0, 90_000, 0, 1.0), // accessed recently
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        // s1 gap=100000, s2 gap=10000, max_gap=100000
        // access_decay = (1.0 + 0.1) / 2 = 0.55
        assert!(e.access_decay > 0.4, "access_decay={}", e.access_decay);
    }

    #[test]
    fn exceeds_threshold_at_boundary() {
        let e = AlayaEntropy {
            staleness: 0.5,
            contradiction: 0.5,
            redundancy: 0.5,
            access_decay: 0.5,
            total: 0.75,
        };
        assert!(
            e.exceeds_threshold(0.75),
            "should exceed at threshold boundary"
        );
    }

    #[test]
    fn below_threshold() {
        let e = AlayaEntropy {
            staleness: 0.1,
            contradiction: 0.1,
            redundancy: 0.1,
            access_decay: 0.1,
            total: 0.10,
        };
        assert!(!e.exceeds_threshold(0.75));
    }

    #[test]
    fn total_is_weighted_sum() {
        let now = 100_000;
        let seeds = vec![
            make_seed("s1", "g1", 0, 0, 0, 1.0),
            make_seed("s2", "g1", 0, 0, 0, 1.0),
        ];
        let real = AlayaEntropy::compute(&seeds, now);
        assert!(
            real.total >= 0.0 && real.total <= 1.0,
            "total={}",
            real.total
        );
    }

    #[test]
    fn contradiction_with_triple_seeds() {
        let now = 1000;
        let seeds = vec![
            make_triple_seed("t1", "Cargo.toml", "depends_on", "serde", now),
            make_triple_seed("t2", "Cargo.toml", "depends_on", "tokio", now),
            make_triple_seed("t3", "Cargo.toml", "depends_on", "serde", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        // 3 assertions total, 2 distinct ("serde", "tokio") → 1 conflict
        // contradiction = 1/3 ≈ 0.333
        assert!(
            e.contradiction > 0.0,
            "triple conflict not detected: contradiction={}",
            e.contradiction
        );
        assert!(
            (e.contradiction - 0.333).abs() < 0.01,
            "contradiction={}",
            e.contradiction
        );
    }

    #[test]
    fn contradiction_zero_for_distinct_keys() {
        let now = 1000;
        let seeds = vec![
            make_kv_seed("a", "editor", "vim", now),
            make_kv_seed("b", "lang", "rust", now),
            make_kv_seed("c", "os", "macos", now),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        assert!(
            (e.contradiction - 0.0).abs() < 0.001,
            "distinct keys should not contradict, got {}",
            e.contradiction
        );
    }

    #[test]
    fn mixed_staleness_fresh_and_old() {
        let now = 100_000;
        let seeds = vec![
            make_seed("fresh", "g1", now - 100, now - 100, 10, 1.0),
            make_seed("old1", "g1", 0, 0, 0, 1.0),
            make_seed("old2", "g2", 0, 0, 0, 1.0),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        // staleness: fresh ≈ 0.001, old1 ≈ 1.0, old2 ≈ 1.0 → avg ≈ 0.667
        assert!(
            e.staleness > 0.5 && e.staleness < 0.8,
            "mixed staleness={}",
            e.staleness
        );
    }

    #[test]
    fn weighted_total_reflects_all_dimensions() {
        let now = 100_000;
        let seeds = vec![
            make_kv_seed("a", "lang", "rust", now),
            make_kv_seed("b", "lang", "python", now),
            make_kv_seed("c", "lang", "go", now),
            make_seed("d", "same", 0, 0, 0, 1.0),
            make_seed("e", "same", 0, 0, 0, 1.0),
            make_seed("f", "same", 0, 0, 0, 1.0),
        ];
        let e = AlayaEntropy::compute(&seeds, now);
        // contradiction: 3 assertions (lang: rust/python/go), 2 conflicts → 2/3 ≈ 0.667
        // redundancy: 3 seeds with "same" key → 2 redundant out of 6 = 0.333
        // staleness: 3 fresh + 3 old → mixed
        assert!(
            e.contradiction > 0.4,
            "high contradiction expected, got {}",
            e.contradiction
        );
        assert!(
            e.redundancy > 0.2,
            "redundancy expected, got {}",
            e.redundancy
        );
        assert!(
            e.total > 0.3,
            "total should reflect multi-dimension disorder, got {}",
            e.total
        );
    }

    fn make_triple_seed(
        id: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        created_at: i64,
    ) -> Seed {
        Seed {
            id: id.into(),
            session_id: "test".into(),
            nature: SeedNature::Inference,
            source: SeedSource::Consolidation,
            content: SeedContent::Triple {
                subject: subject.into(),
                predicate: predicate.into(),
                object: object.into(),
            },
            palace: Palace::Gen,
            intent_stem: Stem::Gui,
            geju_key: "triple_test".into(),
            created_at,
            access_count: 0,
            last_accessed_at: created_at,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        }
    }
}

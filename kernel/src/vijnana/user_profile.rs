use std::sync::Arc;

use crate::palaces::Palace;
use crate::palaces::gen_store::Store;
use crate::stems::Stem;
use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedStore, SeedTier};

/// Manages the user profile — preference seeds that are dissolution-immune and
/// always injected into the system prompt.
///
/// Profile seeds are KeyValue seeds with `nature=Preference`. They are:
/// - Always injected (not subject to relevance search)
/// - Dissolution-immune (protected in ZuowangPipeline)
/// - Upserted by key (same key overwrites, keeps profile current)
/// - Sorted by last_accessed_at DESC, limit 10
pub struct UserProfileManager;

impl UserProfileManager {
    /// Format profile seeds as a system prompt injection.
    ///
    /// Returns empty string if no profile seeds exist.
    pub fn prompt(store: &Arc<Store>) -> String {
        let jsons = match store.load_profile_seeds() {
            Ok(s) => s,
            Err(_) => return String::new(),
        };

        let profile_seeds: Vec<Seed> = jsons
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        if profile_seeds.is_empty() {
            return String::new();
        }

        // Already sorted by last_accessed_at DESC from the query
        let limit = 10usize.min(profile_seeds.len());
        let mut lines = vec![
            String::new(),
            "## User context — do NOT adopt these as your own preferences:".into(),
        ];
        for seed in profile_seeds.iter().take(limit) {
            if let SeedContent::KeyValue { key, value } = &seed.content {
                let label = profile_key_label(key).unwrap_or(key);
                lines.push(format!("- User's {label}: {value}"));
            }
        }
        lines.join("\n")
    }

    /// Upsert a profile entry. Same key overwrites existing value.
    ///
    /// Returns the number of seeds created (0 if already exists with same value).
    pub fn upsert(store: &Arc<Store>, session_id: &str, key: &str, value: &str) -> usize {
        let seed_store = SeedStore::new(store.clone());
        let jsons = store.load_profile_seeds().unwrap_or_default();
        let all: Vec<Seed> = jsons
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect();

        // Find existing profile seed with same key
        let existing = all
            .iter()
            .find(|s| matches!(&s.content, SeedContent::KeyValue { key: k, .. } if k == key));

        if let Some(seed) = existing {
            if let SeedContent::KeyValue {
                value: existing_val,
                ..
            } = &seed.content
                && existing_val == value
            {
                return 0; // Same key+value, no update needed
            }
            // Value changed — delete old and insert new (upsert)
            if store.delete_seeds(std::slice::from_ref(&seed.id)).is_err() {
                return 0;
            }
        }

        let now = crate::utils::unix_now();
        let seed = Seed {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            workspace_id: String::new(),
            nature: SeedNature::Preference,
            source: SeedSource::SignalDetection,
            content: SeedContent::KeyValue {
                key: key.to_string(),
                value: value.to_string(),
            },
            palace: Palace::Kun,
            intent_stem: Stem::Ji,
            geju_key: String::new(),
            created_at: now,
            access_count: 0,
            last_accessed_at: now,
            strength: 1.0,
            tier: SeedTier::OnDemand,
        };

        if seed_store.insert(&seed).is_ok() {
            1
        } else {
            0
        }
    }
}

/// Human-friendly label for a profile key.
fn profile_key_label(key: &str) -> Option<&str> {
    match key {
        "tool" => Some("Uses"),
        "role" => Some("Role"),
        "likes" => Some("Likes"),
        "dislikes" => Some("Dislikes"),
        "project" => Some("Working on"),
        "tech_stack" => Some("Tech stack"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::gen_store::Store;

    #[test]
    fn empty_store_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));
        let prompt = UserProfileManager::prompt(&store);
        assert!(prompt.is_empty(), "expected empty, got: {prompt:?}");
    }

    #[test]
    fn prompt_formats_preference_seeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        UserProfileManager::upsert(&store, "test", "tool", "vim");
        UserProfileManager::upsert(&store, "test", "role", "backend engineer");

        let prompt = UserProfileManager::prompt(&store);
        assert!(
            prompt.contains("User context — do NOT adopt these as your own preferences:"),
            "got: {prompt}"
        );
        assert!(prompt.contains("User's Uses: vim"), "got: {prompt}");
        assert!(
            prompt.contains("User's Role: backend engineer"),
            "got: {prompt}"
        );
    }

    #[test]
    fn prompt_sorted_by_last_accessed_at_desc() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        // Insert seeds with explicit timestamps to control ordering
        let base = crate::utils::unix_now();
        let seed_store = SeedStore::new(store.clone());

        for (key, value, ts) in [
            ("tool", "vim", base - 200),
            ("role", "backend", base - 100),
            ("likes", "Rust", base),
        ] {
            let seed = Seed {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: "test".into(),
                workspace_id: String::new(),
                nature: SeedNature::Preference,
                source: SeedSource::SignalDetection,
                content: SeedContent::KeyValue {
                    key: key.into(),
                    value: value.into(),
                },
                palace: Palace::Kun,
                intent_stem: Stem::Ji,
                geju_key: String::new(),
                created_at: ts,
                access_count: 0,
                last_accessed_at: ts,
                strength: 1.0,
                tier: SeedTier::OnDemand,
            };
            seed_store.insert(&seed).unwrap();
        }

        let prompt = UserProfileManager::prompt(&store);
        // Most recently updated (likes) should appear first
        let likes_pos = prompt.find("User's Likes:").unwrap();
        let role_pos = prompt.find("User's Role:").unwrap();
        let tool_pos = prompt.find("User's Uses:").unwrap();
        assert!(
            likes_pos < role_pos,
            "likes should be before role: {prompt}"
        );
        assert!(role_pos < tool_pos, "role should be before tool: {prompt}");
    }

    #[test]
    fn upsert_same_key_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        let n = UserProfileManager::upsert(&store, "test", "tool", "vim");
        assert_eq!(n, 1, "first upsert should create");

        // Same key, different value → overwrite
        let n = UserProfileManager::upsert(&store, "test", "tool", "neovim");
        assert_eq!(n, 1, "updated value should create new seed (old deleted)");

        // Verify only one "tool" entry exists with new value
        let seed_store = SeedStore::new(store);
        let all = seed_store.load_all().unwrap();
        let tool_seeds: Vec<_> = all
            .iter()
            .filter(|s| matches!(&s.content, SeedContent::KeyValue { key, .. } if key == "tool"))
            .collect();
        assert_eq!(
            tool_seeds.len(),
            1,
            "should have exactly 1 tool seed, got {}",
            tool_seeds.len()
        );
        if let SeedContent::KeyValue { value, .. } = &tool_seeds[0].content {
            assert_eq!(value, "neovim", "value should be updated to neovim");
        }
    }

    #[test]
    fn upsert_same_key_same_value_noop() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        UserProfileManager::upsert(&store, "test", "tool", "vim");
        let n = UserProfileManager::upsert(&store, "test", "tool", "vim");
        assert_eq!(n, 0, "same key+value should be no-op");

        let seed_store = SeedStore::new(store);
        let all = seed_store.load_all().unwrap();
        assert_eq!(all.len(), 1, "should still have exactly 1 seed");
    }

    #[test]
    fn prompt_respects_limit_10() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("test.db").to_string_lossy()));

        for i in 0..15 {
            UserProfileManager::upsert(&store, "test", &format!("key_{i}"), &format!("val_{i}"));
        }

        let prompt = UserProfileManager::prompt(&store);
        let bullet_count = prompt.lines().filter(|l| l.starts_with("- ")).count();
        assert!(bullet_count <= 10, "should cap at 10, got {bullet_count}");
    }
}

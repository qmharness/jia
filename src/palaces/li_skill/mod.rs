use std::sync::Arc;
pub mod evolution;
pub mod loader;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Duration;

/// A loaded skill with its prompt and metadata.
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The prompt content to inject into the system message.
    pub prompt: String,
    /// Source file path for diagnostics.
    pub source_path: PathBuf,
    /// If true, this skill is always injected into the system prompt
    /// (e.g., base safety rules). Default: false.
    pub always: bool,
    /// Glob patterns for conditional activation. When a file tool touches
    /// a path matching one of these patterns, the skill is activated.
    /// None means the skill is unconditionally available.
    /// Patterns are precompiled at load time.
    pub paths: Option<Vec<glob::Pattern>>,
    /// Author-designated key guidance requiring special attention.
    /// Parsed from the last `## Emphasis` section in SKILL.md.
    /// Unlike prompt, emphasis does NOT introduce new rules — it
    /// highlights critical ones from the main prompt body.
    pub emphasis: Option<String>,
    /// If true, this skill participates in automatic evolution (Phase 0).
    pub auto_evolve: bool,
    /// Minimum average reflection confidence for auto-apply (default 0.7).
    pub evolve_min_confidence: f64,
    /// Max revisions per session (default 3).
    pub evolve_max_revisions_per_session: u32,
    /// Same-type reflections needed to trigger revision (default 3).
    pub evolve_reflection_threshold: u32,
    /// Bundled script files, keyed by relative path (e.g., "check.sh").
    /// Stored in `scripts/` subdirectory. Agent reads them via the
    /// `skill()` tool with `script` parameter.
    #[allow(dead_code)]
    pub scripts: HashMap<String, String>,
    /// Bundled reference files, keyed by relative path (e.g., "api-docs.md").
    /// Stored in `references/` subdirectory. Agent reads them via the
    /// `skill()` tool with `reference` parameter.
    #[allow(dead_code)]
    pub references: HashMap<String, String>,
}

/// 离九宫 — Skill Registry
///
/// Manages skill loading, parsing, and injection.
/// Skills from `skills/` directory are parsed and made available
/// for ShengMen-gated injection into the agent system prompt.
pub struct SkillRegistry {
    skills: HashMap<String, Arc<Skill>>,
    disabled: HashSet<String>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            disabled: HashSet::new(),
        }
    }

    /// Register a skill directly (for testing or programmatic use).
    pub fn register(&mut self, skill: Skill) {
        self.skills.insert(skill.name.clone(), Arc::new(skill));
    }

    /// Look up a skill by name (active skills only).
    pub fn get(&self, name: &str) -> Option<Arc<Skill>> {
        if self.disabled.contains(name) {
            return None;
        }
        self.skills.get(name).cloned()
    }

    /// List all registered skill names (active only).
    pub fn list_names(&self) -> Vec<&String> {
        self.skills
            .keys()
            .filter(|n| !self.disabled.contains(*n))
            .collect()
    }

    /// List all active skills (excludes disabled).
    pub fn list_all(&self) -> Vec<Arc<Skill>> {
        self.skills
            .iter()
            .filter(|(n, _)| !self.disabled.contains(*n))
            .map(|(_, s)| s.clone())
            .collect()
    }

    /// List all skills including disabled, returning (skill, disabled) pairs.
    pub fn list_all_with_status(&self) -> Vec<(Arc<Skill>, bool)> {
        self.skills
            .values()
            .map(|s| {
                let disabled = self.disabled.contains(&s.name);
                (s.clone(), disabled)
            })
            .collect()
    }

    /// Number of active (non-disabled) skills.
    pub fn len(&self) -> usize {
        self.skills.len() - self.disabled.len()
    }

    /// Whether there are no active skills.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Disable a skill by name.
    pub fn disable(&mut self, name: &str) -> bool {
        if self.skills.contains_key(name) {
            self.disabled.insert(name.to_string());
            true
        } else {
            false
        }
    }

    /// Enable a previously disabled skill by name.
    pub fn enable(&mut self, name: &str) -> bool {
        self.disabled.remove(name)
    }

    /// Check if a skill is disabled.
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled.contains(name)
    }

    /// Persist disabled set to JSON string (for config file).
    pub fn disabled_names(&self) -> Vec<&String> {
        self.disabled.iter().collect()
    }

    /// Restore disabled set from names.
    pub fn set_disabled(&mut self, names: &HashSet<String>) {
        self.disabled = names.clone();
    }

    /// Build a prompt section describing all active skills.
    pub fn build_skill_prompt(&self) -> String {
        let skills = self.list_all();
        if skills.is_empty() {
            return String::new();
        }

        let mut prompt = String::from("\n\n## Available Skills\n\n");
        prompt.push_str("You have specialized skills available. Follow the instructions below when relevant:\n\n");

        for skill in &skills {
            prompt.push_str(&format!("### {}\n{}\n\n", skill.name, skill.prompt));
        }

        prompt
    }

    /// Activate conditional skills whose `paths` glob patterns match any of
    /// the given `touched_paths`. Returns the names of skills that were
    /// activated (i.e. had matching paths but were not `always`).
    ///
    /// This is called when file tools (read_file, write_file, edit) touch
    /// paths, so the agent can inject relevant skill instructions into the
    /// conversation for subsequent turns.
    pub fn activate_for_paths(&self, touched_paths: &[&str]) -> Vec<String> {
        let mut activated = Vec::new();
        for skill in self.skills.values() {
            if self.disabled.contains(&skill.name) {
                continue;
            }
            if skill.always {
                continue;
            }
            let Some(patterns) = &skill.paths else {
                continue;
            };
            let matches = touched_paths
                .iter()
                .any(|path| patterns.iter().any(|p| p.matches(path)));
            if matches {
                activated.push(skill.name.clone());
            }
        }
        activated
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a file watcher that reloads skills on SKILL.md changes.
///
/// Uses `notify` with a 500ms debounce. On any filesystem event in the skills
/// directory, rebuilds the entire registry from disk and atomically swaps it.
pub fn spawn_skill_watcher(registry: Arc<RwLock<SkillRegistry>>, skills_dir: PathBuf) {
    tokio::spawn(async move {
        use notify::{Config, Event, RecursiveMode, Watcher};

        let (event_tx, mut event_rx) =
            tokio::sync::mpsc::unbounded_channel::<notify::Result<Event>>();

        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = event_tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("SkillWatcher: failed to create watcher: {e}");
                return;
            }
        };

        if let Err(e) =
            watcher.configure(Config::default().with_poll_interval(Duration::from_secs(2)))
        {
            tracing::warn!("SkillWatcher: configure error: {e}");
        }

        if let Err(e) = watcher.watch(&skills_dir, RecursiveMode::Recursive) {
            tracing::warn!(
                "SkillWatcher: failed to watch {}: {e}",
                skills_dir.display()
            );
            return;
        }

        tracing::info!(
            "SkillWatcher: watching {} for SKILL.md changes",
            skills_dir.display()
        );

        let debounce = Duration::from_millis(500);
        let mut dirty = false;

        loop {
            match tokio::time::timeout(debounce, event_rx.recv()).await {
                Ok(Some(Ok(_event))) => {
                    dirty = true;
                    continue;
                }
                Ok(Some(Err(e))) => {
                    tracing::warn!("SkillWatcher: notify error: {e}");
                    continue;
                }
                _ => {
                    // timeout or channel closed — flush
                    if dirty {
                        let mut new_reg = SkillRegistry::new();
                        match loader::SkillLoader::load_directory_sync(&skills_dir, &mut new_reg) {
                            Ok(n) => {
                                // Preserve disabled state across reload
                                let old_disabled = {
                                    let old = registry.read().unwrap_or_else(|e| e.into_inner());
                                    old.disabled.clone()
                                };
                                new_reg.disabled = old_disabled;
                                *registry.write().unwrap_or_else(|e| e.into_inner()) = new_reg;
                                tracing::info!("SkillWatcher: reloaded {n} skills");
                            }
                            Err(e) => {
                                tracing::warn!("SkillWatcher: reload failed: {e}");
                            }
                        }
                        dirty = false;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill {
            name: "test-skill".into(),
            description: "A test skill".into(),
            prompt: "Do the thing.".into(),
            source_path: PathBuf::from("skills/test.md"),
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
        assert_eq!(reg.len(), 1);
        assert!(reg.get("test-skill").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert_eq!(reg.list_names().len(), 1);
    }

    #[test]
    fn build_prompt_includes_skills() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill {
            name: "code-review".into(),
            description: "Reviews code".into(),
            prompt: "Always check for SQL injection.".into(),
            source_path: PathBuf::from("skills/code-review.md"),
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
        let prompt = reg.build_skill_prompt();
        assert!(prompt.contains("code-review"));
        assert!(prompt.contains("SQL injection"));
    }

    #[test]
    fn activate_for_paths_matches_glob() {
        let mut reg = SkillRegistry::new();
        reg.register(Skill {
            name: "rust-reviewer".into(),
            description: "Rust code review".into(),
            prompt: "Check for unsafe blocks.".into(),
            source_path: PathBuf::from("skills/rust.md"),
            always: false,
            paths: Some(vec![glob::Pattern::new("src/**/*.rs").unwrap()]),
            emphasis: None,
            auto_evolve: false,
            evolve_min_confidence: 0.7,
            evolve_max_revisions_per_session: 3,
            evolve_reflection_threshold: 3,
            scripts: HashMap::new(),
            references: HashMap::new(),
        });
        reg.register(Skill {
            name: "config-checker".into(),
            description: "Config review".into(),
            prompt: "Check TOML validity.".into(),
            source_path: PathBuf::from("skills/config.md"),
            always: false,
            paths: Some(vec![
                glob::Pattern::new("*.toml").unwrap(),
                glob::Pattern::new("*.yaml").unwrap(),
            ]),
            emphasis: None,
            auto_evolve: false,
            evolve_min_confidence: 0.7,
            evolve_max_revisions_per_session: 3,
            evolve_reflection_threshold: 3,
            scripts: HashMap::new(),
            references: HashMap::new(),
        });
        reg.register(Skill {
            name: "base-safety".into(),
            description: "Always-active safety".into(),
            prompt: "Never run rm -rf.".into(),
            source_path: PathBuf::from("skills/safety.md"),
            always: true,
            paths: None,
            emphasis: None,
            auto_evolve: false,
            evolve_min_confidence: 0.7,
            evolve_max_revisions_per_session: 3,
            evolve_reflection_threshold: 3,
            scripts: HashMap::new(),
            references: HashMap::new(),
        });

        let activated = reg.activate_for_paths(&["src/main.rs", "src/lib.rs"]);
        assert_eq!(activated, vec!["rust-reviewer"]);

        let activated = reg.activate_for_paths(&["Cargo.toml"]);
        assert_eq!(activated, vec!["config-checker"]);

        // always=true skills are skipped (already active)
        let activated = reg.activate_for_paths(&["README.md"]);
        assert!(activated.is_empty());
    }
}

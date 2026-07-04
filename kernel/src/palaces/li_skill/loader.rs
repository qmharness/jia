use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use super::{Skill, SkillRegistry};

/// YAML frontmatter parsed from SKILL.md.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillFrontmatter {
    /// Skill name. If set, overrides the directory name.
    /// Required by the Agent Skills standard.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub description: Option<String>,
    // ── Evolution fields (Phase 0) ──
    #[serde(default)]
    pub auto_evolve: bool,
    #[serde(default = "default_min_confidence")]
    pub evolve_min_confidence: f64,
    #[serde(default = "default_max_revisions")]
    pub evolve_max_revisions_per_session: u32,
    #[serde(default = "default_reflection_threshold")]
    pub evolve_reflection_threshold: u32,
}

fn default_min_confidence() -> f64 {
    0.7
}
fn default_max_revisions() -> u32 {
    3
}
fn default_reflection_threshold() -> u32 {
    3
}

/// Scan `scripts/` and `references/` subdirectories, reading all files
/// into HashMaps keyed by relative path (e.g. `"check.sh"`, `"api-docs.md"`).
fn collect_bundled_files(skill_dir: &Path) -> (HashMap<String, String>, HashMap<String, String>) {
    let read_dir_files = |subdir: &str| -> HashMap<String, String> {
        let dir = skill_dir.join(subdir);
        let mut files = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let key = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(String::from)
                    .unwrap_or_default();
                if key.is_empty() || key.starts_with('.') {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&path) {
                    files.insert(key, content);
                }
            }
        }
        files
    };
    (read_dir_files("scripts"), read_dir_files("references"))
}

/// Parses SKILL.md files and populates a SkillRegistry.
pub struct SkillLoader;

impl SkillLoader {
    /// Synchronous version: scan subdirectories for SKILL.md files.
    /// Each skill lives in its own directory: `skills/<name>/SKILL.md`.
    /// Used during startup assembly.
    pub fn load_directory_sync(dir: &Path, registry: &mut SkillRegistry) -> std::io::Result<usize> {
        let mut count = 0;
        let entries = std::fs::read_dir(dir)?;

        for entry in entries {
            let entry = entry?;
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }
            let skill_file = subdir.join("SKILL.md");
            if skill_file.is_file() {
                match Self::load_file_sync(&skill_file) {
                    Ok(skill) => {
                        tracing::info!(
                            "SkillLoader: loaded '{}' from {}",
                            skill.name,
                            skill_file.display()
                        );
                        registry.register(skill);
                        count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "SkillLoader: failed to parse {}: {e}",
                            skill_file.display()
                        );
                    }
                }
            }
        }

        // Warn if always-active skills are collectively large — they are injected
        // into every system prompt and the ContextWindow cannot compact them.
        let always_chars: usize = registry
            .list_all()
            .iter()
            .filter(|s| s.always)
            .map(|s| s.prompt.len())
            .sum();
        if always_chars > 4096 {
            tracing::warn!(
                "always-active skills total {always_chars} chars — injected into every system prompt and cannot be compacted"
            );
        }

        Ok(count)
    }

    /// Load all skill directories from a directory into the registry (async).
    pub async fn load_directory(
        dir: &Path,
        registry: &mut SkillRegistry,
    ) -> std::io::Result<usize> {
        let mut count = 0;
        let mut entries = tokio::fs::read_dir(dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }
            let skill_file = subdir.join("SKILL.md");
            if tokio::fs::try_exists(&skill_file).await.unwrap_or(false) {
                match Self::load_file(&skill_file).await {
                    Ok(skill) => {
                        tracing::info!(
                            "SkillLoader: loaded '{}' from {}",
                            skill.name,
                            skill_file.display()
                        );
                        registry.register(skill);
                        count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "SkillLoader: failed to parse {}: {e}",
                            skill_file.display()
                        );
                    }
                }
            }
        }

        let always_chars: usize = registry
            .list_all()
            .iter()
            .filter(|s| s.always)
            .map(|s| s.prompt.len())
            .sum();
        if always_chars > 4096 {
            tracing::warn!(
                "always-active skills total {always_chars} chars — injected into every system prompt and cannot be compacted"
            );
        }

        Ok(count)
    }

    /// Parse a single SKILL.md file into a Skill.
    pub async fn load_file(path: &Path) -> Result<Skill, String> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse_skill(path, &content)
    }

    /// Parse a SKILL.md file synchronously.
    pub fn load_file_sync(path: &Path) -> Result<Skill, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse_skill(path, &content)
    }

    fn parse_skill(path: &Path, content: &str) -> Result<Skill, String> {
        // Parse optional YAML frontmatter (delimited by --- lines)
        let (frontmatter, body) = parse_frontmatter(content);

        // Use frontmatter `name` if present (Agent Skills standard),
        // otherwise fall back to the parent directory name.
        let name = frontmatter
            .as_ref()
            .and_then(|fm| fm.name.clone())
            .unwrap_or_else(|| {
                path.parent()
                    .and_then(|p| p.file_stem())
                    .and_then(|s| s.to_str())
                    .map(String::from)
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
            });

        let description = frontmatter
            .as_ref()
            .and_then(|fm| fm.description.clone())
            .or_else(|| {
                body.lines()
                    .find(|l| l.starts_with('#'))
                    .map(|l| l.trim_start_matches('#').trim().to_string())
            })
            .unwrap_or_else(|| body.lines().next().unwrap_or("").to_string());

        let paths = frontmatter
            .as_ref()
            .and_then(|fm| fm.paths.as_ref())
            .map(|raw_patterns| {
                raw_patterns
                    .iter()
                    .filter_map(|p| {
                        glob::Pattern::new(p)
                            .inspect_err(|e| {
                                tracing::warn!(
                                    "SkillLoader: bad glob pattern '{}' in {}: {e}",
                                    p,
                                    path.display()
                                )
                            })
                            .ok()
                    })
                    .collect::<Vec<_>>()
            });

        // Extract emphasis from body
        let (body, emphasis) = split_emphasis(body);

        let auto_evolve = frontmatter.as_ref().is_some_and(|fm| fm.auto_evolve);
        let evolve_min_confidence = frontmatter
            .as_ref()
            .map_or(default_min_confidence(), |fm| fm.evolve_min_confidence);
        let evolve_max_revisions_per_session =
            frontmatter.as_ref().map_or(default_max_revisions(), |fm| {
                fm.evolve_max_revisions_per_session
            });
        let evolve_reflection_threshold = frontmatter
            .as_ref()
            .map_or(default_reflection_threshold(), |fm| {
                fm.evolve_reflection_threshold
            });

        let skill_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let (scripts, references) = collect_bundled_files(skill_dir);

        Ok(Skill {
            name,
            description,
            prompt: body.to_string(),
            source_path: path.to_path_buf(),
            always: frontmatter.as_ref().is_some_and(|fm| fm.always),
            paths,
            emphasis,
            auto_evolve,
            evolve_min_confidence,
            evolve_max_revisions_per_session,
            evolve_reflection_threshold,
            scripts,
            references,
        })
    }
}

/// Extract emphasis section from skill body.
/// Recognizes only the LAST top-level `## Emphasis` heading.
/// The heading line itself is stripped; only the content below it is kept.
fn split_emphasis(content: &str) -> (String, Option<String>) {
    // Find last occurrence of "\n## Emphasis" or "\r\n## Emphasis"
    // (not "###Emphasis" or more hashes)
    let mut last_pos: Option<usize> = None;
    let mut search_start = 0;
    while let Some(pos) = content[search_start..].find("## Emphasis") {
        let abs_pos = search_start + pos;
        // Check that it's a level-2 heading: preceded by \n (or at start of file)
        let is_top_level = if abs_pos == 0 {
            true
        } else {
            content.as_bytes().get(abs_pos - 1) == Some(&b'\n')
        };
        if is_top_level {
            let after = &content[abs_pos..];
            if let Some(rest) = after.strip_prefix("## Emphasis")
                && !rest.starts_with('#')
            {
                last_pos = Some(abs_pos);
            }
        }
        search_start = abs_pos + 2;
    }

    match last_pos {
        Some(pos) => {
            let body = content[..pos].trim_end().to_string();
            let raw_emphasis = content[pos..].trim();
            // Strip the "## Emphasis" heading line from the emphasis content
            let emphasis = raw_emphasis
                .strip_prefix("## Emphasis")
                .map(|s| s.trim_start().to_string())
                .unwrap_or_else(|| raw_emphasis.to_string());
            let emphasis = if emphasis.is_empty() {
                None
            } else {
                Some(emphasis)
            };
            (body, emphasis)
        }
        None => (content.to_string(), None),
    }
}

/// Parse YAML frontmatter from content.
///
/// Frontmatter is delimited by `---` lines at the start of the file:
/// ```markdown
/// ---
/// always: true
/// paths:
///   - "src/**/*.rs"
/// ---
/// # Skill Title
/// ...
/// ```
fn parse_frontmatter(content: &str) -> (Option<SkillFrontmatter>, &str) {
    // Try \n line endings first, then \r\n
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"));
    if let Some(rest) = rest {
        // Find closing --- delimiter (on its own line)
        let (fm_str, body) = if let Some(end) = rest.find("\n---\n") {
            (&rest[..end], &rest[end + 5..])
        } else if let Some(end) = rest.find("\n---\r\n") {
            (&rest[..end], &rest[end + 6..])
        } else if let Some(end) = rest.find("\r\n---\r\n") {
            (&rest[..end], &rest[end + 7..])
        } else if let Some(end) = rest.find("\r\n---\n") {
            (&rest[..end], &rest[end + 6..])
        } else if let Some(end) = rest.find("\n---") {
            // End of file (no trailing newline after ---)
            (&rest[..end], &rest[end + 4..])
        } else if let Some(end) = rest.find("\r\n---") {
            (&rest[..end], &rest[end + 5..])
        } else {
            return (None, content);
        };
        match serde_yaml::from_str::<SkillFrontmatter>(fm_str) {
            Ok(fm) => return (Some(fm), body),
            Err(e) => {
                tracing::warn!("Failed to parse frontmatter: {e}");
            }
        }
    }
    (None, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_skill(dir: &Path, name: &str, content: &str) {
        let subdir = dir.join(name);
        std::fs::create_dir(&subdir).unwrap();
        let path = subdir.join("SKILL.md");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[tokio::test]
    async fn load_single_file() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "code-review",
            "# Code Review\n\nAlways check for security vulnerabilities before writing code.\n",
        );

        let skill = SkillLoader::load_file(&dir.path().join("code-review").join("SKILL.md"))
            .await
            .unwrap();
        assert_eq!(skill.name, "code-review");
        assert_eq!(skill.description, "Code Review");
        assert!(skill.prompt.contains("security vulnerabilities"));
    }

    #[tokio::test]
    async fn load_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "safety", "# Safety\nVerify commands.\n");
        write_skill(dir.path(), "style", "# Style\nUse Rust idioms.\n");
        // Empty subdir without SKILL.md should be ignored
        std::fs::create_dir(dir.path().join("empty")).unwrap();
        // Non-directory file should be ignored
        std::fs::write(dir.path().join("readme.txt"), "not a skill").unwrap();

        let mut reg = SkillRegistry::new();
        let count = SkillLoader::load_directory(dir.path(), &mut reg)
            .await
            .unwrap();
        assert_eq!(count, 2);
        assert!(reg.get("safety").is_some());
        assert!(reg.get("style").is_some());
        assert!(reg.get("readme").is_none());
        assert!(reg.get("empty").is_none());
    }

    #[test]
    fn parse_frontmatter_always() {
        let content = "---\nalways: true\n---\n# Always Skill\n\nInstructions here.\n";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(fm.always);
        assert!(fm.paths.is_none());
        assert_eq!(body, "# Always Skill\n\nInstructions here.\n");
    }

    #[test]
    fn parse_frontmatter_paths() {
        let content = "---\npaths:\n  - \"src/**/*.rs\"\n  - \"Cargo.toml\"\n---\n# Rust Reviewer\n\nCheck for unsafe.\n";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(!fm.always);
        assert_eq!(
            fm.paths,
            Some(vec!["src/**/*.rs".to_string(), "Cargo.toml".to_string()])
        );
        assert_eq!(body, "# Rust Reviewer\n\nCheck for unsafe.\n");
    }

    #[test]
    fn parse_frontmatter_none() {
        let content = "# Plain Skill\n\nNo frontmatter.\n";
        let (fm, body) = parse_frontmatter(content);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn parse_frontmatter_description_override() {
        let content = "---\ndescription: \"Custom desc\"\n---\n# Title\n\nBody.\n";
        let (fm, _body) = parse_frontmatter(content);
        assert!(fm.is_some());
        assert_eq!(fm.unwrap().description, Some("Custom desc".to_string()));
    }

    #[tokio::test]
    async fn load_file_with_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "conditional",
            "---\nalways: true\npaths:\n  - \"src/**/*.rs\"\n---\n# Conditional Skill\n\nOnly for Rust files.\n",
        );

        let skill = SkillLoader::load_file(&dir.path().join("conditional").join("SKILL.md"))
            .await
            .unwrap();
        assert_eq!(skill.name, "conditional");
        assert_eq!(skill.description, "Conditional Skill");
        assert!(skill.always);
        assert!(skill.paths.is_some());
        assert_eq!(skill.paths.as_ref().unwrap().len(), 1);
        assert!(skill.paths.as_ref().unwrap()[0].matches("src/main.rs"));
        assert_eq!(
            skill.prompt,
            "# Conditional Skill\n\nOnly for Rust files.\n"
        );
        assert!(skill.emphasis.is_none());
    }

    // ── Emphasis tests ──────────────────────────────────────

    #[test]
    fn parse_skill_with_emphasis() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "test-skill",
            "# Test Skill\n\nSome instructions.\n\n## Emphasis\n\nKey point: always validate inputs.\n",
        );
        let skill =
            SkillLoader::load_file_sync(&dir.path().join("test-skill").join("SKILL.md")).unwrap();
        assert_eq!(skill.prompt, "# Test Skill\n\nSome instructions.");
        assert_eq!(
            skill.emphasis.as_deref(),
            Some("Key point: always validate inputs.")
        );
    }

    #[test]
    fn parse_skill_emphasis_strips_heading() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "test-skill",
            "# Test\n\nBody.\n\n## Emphasis\n\nRemember this.\n",
        );
        let skill =
            SkillLoader::load_file_sync(&dir.path().join("test-skill").join("SKILL.md")).unwrap();
        // The heading "## Emphasis" should not appear in the emphasis content
        assert!(
            !skill
                .emphasis
                .as_deref()
                .unwrap_or("")
                .contains("## Emphasis")
        );
        assert_eq!(skill.emphasis.as_deref(), Some("Remember this."));
    }

    #[test]
    fn parse_skill_emphasis_last_only() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "test-skill",
            "# Test\n\nBody.\n\n## Emphasis\n\nFirst emphasis.\n\n## Emphasis\n\nLast emphasis.\n",
        );
        let skill =
            SkillLoader::load_file_sync(&dir.path().join("test-skill").join("SKILL.md")).unwrap();
        assert_eq!(skill.emphasis.as_deref(), Some("Last emphasis."));
        // Body should contain the first emphasis section (only the last ## Emphasis is extracted)
        assert!(skill.prompt.contains("First emphasis."));
    }

    #[test]
    fn parse_skill_no_emphasis() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(
            dir.path(),
            "test-skill",
            "# Plain Skill\n\nJust instructions, no emphasis heading.\n",
        );
        let skill =
            SkillLoader::load_file_sync(&dir.path().join("test-skill").join("SKILL.md")).unwrap();
        assert_eq!(skill.emphasis, None);
        assert_eq!(
            skill.prompt,
            "# Plain Skill\n\nJust instructions, no emphasis heading.\n"
        );
    }

    #[test]
    fn parse_skill_empty_emphasis() {
        let dir = tempfile::tempdir().unwrap();
        write_skill(dir.path(), "test-skill", "# Test\n\nBody.\n\n## Emphasis\n");
        let skill =
            SkillLoader::load_file_sync(&dir.path().join("test-skill").join("SKILL.md")).unwrap();
        assert_eq!(skill.emphasis, None);
        assert_eq!(skill.prompt, "# Test\n\nBody.");
    }
}

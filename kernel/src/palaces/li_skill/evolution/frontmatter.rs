//! Frontmatter utilities for skill revision — YAML parsing, protection, and comparison.

use crate::palaces::li_skill::Skill;

/// Evolution frontmatter fields to protect from LLM modification.
const EVOLUTION_FIELDS: &[&str] = &[
    "auto_evolve",
    "evolve_min_confidence",
    "evolve_max_revisions_per_session",
    "evolve_reflection_threshold",
];

/// Inject evolution fields into a YAML frontmatter value.
///
/// If `fm` is a Mapping, mutates it in place. If it parsed as scalar/null/sequence
/// (malformed LLM output), replaces it with a fresh Mapping containing only the
/// evolution fields.
pub(crate) fn inject_evolution_fields(fm: &mut serde_yaml::Value, skill: &Skill) {
    use serde_yaml::Value;

    // Coerce non-mapping YAML to a fresh Mapping
    if !fm.is_mapping() {
        *fm = Value::Mapping(serde_yaml::Mapping::new());
    }

    let mapping = fm.as_mapping_mut().unwrap(); // guaranteed after coercion

    mapping.insert(
        Value::String("auto_evolve".into()),
        Value::Bool(skill.auto_evolve),
    );
    mapping.insert(
        Value::String("evolve_min_confidence".into()),
        Value::Number(serde_yaml::Number::from(
            if skill.evolve_min_confidence.is_finite() {
                skill.evolve_min_confidence
            } else {
                0.7
            },
        )),
    );
    mapping.insert(
        Value::String("evolve_max_revisions_per_session".into()),
        Value::Number(serde_yaml::Number::from(
            skill.evolve_max_revisions_per_session,
        )),
    );
    mapping.insert(
        Value::String("evolve_reflection_threshold".into()),
        Value::Number(serde_yaml::Number::from(skill.evolve_reflection_threshold)),
    );
}

/// Extract the YAML string between --- delimiters.
pub(crate) fn extract_frontmatter_str(content: &str) -> &str {
    let cleaned = super::helpers::strip_markdown_fence(content);

    if let Some(rest) = cleaned
        .strip_prefix("---\n")
        .or_else(|| cleaned.strip_prefix("---\r\n"))
        && let Some(end) = rest.find("\n---").or_else(|| rest.find("\r\n---"))
    {
        return &rest[..end];
    }
    ""
}

/// Split frontmatter content into (fm_str, body_str, line_ending).
/// Handles all 6 closing delimiter variants and both \n / \r\n line endings.
pub(crate) fn split_frontmatter_parts(cleaned: &str) -> Result<(&str, &str, &str), String> {
    let (le, rest) = if let Some(r) = cleaned.strip_prefix("---\r\n") {
        ("\r\n", r)
    } else if let Some(r) = cleaned.strip_prefix("---\n") {
        ("\n", r)
    } else if let Some(r) = cleaned.strip_prefix("---") {
        // Bare --- with no newline (unusual but handled)
        ("\n", r)
    } else {
        return Err("revised content missing frontmatter".into());
    };

    // Find closing delimiter — try all 6 variants
    let (fm_str, body) = if let Some(e) = rest.find("\n---\n") {
        (&rest[..e], &rest[e + 5..])
    } else if let Some(e) = rest.find("\n---\r\n") {
        (&rest[..e], &rest[e + 6..])
    } else if let Some(e) = rest.find("\r\n---\r\n") {
        (&rest[..e], &rest[e + 7..])
    } else if let Some(e) = rest.find("\r\n---\n") {
        (&rest[..e], &rest[e + 6..])
    } else if let Some(e) = rest.find("\n---") {
        (&rest[..e], &rest[e + 4..])
    } else if let Some(e) = rest.find("\r\n---") {
        (&rest[..e], &rest[e + 5..])
    } else {
        return Err("revised content has unclosed frontmatter".into());
    };

    Ok((fm_str, body, le))
}

/// Split content into (frontmatter_str, body_str).
pub(crate) fn split_fm_and_body(content: &str) -> (&str, &str) {
    let cleaned = super::helpers::strip_markdown_fence(content);
    match split_frontmatter_parts(cleaned) {
        Ok((fm, body, _)) => (fm, body),
        Err(_) => ("", cleaned),
    }
}

/// Compare YAML values, ignoring evolution fields and serialization formatting.
/// Returns true if both frontmatter AND body are semantically identical.
pub(crate) fn revision_semantically_equal(old: &str, new: &str) -> bool {
    let (fm_old, body_old) = split_fm_and_body(old);
    let (fm_new, body_new) = split_fm_and_body(new);
    if fm_old.is_empty() || fm_new.is_empty() {
        return false;
    }
    // Body must match exactly
    if body_old.trim() != body_new.trim() {
        return false;
    }
    // Frontmatter: compare YAML values, stripping evolution fields
    let mut val_old: serde_yaml::Value = match serde_yaml::from_str(fm_old) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let mut val_new: serde_yaml::Value = match serde_yaml::from_str(fm_new) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if let Some(m) = val_old.as_mapping_mut() {
        for field in EVOLUTION_FIELDS {
            m.remove(serde_yaml::Value::String((*field).into()));
        }
    }
    if let Some(m) = val_new.as_mapping_mut() {
        for field in EVOLUTION_FIELDS {
            m.remove(serde_yaml::Value::String((*field).into()));
        }
    }
    val_old == val_new
}

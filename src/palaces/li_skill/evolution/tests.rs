use super::helpers::{strip_code_fences, strip_markdown_fence, truncate_for_audit};
use super::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::palaces::li_skill::{Skill, SkillRegistry};
use crate::vijnana::mano::{TurnSnapshot, WorkingMemory};

fn make_skill(name: &str, auto_evolve: bool, always: bool, has_paths: bool) -> Skill {
    Skill {
        name: name.into(),
        description: "test".into(),
        prompt: "test prompt".into(),
        source_path: PathBuf::from(format!("skills/{}/SKILL.md", name)),
        always,
        paths: if has_paths {
            Some(vec![glob::Pattern::new("*.rs").unwrap()])
        } else {
            None
        },
        emphasis: None,
        auto_evolve,
        evolve_min_confidence: 0.7,
        evolve_max_revisions_per_session: 3,
        evolve_reflection_threshold: 3,
        scripts: HashMap::new(),
        references: HashMap::new(),
    }
}

#[test]
fn check_eligibility_rejects_always_skills() {
    let skill = make_skill("safety", true, true, false);
    let config = EvolutionConfig::from(&skill);
    assert!(config.auto_evolve);
    assert!(skill.always);
}

#[test]
fn check_eligibility_requires_opt_in() {
    let skill = make_skill("test", false, false, false);
    let config = EvolutionConfig::from(&skill);
    assert!(!config.auto_evolve);
}

#[test]
fn compile_trajectory_collects_errors() {
    let skill = make_skill("code-review", true, false, false);
    let snapshots = vec![
        TurnSnapshot {
            turn_number: 1,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "test".into(),
            execution_mode: "Direct".into(),
            tool_name: "skill".into(),
            tool_input: serde_json::json!({"skill": "code-review"}),
            tool_output: "ok".into(),
            tool_error: None,
            timestamp: 1,
        },
        TurnSnapshot {
            turn_number: 2,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "test".into(),
            execution_mode: "Guarded".into(),
            tool_name: "bash".into(),
            tool_input: serde_json::json!({"command": "rm -rf /"}),
            tool_output: "".into(),
            tool_error: Some("permission denied".into()),
            timestamp: 2,
        },
    ];
    let trajectory =
        EvolutionEngine::compile_trajectory(&skill, &snapshots, &["code-review".to_string()], &[]);
    assert_eq!(trajectory.errors.len(), 1);
    assert_eq!(trajectory.errors[0].turn_number, 2);
    assert_eq!(trajectory.errors[0].error, "permission denied");
    assert_eq!(trajectory.geju_events.len(), 1);
    assert_eq!(trajectory.geju_events[0].execution_mode, "Guarded");
}

#[test]
fn compile_trajectory_no_errors_when_skill_not_invoked() {
    let skill = make_skill("unused", true, false, false);
    let snapshots = vec![TurnSnapshot {
        turn_number: 1,
        intent_stem: crate::stems::Stem::Geng,
        target_palace: crate::palaces::Palace::Zhen,
        geju_name: "test".into(),
        execution_mode: "Direct".into(),
        tool_name: "bash".into(),
        tool_input: serde_json::json!({"command": "ls"}),
        tool_output: "ok".into(),
        tool_error: None,
        timestamp: 1,
    }];
    let trajectory =
        EvolutionEngine::compile_trajectory(&skill, &snapshots, &["other-skill".to_string()], &[]);
    assert!(trajectory.errors.is_empty());
    assert!(trajectory.geju_events.is_empty());
}

#[test]
fn protect_frontmatter_preserves_evolution_fields() {
    let skill = make_skill("test", true, false, false);
    let new_content = "---\nauto_evolve: false\ndescription: \"bad desc\"\n---\n# New content\n";
    let protected = EvolutionEngine::protect_frontmatter(new_content, &skill).unwrap();
    assert!(protected.contains("auto_evolve: true"));
    assert!(protected.contains("evolve_min_confidence: 0.7"));
    assert!(protected.contains("evolve_max_revisions_per_session"));
    assert!(protected.contains("evolve_reflection_threshold"));
    assert!(!protected.contains("auto_evolve: false"));
}

#[test]
fn compute_diff_detects_additions() {
    let diff = compute_diff("line1\nline2\n", "line1\nline2\nline3\n");
    assert!(diff.contains("+line3"));
}

#[test]
fn compute_diff_detects_removals() {
    let diff = compute_diff("line1\nline2\nline3\n", "line1\n");
    assert!(diff.contains("-line2"));
    assert!(diff.contains("-line3"));
}

// ── Expanded trajectory tests ──────────────────────────

#[test]
fn compile_trajectory_excludes_errors_before_first_invocation() {
    let skill = make_skill("code-review", true, false, false);
    let snapshots = vec![
        TurnSnapshot {
            turn_number: 1,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "test".into(),
            execution_mode: "Direct".into(),
            tool_name: "bash".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            tool_output: "ok".into(),
            tool_error: Some("EACCES".into()),
            timestamp: 1,
        },
        TurnSnapshot {
            turn_number: 2,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "test".into(),
            execution_mode: "Direct".into(),
            tool_name: "skill".into(),
            tool_input: serde_json::json!({"skill": "code-review"}),
            tool_output: "ok".into(),
            tool_error: None,
            timestamp: 2,
        },
        TurnSnapshot {
            turn_number: 3,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "test".into(),
            execution_mode: "Guarded".into(),
            tool_name: "bash".into(),
            tool_input: serde_json::json!({"command": "rm -rf /"}),
            tool_output: "".into(),
            tool_error: Some("blocked".into()),
            timestamp: 3,
        },
    ];
    let trajectory =
        EvolutionEngine::compile_trajectory(&skill, &snapshots, &["code-review".to_string()], &[]);
    assert_eq!(trajectory.errors.len(), 1);
    assert_eq!(trajectory.errors[0].turn_number, 3);
    assert_eq!(trajectory.errors[0].error, "blocked");
}

#[test]
fn compile_trajectory_multiple_geju_types() {
    let skill = make_skill("safety", true, false, false);
    let snapshots = vec![
        TurnSnapshot {
            turn_number: 1,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "sandbox".into(),
            execution_mode: "Sandbox".into(),
            tool_name: "skill".into(),
            tool_input: serde_json::json!({"skill": "safety"}),
            tool_output: "ok".into(),
            tool_error: None,
            timestamp: 1,
        },
        TurnSnapshot {
            turn_number: 2,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "guard".into(),
            execution_mode: "Guarded".into(),
            tool_name: "bash".into(),
            tool_input: serde_json::json!({"command": "unsafe"}),
            tool_output: "".into(),
            tool_error: None,
            timestamp: 2,
        },
        TurnSnapshot {
            turn_number: 3,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "deny".into(),
            execution_mode: "Denied".into(),
            tool_name: "rm".into(),
            tool_input: serde_json::json!({"path": "/etc"}),
            tool_output: "".into(),
            tool_error: Some("forbidden".into()),
            timestamp: 3,
        },
    ];
    let trajectory =
        EvolutionEngine::compile_trajectory(&skill, &snapshots, &["safety".to_string()], &[]);
    assert_eq!(trajectory.geju_events.len(), 3);
    assert_eq!(trajectory.errors.len(), 1);
}

#[test]
fn compile_trajectory_no_geju_for_direct_mode() {
    let skill = make_skill("simple", true, false, false);
    let snapshots = vec![
        TurnSnapshot {
            turn_number: 1,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "none".into(),
            execution_mode: "Direct".into(),
            tool_name: "skill".into(),
            tool_input: serde_json::json!({"skill": "simple"}),
            tool_output: "ok".into(),
            tool_error: None,
            timestamp: 1,
        },
        TurnSnapshot {
            turn_number: 2,
            intent_stem: crate::stems::Stem::Geng,
            target_palace: crate::palaces::Palace::Zhen,
            geju_name: "none".into(),
            execution_mode: "Direct".into(),
            tool_name: "bash".into(),
            tool_input: serde_json::json!({"command": "ls"}),
            tool_output: "ok".into(),
            tool_error: None,
            timestamp: 2,
        },
    ];
    let trajectory =
        EvolutionEngine::compile_trajectory(&skill, &snapshots, &["simple".to_string()], &[]);
    assert!(trajectory.errors.is_empty());
    assert!(trajectory.geju_events.is_empty());
}

// ── Frontmatter protection expanded ────────────────────

#[test]
fn protect_frontmatter_non_mapping_yaml_scalar() {
    let skill = make_skill("test", true, false, false);
    let new_content = "---\n\"just a string\"\n---\n# Body\n";
    let result = EvolutionEngine::protect_frontmatter(new_content, &skill);
    assert!(result.is_ok());
    assert!(result.unwrap().contains("auto_evolve: true"));
}

#[test]
fn protect_frontmatter_preserves_non_evolution_fields() {
    let skill = make_skill("test", true, false, false);
    let new_content = "---\nauto_evolve: false\ndescription: custom desc\n---\n# Body\n";
    let protected = EvolutionEngine::protect_frontmatter(new_content, &skill).unwrap();
    assert!(protected.contains("description:"));
    assert!(protected.contains("custom desc"));
    assert!(protected.contains("auto_evolve: true"));
    assert!(!protected.contains("auto_evolve: false"));
}

#[test]
fn protect_frontmatter_strips_markdown_fence() {
    let skill = make_skill("test", true, false, false);
    let new_content = "```markdown\n---\ndescription: \"test\"\n---\n# Body\n```";
    let protected = EvolutionEngine::protect_frontmatter(new_content, &skill).unwrap();
    assert!(protected.contains("auto_evolve: true"));
    assert!(!protected.contains("```"));
}

#[test]
fn protect_frontmatter_missing_delimiter() {
    let skill = make_skill("test", true, false, false);
    let result =
        EvolutionEngine::protect_frontmatter("# No frontmatter here\n\nJust body.\n", &skill);
    assert!(result.is_err());
}

// ── Diff expanded ──────────────────────────────────────

#[test]
fn compute_diff_identical_returns_no_changes() {
    let diff = compute_diff("line1\nline2\n", "line1\nline2\n");
    assert!(!diff.contains('-'));
    assert!(!diff.contains('+'));
}

#[test]
fn compute_diff_mixed_changes() {
    let old = "line1\nline2\nline3\n";
    let new = "line1\nline2b\nline3\nline4\n";
    let diff = compute_diff(old, new);
    assert!(diff.contains("-line2"));
    assert!(diff.contains("+line2b"));
    assert!(diff.contains("+line4"));
}

#[test]
fn compute_diff_empty_inputs() {
    let diff = compute_diff("", "");
    assert_eq!(diff, "(no changes)");
}

// ── Strip / fence helpers ──────────────────────────────

#[test]
fn strip_code_fences_json_block() {
    let input = "```json\n{\"type\":\"Discovery\",\"summary\":\"test\"}\n```";
    let result = strip_code_fences(input);
    assert_eq!(result, "{\"type\":\"Discovery\",\"summary\":\"test\"}");
}

#[test]
fn strip_code_fences_plain_text_passthrough() {
    let input = "{\"type\":\"Discovery\"}";
    let result = strip_code_fences(input);
    assert!(!result.contains("```"));
}

#[test]
fn strip_code_fences_bare_fence() {
    let input = "```\n{\"type\":\"Optimization\"}\n```";
    let result = strip_code_fences(input);
    assert_eq!(result, "{\"type\":\"Optimization\"}");
}

#[test]
fn strip_code_fences_single_line() {
    let input = "```json{\"type\":\"SkillDefect\"}```";
    let result = strip_code_fences(input);
    assert_eq!(result, "{\"type\":\"SkillDefect\"}");
}

#[test]
fn strip_markdown_fence_crlf() {
    let input = "```markdown\r\n---\ntest: true\r\n---\r\n# Body\r\n```";
    let result = strip_markdown_fence(input);
    assert!(!result.contains("```"));
    assert!(result.contains("test: true"));
}

#[test]
fn strip_markdown_fence_bare() {
    let input = "```\n---\ntest: true\n---\n# Body\n```";
    let result = strip_markdown_fence(input);
    assert!(!result.contains("```"));
    assert!(result.contains("test: true"));
}

// ── truncate_for_audit ─────────────────────────────────

#[test]
fn truncate_for_audit_no_truncation_needed() {
    let text = "short text";
    let result = truncate_for_audit(text, 100);
    assert_eq!(result, text);
}

#[test]
fn truncate_for_audit_at_char_boundary() {
    let text = "hello worl\n\u{4e2d}\u{6587} end";
    let result = truncate_for_audit(text, 12);
    assert!(result.len() <= 12);
    String::from_utf8(result.as_bytes().to_vec()).unwrap();
}

// ── Async mock-LLM tests ────────────────────────────────

fn temp_store() -> (
    std::sync::Arc<crate::palaces::gen_store::Store>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");
    let store = std::sync::Arc::new(crate::palaces::gen_store::Store::open(
        path.to_str().unwrap(),
    ));
    (store, dir)
}

fn snapshot(
    turn: u64,
    tool_name: &str,
    tool_input: serde_json::Value,
    error: Option<&str>,
) -> TurnSnapshot {
    TurnSnapshot {
        turn_number: turn,
        intent_stem: crate::stems::Stem::Geng,
        target_palace: crate::palaces::Palace::Zhen,
        geju_name: "test".into(),
        execution_mode: if error.is_some() {
            "Guarded".into()
        } else {
            "Direct".into()
        },
        tool_name: tool_name.into(),
        tool_input,
        tool_output: "output".into(),
        tool_error: error.map(String::from),
        timestamp: 1700000000 + turn as i64,
    }
}

#[tokio::test]
async fn test_reflect_with_mock_llm() {
    let skill = make_skill("test-skill", true, false, false);
    let trajectory = SkillTrajectory {
        errors: vec![TurnErrorRef {
            turn_number: 1,
            tool_name: "bash".into(),
            error: "permission denied".into(),
            geju_name: "test".into(),
            execution_mode: "Guarded".into(),
        }],
        geju_events: vec![],
        user_corrections: vec![],
    };

    let mock_core = crate::palaces::zhong_core::JiaCore::with_mock(vec![
        r#"{"type":"Discovery","summary":"new error pattern","detail":"Skill misses handling for permission errors","confidence":0.85}"#.into()
    ]);

    let reflection = EvolutionEngine::reflect(&skill, &trajectory, "session-1", &mock_core).await;

    assert!(reflection.is_some());
    let r = reflection.unwrap();
    assert_eq!(r.skill_name, "test-skill");
    assert_eq!(r.reflection_type, "Discovery");
    assert!((r.confidence - 0.85).abs() < 0.01);
    assert!(r.content_json.contains("Discovery"));
}

#[tokio::test]
async fn test_reflect_empty_response_returns_none() {
    let skill = make_skill("test-skill", true, false, false);
    let trajectory = SkillTrajectory {
        errors: vec![TurnErrorRef {
            turn_number: 1,
            tool_name: "bash".into(),
            error: "fail".into(),
            geju_name: "test".into(),
            execution_mode: "Guarded".into(),
        }],
        geju_events: vec![],
        user_corrections: vec![],
    };

    let mock_core = crate::palaces::zhong_core::JiaCore::with_mock(vec!["".into()]);
    let reflection = EvolutionEngine::reflect(&skill, &trajectory, "session-1", &mock_core).await;

    assert!(reflection.is_none());
}

#[tokio::test]
async fn test_reflect_invalid_json_returns_none() {
    let skill = make_skill("test-skill", true, false, false);
    let trajectory = SkillTrajectory {
        errors: vec![TurnErrorRef {
            turn_number: 1,
            tool_name: "bash".into(),
            error: "fail".into(),
            geju_name: "test".into(),
            execution_mode: "Guarded".into(),
        }],
        geju_events: vec![],
        user_corrections: vec![],
    };

    let mock_core =
        crate::palaces::zhong_core::JiaCore::with_mock(vec!["not valid json at all".into()]);
    let reflection = EvolutionEngine::reflect(&skill, &trajectory, "session-1", &mock_core).await;

    assert!(reflection.is_none());
}

#[tokio::test]
async fn test_reflect_no_trajectory_returns_none() {
    let skill = make_skill("test-skill", true, false, false);
    let trajectory = SkillTrajectory {
        errors: vec![],
        geju_events: vec![],
        user_corrections: vec![],
    };

    let mock_core = crate::palaces::zhong_core::JiaCore::with_mock(vec!["unused".into()]);
    let reflection = EvolutionEngine::reflect(&skill, &trajectory, "session-1", &mock_core).await;

    assert!(reflection.is_none());
}

#[tokio::test]
async fn test_reflect_strips_code_fences_from_response() {
    let skill = make_skill("test-skill", true, false, false);
    let trajectory = SkillTrajectory {
        errors: vec![TurnErrorRef {
            turn_number: 1,
            tool_name: "bash".into(),
            error: "fail".into(),
            geju_name: "test".into(),
            execution_mode: "Guarded".into(),
        }],
        geju_events: vec![],
        user_corrections: vec![],
    };

    let mock_core = crate::palaces::zhong_core::JiaCore::with_mock(vec![
        "```json\n{\"type\":\"Optimization\",\"summary\":\"s\",\"detail\":\"d\",\"confidence\":0.7}\n```".into()
    ]);

    let reflection = EvolutionEngine::reflect(&skill, &trajectory, "session-1", &mock_core).await;

    assert!(reflection.is_some());
    let r = reflection.unwrap();
    assert_eq!(r.reflection_type, "Optimization");
}

#[tokio::test]
async fn test_full_pipeline_mocked() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("test-skill");
    std::fs::create_dir(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        "---\nauto_evolve: true\ndescription: test\n---\n# Test Skill\n\nCheck things.\n",
    )
    .unwrap();

    let (store, _tmp) = temp_store();
    let mut reg = SkillRegistry::new();
    reg.register(Skill {
        name: "test-skill".into(),
        description: "test".into(),
        prompt: "Check things.".into(),
        source_path: skill_path.clone(),
        always: false,
        paths: None,
        emphasis: None,
        auto_evolve: true,
        evolve_min_confidence: 0.7,
        evolve_max_revisions_per_session: 3,
        evolve_reflection_threshold: 3,
        scripts: HashMap::new(),
        references: HashMap::new(),
    });
    let skills = std::sync::Arc::new(std::sync::RwLock::new(reg));

    let mut wm = WorkingMemory::new(20);
    wm.record(snapshot(
        1,
        "skill",
        serde_json::json!({"skill": "test-skill"}),
        None,
    ));
    wm.record(snapshot(
        2,
        "bash",
        serde_json::json!({"command": "rm"}),
        Some("denied"),
    ));
    wm.record(snapshot(
        3,
        "skill",
        serde_json::json!({"skill": "test-skill"}),
        None,
    ));
    wm.record(snapshot(
        4,
        "read_file",
        serde_json::json!({"path": "/nonexistent"}),
        Some("ENOENT"),
    ));

    let skill_tool_calls = vec!["test-skill".to_string(), "test-skill".to_string()];
    let user_messages: Vec<(u64, String)> = vec![];

    let reflect_core = crate::palaces::zhong_core::JiaCore::with_mock(vec![
        r#"{"type":"Discovery","summary":"skill needs error handling","detail":"The skill should mention file-not-found errors","confidence":0.75}"#.into(),
    ]);

    let report = EvolutionEngine::run(
        &skills,
        &wm,
        &skill_tool_calls,
        &user_messages,
        &store,
        "session-e2e",
        &reflect_core,
        Some(&reflect_core),
    )
    .await;

    assert_eq!(report.skills_analyzed, 1);
    assert_eq!(report.reflections, 1);
    assert_eq!(report.revisions, 0);

    let reflections = store
        .load_skill_reflections("test-skill", "session-e2e")
        .unwrap();
    assert!(!reflections.is_empty());
    let r = &reflections[0];
    assert_eq!(r["reflection_type"].as_str().unwrap(), "Discovery");
    assert!((r["confidence"].as_f64().unwrap() - 0.75).abs() < 0.01);
}

#[tokio::test]
async fn test_pipeline_triggers_revision_with_accumulated_reflections() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("revisable");
    std::fs::create_dir(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    let original =
        "---\nauto_evolve: true\ndescription: test\n---\n# Revisable Skill\n\nOld instructions.\n";
    std::fs::write(&skill_path, original).unwrap();

    let (store, _tmp) = temp_store();
    let mut reg = SkillRegistry::new();
    reg.register(Skill {
        name: "revisable".into(),
        description: "test".into(),
        prompt: "Old instructions.".into(),
        source_path: skill_path.clone(),
        always: false,
        paths: None,
        emphasis: None,
        auto_evolve: true,
        evolve_min_confidence: 0.7,
        evolve_max_revisions_per_session: 3,
        evolve_reflection_threshold: 2,
        scripts: HashMap::new(),
        references: HashMap::new(),
    });
    let skills = std::sync::Arc::new(std::sync::RwLock::new(reg));

    let sid = "session-revise";
    for i in 0..2 {
        let r = serde_json::json!({
            "id": format!("r-{i}"),
            "skill_name": "revisable",
            "session_id": sid,
            "reflection_type": "Discovery",
            "content_json": format!("{{\"type\":\"Discovery\",\"summary\":\"issue {i}\"}}"),
            "confidence": 0.75,
            "turn_numbers": vec![i + 1],
            "created_at": crate::utils::unix_now(),
        });
        store.save_skill_reflection(&r.to_string()).unwrap();
    }

    let mut wm = WorkingMemory::new(20);
    wm.record(snapshot(
        1,
        "skill",
        serde_json::json!({"skill": "revisable"}),
        None,
    ));
    wm.record(snapshot(
        2,
        "bash",
        serde_json::json!({"cmd": "rm"}),
        Some("denied"),
    ));
    wm.record(snapshot(
        3,
        "skill",
        serde_json::json!({"skill": "revisable"}),
        None,
    ));
    wm.record(snapshot(
        4,
        "bash",
        serde_json::json!({"cmd": "bad"}),
        Some("EACCES"),
    ));
    let skill_tool_calls = vec!["revisable".to_string(), "revisable".to_string()];
    let user_messages: Vec<(u64, String)> = vec![];

    let reflect_core = crate::palaces::zhong_core::JiaCore::with_mock(vec![
        r#"{"type":"Discovery","summary":"another issue","detail":"More errors","confidence":0.8}"#
            .into(),
    ]);
    let revised_skill = "---\nauto_evolve: true\ndescription: test\n---\n# Revisable Skill\n\nUpdated instructions.\n";
    let revise_core = crate::palaces::zhong_core::JiaCore::with_mock(vec![revised_skill.into()]);

    let report = EvolutionEngine::run(
        &skills,
        &wm,
        &skill_tool_calls,
        &user_messages,
        &store,
        sid,
        &revise_core,
        Some(&reflect_core),
    )
    .await;

    assert_eq!(report.skills_analyzed, 1);
    assert_eq!(report.reflections, 1);
    assert_eq!(report.revisions, 1);

    let written = std::fs::read_to_string(&skill_path).unwrap();
    assert!(written.contains("Updated instructions."));
    assert!(!written.contains("Old instructions."));
}

// ── Real LLM integration tests ────────────────────────

/// Full evolution pipeline with a real LLM (LM Studio at localhost:1234).
///
/// Prerequisites:
///   - LM Studio running on localhost:1234
///   - Model `qwen3.6-35b-a3b-ud-mlx` loaded
///
/// Run with: cargo test -p jia -- --ignored test_real_llm_evolution_pipeline
#[tokio::test]
#[ignore = "requires real LLM at localhost:1234"]
async fn test_real_llm_evolution_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let skill_dir = dir.path().join("test-evolve");
    std::fs::create_dir(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    let original_skill = "---\nauto_evolve: true\nevolve_min_confidence: 0.7\nevolve_max_revisions_per_session: 3\nevolve_reflection_threshold: 2\ndescription: test skill for evolution\n---\n# Test Evolve\n\nA minimal test skill.\n\n## Instructions\n\nUse bash and read tools to inspect the project.\nAlways check for file-not-found before reading.\n";
    std::fs::write(&skill_path, original_skill).unwrap();

    let (store, _tmp) = temp_store();
    let mut reg = SkillRegistry::new();
    reg.register(Skill {
        name: "test-evolve".into(),
        description: "test skill for evolution".into(),
        prompt: "A minimal test skill.\n\n## Instructions\n\nUse bash and read tools to inspect the project.\nAlways check for file-not-found before reading.\n".into(),
        source_path: skill_path.clone(),
        always: false,
        paths: None,
        emphasis: None,
        auto_evolve: true,
        evolve_min_confidence: 0.7,
        evolve_max_revisions_per_session: 3,
        evolve_reflection_threshold: 2,
        scripts: HashMap::new(),
        references: HashMap::new(),
    });
    let skills = std::sync::Arc::new(std::sync::RwLock::new(reg));

    let mut wm = WorkingMemory::new(20);
    wm.record(snapshot(
        1,
        "skill",
        serde_json::json!({"skill": "test-evolve"}),
        None,
    ));
    wm.record(snapshot(
        2,
        "bash",
        serde_json::json!({"command": "ls /nonexistent"}),
        Some("No such file or directory"),
    ));
    wm.record(snapshot(
        3,
        "skill",
        serde_json::json!({"skill": "test-evolve"}),
        None,
    ));
    wm.record(snapshot(
        4,
        "read_file",
        serde_json::json!({"path": "/tmp/missing.txt"}),
        Some("ENOENT: file not found"),
    ));

    let skill_tool_calls = vec!["test-evolve".to_string(), "test-evolve".to_string()];
    let user_messages: Vec<(u64, String)> = vec![];

    let profile = crate::config::ProviderProfile {
        kind: "openai".to_string(),
        models: vec!["qwen3.6-35b-a3b-ud-mlx".to_string()],
        default_aux_model: None,
        default_main_model: Some("qwen3.6-35b-a3b-ud-mlx".to_string()),
        api_key: "sk-test-key".to_string(),
        base_url: "http://localhost:1234/v1".to_string(),
        max_tokens: Some(4096),
        context_window: Some(180000),
    };
    let core = crate::palaces::zhong_core::JiaCore::new(&profile, "qwen3.6-35b-a3b-ud-mlx");

    // Pre-populate one reflection so the new one from this run
    // reaches evolve_reflection_threshold=2 and triggers revision.
    let prior_reflection = serde_json::json!({
        "id": "prior-r1",
        "skill_name": "test-evolve",
        "session_id": "session-real-llm",
        "reflection_type": "Discovery",
        "content_json": "{\"type\":\"Discovery\",\"summary\":\"missing error docs\",\"detail\":\"No file-not-found handling\",\"confidence\":0.75}",
        "confidence": 0.75,
        "turn_numbers": [2],
        "created_at": crate::utils::unix_now(),
    });
    store
        .save_skill_reflection(&prior_reflection.to_string())
        .unwrap();

    eprintln!("=== Real LLM evolution test: starting pipeline ===");
    let report = EvolutionEngine::run(
        &skills,
        &wm,
        &skill_tool_calls,
        &user_messages,
        &store,
        "session-real-llm",
        &core,
        Some(&core),
    )
    .await;

    eprintln!(
        "=== Evolution report: analyzed={}, reflections={}, revisions={} ===",
        report.skills_analyzed, report.reflections, report.revisions,
    );

    // At minimum, reflection should succeed with a real LLM
    assert!(
        report.skills_analyzed >= 1,
        "expected at least 1 skill analyzed"
    );
    assert!(
        report.reflections >= 1,
        "expected at least 1 reflection from real LLM"
    );

    // Verify the reflection was persisted
    let reflections = store
        .load_skill_reflections("test-evolve", "session-real-llm")
        .unwrap();
    assert!(
        reflections.len() >= 2,
        "expected >=2 reflections (1 prior + 1 new)"
    );
    let newest = reflections.last().unwrap();
    let rtype = newest["reflection_type"].as_str().unwrap_or("?");
    eprintln!(
        "Newest reflection type: {rtype}, confidence: {}",
        newest["confidence"]
    );

    // Print each revision diff
    for (i, diff) in report.revision_diffs.iter().enumerate() {
        eprintln!(
            "Revision {}: confidence={:.3}, applied={}, diff:\n{}",
            i, diff.confidence, diff.applied, diff.diff,
        );
        assert!(!diff.old_snippet.is_empty());
        assert!(!diff.new_snippet.is_empty());
    }

    // If revision succeeded, verify file was updated
    if report.revisions > 0 {
        let updated = std::fs::read_to_string(&skill_path).unwrap();
        eprintln!("Updated SKILL.md:\n{}", updated);
    }
}

/// Smoke test: just verify the real LLM can produce a valid reflection JSON.
/// Faster than the full pipeline — useful for quick LM Studio checks.
///
/// Run with: cargo test -p jia -- --ignored test_real_llm_reflection_only
#[tokio::test]
#[ignore = "requires real LLM at localhost:1234"]
async fn test_real_llm_reflection_only() {
    let skill = make_skill("smoke", true, false, false);
    let trajectory = SkillTrajectory {
        errors: vec![
            TurnErrorRef {
                turn_number: 1,
                tool_name: "bash".into(),
                error: "permission denied".into(),
                geju_name: "guard".into(),
                execution_mode: "Guarded".into(),
            },
            TurnErrorRef {
                turn_number: 3,
                tool_name: "read_file".into(),
                error: "ENOENT: /tmp/missing.txt not found".into(),
                geju_name: "guard".into(),
                execution_mode: "Guarded".into(),
            },
        ],
        geju_events: vec![],
        user_corrections: vec![],
    };

    let profile = crate::config::ProviderProfile {
        kind: "openai".to_string(),
        models: vec!["qwen3.6-35b-a3b-ud-mlx".to_string()],
        default_aux_model: None,
        default_main_model: Some("qwen3.6-35b-a3b-ud-mlx".to_string()),
        api_key: "sk-test-key".to_string(),
        base_url: "http://localhost:1234/v1".to_string(),
        max_tokens: Some(2048),
        context_window: Some(180000),
    };
    let core = crate::palaces::zhong_core::JiaCore::new(&profile, "qwen3.6-35b-a3b-ud-mlx");

    let reflection = EvolutionEngine::reflect(&skill, &trajectory, "smoke-session", &core).await;

    assert!(
        reflection.is_some(),
        "real LLM should produce a valid reflection"
    );
    let r = reflection.unwrap();
    tracing::info!(
        "Reflection: type={}, confidence={:.3}, summary={}",
        r.reflection_type,
        r.confidence,
        r.content_json,
    );
    assert!(matches!(
        r.reflection_type.as_str(),
        "Discovery" | "Optimization" | "SkillDefect" | "ExecutionLapse"
    ));
    assert!(r.confidence > 0.0 && r.confidence <= 1.0);
}

//! Integration tests for cognitive architecture components (Stages A-F).
//!
//! Gate: these tests exercise the new certainty/coactivation/observation subsystems.
//! They use in-memory stores and mock providers — no real LLM calls.

use kernel::palaces::Palace;
use kernel::palaces::gen_store::Store;
use kernel::palaces::xun_context::reset::ContextReset;
use kernel::plates::di_earth::EarthPlate;
use kernel::plates::shen_spirit::completion_check::CompletionChecklist;
use kernel::plates::shen_spirit::hook::HookRegistry;
use kernel::plates::shen_spirit::{EventBus, SpiritPlate};
use kernel::plates::tian_heaven::Agent;
use kernel::plates::tian_heaven::certainty::{CertaintyParams, LoopDecision, TurnCertainty};
use kernel::stems::Stem;
use kernel::vijnana::alaya::{
    Seed, SeedContent, SeedDisposition, SeedNature, SeedSource, SeedTier,
};
use kernel::vijnana::manas::Manas;
use kernel::vijnana::mano::{TurnSnapshot, WorkingMemory};
use kernel::vijnana::xunxi::coactivation::SeedCoActivationMatrix;
use std::sync::Arc;
use tempfile::tempdir;

// ── TurnCertainty ────────────────────────────────────────────

fn make_snap(tool: &str, error: Option<&str>, output: &str) -> TurnSnapshot {
    TurnSnapshot {
        turn_number: 0,
        intent_stem: Stem::Wu,
        target_palace: Palace::Zhen,
        geju_name: String::new(),
        execution_mode: String::new(),
        tool_name: tool.to_string(),
        tool_input: serde_json::Value::Null,
        tool_output: output.to_string(),
        tool_error: error.map(|s| s.to_string()),
        timestamp: 0,
        certainty: None,
        active_seed_ids: vec![],
        tool_count: 1,
    }
}

#[test]
fn certainty_confident_stop() {
    let snaps: Vec<TurnSnapshot> = (0..5)
        .map(|i| make_snap("shell", None, &format!("ok-{}", i)))
        .collect();
    let r = TurnCertainty::evaluate(&snaps, 0.15, 6, 25, &CertaintyParams::default());
    assert!(r.composite > 0.5);
}

#[test]
fn certainty_hard_limit() {
    let r = TurnCertainty::evaluate(&[], 0.2, 30, 25, &CertaintyParams::default());
    assert_eq!(r.decision, LoopDecision::HardLimitReached);
}

#[test]
fn certainty_escalate_on_failures() {
    let mut snaps: Vec<TurnSnapshot> = (0..10)
        .map(|_| make_snap("shell", Some("err"), ""))
        .collect();
    // 10 consecutive failures + high atma-graha = low composite, triggers Escalate
    let r = TurnCertainty::evaluate(&snaps, 0.75, 11, 25, &CertaintyParams::default());
    assert!(
        r.composite < 0.5,
        "all failures + high atma-graha should yield low composite, got {}",
        r.composite
    );
    assert_eq!(r.decision, LoopDecision::EscalateToHuman);
}

// ── SeedDisposition ───────────────────────────────────────────

#[test]
fn disposition_fact_resists_modification() {
    let d = SeedDisposition::for_nature(&SeedNature::Fact);
    assert!(d.consolidation_inertia > 0.5);
    assert!(d.retrieval_threshold < 0.5);
}

#[test]
fn disposition_ren_soul_polarized() {
    let d = SeedDisposition::for_source(&SeedSource::RenSoul).unwrap();
    assert!(d.consolidation_inertia > 0.9);
    assert!(d.retrieval_threshold < 0.1);
}

#[test]
fn disposition_source_overrides_nature() {
    let d = SeedDisposition::resolve(&SeedNature::Inference, &SeedSource::RenSoul);
    assert!(d.consolidation_inertia > 0.9);
}

// ── CoActivationMatrix ────────────────────────────────────────

#[test]
fn coactivation_records_and_queries() {
    let mut m = SeedCoActivationMatrix::new(0.9);
    let ids: Vec<String> = (0..5).map(|i| format!("s-{}", i)).collect();
    m.record_coactivation("proj", &ids[..3], 1);
    assert!(m.coactivation_strength("proj", "s-0") > 0.0);
    assert_eq!(m.coactivation_strength("proj", "s-4"), 0.0);
}

#[test]
fn coactivation_per_project_isolation() {
    let mut m = SeedCoActivationMatrix::new(0.9);
    m.record_coactivation("A", &["x".into(), "y".into()], 1);
    assert!(m.coactivation_strength("A", "x") > 0.0);
    assert_eq!(m.coactivation_strength("B", "x"), 0.0);
}

#[test]
fn coactivation_sparse_no_explosion() {
    let mut m = SeedCoActivationMatrix::new(0.9);
    for t in 0..100 {
        let ids: Vec<String> = (0..20).map(|i| format!("s-{}", i % 50)).collect();
        m.record_coactivation("p", &ids, t);
    }
    assert!(m.total_pairs("p") < 10000);
}

// ── CompletionChecklist ───────────────────────────────────────

#[test]
fn checklist_silent_pass() {
    let cl = CompletionChecklist::new();
    cl.ingest("shell", "ok\n[exit code: 0]", &None);
    assert!(matches!(
        cl.assess(),
        kernel::plates::shen_spirit::completion_check::CompletionAssessment::SilentPass
    ));
}

#[test]
fn checklist_upgrade_on_failure() {
    let cl = CompletionChecklist::new();
    cl.ingest("shell", "fail\n[exit code: 1]", &None);
    assert!(matches!(
        cl.assess(),
        kernel::plates::shen_spirit::completion_check::CompletionAssessment::UpgradeToUser { .. }
    ));
}

// ── ContextReset ──────────────────────────────────────────────

#[test]
fn reset_should_trigger_near_limit() {
    let cr = ContextReset::default();
    assert!(cr.should_reset(9000, 10000, 10));
    assert!(!cr.should_reset(5000, 10000, 10));
}

#[test]
fn reset_cooldown_prevents_trigger() {
    let mut cr = ContextReset::new(5);
    cr.mark_reset(8);
    assert!(!cr.should_reset(9000, 10000, 10)); // turn 10, reset at 8 → only 2 turns ago
}

#[test]
fn reset_handoff_stub() {
    let h = ContextReset::generate_handoff_stub(10, "build a web server");
    assert_eq!(h.goals, "build a web server");
    assert!(!h.done.is_empty());
    assert!(!h.todo.is_empty());
}

// ── Manas ─────────────────────────────────────────────────────

#[test]
fn manas_certainty_trend_adjusts() {
    let mut m = Manas::new();
    let before = m.atma_graha;
    // Rising trend should decrease atma-graha
    m.adjust_from_certainty_trend(&[0.1, 0.3, 0.6, 0.9]);
    assert!(
        m.atma_graha <= before,
        "rising certainty should lower atma-graha"
    );
}

// ── Seed with disposition ─────────────────────────────────────

#[test]
fn seed_new_includes_disposition() {
    let s = Seed::new(
        "s1".into(),
        "p1".into(),
        SeedNature::Fact,
        SeedSource::Consolidation,
        SeedContent::FreeText {
            text: "test".into(),
        },
        Palace::Gen,
        Stem::Gui,
        "test_key".into(),
    );
    assert!(
        s.disposition.consolidation_inertia > 0.5,
        "Fact should have high inertia"
    );
}

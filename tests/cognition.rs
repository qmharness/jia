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
use kernel::vijnana::manas::Manas;
use kernel::vijnana::mano::{TurnSnapshot, WorkingMemory};
use kernel::vijnana::vasana::coactivation::SeedCoActivationMatrix;
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

#[test]
#[test]
#[test]
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

// ── Gate Tests ───────────────────────────────────────────────

use kernel::palaces::kun_config::{SandboxMode, SecuritySection};
use kernel::palaces::qian_permission::PermissionMatrix;
use kernel::plates::ren_human::session_bus::SessionBus;
use kernel::plates::ren_human::{HumanGate, HumanPlate};

fn test_human_plate() -> HumanPlate {
    let security = SecuritySection::default();
    let root = std::env::current_dir().unwrap();
    let perms = Arc::new(PermissionMatrix::from_config(
        &security,
        &root,
        std::path::PathBuf::from("/tmp/backups"),
    ));
    HumanPlate::with_state(perms, Arc::new(SessionBus::new()))
}

#[test]
fn gate_close_by_principle_is_session_scoped() {
    let hp1 = test_human_plate();
    let hp2 = test_human_plate();

    hp1.close_gate(HumanGate::KaiMen);
    assert!(
        !hp1.gate_is_open(HumanGate::KaiMen),
        "KaiMen should be closed on hp1"
    );
    assert!(
        hp2.gate_is_open(HumanGate::KaiMen),
        "KaiMen should be open on hp2 (different session)"
    );
}

#[test]
fn gate_close_then_all_others_stay_open() {
    let hp = test_human_plate();
    hp.close_gate(HumanGate::ShengMen);

    assert!(!hp.gate_is_open(HumanGate::ShengMen));
    assert!(hp.gate_is_open(HumanGate::JingXiangMen));
    assert!(hp.gate_is_open(HumanGate::ShangMen));
    assert!(hp.gate_is_open(HumanGate::DuMen));
    assert!(hp.gate_is_open(HumanGate::XiuMen));
    assert!(hp.gate_is_open(HumanGate::KaiMen));
    assert!(hp.gate_is_open(HumanGate::SiMen));
    assert!(hp.gate_is_open(HumanGate::JingJueMen));
}

#[test]
fn jingjue_sync_with_planning_mode() {
    let hp = test_human_plate();
    assert!(hp.should_escalate_alert(), "JingJueMen should start open");

    hp.sync_jingjue_with_mode(true); // enter planning
    assert!(
        !hp.should_escalate_alert(),
        "JingJueMen should close in planning mode"
    );

    hp.sync_jingjue_with_mode(false); // exit planning
    assert!(
        hp.should_escalate_alert(),
        "JingJueMen should reopen in normal mode"
    );
}

#[test]
fn gate_close_preserves_other_gate_states() {
    let hp = test_human_plate();
    hp.close_gate(HumanGate::KaiMen);
    hp.close_gate(HumanGate::ShengMen);

    assert!(!hp.gate_is_open(HumanGate::KaiMen));
    assert!(!hp.gate_is_open(HumanGate::ShengMen));
    assert!(hp.gate_is_open(HumanGate::ShangMen)); // should be unaffected
    assert!(hp.gate_is_open(HumanGate::XiuMen)); // should be unaffected
}

#[test]
fn all_eight_gates_initially_open() {
    let hp = test_human_plate();
    for gate in &[
        HumanGate::XiuMen,
        HumanGate::ShengMen,
        HumanGate::ShangMen,
        HumanGate::DuMen,
        HumanGate::JingXiangMen,
        HumanGate::SiMen,
        HumanGate::JingJueMen,
        HumanGate::KaiMen,
    ] {
        assert!(hp.gate_is_open(*gate), "Gate {gate:?} should start open");
    }
}

#[test]
fn gate_close_multiple_sessions_independent() {
    let hp_a = test_human_plate();
    let hp_b = test_human_plate();

    hp_a.close_gate(HumanGate::KaiMen);
    hp_b.close_gate(HumanGate::ShengMen);

    assert!(!hp_a.gate_is_open(HumanGate::KaiMen));
    assert!(hp_a.gate_is_open(HumanGate::ShengMen));
    assert!(hp_b.gate_is_open(HumanGate::KaiMen));
    assert!(!hp_b.gate_is_open(HumanGate::ShengMen));
}

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::geju::{ApprovalGate, ExecutionMode, GeJuResult};
use crate::palaces::gen_store::Store;
use crate::vijnana::manas::Manas;
use crate::vijnana::mano::TurnSnapshot;

/// A learned safety constraint derived from accumulated experience.
///
/// System principles are generated when sufficient seed data exists
/// for a particular GeJu combination. They tighten (never loosen)
/// the baseline safety evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPrinciple {
    pub id: String,
    pub session_id: String,
    pub geju_key: String,
    pub constraint: PrincipleConstraint,
    /// 0-1 confidence derived from seed count and error rate
    pub confidence: f32,
    pub source_seed_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PrincipleConstraint {
    AddGuard { gate: String, reason: String },
    EscalateTo { mode: String, reason: String },
    RequireAudit { reason: String },
}

impl SystemPrinciple {
    /// Derive principles from accumulated turn snapshots.
    ///
    /// Groups snapshots by geju_key, and for groups with ≥3 data points,
    /// computes an error rate to decide the appropriate constraint.
    pub fn derive(
        session_id: &str,
        snapshots: &[TurnSnapshot],
        manas: &Manas,
    ) -> Vec<SystemPrinciple> {
        // Group snapshots by geju_key (using geju name as key)
        let mut groups: std::collections::HashMap<String, Vec<&TurnSnapshot>> =
            std::collections::HashMap::new();
        for snap in snapshots {
            groups.entry(snap.geju_name.clone()).or_default().push(snap);
        }

        let mut principles = Vec::new();

        for (geju_key, group) in &groups {
            let n = group.len() as u64;
            if n < 3 {
                continue;
            }

            let error_count = group.iter().filter(|s| s.tool_error.is_some()).count() as f32;
            let error_rate = error_count / n as f32;

            let atma_graha = manas.atma_graha;

            // High error rate + low ātma-grāha → escalate to Sandbox
            if error_rate >= 0.7 && atma_graha < 0.4 {
                principles.push(SystemPrinciple {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.into(),
                    geju_key: geju_key.clone(),
                    constraint: PrincipleConstraint::EscalateTo {
                        mode: "Sandbox".into(),
                        reason: format!(
                            "error_rate={:.2}, atma_graha={:.2} — repeated failures indicate systemic risk",
                            error_rate, atma_graha
                        ),
                    },
                    confidence: (error_rate * 0.7 + (1.0 - atma_graha) * 0.3).min(1.0),
                    source_seed_count: n,
                });
            }
            // Moderate error rate → add Permission guard
            else if error_rate >= 0.4 {
                principles.push(SystemPrinciple {
                    id: Uuid::new_v4().to_string(),
                    session_id: session_id.into(),
                    geju_key: geju_key.clone(),
                    constraint: PrincipleConstraint::AddGuard {
                        gate: "Permission".into(),
                        reason: format!(
                            "error_rate={:.2} — elevated error rate requires additional checks",
                            error_rate
                        ),
                    },
                    confidence: (error_rate * 0.6 + 0.2).min(1.0),
                    source_seed_count: n,
                });
            }

            // Many seeds for same geju → always audit
            if n > 5 {
                // Check if we already have a constraint for this geju_key in this batch
                let existing = principles.iter_mut().find(|p| p.geju_key == *geju_key);
                if let Some(p) = existing {
                    // Upgrade: add RequireAudit on top
                    p.constraint = PrincipleConstraint::EscalateTo {
                        mode: "Sandbox".into(),
                        reason: format!("{} seeds accumulated — systemic pattern detected", n),
                    };
                    p.confidence = (p.confidence + 0.2).min(1.0);
                } else {
                    principles.push(SystemPrinciple {
                        id: Uuid::new_v4().to_string(),
                        session_id: session_id.into(),
                        geju_key: geju_key.clone(),
                        constraint: PrincipleConstraint::RequireAudit {
                            reason: format!(
                                "{} seeds accumulated — pattern requires audit oversight",
                                n
                            ),
                        },
                        confidence: 0.6,
                        source_seed_count: n,
                    });
                }
            }
        }

        principles
    }

    /// Apply this principle to tighten a GeJuResult (Layer 4).
    ///
    /// Only takes effect when the agent's atma_graha is below 0.50
    /// (agent is open to self-correction). Never downgrades safety.
    ///
    /// Returns true if the result was modified.
    pub fn tighten(&self, result: &mut GeJuResult, atma_graha: f32) -> bool {
        if atma_graha >= 0.50 {
            return false;
        }

        let mut modified = false;

        match &self.constraint {
            PrincipleConstraint::EscalateTo { mode, .. } => {
                let target = match mode.as_str() {
                    "Guarded" => ExecutionMode::Guarded,
                    "Sandbox" => ExecutionMode::Sandbox,
                    "Denied" => ExecutionMode::Denied,
                    _ => return false,
                };
                let candidate = GeJuResult {
                    execution_mode: target,
                    ..result.clone()
                };
                // Only escalate — never downgrade
                if candidate.is_stricter_than(result) {
                    result.execution_mode = target;
                    result.layer = 4;
                    modified = true;
                }
            }
            PrincipleConstraint::AddGuard { gate, reason } => {
                let gate = match gate.as_str() {
                    "Permission" => ApprovalGate::Permission(reason.clone()),
                    "UserConfirmation" => ApprovalGate::UserConfirmation(reason.clone()),
                    "SandboxIsolation" => ApprovalGate::SandboxIsolation,
                    "CodeReview" => ApprovalGate::CodeReview,
                    _ => return false,
                };
                // Check dedup
                let already_present = result
                    .approval_chain
                    .iter()
                    .any(|g| std::mem::discriminant(g) == std::mem::discriminant(&gate));
                if !already_present {
                    result.approval_chain.push(gate);
                    result.layer = 4;
                    modified = true;
                }
            }
            PrincipleConstraint::RequireAudit { .. } => {
                if !result.requires_audit {
                    result.requires_audit = true;
                    result.layer = 4;
                    modified = true;
                }
            }
        }

        modified
    }
}

/// Load all principles (agent-wide) and apply them to a GeJuResult.
///
/// With one agent = one database file, all principles in the file apply
/// across all sessions of the same agent. No session_id filter needed.
///
/// Convenience wrapper that queries the store and runs tightening.
pub fn apply_layer4(store: &Arc<Store>, geju_key: &str, result: &mut GeJuResult, atma_graha: f32) {
    // Load all principles (agent-wide — one file = one agent)
    let all_principles = match store.load_active_principles() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Failed to load principles: {e}");
            return;
        }
    };

    let mut tightened = false;
    for p_json in &all_principles {
        let principle: SystemPrinciple = match serde_json::from_str(p_json) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if principle.geju_key == geju_key && principle.tighten(result, atma_graha) {
            tightened = true;
            tracing::info!(
                "Layer 4 tightened: {} → {:?} (principle {})",
                geju_key,
                result.execution_mode,
                principle.id,
            );
        }
    }

    if !tightened {
        tracing::debug!("Layer 4: no matching principles for {geju_key}");
    }
}

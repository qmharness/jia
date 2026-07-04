//! manas — Self-Model / Ego (末那识)

use serde::{Deserialize, Serialize};

use crate::zuowang::trigger::AlayaEntropy;

/// Self-model — the agent's meta-cognitive state.
///
/// `atma_graha` (ātma-grāha, 我执) oscillates between `ATMA_MIN` (open, trusts
/// consolidated memory) and `ATMA_MAX` (grasping at self, distrusts own memory).
///
/// It is now **data-driven**: recalibrated from AlayaEntropy metrics (contradiction,
/// staleness, redundancy) and consolidation success rate. The mechanical decay
/// per turn is still present as a slow drift, but `recalibrate()` overrides it
/// with actual memory-health data.
/// Constants for atma_graha recalibration dynamics.
const ATMA_MIN: f32 = 0.05;
const ATMA_MAX: f32 = 0.80;
const ATMA_INITIAL: f32 = 0.80;
const ENTROPY_FLOOR: f32 = 0.05;
const ENTROPY_COEFFICIENT: f32 = 0.70;
const CONTRADICTION_THRESHOLD: f32 = 0.3;
const CONTRADICTION_PENALTY: f32 = 0.15;
const VOLUME_SEED_THRESHOLD: usize = 10;
const VOLUME_MAX_REDUNDANCY: f32 = 0.4;
const VOLUME_MAX_CONTRADICTION: f32 = 0.1;
const VOLUME_BONUS: f32 = -0.05;
/// Exponential moving average blend ratio for recalibrate (60% new, 40% old).
const EMA_NEW: f32 = 0.6;
const EMA_OLD: f32 = 1.0 - EMA_NEW;
const STABILITY_THRESHOLD: f32 = 0.30;
const STABILITY_EPOCHS_NEEDED: u64 = 3;
/// Per-turn decay rate (halved from 0.002 to account for increased seed
/// creation rate from L1 SignalDetector).
const TURN_DECAY: f32 = 0.001;
const CONSOLIDATION_DROP: f32 = 0.15;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manas {
    pub atma_graha: f32,
    pub total_turns: u64,
    pub consolidation_count: u64,
    pub stable_pattern_count: u64,
    pub last_consolidation_at: i64,
    /// Track successive stable epochs to boost confidence
    stable_epochs: u64,
}

impl Manas {
    pub fn new() -> Self {
        Self {
            atma_graha: ATMA_INITIAL,
            total_turns: 0,
            consolidation_count: 0,
            stable_pattern_count: 0,
            last_consolidation_at: 0,
            stable_epochs: 0,
        }
    }

    /// Called after each turn. Gradual decline as agent gains experience.
    pub fn record_turn(&mut self) {
        self.total_turns += 1;
        self.atma_graha = f32::max(ATMA_MIN, self.atma_graha - TURN_DECAY);
    }

    /// Called after consolidation. Sharp drop as new understanding crystallizes.
    pub fn on_consolidation(&mut self, new_patterns: u64) {
        self.consolidation_count += 1;
        self.stable_pattern_count += new_patterns;
        self.last_consolidation_at = crate::utils::unix_now();
        self.atma_graha = f32::max(ATMA_MIN, self.atma_graha - CONSOLIDATION_DROP);
    }

    /// Recalibrate atma_graha from actual memory metrics (AlayaEntropy).
    ///
    /// Called after consolidation or zuowang. Data-driven override of the
    /// mechanical sawtooth: healthy memory (low entropy) pulls atma_graha down;
    /// contradictory/stale memory pushes it up (agent distrusts memory).
    pub fn recalibrate(&mut self, entropy: &AlayaEntropy, seed_count: usize) {
        // Base: entropy total drives atma_graha. High entropy → distrust memory → high.
        // Low entropy → healthy memory → low (more open to memory guidance).
        let entropy_driven = ENTROPY_FLOOR + entropy.total * ENTROPY_COEFFICIENT;

        // Adjustment: contradiction spikes atma_graha more than staleness
        let contradiction_penalty = if entropy.contradiction > CONTRADICTION_THRESHOLD {
            CONTRADICTION_PENALTY
        } else {
            0.0
        };

        // Healthy signal: lots of seeds, low redundancy, low contradiction → trust memory more
        let volume_bonus = if seed_count > VOLUME_SEED_THRESHOLD
            && entropy.redundancy < VOLUME_MAX_REDUNDANCY
            && entropy.contradiction < VOLUME_MAX_CONTRADICTION
        {
            VOLUME_BONUS
        } else {
            0.0
        };

        let calibrated =
            (entropy_driven + contradiction_penalty + volume_bonus).clamp(ATMA_MIN, ATMA_MAX);

        // Weighted blend: EMA_NEW data-driven, EMA_OLD momentum from existing value
        self.atma_graha = calibrated * EMA_NEW + self.atma_graha * EMA_OLD;

        // Track stability: if atma_graha stays low, increment stable epochs
        if self.atma_graha < STABILITY_THRESHOLD {
            self.stable_epochs += 1;
        } else {
            self.stable_epochs = 0;
        }
    }

    /// Whether the agent is in a stable manas state (trusts its memory).
    pub fn is_stable(&self) -> bool {
        self.stable_epochs >= STABILITY_EPOCHS_NEEDED
    }

    /// How many consecutive epochs atma_graha has been low.
    pub fn stable_epochs(&self) -> u64 {
        self.stable_epochs
    }
}

impl Default for Manas {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_has_high_atma_graha() {
        let sm = Manas::new();
        assert!((sm.atma_graha - 0.80).abs() < 0.001);
        assert_eq!(sm.total_turns, 0);
    }

    #[test]
    fn record_turn_decays_atma_graha() {
        let mut sm = Manas::new();
        for _ in 0..150 {
            sm.record_turn();
        }
        assert!(
            sm.atma_graha < 0.70,
            "atma_graha should decay: {}",
            sm.atma_graha
        );
        assert!(sm.atma_graha >= 0.05, "floor: {}", sm.atma_graha);
    }

    #[test]
    fn recalibrate_low_entropy_lowers_atma_graha() {
        let mut sm = Manas::new();
        let entropy = AlayaEntropy {
            staleness: 0.1,
            contradiction: 0.0,
            redundancy: 0.1,
            access_decay: 0.1,
            total: 0.1,
        };
        sm.recalibrate(&entropy, 20);
        assert!(
            sm.atma_graha < 0.60,
            "should drop on healthy memory: {}",
            sm.atma_graha
        );
    }

    #[test]
    fn recalibrate_high_contradiction_raises_atma_graha() {
        let mut sm = Manas::new();
        sm.atma_graha = 0.30; // Start low
        let entropy = AlayaEntropy {
            staleness: 0.5,
            contradiction: 0.8,
            redundancy: 0.6,
            access_decay: 0.5,
            total: 0.6,
        };
        sm.recalibrate(&entropy, 5);
        assert!(
            sm.atma_graha > 0.30,
            "should rise on bad memory: {}",
            sm.atma_graha
        );
    }

    #[test]
    fn stable_epochs_tracking() {
        let mut sm = Manas::new();
        assert!(!sm.is_stable());

        let entropy = AlayaEntropy {
            staleness: 0.05,
            contradiction: 0.0,
            redundancy: 0.05,
            access_decay: 0.05,
            total: 0.05,
        };
        for _ in 0..5 {
            sm.recalibrate(&entropy, 20);
        }
        assert!(
            sm.is_stable(),
            "should be stable after 5 healthy epochs: atma_graha={}",
            sm.atma_graha
        );
    }

    #[test]
    fn on_consolidation_sharp_drop() {
        let mut sm = Manas::new();
        assert!((sm.atma_graha - 0.80).abs() < 0.001);
        sm.on_consolidation(3);
        assert!(
            (sm.atma_graha - 0.65).abs() < 0.001,
            "on_consolidation should subtract 0.15, got {}",
            sm.atma_graha
        );
        assert_eq!(sm.consolidation_count, 1);
        assert_eq!(sm.stable_pattern_count, 3);
        assert!(sm.last_consolidation_at > 0);
    }

    #[test]
    fn on_consolidation_never_below_floor() {
        let mut sm = Manas::new();
        // 10 consolidations: 0.80 - 10*0.15 = -0.70 → floor at 0.05
        for _ in 0..10 {
            sm.on_consolidation(1);
        }
        assert!(
            (sm.atma_graha - 0.05).abs() < 0.001,
            "atma_graha should floor at 0.05, got {}",
            sm.atma_graha
        );
    }

    #[test]
    fn full_lifecycle_from_high_to_stable() {
        let mut sm = Manas::new();
        // Phase 1: Start high
        assert!((sm.atma_graha - 0.80).abs() < 0.001);
        assert!(!sm.is_stable());

        // Phase 2: Healthy memory data pulls atma_graha down
        let healthy = AlayaEntropy {
            staleness: 0.1,
            contradiction: 0.0,
            redundancy: 0.1,
            access_decay: 0.1,
            total: 0.1,
        };
        // Need ~8 iterations due to 60/40 momentum blend from 0.80
        for _ in 0..10 {
            sm.recalibrate(&healthy, 20);
        }
        assert!(
            sm.atma_graha < 0.30,
            "should converge low, got {}",
            sm.atma_graha
        );
        assert!(sm.is_stable(), "should be stable after healthy epochs");

        // Phase 3: Contradiction spike disrupts stability
        let bad = AlayaEntropy {
            staleness: 0.4,
            contradiction: 0.5,
            redundancy: 0.5,
            access_decay: 0.4,
            total: 0.45,
        };
        sm.recalibrate(&bad, 10);
        assert!(
            sm.atma_graha > 0.30,
            "contradiction should raise atma_graha, got {}",
            sm.atma_graha
        );
        assert!(!sm.is_stable(), "should lose stability after contradiction");

        // Phase 4: Recovery — healthy entropy pulls it back down
        for _ in 0..10 {
            sm.recalibrate(&healthy, 20);
        }
        assert!(
            sm.atma_graha < 0.30,
            "should recover to low, got {}",
            sm.atma_graha
        );
        assert!(sm.is_stable(), "should regain stability");
    }

    #[test]
    fn record_turn_respects_floor() {
        let mut sm = Manas::new();
        sm.atma_graha = 0.051;
        sm.record_turn(); // 0.051 - 0.002 = 0.049 → floor 0.05
        assert!(
            (sm.atma_graha - 0.05).abs() < 0.001,
            "record_turn must respect floor, got {}",
            sm.atma_graha
        );
    }

    #[test]
    fn recalibrate_volume_bonus_with_many_seeds() {
        let mut sm = Manas::new();
        sm.atma_graha = 0.50;
        // Low redundancy + high seed count → -0.05 bonus
        let entropy = AlayaEntropy {
            staleness: 0.2,
            contradiction: 0.0,
            redundancy: 0.1,
            access_decay: 0.2,
            total: 0.15,
        };
        let before = sm.atma_graha;
        sm.recalibrate(&entropy, 50); // >10 seeds, redundancy < 0.4 → bonus triggers
        // calibrated = (0.05 + 0.15*0.70 - 0.05 + 0.0) = 0.105; clamp(0.05,0.80)=0.105
        // blend = 0.105 * 0.6 + 0.50 * 0.4 = 0.063 + 0.20 = 0.263
        let expected: f32 = 0.105_f32.mul_add(0.6, before * 0.4);
        assert!(
            (sm.atma_graha - expected).abs() < 0.01,
            "expected {expected}, got {}",
            sm.atma_graha
        );
        assert!(
            sm.atma_graha < before,
            "volume bonus should reduce atma_graha"
        );
    }

    /// ── Long-term evolution: 25-turn simulation ──────────────
    ///
    /// Simulates an agent over 25 turns with periodic consolidation,
    /// entropy computation, and manas recalibration. Verifies:
    ///   - ātma-grāha converges downward with healthy memory
    ///   - Stable epochs accumulate when entropy stays low
    ///   - Entropy spikes (contradiction) disrupt stability temporarily
    ///   - System re-converges after disruption
    #[test]
    fn long_term_evolution_25_turns() {
        let mut manas = Manas::new();
        let healthy = AlayaEntropy {
            staleness: 0.1,
            contradiction: 0.0,
            redundancy: 0.1,
            access_decay: 0.1,
            total: 0.1,
        };
        let moderate = AlayaEntropy {
            staleness: 0.3,
            contradiction: 0.1,
            redundancy: 0.3,
            access_decay: 0.2,
            total: 0.25,
        };
        let spike = AlayaEntropy {
            staleness: 0.4,
            contradiction: 0.6,
            redundancy: 0.5,
            access_decay: 0.3,
            total: 0.5,
        };

        // Track trajectory
        let mut trajectory: Vec<f32> = Vec::with_capacity(26);
        trajectory.push(manas.atma_graha);

        // Phase 1: 10 turns of healthy operation, consolidate every 3 turns
        for turn in 0..10 {
            manas.record_turn();
            if turn % 3 == 2 {
                manas.on_consolidation(2);
            }
            manas.recalibrate(&healthy, 15 + turn as usize);
            trajectory.push(manas.atma_graha);
        }

        // After 10 healthy turns, ātma-grāha should have dropped significantly
        assert!(
            manas.atma_graha < 0.40,
            "after 10 healthy turns, atma_graha should be < 0.40, got {:.3}",
            manas.atma_graha
        );
        assert!(manas.is_stable(), "should be stable after 10 healthy turns");

        // Phase 2: 5 turns with moderate entropy (aging seeds, some redundancy)
        for turn in 0..5 {
            manas.record_turn();
            if turn == 2 {
                manas.on_consolidation(1);
            }
            manas.recalibrate(&moderate, 18);
            trajectory.push(manas.atma_graha);
        }

        // Moderate entropy may slightly raise ātma-grāha but not destroy stability
        // With momentum blend, moderate entropy shouldn't push it too high
        assert!(
            manas.atma_graha < 0.50,
            "moderate entropy should not push atma_graha too high, got {:.3}",
            manas.atma_graha
        );

        // Phase 3: 3 turns of contradiction spike (conflicting information detected)
        for _turn in 0..3 {
            manas.record_turn();
            manas.recalibrate(&spike, 12);
            trajectory.push(manas.atma_graha);
        }

        // Contradiction spike should raise ātma-grāha
        // But momentum should prevent it from jumping to full spike level immediately
        let before_spike = trajectory[11];
        let after_spike = manas.atma_graha;
        assert!(
            after_spike > before_spike + 0.03,
            "contradiction should raise atma_graha: {:.3} → {:.3}",
            before_spike,
            after_spike
        );
        assert!(
            !manas.is_stable(),
            "contradiction spike should disrupt stability"
        );

        // Phase 4: 7 turns of recovery with healthy entropy
        for turn in 0..7 {
            manas.record_turn();
            if turn == 3 {
                manas.on_consolidation(2);
            }
            manas.recalibrate(&healthy, 20);
            trajectory.push(manas.atma_graha);
        }

        // Should recover to stable state
        assert!(
            manas.atma_graha < 0.35,
            "should recover after disruption, got {:.3}",
            manas.atma_graha
        );
        assert!(
            manas.is_stable(),
            "should regain stability after recovery, stable_epochs={}",
            manas.stable_epochs()
        );

        // Convergence: final value should be lower than mid-point
        let mid = trajectory[trajectory.len() / 2];
        let final_ = *trajectory.last().unwrap();
        assert!(
            final_ < mid + 0.1,
            "atma_graha should converge downward: mid={:.3} final={:.3}",
            mid,
            final_
        );

        // Trajectory should show overall downward trend
        let first_quarter_avg: f32 = trajectory[..6].iter().sum::<f32>() / 6.0;
        let last_quarter_avg: f32 = trajectory[trajectory.len() - 6..].iter().sum::<f32>() / 6.0;
        assert!(
            last_quarter_avg < first_quarter_avg + 0.05,
            "long-term trend should be downward: {:.3} → {:.3}",
            first_quarter_avg,
            last_quarter_avg
        );
    }

    /// ── Oscillation stability ────────────────────────────────
    ///
    /// Alternating healthy/contradictory entropy should NOT cause wild
    /// oscillations. The 60/40 momentum blend acts as a low-pass filter.
    #[test]
    fn oscillation_stability_alternating_entropy() {
        let mut manas = Manas::new();
        let healthy = AlayaEntropy {
            staleness: 0.05,
            contradiction: 0.0,
            redundancy: 0.05,
            access_decay: 0.1,
            total: 0.06,
        };
        let bad = AlayaEntropy {
            staleness: 0.4,
            contradiction: 0.7,
            redundancy: 0.5,
            access_decay: 0.4,
            total: 0.55,
        };

        // First, converge to a stable low state
        for _ in 0..8 {
            manas.recalibrate(&healthy, 20);
        }
        let converged_low = manas.atma_graha;
        assert!(
            converged_low < 0.30,
            "should converge low, got {:.3}",
            converged_low
        );

        // Now alternate: bad → healthy → bad → healthy → bad → healthy
        let mut peaks: Vec<f32> = Vec::new();
        let mut valleys: Vec<f32> = Vec::new();

        for _cycle in 0..3 {
            // Bad epoch
            for _ in 0..3 {
                manas.recalibrate(&bad, 10);
            }
            peaks.push(manas.atma_graha);

            // Healthy epoch
            for _ in 0..3 {
                manas.recalibrate(&healthy, 20);
            }
            valleys.push(manas.atma_graha);
        }

        // Each successive peak should be lower due to momentum
        assert!(
            peaks[1] <= peaks[0] + 0.05,
            "second peak should not exceed first: {:.3} vs {:.3}",
            peaks[1],
            peaks[0]
        );
        assert!(
            peaks[2] <= peaks[1] + 0.05,
            "third peak should not exceed second: {:.3} vs {:.3}",
            peaks[2],
            peaks[1]
        );

        // Each successive valley should be lower
        assert!(
            valleys[1] <= valleys[0] + 0.05,
            "second valley should not exceed first: {:.3} vs {:.3}",
            valleys[1],
            valleys[0]
        );
        assert!(
            valleys[2] <= valleys[1] + 0.05,
            "third valley should not exceed second: {:.3} vs {:.3}",
            valleys[2],
            valleys[1]
        );

        // Peak-to-valley swing should be bounded (not wild oscillation).
        // With 3 bad steps per cycle, the 60/40 momentum blend allows ~0.45 swing.
        let max_swing: f32 = peaks
            .iter()
            .zip(valleys.iter())
            .map(|(p, v)| p - v)
            .fold(0.0, f32::max);
        assert!(
            max_swing < 0.50,
            "peak-to-valley swing should be bounded (< 0.50), got {:.3}",
            max_swing
        );

        // After alternating stops and healthy continues, should re-converge
        for _ in 0..8 {
            manas.recalibrate(&healthy, 20);
        }
        assert!(
            manas.atma_graha < 0.30,
            "should re-converge low after alternation stops, got {:.3}",
            manas.atma_graha
        );
        assert!(manas.is_stable(), "should be stable after re-convergence");
    }

    /// Oscillation with rapid alternation (1 step each) should still be damped.
    #[test]
    fn oscillation_stability_rapid_alternation() {
        let mut manas = Manas::new();
        // Start from a moderate point
        manas.atma_graha = 0.40;

        let healthy = AlayaEntropy {
            staleness: 0.05,
            contradiction: 0.0,
            redundancy: 0.05,
            access_decay: 0.1,
            total: 0.06,
        };
        let bad = AlayaEntropy {
            staleness: 0.4,
            contradiction: 0.7,
            redundancy: 0.5,
            access_decay: 0.4,
            total: 0.55,
        };

        let mut max_observed = manas.atma_graha;
        let mut min_observed = manas.atma_graha;

        // Rapid alternation: 1 bad → 1 healthy → 1 bad → ...
        for _ in 0..10 {
            manas.recalibrate(&bad, 10);
            max_observed = max_observed.max(manas.atma_graha);
            manas.recalibrate(&healthy, 20);
            min_observed = min_observed.min(manas.atma_graha);
        }

        let swing = max_observed - min_observed;
        // With 1-step alternation, momentum limits the swing to ~0.35
        assert!(
            swing < 0.40,
            "rapid alternation swing should be bounded (< 0.40), got {:.3} (max={:.3}, min={:.3})",
            swing,
            max_observed,
            min_observed
        );

        // After alternation ends, should stabilize quickly
        for _ in 0..6 {
            manas.recalibrate(&healthy, 20);
        }
        assert!(
            manas.is_stable(),
            "should stabilize after rapid alternation stops, stable_epochs={}",
            manas.stable_epochs()
        );
    }
}

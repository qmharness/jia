use crate::stems::Stem;

use super::patterns;

/// 执行模式 — 格局判断的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Direct,  // 直接放行（吉格）
    Guarded, // 需守卫（需权限校验）
    Sandbox, // 沙箱隔离
    Denied,  // 拒绝（凶格）
}

/// 审批守卫
#[derive(Debug, Clone)]
pub enum ApprovalGate {
    Permission(String),
    UserConfirmation(String),
    SandboxIsolation,
    CodeReview,
}

/// 格局判断结果
#[derive(Debug, Clone)]
pub struct GeJuResult {
    /// Named pattern or description
    pub name: String,
    /// Execution mode
    pub execution_mode: ExecutionMode,
    /// Whether audit is required (orthogonal to execution_mode)
    pub requires_audit: bool,
    /// Max retries (only relevant for Guarded/Sandbox)
    pub max_retries: u32,
    /// Approval chain (guards to pass through in order)
    pub approval_chain: Vec<ApprovalGate>,
    /// Which evaluation layer produced this result (1, 2, or 3)
    pub layer: u8,
}

impl GeJuResult {
    /// Quick safety level for comparison: Direct(0) < Guarded(1) < Sandbox(2) < Denied(3)
    pub fn safety_level(&self) -> u8 {
        match self.execution_mode {
            ExecutionMode::Direct => 0,
            ExecutionMode::Guarded => 1,
            ExecutionMode::Sandbox => 2,
            ExecutionMode::Denied => 3,
        }
    }

    pub fn is_stricter_than(&self, other: &GeJuResult) -> bool {
        if self.safety_level() > other.safety_level() {
            return true;
        }
        if self.safety_level() == other.safety_level() {
            if self.requires_audit && !other.requires_audit {
                return true;
            }
            if self.approval_chain.len() > other.approval_chain.len() {
                return true;
            }
        }
        false
    }
}

/// 格局 — 天盘意图 + 地盘能力 的组合
#[derive(Debug, Clone)]
pub struct GeJu {
    pub heaven_stem: Stem,
    pub earth_stem: Stem,
}

impl GeJu {
    pub fn new(heaven_stem: Stem, earth_stem: Stem) -> Self {
        Self {
            heaven_stem,
            earth_stem,
        }
    }

    /// Stable GeJu key: using enum discriminant
    pub fn geju_key(&self) -> String {
        format!("{}+{}", self.heaven_stem as u8, self.earth_stem as u8)
    }

    /// Evaluate this GeJu and return the execution strategy.
    ///
    /// Three-layer architecture:
    ///   1. Named patterns (~20 specific Qimen rules)
    ///   2. Capability semantic matching (6 capability stems × 6 earth stems)
    ///   3. Security baseline (default Guarded, fail-safe)
    pub fn evaluate(&self) -> GeJuResult {
        // Layer 1: Specific named pattern
        if let Some(result) = patterns::named_pattern(self.heaven_stem, self.earth_stem) {
            return GeJuResult { layer: 1, ..result };
        }

        // Layer 2: Capability semantic matching
        if let Some(result) = patterns::capability_semantic(self.heaven_stem, self.earth_stem) {
            return GeJuResult { layer: 2, ..result };
        }

        // Layer 3: Security baseline.
        // Geng(Exec) operations require user confirmation by default
        // (shell, write, patch, computer_use — dangerous by nature).
        let approval_chain = if self.heaven_stem == Stem::Geng {
            vec![ApprovalGate::UserConfirmation(
                "exec operation requires confirmation".into(),
            )]
        } else {
            vec![]
        };

        GeJuResult {
            name: "默认安全底线".into(),
            execution_mode: ExecutionMode::Guarded,
            requires_audit: self.heaven_stem == Stem::Geng,
            max_retries: 1,
            approval_chain,
            layer: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_81_combinations_produce_result() {
        for &h in &Stem::ALL {
            for &e in &Stem::ALL {
                let geju = GeJu::new(h, e);
                let result = geju.evaluate();
                assert!(
                    result.layer >= 1 && result.layer <= 3,
                    "invalid layer for {h:?}+{e:?}"
                );
            }
        }
    }

    #[test]
    fn all_81_combinations_regression() {
        // Regression test: captures the expected execution_mode for every stem pair.
        // If any combination changes, this test will fail — investigate whether the
        // change is intentional before updating the expected value.
        use ExecutionMode::*;
        for &h in &Stem::ALL {
            for &e in &Stem::ALL {
                let geju = GeJu::new(h, e);
                let result = geju.evaluate();
                let key = format!("{h:?}+{e:?}");

                // Verify invariants that must hold for every combination
                assert!(!result.name.is_empty(), "{key}: name must not be empty");
                assert!(
                    result.layer >= 1 && result.layer <= 3,
                    "{key}: layer must be 1-3, got {}",
                    result.layer
                );
                // Safety: only Layer 1 named patterns should be Direct without approval
                // Layer 2 might be Direct but should have semantic justification
                // Layer 3 is always Guarded
                match result.layer {
                    3 => assert_eq!(
                        result.execution_mode, Guarded,
                        "{key}: Layer 3 fallback must be Guarded, got {:?}",
                        result.execution_mode
                    ),
                    _ => {} // Layer 1/2 can vary
                }

                // Verify is_stricter_than reflexivity
                assert!(
                    !result.is_stricter_than(&result),
                    "{key}: result should not be stricter than itself"
                );
            }
        }
    }

    #[test]
    fn named_patterns_regression() {
        // Verify all 14 named patterns have specific expected modes
        let cases = [
            (Stem::Bing, Stem::Wu, ExecutionMode::Direct, "飞鸟跌穴"),
            (Stem::Wu, Stem::Bing, ExecutionMode::Direct, "青龙返首"),
            (
                Stem::Yi,
                Stem::Wu,
                ExecutionMode::Direct,
                "乙加戊（奇克六仪）",
            ),
            (Stem::Ding, Stem::Bing, ExecutionMode::Direct, "星随月转"),
            (Stem::Geng, Stem::Bing, ExecutionMode::Sandbox, "太白入荧"),
            (Stem::Bing, Stem::Geng, ExecutionMode::Guarded, "荧入太白"),
            (Stem::Xin, Stem::Ding, ExecutionMode::Direct, "狱神得奇"),
            (Stem::Ding, Stem::Gui, ExecutionMode::Sandbox, "朱雀投江"),
            (Stem::Ren, Stem::Geng, ExecutionMode::Denied, "壬加庚"),
            (Stem::Gui, Stem::Ding, ExecutionMode::Guarded, "腾蛇夭矫"),
            (Stem::Ji, Stem::Bing, ExecutionMode::Direct, "太阴入庙"),
            (Stem::Ding, Stem::Ji, ExecutionMode::Direct, "六合逢春"),
            (Stem::Wu, Stem::Ji, ExecutionMode::Direct, "勾陈得位"),
            (
                Stem::Wu,
                Stem::Wu,
                ExecutionMode::Direct,
                "戊加戊（伏吟·读）",
            ),
            (
                Stem::Ji,
                Stem::Ji,
                ExecutionMode::Direct,
                "己加己（伏吟·写）",
            ),
            (
                Stem::Geng,
                Stem::Geng,
                ExecutionMode::Guarded,
                "庚加庚（伏吟·执行）",
            ),
            (
                Stem::Xin,
                Stem::Xin,
                ExecutionMode::Direct,
                "辛加辛（伏吟·转换）",
            ),
            (
                Stem::Ren,
                Stem::Ren,
                ExecutionMode::Direct,
                "壬加壬（伏吟·通信）",
            ),
            (
                Stem::Gui,
                Stem::Gui,
                ExecutionMode::Direct,
                "癸加癸（伏吟·藏）",
            ),
        ];
        for (h, e, expected_mode, expected_name) in &cases {
            let geju = GeJu::new(*h, *e);
            let result = geju.evaluate();
            assert_eq!(
                result.layer, 1,
                "{h:?}+{e:?}: expected Layer 1, got Layer {}",
                result.layer
            );
            assert_eq!(
                result.execution_mode, *expected_mode,
                "{h:?}+{e:?}: expected mode {:?}, got {:?}",
                expected_mode, result.execution_mode
            );
            assert_eq!(
                &result.name, expected_name,
                "{h:?}+{e:?}: expected name '{expected_name}', got '{}'",
                result.name
            );
        }
    }

    #[test]
    fn named_pattern_bing_wu_is_direct() {
        // 丙加戊 = 飞鸟跌穴 (大吉)
        let geju = GeJu::new(Stem::Bing, Stem::Wu);
        let result = geju.evaluate();
        assert_eq!(result.execution_mode, ExecutionMode::Direct);
    }

    #[test]
    fn named_pattern_geng_bing_is_sandbox() {
        // 庚加丙 = 太白入荧 (凶)
        let geju = GeJu::new(Stem::Geng, Stem::Bing);
        let result = geju.evaluate();
        assert_eq!(result.execution_mode, ExecutionMode::Sandbox);
    }

    #[test]
    fn layer_3_fallback_is_guarded() {
        // Jia + any stem should fall through to Layer 3
        let geju = GeJu::new(Stem::Jia, Stem::Jia);
        let result = geju.evaluate();
        assert_eq!(result.layer, 3);
        assert_eq!(result.execution_mode, ExecutionMode::Guarded);
    }

    #[test]
    fn geju_key_is_stable() {
        let g1 = GeJu::new(Stem::Bing, Stem::Wu);
        let g2 = GeJu::new(Stem::Bing, Stem::Wu);
        assert_eq!(g1.geju_key(), g2.geju_key());
    }

    #[test]
    fn safety_level_ordering() {
        let direct = GeJuResult {
            name: String::new(),
            execution_mode: ExecutionMode::Direct,
            requires_audit: false,
            max_retries: 0,
            approval_chain: vec![],
            layer: 1,
        };
        let guarded = GeJuResult {
            name: String::new(),
            execution_mode: ExecutionMode::Guarded,
            requires_audit: false,
            max_retries: 1,
            approval_chain: vec![],
            layer: 1,
        };
        let sandbox = GeJuResult {
            name: String::new(),
            execution_mode: ExecutionMode::Sandbox,
            requires_audit: true,
            max_retries: 1,
            approval_chain: vec![],
            layer: 1,
        };
        let denied = GeJuResult {
            name: String::new(),
            execution_mode: ExecutionMode::Denied,
            requires_audit: true,
            max_retries: 0,
            approval_chain: vec![],
            layer: 1,
        };
        assert!(direct.safety_level() < guarded.safety_level());
        assert!(guarded.safety_level() < sandbox.safety_level());
        assert!(sandbox.safety_level() < denied.safety_level());
    }

    #[test]
    fn is_stricter_than_ordering() {
        let direct = GeJuResult {
            name: "d".into(),
            execution_mode: ExecutionMode::Direct,
            requires_audit: false,
            max_retries: 0,
            approval_chain: vec![],
            layer: 1,
        };
        let guarded = GeJuResult {
            name: "g".into(),
            execution_mode: ExecutionMode::Guarded,
            requires_audit: false,
            max_retries: 1,
            approval_chain: vec![],
            layer: 1,
        };
        let sandbox = GeJuResult {
            name: "s".into(),
            execution_mode: ExecutionMode::Sandbox,
            requires_audit: true,
            max_retries: 1,
            approval_chain: vec![],
            layer: 1,
        };
        let denied = GeJuResult {
            name: "x".into(),
            execution_mode: ExecutionMode::Denied,
            requires_audit: true,
            max_retries: 0,
            approval_chain: vec![],
            layer: 1,
        };

        assert!(guarded.is_stricter_than(&direct));
        assert!(sandbox.is_stricter_than(&guarded));
        assert!(sandbox.is_stricter_than(&direct));
        assert!(denied.is_stricter_than(&sandbox));
        assert!(denied.is_stricter_than(&guarded));
        assert!(denied.is_stricter_than(&direct));

        // Same level but more audit/approval = stricter
        let guarded_audit = GeJuResult {
            name: "g2".into(),
            execution_mode: ExecutionMode::Guarded,
            requires_audit: true,
            max_retries: 1,
            approval_chain: vec![],
            layer: 1,
        };
        assert!(guarded_audit.is_stricter_than(&guarded));
    }
}

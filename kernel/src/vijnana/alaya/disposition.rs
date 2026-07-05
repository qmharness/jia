//! 种子习性 SeedDisposition — 唯识"势力"的工程实现。
//!
//! 每颗种子带一组可变的响应习性，区别于固定的 SeedNature"性"。
//! 两个字段分别对应唯识种子六义中的两项：
//! - consolidation_inertia：性决定 (svabhāva-niyata) — 种子性质决定发展方向
//! - retrieval_threshold：待众缘 (pratyaya-pratibaddha) — 显现所需条件
//!
//! 此处"势力"取其广义——种子被熏习积累后可变的响应倾向——
//! 非其狭义唯识含义（bīja-bala，种子产生现行效果的向外力量）。

use serde::{Deserialize, Serialize};

use super::{SeedNature, SeedSource};

/// 种子可变响应习性。
///
/// 两个字段均对应唯识种子六义：
/// - `consolidation_inertia` → 性决定（svabhāva-niyata，种子不易转向他果）
/// - `retrieval_threshold` → 待众缘（pratyaya-pratibaddha，显现所需条件聚集）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedDisposition {
    /// 抗熏习更新（性决定·修改阻力）。
    ///
    /// 高惯性 = 不易被新经验改变方向。“某种子只生某果，不转他果”。
    pub consolidation_inertia: f32,

    /// 检索激活阈值（待众缘·显现条件）。
    ///
    /// 高阈值 = 需要更高相似度才能从隐位转现行位。
    pub retrieval_threshold: f32,
}

impl Default for SeedDisposition {
    fn default() -> Self {
        Self {
            consolidation_inertia: 0.5,
            retrieval_threshold: 0.5,
        }
    }
}

impl SeedDisposition {
    /// 按 SeedNature 初始化默认值。
    ///
    /// Fact 高惯性低阈值——抗修改、易显现；
    /// Inference 低惯性高阈值——易修改、需高相似度才激活。
    pub fn for_nature(nature: &SeedNature) -> Self {
        match nature {
            SeedNature::Fact => Self {
                consolidation_inertia: 0.70,
                retrieval_threshold: 0.30,
            },
            SeedNature::Inference => Self {
                consolidation_inertia: 0.30,
                retrieval_threshold: 0.60,
            },
            SeedNature::Preference => Self {
                consolidation_inertia: 0.80,
                retrieval_threshold: 0.20,
            },
            SeedNature::Procedure => Self {
                consolidation_inertia: 0.50,
                retrieval_threshold: 0.50,
            },
        }
    }

    /// 按 SeedSource 特化。
    ///
    /// RenSoul 极化保护——最高惯性（0.95）、最低阈值（0.05）。
    /// 注：RenSoul 是 SeedSource 变体，非 SeedNature；此处通过 source 特化
    /// 逻辑绕过 nature 默认赋值。
    pub fn for_source(source: &SeedSource) -> Option<Self> {
        match source {
            SeedSource::RenSoul => Some(Self {
                consolidation_inertia: 0.95,
                retrieval_threshold: 0.05,
            }),
            _ => None,
        }
    }

    /// 组合 nature 和 source 确定 disposition。
    pub fn resolve(nature: &SeedNature, source: &SeedSource) -> Self {
        // source 特化优先
        if let Some(disp) = Self::for_source(source) {
            return disp;
        }
        Self::for_nature(nature)
    }

    /// 由使用反馈自适应：被检索且用上 → 降阈值（更易未来显现）。
    pub fn on_used(&mut self) {
        self.retrieval_threshold = (self.retrieval_threshold - 0.02).max(0.05);
    }

    /// 由使用反馈自适应：长期沉寂 → 升阈值（更难以被激活）。
    pub fn on_idle(&mut self) {
        self.retrieval_threshold = (self.retrieval_threshold + 0.03).min(0.95);
    }

    /// Clamp 到安全范围 [0.05, 0.95]。
    pub fn clamp(&mut self) {
        self.consolidation_inertia = self.consolidation_inertia.clamp(0.05, 0.95);
        self.retrieval_threshold = self.retrieval_threshold.clamp(0.05, 0.95);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_has_high_inertia_low_threshold() {
        let d = SeedDisposition::for_nature(&SeedNature::Fact);
        assert!(
            d.consolidation_inertia > 0.5,
            "Fact should resist modification"
        );
        assert!(
            d.retrieval_threshold < 0.5,
            "Fact should be easy to retrieve"
        );
    }

    #[test]
    fn inference_has_low_inertia_high_threshold() {
        let d = SeedDisposition::for_nature(&SeedNature::Inference);
        assert!(
            d.consolidation_inertia < 0.5,
            "Inference should be easy to modify"
        );
        assert!(
            d.retrieval_threshold > 0.5,
            "Inference should need high similarity"
        );
    }

    #[test]
    fn ren_soul_polarized_protection() {
        let d = SeedDisposition::for_source(&SeedSource::RenSoul).unwrap();
        assert!(
            d.consolidation_inertia > 0.9,
            "RenSoul should have maximal inertia"
        );
        assert!(
            d.retrieval_threshold < 0.1,
            "RenSoul should have minimal threshold"
        );
    }

    #[test]
    fn source_overrides_nature() {
        let d = SeedDisposition::resolve(&SeedNature::Inference, &SeedSource::RenSoul);
        assert!(
            d.consolidation_inertia > 0.9,
            "RenSoul source should override nature"
        );
    }

    #[test]
    fn on_used_lowers_threshold() {
        let mut d = SeedDisposition::default();
        let before = d.retrieval_threshold;
        d.on_used();
        assert!(d.retrieval_threshold < before);
    }

    #[test]
    fn on_idle_raises_threshold() {
        let mut d = SeedDisposition::default();
        let before = d.retrieval_threshold;
        d.on_idle();
        assert!(d.retrieval_threshold > before);
    }
}

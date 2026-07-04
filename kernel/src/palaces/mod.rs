pub mod dui_gateway;
pub mod gen_store;
pub mod kan_io;
pub mod kun_config;
pub mod li_skill;
pub mod qian_permission;
pub mod xun_context;
pub mod zhen_tool;
pub mod zhong_core;

use crate::stems::Stem;

/// 九宫 — Nine Palaces
///
/// 阳遁三局地盘分布。九大功能域，每宫一干，甲隐中五。
/// 干是地势能量（五行气质），域是系统职责，两者正交——格局因错位而生。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Palace {
    Kan,   // 坎一 ☵ 北 — I/O 通道        丙 阳火 · 明
    Kun,   // 坤二 ☷ 西南 — 配置          乙 阴木 · 韧
    Zhen,  // 震三 ☳ 东 — 工具执行        戊 阳土 · 稳
    Xun,   // 巽四 ☴ 东南 — 上下文        己 阴土 · 容
    Zhong, // 中五 ◎ 中 — LLM 核心, 甲隐  庚 阳金 · 断
    Qian,  // 乾六 ☰ 西北 — 权限          辛 阴金 · 炼
    Dui,   // 兑七 ☱ 西 — 网关            壬 阳水 · 通
    Gen,   // 艮八 ☶ 东北 — 持久化        癸 阴水 · 藏
    Li,    // 离九 ☲ 南 — 技能            丁 阴火 · 星
}

impl Palace {
    /// 返回该宫的地盘天干（阳遁三局分布）
    ///
    /// 三奇六仪从震三宫起顺排：戊→己→庚→辛→壬→癸→丁→丙→乙。
    /// 甲遁于六仪，不占宫位。中五寄庚。
    pub fn stem(&self) -> Stem {
        match self {
            Palace::Kan => Stem::Bing,   // 丙 · 明
            Palace::Kun => Stem::Yi,     // 乙 · 韧
            Palace::Zhen => Stem::Wu,    // 戊 · 稳
            Palace::Xun => Stem::Ji,     // 己 · 容
            Palace::Zhong => Stem::Geng, // 庚 · 断
            Palace::Qian => Stem::Xin,   // 辛 · 炼
            Palace::Dui => Stem::Ren,    // 壬 · 通
            Palace::Gen => Stem::Gui,    // 癸 · 藏
            Palace::Li => Stem::Ding,    // 丁 · 星
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palace_stem_mapping() {
        // 阳遁三局：戊己庚辛壬癸丁丙乙，震三顺排
        assert_eq!(Palace::Zhen.stem(), Stem::Wu); // 戊
        assert_eq!(Palace::Xun.stem(), Stem::Ji); // 己
        assert_eq!(Palace::Zhong.stem(), Stem::Geng); // 庚
        assert_eq!(Palace::Qian.stem(), Stem::Xin); // 辛
        assert_eq!(Palace::Dui.stem(), Stem::Ren); // 壬
        assert_eq!(Palace::Gen.stem(), Stem::Gui); // 癸
        assert_eq!(Palace::Li.stem(), Stem::Ding); // 丁
        assert_eq!(Palace::Kan.stem(), Stem::Bing); // 丙
        assert_eq!(Palace::Kun.stem(), Stem::Yi); // 乙
    }
}

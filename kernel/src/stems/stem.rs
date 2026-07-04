/// 十天干 — Ten Heavenly Stems
///
/// 甲隐不显，其余九干分三奇六仪。
/// 天盘时干为意图语义，地盘时干为地势能量（五行气质）。
///
/// 六仪（地盘操作）：戊(稳) 己(容) 庚(断) 辛(炼) 壬(通) 癸(藏)
/// 三奇（超越操作）：乙(韧·Skill) 丙(明·Compact) 丁(星·Hook)
/// 甲：LLM 核心，隐于中五
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Stem {
    Jia,  // 甲 — LLM, hidden
    Yi,   // 乙 — 阴木 · 韧（日奇 / Skill）
    Bing, // 丙 — 阳火 · 明（月奇 / Compaction）
    Ding, // 丁 — 阴火 · 星（星奇 / Hook）
    Wu,   // 戊 — 阳土 · 稳（Read）
    Ji,   // 己 — 阴土 · 容（Write）
    Geng, // 庚 — 阳金 · 断（Exec）
    Xin,  // 辛 — 阴金 · 炼（Transform）
    Ren,  // 壬 — 阳水 · 通（Communicate）
    Gui,  // 癸 — 阴水 · 藏（Store）
}

impl Stem {
    pub const ALL: [Stem; 10] = [
        Stem::Jia,
        Stem::Yi,
        Stem::Bing,
        Stem::Ding,
        Stem::Wu,
        Stem::Ji,
        Stem::Geng,
        Stem::Xin,
        Stem::Ren,
        Stem::Gui,
    ];

    pub const CEREMONY_STEMS: [Stem; 6] = [
        Stem::Wu,
        Stem::Ji,
        Stem::Geng,
        Stem::Xin,
        Stem::Ren,
        Stem::Gui,
    ];

    /// 六仪干返回 Some(自身)，三奇与甲返回 None
    pub fn as_ceremony(&self) -> Option<Stem> {
        match self {
            Stem::Wu | Stem::Ji | Stem::Geng | Stem::Xin | Stem::Ren | Stem::Gui => Some(*self),
            Stem::Jia | Stem::Yi | Stem::Bing | Stem::Ding => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceremony_mapping() {
        assert_eq!(Stem::Wu.as_ceremony(), Some(Stem::Wu));
        assert_eq!(Stem::Ji.as_ceremony(), Some(Stem::Ji));
        assert_eq!(Stem::Geng.as_ceremony(), Some(Stem::Geng));
        assert_eq!(Stem::Xin.as_ceremony(), Some(Stem::Xin));
        assert_eq!(Stem::Ren.as_ceremony(), Some(Stem::Ren));
        assert_eq!(Stem::Gui.as_ceremony(), Some(Stem::Gui));
    }

    #[test]
    fn marvels_and_jia_return_none() {
        assert_eq!(Stem::Jia.as_ceremony(), None);
        assert_eq!(Stem::Yi.as_ceremony(), None);
        assert_eq!(Stem::Bing.as_ceremony(), None);
        assert_eq!(Stem::Ding.as_ceremony(), None);
    }
}

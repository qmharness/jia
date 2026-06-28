/// 操作意图 — Operation Intents
///
/// 三奇（Marvels）和六仪（Ceremonies）是意图的两大类。
/// 甲（LLM）通过六仪间接行动：每个工具调用、每个决策都归属六仪之一。
/// 三奇 — 超越性意图（Marvels: Yi/Bing/Ding）
#[derive(Debug, Clone)]
pub enum MarvelsIntent {
    Yi(SkillInvocation), // 乙奇: 技能注入
    Bing(Compaction),    // 丙奇: 上下文压缩
    Ding(HookTrigger),   // 丁奇: 触发钩子
}

/// 六仪 — 基础操作意图（Ceremonies: Wu..Gui）
#[derive(Debug, Clone)]
pub enum CeremoniesIntent {
    Wu(ReadAction),         // 戊仪: 读取
    Ji(WriteAction),        // 己仪: 写入
    Geng(ExecAction),       // 庚仪: 执行
    Xin(TransformAction),   // 辛仪: 转换
    Ren(CommunicateAction), // 壬仪: 通信
    Gui(StoreAction),       // 癸仪: 存储
}

/// 意图 = 三奇或六仪
#[derive(Debug, Clone)]
pub enum Intent {
    Marvels(MarvelsIntent),
    Ceremonies(CeremoniesIntent),
}

// ── MarvelsIntent variants ─────────────────────────────────

#[derive(Debug, Clone)]
pub struct SkillInvocation {
    pub skill_name: String,
}

#[derive(Debug, Clone)]
pub struct Compaction {
    pub reason: String,
}

/// Reserved: LLM-triggered hook dispatch path.
///
/// The core hook system fires via fire_void_hooks / fire_guarding_hooks
/// in the agent loop, not through this intent type.
///
/// Future: when the LLM can express Ding intent, this struct carries
/// the target hook name and optional filtering criteria.
#[derive(Debug, Clone)]
pub struct HookTrigger {
    pub hook_name: String,
}

// ── CeremoniesIntent variants ──────────────────────────────

#[derive(Debug, Clone)]
pub struct ReadAction {
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct WriteAction {
    pub target: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ExecAction {
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct TransformAction {
    pub input: String,
    pub transform_kind: String,
}

#[derive(Debug, Clone)]
pub struct CommunicateAction {
    pub endpoint: String,
    pub payload: String,
}

#[derive(Debug, Clone)]
pub struct StoreAction {
    pub key: String,
    pub value: String,
}

impl Intent {
    /// 提取意图对应的天干（用于格局判断）
    pub fn to_stem(&self) -> crate::stems::Stem {
        use crate::stems::Stem;
        match self {
            Intent::Marvels(qi) => match qi {
                MarvelsIntent::Yi(_) => Stem::Yi,
                MarvelsIntent::Bing(_) => Stem::Bing,
                MarvelsIntent::Ding(_) => Stem::Ding,
            },
            Intent::Ceremonies(yi) => match yi {
                CeremoniesIntent::Wu(_) => Stem::Wu,
                CeremoniesIntent::Ji(_) => Stem::Ji,
                CeremoniesIntent::Geng(_) => Stem::Geng,
                CeremoniesIntent::Xin(_) => Stem::Xin,
                CeremoniesIntent::Ren(_) => Stem::Ren,
                CeremoniesIntent::Gui(_) => Stem::Gui,
            },
        }
    }
}

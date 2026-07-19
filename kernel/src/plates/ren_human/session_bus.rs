//! session_bus — 人盘会话总线 (Session Bus)
//!
//! 哲学依据:人盘 = 人机交互边界。pending 确认/提问、会话交互模式、
//! 会话锁、子代理会话,皆是"人与机之间进行中的交互状态",当归人盘
//! 而非地盘(地盘 = 一局不变的静态基础设施)。用户已裁决(P2-1)。
//!
//! 方向守护(P2-2 复盘后,如实记录):InteractionMode 已随 P2-2 下沉
//! 天干层(stems),ren→tian 边消解。残余过渡态:zhen_tool::delegate
//! ::SubagentSession(ren→zhen)经 P2-1 复核裁为可接受过渡,不动;
//! ask_user.rs 对本模块 PendingQuestion 的引用为 zhen→ren 边(人盘 =
//! 人机交互边界,工具向边界取待答状态,语义自洽),保留观察。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use super::PendingConfirmation;
use crate::palaces::zhen_tool::builtin::delegate::SubagentSession;
use crate::stems::InteractionMode;

/// A pending question awaiting user answer.
///
/// 原居 zhen_tool::builtin::ask_user;随迁人盘以避免"盘→宫"方向违规
/// (人盘持有它,ask_user 反向引用——该 zhen→ren 边为本重构新增的过渡态,
/// 见模块头方向守护记录)。
pub struct PendingQuestion {
    pub sender: tokio::sync::oneshot::Sender<String>,
    pub token: String,
    pub created_at: i64,
    /// 所属会话 — 断连时按会话清扫(rin 连接结束 → 该连接会话的
    /// pending 条目被 remove,sender drop,orx 醒为 Err)。
    pub session_id: String,
}

/// 会话总线 — 人盘持有的全部可变会话状态。
///
/// 五簇共享表,经 `Arc<SessionBus>` 在地盘装配时构造一次,由
/// EarthPlate / AppState / rin / agent loop 各处克隆共享同一份。
pub struct SessionBus {
    /// 待裁决的用户确认(ask 确认 / 建项确认)。
    pub(crate) pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    /// 待回答的用户提问(ask_user 工具 ↔ REST /answer、rin answer)。
    pub(crate) pending_questions: Arc<Mutex<HashMap<String, PendingQuestion>>>,
    /// P3 · per-session interaction mode (谋划态), set by user slash command
    /// (/plan) and read when the next agent run starts. Kept in sync with the
    /// agent's actual mode via InteractionModeChanged events.
    pub(crate) session_modes: Arc<Mutex<HashMap<String, InteractionMode>>>,
    /// Per-session locks — serializes concurrent messages from the same source
    /// so they don't race on history read/write in post_loop.
    pub(crate) session_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// P8 · persisted sub-agent sessions for continuation via send_message.
    pub(crate) subagent_sessions: Arc<Mutex<HashMap<String, SubagentSession>>>,
}

impl SessionBus {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for SessionBus {
    fn default() -> Self {
        Self {
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            session_modes: Arc::new(Mutex::new(HashMap::new())),
            session_locks: Arc::new(Mutex::new(HashMap::new())),
            subagent_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

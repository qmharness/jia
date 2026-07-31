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
//! 人机交互边界,工具向边界取待答状态,语义自洽),保留观察;#15
//! task.rs 对本模块 SessionBus 的引用为同款 zhen→ren 边(工具向边界
//! 存取会话级验收标准),沿用 ask_user 先例。

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use super::PendingConfirmation;
use crate::palaces::zhen_tool::builtin::delegate::SubagentSession;
use crate::stems::InteractionMode;

/// #9 · steer 优先级 — turn 内用户插话的处置时机(参照 kimi-code
/// 消息队列 now/next/later 语义)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerPriority {
    /// 立即打断:走现有取消路径(与 Esc 相同)。
    Now,
    /// 下一检查点(工具批屏障、下一次 LLM 调用前)必折入 history。
    Next,
    /// turn 自然结束前折入;若 turn 即将结束,作为下一条用户输入处理。
    Later,
}

/// #9 · 一条 steer 插话 — 真实用户消息(非 ephemeral 通知):折入时写入
/// history、可被熏习,附 `[steer]` 轻量标记与普通人话区分。
#[derive(Debug, Clone)]
pub struct SteerMessage {
    pub content: String,
    pub priority: SteerPriority,
}

/// #15 · Goal 式验收标准(会话级,内存态):模型宣布完成(无工具调用
/// 收尾)前须逐条对照。生产侧:task 工具 set_criteria / check_criterion;
/// 消费侧:天盘 loop 收尾门禁(unchecked_criteria 非空则注入提醒续跑)。
#[derive(Debug, Clone)]
pub struct Criterion {
    pub text: String,
    pub checked: bool,
}

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
    /// N1 · 会话级批准记忆:session_id → 已获用户批准的"工具+入参"键集。
    /// 仅记忆用户的主动批准(首次仍须询问,绝不自动放行);内存态,不持久化。
    pub(crate) session_approvals: Arc<Mutex<HashMap<String, std::collections::HashSet<String>>>>,
    /// #9 · per-session steer 插话队列(FIFO)。生产侧:rin/gateway/TUI 在
    /// agent busy 时推入;消费侧:天盘 loop 在批屏障检查点 drain 折入。
    /// 与 ChannelManager 的 ChannelInput 管道(新 run 的输入)不同——steer
    /// 队列挂会话,读者是【进行中】的 run。
    pub(crate) steer_queues: Arc<Mutex<HashMap<String, VecDeque<SteerMessage>>>>,
    /// #15 · per-session 验收标准(内存即可,挂法同 steer_queues)。
    pub(crate) completion_criteria: Arc<Mutex<HashMap<String, Vec<Criterion>>>>,
}

impl SessionBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 清扫某会话的批准记忆(会话结束/断连清扫时调用)。
    pub fn clear_session_approvals(&self, session_id: &str) {
        self.session_approvals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
    }

    /// #9 · 推入一条 steer 插话(FIFO)。会话无活跃 run 时不报错——
    /// 消息留在队列中,下一次 run 的首个检查点会折入(类"type-ahead")。
    pub fn push_steer(&self, session_id: &str, msg: SteerMessage) {
        self.steer_queues
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(session_id.to_string())
            .or_default()
            .push_back(msg);
    }

    /// #9 · 取走该会话全部待折入 steer(保持 FIFO 顺序)。空队列返回空 Vec。
    pub fn drain_steer(&self, session_id: &str) -> Vec<SteerMessage> {
        let mut queues = self.steer_queues.lock().unwrap_or_else(|e| e.into_inner());
        let Some(queue) = queues.get_mut(session_id) else {
            return Vec::new();
        };
        let drained: Vec<SteerMessage> = queue.drain(..).collect();
        if queue.is_empty() {
            queues.remove(session_id);
        }
        drained
    }

    /// #9 · 回灌未消费的 steer(如 Later 在中途检查点不折入、留待 turn
    /// 末)。调用方保证 `msgs` 取自同一次 drain,顺序不变;回灌到队首,
    /// 与回灌后新到的消息保持到达序。
    pub fn requeue_steer(&self, session_id: &str, msgs: Vec<SteerMessage>) {
        if msgs.is_empty() {
            return;
        }
        let mut queues = self.steer_queues.lock().unwrap_or_else(|e| e.into_inner());
        let queue = queues.entry(session_id.to_string()).or_default();
        for (i, msg) in msgs.into_iter().enumerate() {
            queue.insert(i, msg);
        }
    }

    /// #15 · 设置会话验收标准(整体替换,全部置为未对照)。
    pub fn set_criteria(&self, session_id: &str, criteria: Vec<String>) {
        let list: Vec<Criterion> = criteria
            .into_iter()
            .map(|text| Criterion {
                text,
                checked: false,
            })
            .collect();
        self.completion_criteria
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session_id.to_string(), list);
    }

    /// #15 · 对照一条验收标准:先精确匹配,退化为子串匹配(大小写不敏感),
    /// 命中第一个未对照项并勾选。返回剩余未对照数;未命中为 Err。
    pub fn check_criterion(&self, session_id: &str, text: &str) -> Result<usize, String> {
        let mut map = self
            .completion_criteria
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let list = map
            .get_mut(session_id)
            .ok_or_else(|| "No completion criteria set for this session".to_string())?;
        // 先精确匹配,退化为子串匹配(大小写不敏感),命中第一个未对照项。
        let needle = text.to_lowercase();
        let idx = list
            .iter()
            .position(|c| !c.checked && c.text == text)
            .or_else(|| {
                list.iter()
                    .position(|c| !c.checked && c.text.to_lowercase().contains(&needle))
            });
        match idx {
            Some(i) => {
                list[i].checked = true;
                Ok(list.iter().filter(|c| !c.checked).count())
            }
            None => Err(format!("No unchecked criterion matching '{text}'")),
        }
    }

    /// #15 · 未对照的验收标准清单(收尾门禁用;空 = 可收尾)。
    pub fn unchecked_criteria(&self, session_id: &str) -> Vec<String> {
        self.completion_criteria
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session_id)
            .map(|list| {
                list.iter()
                    .filter(|c| !c.checked)
                    .map(|c| c.text.clone())
                    .collect()
            })
            .unwrap_or_default()
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
            session_approvals: Arc::new(Mutex::new(HashMap::new())),
            steer_queues: Arc::new(Mutex::new(HashMap::new())),
            completion_criteria: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steer(content: &str, priority: SteerPriority) -> SteerMessage {
        SteerMessage {
            content: content.to_string(),
            priority,
        }
    }

    #[test]
    fn steer_queue_fifo_push_and_drain() {
        let bus = SessionBus::new();
        assert!(bus.drain_steer("s1").is_empty());

        bus.push_steer("s1", steer("a", SteerPriority::Next));
        bus.push_steer("s1", steer("b", SteerPriority::Later));
        bus.push_steer("s2", steer("other", SteerPriority::Now));

        let drained = bus.drain_steer("s1");
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].content, "a");
        assert_eq!(drained[1].content, "b");
        // drain 后队列清空;其他会话不受影响。
        assert!(bus.drain_steer("s1").is_empty());
        assert_eq!(bus.drain_steer("s2").len(), 1);
    }

    #[test]
    fn steer_requeue_restores_front_order() {
        let bus = SessionBus::new();
        bus.push_steer("s1", steer("later-1", SteerPriority::Later));
        let drained = bus.drain_steer("s1");
        // 中途检查点不折入 Later → 回灌;期间新到的消息排在回灌之后。
        bus.push_steer("s1", steer("new-arrival", SteerPriority::Next));
        bus.requeue_steer("s1", drained);

        let drained = bus.drain_steer("s1");
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].content, "later-1");
        assert_eq!(drained[1].content, "new-arrival");
    }

    // ── #15 · completion criterion ────────────────────────────

    #[test]
    fn criteria_set_check_and_uncheck() {
        let bus = SessionBus::new();
        assert!(bus.unchecked_criteria("s1").is_empty(), "no criteria → clear");

        bus.set_criteria("s1", vec!["tests pass".into(), "docs updated".into()]);
        assert_eq!(bus.unchecked_criteria("s1").len(), 2);

        // 精确匹配勾选。
        assert_eq!(bus.check_criterion("s1", "tests pass").unwrap(), 1);
        assert_eq!(bus.unchecked_criteria("s1"), ["docs updated"]);

        // 子串匹配(大小写不敏感)勾选。
        assert_eq!(bus.check_criterion("s1", "DOCS").unwrap(), 0);
        assert!(bus.unchecked_criteria("s1").is_empty());

        // 未命中与未设置均为 Err。
        assert!(bus.check_criterion("s1", "nothing left").is_err());
        assert!(bus.check_criterion("other", "tests pass").is_err());

        // 重新设置整体替换并复位勾选态。
        bus.set_criteria("s1", vec!["new bar".into()]);
        assert_eq!(bus.unchecked_criteria("s1"), ["new bar"]);
    }
}

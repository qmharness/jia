# Skill Evolution Pipeline

自动从执行反馈中改进 SKILL.md 的五阶段离线管线。在每次 Agent 会话结束时 (`post_loop`) 触发。

## 架构

```
SESSION END (post_loop)
│
├─→ L2 Consolidation
├─→ Thought Distillation
├─→ Zuowang Dissolution
├─→ Tier Budget Enforcement
│
└─→ SKILL EVOLUTION PIPELINE
    │
    ┌──────────────────────────────────────────────────────┐
    │  for each tool-only skill with auto_evolve: true:    │
    │                                                      │
    │  STAGE 1 ── ELIGIBILITY GATE        (无 LLM)        │
    │  STAGE 2 ── TRAJECTORY COMPILATION  (无 LLM)        │
    │  STAGE 3 ── REFLECTION              (haiku)         │
    │  STAGE 4 ── ACCUMULATION            (无 LLM)        │
    │  STAGE 5 ── REVISION               (sonnet)         │
    └──────────────────────────────────────────────────────┘
```

## 各阶段详解

### Stage 1 — Eligibility Gate

零 LLM 调用。决定一个 skill 是否进入演化管线。

| 检查项 | 条件 | 来源 |
|--------|------|------|
| 显式 opt-in | `auto_evolve: true` | SKILL.md frontmatter |
| v1: tool-only | `always: false` 且 `paths` 为空 | SkillRegistry |
| 速率限制 | 本 session revision 数 `< evolve_max_revisions_per_session` | skill_revisions 表 |
| 冷却期 | 距上次 revision > 1 小时 | skill_revisions 表 |
| 信号充足 | 本 session `skill()` 调用 ≥ 2 次 | Agent.skill_tool_calls |

### Stage 2 — Trajectory Compilation

零 LLM 调用。从 `WorkingMemory` 的快照环缓冲区（容量 20）中提取该 skill 相关的信号：

- **errors[]**: `tool_error != None` 的快照，包含 turn 号、工具名、错误内容、GeJu 格局、执行模式
- **geju_events[]**: `execution_mode ∈ {Guarded, Sandbox, Denied}` 的快照

仅收集 skill 首次调用之后的快照——skill 激活前的错误不归因于它。

### Stage 3 — Reflection

**模型：haiku（light_core），60s 超时。**

将 skill 当前 prompt + emphasis + 执行轨迹发给 LLM，要求输出 JSON：

```json
{"type":"...", "summary":"...", "detail":"...", "confidence":0.0}
```

四种反思类型（来自 EmbodiSkill 论文）：

| 类型 | 含义 | 示例 |
|------|------|------|
| `Discovery` | 新场景/新错误模式，skill 未覆盖 | "skill 没有说明如何处理二进制文件" |
| `Optimization` | 现有规则表述可更精确/简洁 | "工具参数说明可以更具体" |
| `SkillDefect` | 技能规则有误，导致执行错误 | "步骤 3 中的命令参数顺序不对" |
| `ExecutionLapse` | 规则正确但 LLM 未遵守，需 emphasis 加强 | "LLM 忽略了检查返回码的要求" |

### Stage 4 — Accumulation

零 LLM 调用。Reflection 持久化到 `skill_reflections` 表。检查是否触发 revision：

| 触发条件 | 阈值 |
|----------|------|
| 同类型累积 | ≥ `evolve_reflection_threshold`（默认 3） |
| 跨类型总量 | ≥ `max(evolve_reflection_threshold * 2, 4)` |
| 高置信单条 | confidence ≥ 0.85 |

仅统计当前 session 的 reflections。

### Stage 5 — Revision

**模型：sonnet（core），60s 超时。**

1. 读取磁盘上的原始 `SKILL.md`
2. 构建 prompt：当前文件 + 所有 reflections + 6 条修订规则
3. LLM 输出完整修订版 SKILL.md
4. **Frontmatter 保护**：强制 `auto_evolve`、`evolve_min_confidence`、`evolve_max_revisions_per_session`、`evolve_reflection_threshold` 恢复为旧值，防止 LLM 篡改
5. **YAML 校验**：`serde_yaml::from_str::<SkillFrontmatter>()`，失败则仅记录不应用
6. **置信度检查**：`avg_confidence >= evolve_min_confidence`，否则仅记录
7. **独立 Auditor**（`evolve_min_confidence >= 0.85` 时启用，haiku，30s 超时）：
   - 检查无矛盾规则引入
   - 检查无重要规则删除
   - 检查 frontmatter 完整有效
8. **Diff 检查**：新旧一致则跳过
9. **冷却期 double-check**：缩小并发竞争窗口
10. **原子写入**：`SKILL.md.tmp.{uuid}` → `rename()` → file watcher 热加载
11. 记录到 `skill_revisions` 表

## SKILL.md Frontmatter 字段

```yaml
---
name: code-review
description: Review code before committing
auto_evolve: true                  # 必须显式设为 true
evolve_min_confidence: 0.7         # revision 自动应用的最低置信度（1.0 = 仅记录）
evolve_max_revisions_per_session: 3  # 每 session 最多 revision 次数
evolve_reflection_threshold: 3      # 触发 revision 的同类型 reflection 数
---
```

## 数据表

### skill_reflections

```sql
CREATE TABLE skill_reflections (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    session_id TEXT NOT NULL,
    reflection_type TEXT NOT NULL,   -- Discovery|Optimization|SkillDefect|ExecutionLapse
    content_json TEXT NOT NULL,      -- LLM 原始 JSON 响应
    confidence REAL NOT NULL,
    turn_numbers TEXT NOT NULL,      -- JSON array
    created_at INTEGER NOT NULL
);
```

### skill_revisions

```sql
CREATE TABLE skill_revisions (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    session_id TEXT NOT NULL,
    old_content TEXT NOT NULL,
    new_content TEXT NOT NULL,
    diff_text TEXT NOT NULL,
    avg_confidence REAL NOT NULL,
    reflection_ids TEXT NOT NULL,    -- JSON array
    pre_revision_error_rate REAL,
    post_revision_error_rate REAL,   -- 由下一 session backfill
    applied INTEGER NOT NULL,        -- 0=仅记录, 1=已应用
    created_at INTEGER NOT NULL
);
```

## 安全闸门

| 层级 | 闸门 | 位置 |
|------|------|------|
| HARD | 必须 opt-in (`auto_evolve: true`) | Stage 1 |
| HARD | v1 仅 tool-only skill | Stage 1 |
| HARD | 速率限制 + 冷却期 | Stage 1 |
| HARD | Frontmatter 字段强制覆盖 | Stage 5 |
| HARD | YAML 解析失败 → 仅记录 | Stage 5 |
| HARD | 原子写入（tmp + rename） | Stage 5 |
| SOFT | 置信度门槛 | Stage 5 |
| SOFT | 独立 Auditor（可选） | Stage 5 |
| AUDIT | revision 完整历史 + diff | skill_revisions 表 |
| AUDIT | error_rate 回归追踪（pre/post） | skill_revisions 表 |

## 模型选择

| LLM 调用 | 模型 | 超时 |
|----------|------|------|
| Stage 3 — Reflection | haiku (`light_core`) | 60s |
| Stage 5 — Revision | sonnet (`core`) | 60s |
| Stage 5 — Auditor | haiku (`light_core`) | 30s |

配置方式：

```toml
[providers.anthropic]
kind = "anthropic"
models = ["claude-sonnet-4-6", "claude-haiku-4-5"]
default_model = "claude-sonnet-4-6"
light_model = "claude-haiku-4-5"
```

`light_model` 未配置时全部回退到 `default_model`。

## 调用时机

`EvolutionEngine::run()` 在 `Agent::post_loop()` 中调用，即每次 Agent 会话结束后。属于离线处理，不影响对话延迟。

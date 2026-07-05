# 甲（Jia）开发路线图

## Phase 0: Web AI 对话 ✅

已完成。可编译的最小骨架，浏览器内流式 AI 对话。

- [x] Cargo 项目初始化 + 依赖配置
- [x] `src/types.rs` — Message, ChatRequest, StreamEvent
- [x] `src/config.rs` — ConfigLoader（环境变量 > .env > CLI > 默认）
- [x] `src/provider.rs` — LlmProvider trait + Anthropic/OpenAI 实现
- [x] `src/gateway.rs` — 离九 APIGateway（axum HTTP + SSE streaming + 静态文件）
- [x] `src/main.rs` — CLI 入口（`gateway start` / `dashboard` / `tui`）
- [x] `web/index.html` — 单页聊天 UI（EventSource 流式消费 + 工具卡片 + 确认弹窗）

---

## Phase 1: 骨架 ✅

九宫四盘架构 + 奇门遁甲基础模型。

- [x] **P1.1** `src/stems/` — Stem enum (戊己庚辛壬癸 + 乙丙丁 + 甲), YiIntent (六仪意图), Palace mapping
- [x] **P1.2** `src/palaces/` — 九宫基础设施
  - `config.rs` — 坤二 ConfigLoader + SecuritySection + ProviderProfile
  - `io.rs` — 坎一 ChannelManager（待实现）
  - `tool/` — 震三 ToolRegistry + BaseTool trait + 3 内置工具
  - `context.rs` — 巽四 ContextWindow（令牌预算 + 75% 阈值触发滑动窗口裁剪）
  - `core.rs` — 中五 JiaCore（LlmProvider 封装 + pub(crate) infer）
  - `permission.rs` — 乾六 PermissionMatrix（路径沙箱化 + 命令过滤）
  - `skill/` — 离九 SkillRegistry + SkillLoader（SKILL.md 解析 + 注入）
  - `store.rs` — 艮八 Store（SQLite + WAL 模式）
  - `gateway.rs` — 兑七 APIGateway（HTTP + SSE + /confirm 路由）
- [x] **P1.3** `src/plates/` — 四盘组装
  - `earth.rs` — EarthPlate（Arc<T> 一局不变，assemble() 起局）
  - `heaven/` — Agent struct + Agent::run() 主循环 + Agent::post_loop()
  - `human.rs` — HumanPlate（八门开阖 + 四种分发策略 + 用户确认）
  - `spirit.rs` — SpiritPlate（EventBus + 9 种 RuntimeEvent）
- [x] **P1.4** `src/geju/` — 格局评估引擎
  - Layer 1: 14 命名格局规则（飞鸟跌穴、青龙返首、太白入荧 等）
  - Layer 2: 能力语义匹配（戊己庚辛壬癸 × 六域）
  - Layer 3: 安全底线（默认 Guarded fail-safe）
  - Layer 4: SystemPrinciple 叠加收紧
- [x] **P1.5** 3 个内置工具：read_file, write_file, shell（均带 PermissionMatrix 校验）
- [x] **P1.6** `src/lib.rs` — `init()` 起局函数

---

## Phase 2: 核心循环 ✅

Agent 完整 推理→格局判断→工具执行→结果反馈 循环。

- [x] **P2.1** `plates/heaven/loop.rs` — Agent::run() 主循环
  - 系统提示词构建（工具 + 技能注入）
  - LLM 流式推理（SSE delta 逐 token 推送）
  - `<tool_call>` 块解析 → 工具查找 → GeJu 评估
  - Layer 4 原则叠加（atma_graha 驱动）
  - HumanPlate 分发（Direct/Guarded/Sandbox/Denied）
  - 工具结果反馈回 LLM 继续推理
  - 多轮循环直到无工具调用或达到 max_turns
- [x] **P2.2** AgentPhase 状态机（九星定义 + TUI 状态栏实时显示 + 全部 9 相位已调度）
- [x] **P2.3** Boundary + sandbox — 八门实时检查 + 权限校验 + 沙箱路径转换
- [x] **P2.4** ContextWindow 自动裁剪（令牌阈值触发滑动窗口，保护 system prompt）
- [x] **P2.5** Web 前端升级：多轮对话 + 工具调用卡片（折叠展开）+ GeJu 标签 + 确认弹窗

---

## Phase 3: 记忆系统 ✅

完整唯识记忆 + 坐忘消解管道。

- [x] **P3.1** `memory/` — 唯识记忆层
  - `seed.rs` — Seed 定义（SeedNature, SeedSource, SeedContent, relevance_score）
  - `seed_store.rs` — SeedStore（CRUD + session 索引）
  - `working.rs` — WorkingMemory（TurnSnapshot 环形缓冲，最近 20 轮）
  - `manas/mod.rs` — Manas（第七识末那识，数据驱动 atma_graha，AlayaEntropy 校准）
  - `consolidation.rs` — ConsolidationEngine（LLM 推断因果/实体/偏好/过程）
- [x] **P3.2** `zuowang/` — 坐忘管道
  - `pipeline.rs` — ZuowangPipeline::dissolve() 四层消解（SNAPSHOT→COMPUTE→APPLY→VERIFY）
  - `trigger.rs` — AlayaEntropy 多维熵计算（staleness + contradiction + redundancy + access_decay）
- [x] **P3.3** SQLite 持久化（rusqlite + WAL 模式，4 张表：sessions/seeds/self_models/principles）
- [x] **P3.4** `principles/` — SystemPrinciple 涌现（分组快照 → 错误率分析 → 约束推导）

---

## Phase 4: 自进化 ✅

- [x] **P4.1** GreatCommunion 实现（N×Wisdom → 1×SystemPrinciple，derive() + tighten()）
- [x] **P4.2** Layer 4 格局叠加（SystemPrinciple 单向收紧，EscalateTo/AddGuard/RequireAudit）
- [x] **P4.3** `telemetry/` — EventBus + Prometheus 指标 + tracing 日志
- [x] **P4.4** Terminal TUI（ratatui 实现，5 层布局 + 九星相位状态栏 + 多选题面板 + 会话持久化）

---

## Phase 5: 安全与沙箱 ✅

- [x] `SecuritySection` 配置 schema（project_root, allowed_paths, blocked_prefixes, command_allowlist/blocklist）
- [x] PermissionMatrix 完整实现（路径 canonicalize + 根边界检查 + 写操作父目录处理 + 命令提取检查）
- [x] 工具注入沙箱（read_file/write_file/shell 均带权限校验）
- [x] HumanPlate 分发执法（八门实时检查 + approval_chain 遍历 + UserConfirmation oneshot 通道）
- [x] SSE 确认协议（ConfirmRequest → POST /confirm → oneshot resolve）
- [x] Web UI 确认弹窗（30 秒倒计时自动拒绝）

---

## Phase 6: 可靠性 & 技能 ✅

- [x] **ContextWindow** 完整实现 — 令牌近似计数（chars/3.5）+ 滑动窗口裁剪 + 保护 system prompt
- [x] **Provider 加固** — HTTP 状态码分类（RateLimit/Auth/ServerError/ClientError）+ max_tokens 配置化 + 流内错误检测
- [x] **Skill 系统** — SkillRegistry + SkillLoader（SKILL.md 解析 + skills/ 目录扫描 + 系统提示词注入）+ EvolutionEngine（五阶段自动演化管线：eligibility → trajectory → reflect → accumulate → revise）+ Emphasis 段支持 + 文件监视热加载
- [x] **Manas 数据驱动** — atma_graha 从 AlayaEntropy 校准（60% 数据驱动 + 40% 动量），stable_epochs 跟踪

---

## Phase 7: 工具生态 & 通道扩展 & 架构重整 ✅

### P7.1 工具生态扩展 ✅

- [x] `web_search` 工具 — 网页搜索（DuckDuckGo Instant Answer API）
- [x] `web_fetch` 工具 — 网页内容抓取 + HTML→文本转换
- [x] `grep` 工具 — 代码库全文搜索（支持 glob 过滤）
- [x] `edit` 工具 — 精确字符串替换编辑
- [x] `agent` 工具 — 子代理分发（Explore / Plan 模式）

### P7.2 通道扩展 ✅

- [x] **ChannelManager** (坎一) — 多通道输入管理
  - stdin 交互通道（`jia repl` 终端对话）
  - 文件监视通道（`notify` 实时文件变更检测，debounce 批处理）
  - Webhook 触发通道（`POST /webhook` HTTP endpoint）

### P7.3 架构重整 ✅

- [x] **九宫 palaces/ 全部拼音_英文子目录** — kan_io, kun_config, zhen_tool, xun_context, zhong_core, qian_permission, dui_gateway, gen_store, li_skill
- [x] **四盘 plates/ 全部拼音_英文子目录** — di_earth, tian_heaven, ren_human, shen_spirit
- [x] **唯识 memory/ → vijnana/** — 重命名并对齐架构文档 15.2 八识命名：
  - `alaya/` — 第八识阿赖耶识（seed + store）
  - `mano/` — 第六识 Manovijñāna（WorkingMemory）
  - `manas/` — 第七识末那识（Manas）
  - `vasana/` — 熏习（ConsolidationEngine）
- [x] **坐忘 zuowang/ 提升为顶级模块** — pipeline + trigger 纯英文子目录
- [x] 架构文档 `jia-architecture.md` 同步更新

### P7.4 Agent 增强

- [x] 跨会话学习 — 全局原则共享，会话间知识迁移
- [x] 历史压缩 — LLM 驱动的对话摘要替代简单滑动窗口
- [x] 子代理机制 — Agent 内部 spawn Explore/Plan 子代理（多轮工具执行）
- [x] 图像输入 — 视觉模型支持多模态输入（Anthropic/OpenAI 兼容）

### P7.5 测试与质量 ✅

- [x] Agent loop 端到端测试（mock LLM provider）
- [x] GeJu 回归测试（固定 81 组合预期结果）
- [x] HumanPlate 八门组合测试
- [x] 压力测试（长对话内存/令牌使用）

---

## Phase 8: 生态扩展 ✅

- [x] **P8.1** MCP server 集成（modelcontextprotocol.io）
  - JSON-RPC 2.0 over stdio 传输层（`mcp/protocol.rs` + `connection.rs`）
  - `McpTool` — 将 MCP 工具封装为框架 BaseTool
  - `McpManager::connect_all()` — 启动时并行连接所有 MCP server，自动发现并注册工具
  - 配置 schema：`[[mcp_server]]`（`name`, `command`, `args`, `env`）
- [x] **P8.2** 多模型 Provider 扩展（Ollama, Gemini, DeepSeek）
  - Gemini：新建 `GeminiProvider`（Google Generative Language API, `candidates[0].content.parts[0].text` delta 路径）
  - DeepSeek / Ollama：复用 `OpenAIProvider`（OpenAI-compatible API）
  - `create_provider()` factory 扩展显式 arm（`anthropic`, `gemini`, `openai/deepseek/ollama`）
- [x] **P8.3** Cron 定时任务
  - `CronTool`：add/list/remove/enable/disable 五操作
  - `CronRunner`：后台 tokio task（30s 间隔），5 字段 cron 解析（纯 std 无新依赖），到期推送 ChannelInput
- [x] **P8.4** Git 工具
  - `GitTool`：安全 git 子命令执行（status/diff/log/branch/commit/add/checkout/stash/show/blame/tag）
  - 危险操作拦截（push --force, reset --hard, clean -f/d）
- [x] **P8.5** IM 机器人（个人微信 + Telegram）
  - 个人微信（新增）：iLink Bot API 长轮询，QR 码扫码登录，`[bots.wechat]` 配置
  - Telegram：long-polling `getUpdates` 2s 间隔，消息推入 ChannelManager
  - Discord：已移除。JIA 聚焦 WebSocket 和长轮询通道，webhook 模式不在当前范围内
  - 配置：`[bots.wechat]` / `[bots.telegram]`
- [ ] **P11.x** 更多通道（WebSocket 模式）
  - Slack（Socket Mode）：`slack-bolt` WebSocket 连接，无需公网端点
  - QQ：官方 Bot API WebSocket，消息推送 + 群聊支持
  - 飞书（Feishu/Lark）：WebSocket 长连接，事件订阅
  - 全部进入 `channels/` crate，统一长连接模式
- [x] **P8.6** 记忆系统分层预算优化
  - 三层金字塔预算制：Always ≤10 / OnDemand ≤200 / Archive ≤1000
  - `memory_catalog()` SQL 聚合（catalog_stats GROUP BY）替代 load_all，O(1) 与种子数无关
  - FNV-1a 哈希去重修复 Bug A（is_redundant 对相同文本返回 false）
  - CatalogCache 删除修复 Bug B（set_tier_batch 后缓存过期）
  - `enforce_tier_budgets()` 数量闸门接入 post_loop，在 dissolve 之后执行
  - 测试：282 通过，0 失败（含 9 个新增冒烟测试）

- [x] **P8.7** 多平台沙箱 — ExecutionSandbox trait + 进程沙箱 (rlimit/进程组) + Docker 沙箱 (feature gate) + macOS Seatbelt (sandbox-exec) + Linux Landlock (raw syscalls, kernel 5.13+)
- [x] **P8.8** 插件系统 — WASM 插件运行时 (wasmtime, feature gate `wasm-plugin`) + PluginManager 自动发现加载

---

## 依赖关系

```
Phase 0 (Web Chat) ✅
    │
Phase 1 (骨架: stems + palaces + plates + geju) ✅
    │
Phase 2 (核心循环: Agent Loop + 工具执行 + 格局判断) ✅
    │
Phase 3 (记忆系统: 唯识 + 坐忘) ✅
    │
Phase 4 (自进化: SystemPrinciple + TUI) ✅
    │
Phase 5 (安全沙箱: PermissionMatrix + HumanPlate) ✅
    │
Phase 6 (可靠性: ContextWindow + Provider + Skill + Manas) ✅
    │
Phase 7 (工具生态 + 通道 + 架构重整 + Agent 增强) ✅
    │
Phase 8 (生态扩展: MCP + 多模型 + cron/git + IM 机器人 + 沙箱 + WASM 插件) ✅

关键依赖:
  Phase 2 依赖 Phase 1 的 EarthPlate + GeJu
  Phase 3 依赖 Phase 1 的 Store (SQLite 后端)
  Phase 4 依赖 Phase 3 的 SeedStore + SystemPrinciple
  Phase 5 依赖 Phase 1-2 的 HumanPlate + Agent Loop
  Phase 6 依赖 Phase 3 的 AlayaEntropy
  Phase 7 各子任务无强依赖，可并行推进
```

---

## 当前状态

| 指标 | 数值 |
|------|------|
| 源代码文件 | 141 `.rs` 文件 |
| 总代码行数 | ~40,800 |
| 测试数量 | 428（421 单元 + 7 Ignored/E2E） |
| 已实现阶段 | Phase 0–8 (全部完成) |
| 空壳模块 | 0 |
| 内置工具 | 23（read_file, write_file, shell, grep, glob, edit, lsp, web_fetch, web_search, delegate, cron, git, computer_use, browser, ask_user, namarupa, skill, task, web_execute_js, plan_mode, scratchpad, toolsearch, worktree, retrieve_compacted + MCP 发现工具） |
| 命名 GeJu 格局 | 14 |
| Provider 支持 | 3 种 kind + native tools API（openai 兼容 / anthropic / gemini） |
| 工具调用 | Native tools API（openai/anthropic/gemini）优先，XML 文本 fallback |
| 持久化后端 | SQLite（WAL 模式） |
| MCP 协议 | JSON-RPC 2.0 over stdio（tools/list + tools/call） |
| IM 机器人 | 个人微信（iLink polling）+ Telegram（polling）+ Discord（Webhook） |
| UI | Web（SSE）+ Terminal TUI（ratatui, 5 层布局 + 九星相位 + 多选面板） |
| 架构模块 | 10 个顶级模块 + 重构分拆（6 巨石文件 → 42 子模块） |
| 特性门控 (feature gates) | web-search, git, cron, mcp, wasm-plugin, tui, test-utils |

**最后更新**: 2026-06-28

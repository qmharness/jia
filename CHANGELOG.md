# Changelog

All notable changes to Jia (甲) will be documented in this file.

## [Unreleased]

### CLI
- `jia` (bare, no subcommand) now launches TUI directly; `jia tui` still works for compatibility
- `--config` moved to top-level option, shared by all subcommands



### Native Tools API
- openai/anthropic/gemini providers now use native function calling / tool_use API
- `StreamChunk::NativeToolCall` for structured tool calls from providers
- OpenAI: `tools` array + streaming `tool_calls` delta parsing
- Anthropic: `tools` array + `tool_use` content blocks (start/delta/stop) parsing
- Gemini: `functionDeclarations` + `functionCall` response parsing
- XML text fallback preserved for unknown provider kinds
- System prompt conditionally omits tool schema text when using native tools (saves tokens)

### TUI Question Panel
- Deadlock fixed: `agent.run()` spawned instead of blocking reader task, so `ask_user` answers arrive promptly
- Question panel clears on confirm (Enter) / cancel (Esc)
- Input locked during multi-select; only nav/select keys work
- Selected option: Cyan Bold + `❯` arrow indicator; unselected: default color
- Timeout removed from question display (waits indefinitely)
- Placeholder cleared on all Question→Normal transitions
- Tool result output rendered as separate markdown line (headings, bold, code blocks work)

### Nine-Star AgentPhase Status Bar
- `AgentPhase` enum with 9 star names plus `display_name()` method
- Status bar: 九星相位 replaces mode label (`⚡ 天蓬 Reasoning · 12s`)
- Mode label moved to info_bar (`⏵⏵ Normal · model · sid`)
- Phase auto-tracks via stream events (Reasoning/ToolCalling/AwaitingResult/ etc.)
- Added `ContextPressure` and `Compacting` events for 天辅/天英 phases

### Provider Kind Consolidation
- `deepseek` and `ollama` kinds removed; use `kind = "openai"` for all OpenAI-compatible APIs
- Ollama model listing auto-detected via `/api/tags` (URL without `/v1`)

### Bug Fixes
- UTF-8 byte boundary panic in tool call truncation (Emoji/CJK safe)
- JSON fragment stripping for models that leak tool-call text alongside native API calls
- Duplicate blank line prevention in Delta streaming + ToolCall cards
- TUI model info queried from daemon via socket, not config file
- `VIEWPORT_HEIGHT` 14→18 to show all 9 question options
- CI feature matrix fixed (removed non-existent `agent-tool`, `sandbox-docker`)

## [0.2.0] — 2026-06-19

### Megafile Refactoring
- Split 6 monolithic files (>1,500 lines) into 42 cohesive submodules
- `evolution.rs`: 2,238 → 7 files (impl method grouping)
- `gen_store/mod.rs`: 2,134 → 9 files (impl Store partitioning, zero breaking)
- `dui_gateway/mod.rs`: 2,295 → 14 files (syntax-based extraction, 78 items)
- `loop.rs`: 1,510 → 7 files (auxiliary function extraction)
- `vasana/mod.rs`: 2,070 → 230 lines (test extraction)
- `zuowang/pipeline`: 1,549 → 396 lines (test extraction)

### Code Quality
- Warnings: 528 → 0 (100% elimination)
- Clippy errors: 6 → 0
- Tests: 335 → 370 (+35 new tests)
- Benchmarks: 4 → 6 (FTS5 search + entropy computation)
- Build fixed, PID atomic write, backup health check, CORS restricted

### Agent Enhancements (P0-P2)
- Adaptive dissolution threshold (scales with seed count)
- Seed strength active reinforcement (frequent access → stronger)
- Delegate tool ceremony fix (planning mode compatible)
- Sub-agent session SQLite persistence (crash recovery)
- Tool error intelligent fallback (actionable recovery hints)
- FreeText seed redundancy detection (FTS5 fingerprinting)
- Post-revision skill verification (smoke-test after evolve)
- Sub-agent scratchpad progress reporting
- Parallel sub-agent execution (concurrency-safe delegates)

### Security
- SecretsRedactWriter: file log API key redaction
- Gemini/x-api-key pattern redaction
- SSE /events prompt redaction for unauthenticated clients
- PID file atomic write (prevent duplicate daemons)

### Documentation
- ROADMAP.md metrics updated (99→141 files, 303→370 tests)
- architecture.md conceptual code disclaimer added
- .env.example completed with all documented variables
- All `#[ignore]` tests have reason strings

## [0.1.0] — 2026-06-05

### Initial Release

First public release of the Jia AI Agent runtime.

#### Core Architecture
- 9-Palace (九宫) functional domain architecture
- 4-Plate (四盘) operational layer model
- Qimen Dunjia (奇门遁甲) architectural metaphor throughout

#### LLM Providers (5)
- Anthropic (Claude)
- OpenAI (GPT)
- DeepSeek
- Ollama (local models)
- Gemini

#### Built-in Tools (18)
- `read_file`, `write_file`, `edit` — file operations
- `shell` — command execution (sandboxed)
- `grep` — code search
- `web_fetch`, `web_search` — web access
- `delegate` — sub-agent delegation
- `cron` — scheduled tasks
- `git` — version control operations
- `computer_use` — desktop automation
- `browser` — web browser control
- `ask_user` — human-in-the-loop confirmation
- `skill` — skill invocation
- `namarupa` — structured data generation
- And more

#### Interfaces
- HTTP/SSE gateway (Axum)
- Web UI (Svelte 5 + Vite + TypeScript)
- Terminal TUI (ratatui)
- CLI (Clap)
- IM bots: WeChat (iLink), Telegram, Discord

#### Safety & Sandboxing
- Process-level sandbox (rlimit)
- Docker sandbox
- macOS Seatbelt sandbox
- Linux Landlock sandbox
- Permission matrix with path/command filtering
- GeJu (格局) evaluation engine with 14 named patterns

#### Storage & Memory
- SQLite with WAL mode
- FTS5 full-text search
- Vijnana (唯识) memory system with atma-graha recalibration
- Zuowang (坐忘) dissolution pipeline

#### Skills & Evolution
- SKILL.md loading and injection
- Skill evolution engine with reflection and revision
- WASM plugin system (feature-gated via wasmtime)
- 16 built-in skill definitions

#### Observability
- Event bus (Spirit Plate)
- Hook system for lifecycle events
- Prometheus metrics
- Structured tracing

#### Protocol Support
- MCP (Model Context Protocol) via JSON-RPC 2.0 over stdio

#### Testing
- 298 unit tests
- 4 concurrent session tests
- 6 end-to-end tests
- 28 stress tests
- Criterion benchmarks

#### Metrics
- 99 Rust source files
- ~31,000 lines of code
- 336 total tests (0 failures)
- 8 completed development phases

## Cognitive Architecture — 2026-07-05

### New Features
- **TurnCertainty**: Behavior-based confidence signals drive adaptive termination (ConfidentStop / EscalateToHuman / HardLimitReached)
- **SeedDisposition**: Per-seed mutable response tendencies (性决定 + 待众缘), distinct from fixed SeedNature
- **CoActivationMatrix**: Sparse temporal seed co-occurrence tracking with exponential decay (俱有因)
- **Eight Spirits full activation**: TaiYin (certainty trajectory), BaiHu (anomaly detection with 4-level gate), XuanWu (memory loss), JiuTian (strategy emergence)
- **CompletionChecklist**: Deterministic completion signal detection (exit codes, file existence) — zero LLM cost
- **ContextReset**: Session handoff for long-running conversations with anti-thrashing cooldown
- **SSE forwarding**: All 16 RuntimeEvent variants now streamable via `/events`
- **Frontend**: Spirit Observer panel on Vijnana page (real-time 八神 event stream)
- **Config**: `[cognition]` section with feature flags for all new components

### Philosophy
- Two comprehensive philosophy architecture documents (zh-CN + en)
- Architecture-Cognition Fusion axiom replacing dual-framework model
- Position-Consciousness integration interface specification

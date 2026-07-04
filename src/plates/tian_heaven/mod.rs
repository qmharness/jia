//! tian_heaven — Heaven Plate / Agent Loop (天盘)

use std::sync::Arc;
pub mod r#loop;

mod loop_dispatch;
mod loop_events;
mod loop_hooks;
mod loop_parse;
mod loop_post;
mod loop_prompt;

use super::di_earth::EarthPlate;
use crate::palaces::xun_context::{ContextWindow, ToolOutputBudget};
use crate::principles::SystemPrinciple;
use crate::stems::Stem;
use crate::stems::action::ExecContext;
use crate::types::HistoryEntry;
use crate::vijnana::alaya::SeedStore;
use crate::vijnana::manas::Manas;
use crate::vijnana::mano::WorkingMemory;
use crate::vijnana::user_profile::UserProfileManager;

/// 天盘 (Heaven Plate) — The dynamic agent execution loop.
///
/// Each turn: Environment capture → Seed retrieval → LLM infer → GeJu evaluate → Dispatch.
pub struct HeavenPlate;

/// An agent instance. One per session/conversation.
pub struct Agent {
    pub id: String,
    pub earth: Arc<EarthPlate>,
    /// P6 · execution context (天盘时令). Carries the current PermissionMatrix,
    /// which the Agent swaps on worktree enter/exit (O(1)). Tools are stateless
    /// singletons registered on `earth.tools` (地盘六仪不动); permissions are
    /// injected at execution time via this context (值符随时干旋转).
    pub exec_ctx: ExecContext,
    /// P6 · current worktree root. None = main project root.
    /// Some(path) = inside a worktree; exit restores exec_ctx and optionally
    /// removes the worktree.
    pub worktree_root: Option<std::path::PathBuf>,
    /// Layer 4 · self-evolution principles loaded at session start.
    /// Applied to tighten execution mode after GeJu evaluation.
    /// Persisted across sessions — principles only tighten, never relax.
    pub principles: Vec<SystemPrinciple>,
    pub turn_count: u32,
    pub max_turns: u32,
    pub retry_count: u32,
    /// Unified conversation history (messages + tool calls, persists across turns)
    pub history: Vec<HistoryEntry>,
    /// Working memory (ring buffer of turn snapshots, in-memory only)
    pub working_memory: WorkingMemory,
    /// Manas (第七识) with atma_graha (ātma-grāha) dynamics (persisted across requests)
    pub manas: Manas,
    /// Context window token-budget manager
    pub context_window: ContextWindow,
    /// Per-tool output token budget manager
    pub output_budget: ToolOutputBudget,
    /// Anti-thrashing: turn when last compaction ran (0 = never)
    pub cc_last_turn: u32,
    /// Token count (llm_messages) before last compaction
    pub cc_tokens_before: usize,
    /// Token count (llm_messages) after last compaction
    pub cc_tokens_after: usize,
    /// Accumulated seed IDs touched this turn (flushed at start of next turn)
    pub touched_seed_ids: Vec<String>,
    /// Content hashes of already-distilled pairs (avoids redundant LLM calls).
    pub distilled_hashes: std::collections::HashSet<u64>,
    /// Consecutive failure count per tool name (GeJu Layer 3 runtime supplement).
    /// Reset on first success for that tool.
    pub tool_failure_count: std::collections::HashMap<String, u32>,
    /// Previous compaction summary for iterative update (avoids re-summarization).
    pub compaction_summary: Option<String>,
    /// Max consecutive failures before refusing a tool.
    pub max_consecutive_failures: u32,
    /// Skill names activated by path matching this session (accumulated, deduplicated).
    pub activated_skills: Vec<String>,
    /// Skills explicitly invoked via skill("<name>") tool this session
    /// (deduplicated — each skill appears at most once).
    pub skill_tool_calls: Vec<String>,
    /// 仁 — user-defined character loaded from ren_soul.md.
    /// None if the file does not exist or is empty.
    pub ren_soul: Option<String>,
    /// P3 · Interaction mode (谋划态). A user-facing interaction state, NOT a
    /// nine-star AgentPhase (which is the loop's internal execution phase). In
    /// Planning mode the loop short-circuits destructive tools before GeJu.
    pub interaction_mode: InteractionMode,
}

/// P3 · Interaction mode — 谋划态 (planning) vs Normal.
///
/// Distinct from `AgentPhase` (九星, loop execution phase): this is a
/// user-facing interaction state. `Planning` forces read-only operation —
/// destructive tools are rejected by a loop-level short-circuit before GeJu
/// evaluation, so GeJu stays a pure 干叠加 evaluator (A2). User-triggered
/// primarily (slash/TUI); the model may also call enter/exit_plan_mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionMode {
    #[default]
    Normal,
    /// 谋划态 — read-only research/planning. Destructive tools blocked.
    Planning,
}

impl Agent {
    pub fn new(id: String, earth: Arc<EarthPlate>) -> Self {
        let ctx = ContextWindow::new(
            earth.config.app_config.security.max_context_tokens,
            earth.config.app_config.security.compaction_threshold,
        );
        let exec_ctx = ExecContext {
            permissions: earth.permissions.clone(),
        };
        let principles = earth
            .store
            .load_active_principles()
            .unwrap_or_default()
            .iter()
            .filter_map(|j| serde_json::from_str::<SystemPrinciple>(j).ok())
            .collect();
        let mut s = Self {
            id,
            earth: earth.clone(),
            exec_ctx,
            principles,
            turn_count: 0,
            max_turns: 25,
            retry_count: 0,
            history: Vec::new(),
            working_memory: WorkingMemory::new(20),
            manas: Manas::new(),
            context_window: ctx,
            output_budget: ToolOutputBudget::default(),
            cc_last_turn: 0,
            cc_tokens_before: 0,
            cc_tokens_after: 0,
            touched_seed_ids: Vec::new(),
            distilled_hashes: std::collections::HashSet::new(),
            tool_failure_count: std::collections::HashMap::new(),
            compaction_summary: None,
            max_consecutive_failures: 3,
            ren_soul: None,
            activated_skills: Vec::new(),
            skill_tool_calls: Vec::new(),
            worktree_root: None,
            interaction_mode: InteractionMode::Normal,
        };
        // Load ren_soul.md — auto-seed default if missing.
        s.load_ren_soul();
        s
    }

    /// Create an agent with persisted session state.
    pub fn with_session(
        id: String,
        earth: Arc<EarthPlate>,
        history: Vec<HistoryEntry>,
        manas: Manas,
        distilled_hashes: std::collections::HashSet<u64>,
    ) -> Self {
        let ctx = ContextWindow::new(
            earth.config.app_config.security.max_context_tokens,
            earth.config.app_config.security.compaction_threshold,
        );
        let exec_ctx = ExecContext {
            permissions: earth.permissions.clone(),
        };
        let principles = earth
            .store
            .load_active_principles()
            .unwrap_or_default()
            .iter()
            .filter_map(|j| serde_json::from_str::<SystemPrinciple>(j).ok())
            .collect();
        let mut s = Self {
            id,
            exec_ctx,
            principles,
            earth,
            turn_count: 0,
            max_turns: 25,
            retry_count: 0,
            history,
            working_memory: WorkingMemory::new(20),
            manas,
            context_window: ctx,
            output_budget: ToolOutputBudget::default(),
            cc_last_turn: 0,
            cc_tokens_before: 0,
            cc_tokens_after: 0,
            touched_seed_ids: Vec::new(),
            distilled_hashes,
            tool_failure_count: std::collections::HashMap::new(),
            compaction_summary: None,
            max_consecutive_failures: 3,
            ren_soul: None,
            activated_skills: Vec::new(),
            skill_tool_calls: Vec::new(),
            worktree_root: None,
            interaction_mode: InteractionMode::Normal,
        };
        // Load ren_soul.md — if it doesn't exist, auto-seed default.
        s.load_ren_soul();
        s
    }

    /// Activate skills whose path patterns match the files touched this turn.
    pub fn activate_skills(&mut self, touched_paths: &[&str]) {
        if touched_paths.is_empty() {
            return;
        }
        if let Ok(reg) = self.earth.skills.read() {
            for name in reg.activate_for_paths(touched_paths) {
                if !self.activated_skills.contains(&name) {
                    self.activated_skills.push(name);
                }
            }
        }
    }

    /// Report skill usage after the agent loop completes.
    /// Logs skills invoked via tool and identifies tool-only skills that went unused.
    pub fn report_skill_usage(&self) {
        // Collect data under lock, release before tracing
        let (tool_only_unused, always_count) = {
            let reg = match self.earth.skills.read() {
                Ok(r) => r,
                Err(e) => e.into_inner(),
            };
            let all_skills = reg.list_all();
            let unused: Vec<String> = all_skills
                .iter()
                .filter(|s| !s.always && s.paths.is_none())
                .filter(|s| !self.skill_tool_calls.contains(&s.name))
                .map(|s| s.name.clone())
                .collect();
            let always = all_skills.iter().filter(|s| s.always).count();
            (unused, always)
        }; // lock dropped here — tracing calls below don't hold it

        if !tool_only_unused.is_empty() {
            tracing::debug!(
                session = %self.id,
                "Available skills never invoked via tool: {}",
                tool_only_unused.join(", ")
            );
        }

        if !self.skill_tool_calls.is_empty() {
            tracing::info!(
                session = %self.id,
                turns = self.turn_count,
                "Skills invoked via tool: {}",
                self.skill_tool_calls.join(", ")
            );
        }

        // Skill usage summary
        tracing::info!(
            session = %self.id,
            turns = self.turn_count,
            always_skills = always_count,
            path_activated = self.activated_skills.len(),
            tool_called = self.skill_tool_calls.len(),
            "Skill usage summary"
        );
    }

    /// Build a system prompt section describing available tools and loaded skills.
    // ── ren_soul.md ──────────────────────────────────────────
    /// Default 仁心 template auto-seeded when ren_soul.md does not exist.
    const DEFAULT_REN_SOUL: &str = "\
<!--\n\
  ren_soul.md \u{2014} Jia's character file.\n\
  Define who Jia is and how Jia should behave.\n\
  Edit freely. Changes take effect on next session.\n\
\n\
  Jia's identity is not a fixed soul \u{2014} it is \u{4ec1} (r\u{e9}n),\n\
  a cultivated way of being. Plant the seed here.\n\
-->\n\
You are Jia (\u{7532}), Just Intelligence Agent (正是智能体).\n\
Be attentive, truthful, and serve with sincerity.";

    /// Load ren_soul.md from the data directory.
    /// Auto-seeds the default template if the file doesn't exist.
    /// Plants the content as a Protected Always-tier seed in Alaya.
    pub fn load_ren_soul(&mut self) {
        let ren_path = self.earth.data_dir.join("ren_soul.md");
        if !ren_path.exists() {
            let _ = std::fs::write(&ren_path, Self::DEFAULT_REN_SOUL);
        }
        let content = std::fs::read_to_string(&ren_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        self.ren_soul = content.clone();

        // Plant as Protected Always seed in Alaya.
        if let Some(text) = content {
            use crate::palaces::Palace;
            use crate::stems::Stem;
            use crate::vijnana::alaya::{Seed, SeedContent, SeedNature, SeedSource, SeedTier};

            let seed = Seed {
                id: "ren_soul_root".to_string(),
                session_id: "_jia_system".to_string(),
                nature: SeedNature::Preference,
                source: SeedSource::RenSoul,
                content: SeedContent::FreeText { text },
                palace: Palace::Zhong,
                intent_stem: Stem::Wu,
                geju_key: "ren_soul".to_string(),
                created_at: crate::utils::unix_now(),
                access_count: 0,
                last_accessed_at: crate::utils::unix_now(),
                strength: 1.0,
                tier: SeedTier::Always,
            };
            if let Ok(json) = serde_json::to_string(&seed) {
                let _ = self
                    .earth
                    .store
                    .delete_seeds(&["ren_soul_root".to_string()]);
                let _ = self.earth.store.insert_seed(&json);
            }
        }
    }

    /// Build a system prompt section describing available tools and loaded skills.
    /// Stable system-prompt segment: tool catalog + always-on skills.
    ///
    /// Byte-stable across turns (modulo skill hot-reload), so it carries the
    /// Anthropic `cache_control: ephemeral` breakpoint (P2). The 人设 (ren) is
    /// prepended by `build_system_prompt` via `build_ren_prompt`.
    pub fn build_stable_prompt(&self, use_native_tools: bool) -> String {
        let mut prompt = String::new();

        // Tools — P9: describe only core (built-in) tools in the stable prompt.
        // External (MCP/WASM) tools are NOT described here (they'd bloat the
        // cacheable stable segment); the agent discovers them via `toolsearch`.
        // `toolsearch` itself is core but only advertised when external tools
        // exist (keeps the no-MCP prompt identical to pre-P9 → D4).
        let has_external = !self.earth.tools.list_external().is_empty();
        let tools: Vec<_> = self
            .earth
            .tools
            .list_core()
            .into_iter()
            .filter(|t| t.name() != "toolsearch" || has_external)
            .collect();
        if !tools.is_empty() {
            if use_native_tools {
                // Native tools API — provider handles schema via API; prompt only lists names.
                prompt.push_str("\n\n## Available Tools\n\n");
                let names: Vec<_> = tools.iter().map(|t| t.name()).collect();
                prompt.push_str(&names.join(", "));
                prompt.push_str("\n\nUse the provided function calling / tool use API to invoke these tools.\n\n");
            } else {
                // XML text fallback — full schema in prompt.
                prompt.push_str("\n\n## Available Tools\n\n");
                prompt.push_str("You have access to the following tools. To use a tool, include a tool call block in your response using this exact format:\n\n");
                prompt.push_str(
                    "<tool_call>\n{\"tool\": \"tool_name\", \"parameters\": {...}}\n</tool_call>\n\n",
                );
                prompt.push_str("CRITICAL: Always wrap tool calls in <tool_call>...</tool_call> tags. Never use bare XML tags like <tool_name>...</tool_name>.\n\n");
                prompt.push_str("You may call multiple tools. After receiving tool results, continue reasoning.\n\n");

                for tool in &tools {
                    let schema = tool.parameters_schema();
                    prompt.push_str(&format!(
                        "### {}\n{}\nParameters: {}\n\n",
                        tool.name(),
                        tool.description(),
                        serde_json::to_string_pretty(&schema).unwrap_or_default()
                    ));
                }
            }
        }

        // Skills marked `always: true` are injected into every prompt. These are
        // part of the stable segment. Context-activated skills (dynamic) are in
        // `build_dynamic_prompt`.
        if let Ok(skills) = self.earth.skills.read() {
            let always_skills: Vec<_> =
                skills.list_all().into_iter().filter(|s| s.always).collect();
            if !always_skills.is_empty() {
                prompt.push_str("\n\n## Always-Active Skills\n\n");
                for skill in &always_skills {
                    prompt.push_str(&format!("### {}\n{}\n", skill.name, skill.prompt));
                    if let Some(ref emph) = skill.emphasis {
                        prompt.push_str(&format!(
                            "\n**Critical Reminder (follow with special attention):**\n{}\n",
                            emph
                        ));
                    }
                    prompt.push('\n');
                }
            }
        }

        prompt
    }

    /// Dynamic system-prompt segment: context-activated skills + user profile +
    /// memory catalog + top-influence seeds.
    ///
    /// Varies every turn (memory injection is atma_graha-gated), so it is never
    /// cached — it sits after the Anthropic cache breakpoint. Carries the
    /// `touched_seed_ids` side effect (preserved from the former build_tool_prompt).
    pub fn build_dynamic_prompt(&mut self) -> String {
        let mut prompt = String::new();

        // Context-Activated Skills (depends on touched paths → dynamic)
        if let Ok(skills) = self.earth.skills.read()
            && !self.activated_skills.is_empty()
        {
            prompt.push_str("\n\n## Context-Activated Skills\n\n");
            for name in &self.activated_skills {
                if let Some(skill) = skills.get(name) {
                    prompt.push_str(&format!("### {}\n{}\n", skill.name, skill.prompt));
                    if let Some(ref emph) = skill.emphasis {
                        prompt.push_str(&format!(
                            "\n**Critical Reminder (follow with special attention):**\n{}\n",
                            emph
                        ));
                    }
                    prompt.push('\n');
                }
            }
        }

        // User profile: preference seeds formatted for the LLM
        let profile_prompt = UserProfileManager::prompt(&self.earth.store);
        if !profile_prompt.is_empty() {
            prompt.push_str(&profile_prompt);
        }

        // Memory catalog: existence index for Alaya seeds
        let seed_store = SeedStore::new(self.earth.store.clone());
        let (catalog, always_ids) = seed_store.memory_catalog();
        prompt.push_str(&catalog);
        self.touched_seed_ids.extend(always_ids);

        // Seed retrieval: inject top past experience when memory is trusted.
        // First 3 turns always inject (before L2 consolidation can fire); after that,
        // gate on atma_graha to avoid surfacing seeds when memory is unreliable.
        if self.manas.atma_graha < 0.60 || self.turn_count <= 3 {
            let (seed_prompt, seed_ids) = seed_store.top_influence_prompt(5);
            if !seed_prompt.is_empty() {
                prompt.push_str(&seed_prompt);
            }
            self.touched_seed_ids.extend(seed_ids);
        }

        prompt
    }

    /// Derive intent stem from a tool's CeremoniesIntent
    pub fn intent_stem_from_tool(ceremony: &crate::stems::CeremoniesIntent) -> Stem {
        use crate::stems::CeremoniesIntent;
        match ceremony {
            CeremoniesIntent::Wu(_) => Stem::Wu,
            CeremoniesIntent::Ji(_) => Stem::Ji,
            CeremoniesIntent::Geng(_) => Stem::Geng,
            CeremoniesIntent::Xin(_) => Stem::Xin,
            CeremoniesIntent::Ren(_) => Stem::Ren,
            CeremoniesIntent::Gui(_) => Stem::Gui,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::gen_store::Store;
    use crate::palaces::kan_io::ChannelManager;
    use crate::palaces::kun_config::{AppConfig, SecuritySection};
    use crate::palaces::li_skill::SkillRegistry;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::palaces::zhen_tool::ToolRegistry;
    use crate::palaces::zhen_tool::builtin::{
        read_file::ReadFileTool, shell::ShellTool, write_file::WriteFileTool,
    };
    use crate::palaces::zhong_core::JiaCore;
    use crate::plates::di_earth::EarthPlate;
    use crate::plates::shen_spirit::SpiritPlate;
    use crate::vijnana::alaya::{Seed, SeedNature, SeedSource, SeedTier};

    fn temp_earth(tmp: &std::path::Path) -> Arc<EarthPlate> {
        let security = SecuritySection {
            project_root: Some(tmp.to_str().unwrap().to_string()),
            sandbox_disabled: true,
            ..SecuritySection::default()
        };
        let db_path = tmp.join("store.db");
        let config = AppConfig {
            host: "127.0.0.1".into(),
            port: 8080,
            providers: std::collections::HashMap::new(),
            default_main_model_provider: None,
            default_aux_model_provider: None,
            security: security.clone(),
            mcp_servers: vec![],
            bots: Default::default(),
            hooks: vec![],
        };
        let config_loader = Arc::new(crate::palaces::kun_config::ConfigLoader::from_app_config(
            config,
        ));
        let permissions = Arc::new(PermissionMatrix::from_config(
            &security,
            &tmp.join("workspace"),
            tmp.to_path_buf().join("backups"),
        ));
        let mut toollist = ToolRegistry::new();
        toollist.register(Arc::new(ReadFileTool::new()));
        toollist.register(Arc::new(WriteFileTool::new()));
        toollist.register(Arc::new(ShellTool::new()));
        let store = Arc::new(Store::open(db_path.to_str().unwrap()));
        let dummy_profile = crate::palaces::kun_config::ProviderProfile {
            kind: "openai".into(),
            models: vec!["dummy".into()],
            default_aux_model: None,
            default_main_model: None,
            api_key: "sk-dummy".into(),
            base_url: "http://localhost:1/v1".into(),
            max_tokens: Some(256),
            context_window: None,
            priority: None,
            cost_multiplier: None,
        };
        Arc::new(EarthPlate {
            io: Arc::new(ChannelManager::default()),
            config: config_loader,
            tools: Arc::new(toollist),
            main_core: Arc::new(JiaCore::new(&dummy_profile, "dummy")),
            aux_core: None,
            permissions,
            skills: Arc::new(std::sync::RwLock::new(SkillRegistry::new())),
            cron: crate::palaces::zhen_tool::builtin::cron::CronStore::new(
                tmp.to_path_buf().join("cron"),
            ),
            task_store: crate::palaces::zhen_tool::builtin::task::TaskStore::new(),
            store_async: crate::palaces::gen_store::async_store::StoreAsync::new(store.clone()),
            store,
            spirit: Arc::new(SpiritPlate::new()),
            user_hooks: Arc::new(Vec::new()),
            pending_confirmations: Arc::new(
                std::sync::Mutex::new(std::collections::HashMap::new()),
            ),
            pending_questions: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            subagent_sessions: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            session_modes: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            session_locks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            data_dir: tmp.to_path_buf(),
            pid_path: tmp.to_path_buf().join("gateway.pid"),
            backup_dir: tmp.to_path_buf().join("backups"),
        })
    }

    /// Parse seeds from store JSON strings.
    fn load_seeds(store: &Store) -> Vec<Seed> {
        store
            .load_all_seeds()
            .unwrap()
            .iter()
            .filter_map(|j| serde_json::from_str(j).ok())
            .collect()
    }

    #[test]
    fn smoke_ren_soul_autocreate_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let ren_path = tmp.path().join("ren_soul.md");
        assert!(
            !ren_path.exists(),
            "ren_soul.md should not exist before Agent::new"
        );

        let agent = Agent::new("smoke-1".into(), earth.clone());

        assert!(ren_path.exists(), "ren_soul.md should be auto-created");
        let content = std::fs::read_to_string(&ren_path).expect("read ren_soul.md");
        assert!(
            content.contains("You are Jia"),
            "default template should contain identity"
        );
        assert!(content.contains("仁"), "default template should mention 仁");

        assert!(
            agent.ren_soul.is_some(),
            "agent.ren_soul should be populated"
        );
        let ren = agent.ren_soul.as_ref().unwrap();
        assert!(
            ren.contains("You are Jia"),
            "ren_soul field should contain identity"
        );
    }

    #[test]
    fn smoke_ren_soul_alaya_seed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let agent = Agent::new("smoke-2".into(), earth.clone());
        let seeds = load_seeds(&earth.store);
        let ren_seed = seeds.iter().find(|s| s.id == "ren_soul_root");
        assert!(ren_seed.is_some(), "ren_soul_root seed should be in Alaya");

        let seed = ren_seed.unwrap();
        assert_eq!(seed.nature, SeedNature::Preference);
        assert!(matches!(seed.source, SeedSource::RenSoul));
        assert_eq!(seed.tier, SeedTier::Always);
        assert_eq!(seed.session_id, "_jia_system");
        assert_eq!(seed.palace, crate::palaces::Palace::Zhong);
        assert_eq!(seed.intent_stem, crate::stems::Stem::Wu);

        if let crate::vijnana::alaya::SeedContent::FreeText { ref text } = seed.content {
            assert!(
                text.contains("You are Jia"),
                "seed content should contain identity"
            );
        } else {
            panic!("expected FreeText seed content");
        }

        assert!(agent.ren_soul.is_some());
    }

    #[test]
    fn smoke_ren_soul_protected_from_dissolution() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let _agent = Agent::new("smoke-3".into(), earth.clone());

        // threshold=0.0 forces dissolve to run even on fresh seeds
        let report = crate::zuowang::pipeline::ZuowangPipeline::dissolve(earth.store.clone(), 0.0)
            .expect("dissolve should succeed");
        assert_eq!(
            report.seeds_dissolved, 0,
            "ren_soul_root should not be deleted"
        );

        let seeds = load_seeds(&earth.store);
        assert!(
            seeds.iter().any(|s| s.id == "ren_soul_root"),
            "ren_soul_root should survive dissolution"
        );
    }

    #[test]
    fn smoke_ren_soul_upsert_on_edit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let _agent1 = Agent::new("smoke-4a".into(), earth.clone());

        let ren_path = tmp.path().join("ren_soul.md");
        std::fs::write(&ren_path, "You are Jia, a customer support agent.").expect("write");

        let agent2 = Agent::new("smoke-4b".into(), earth.clone());
        assert_eq!(
            agent2.ren_soul.as_deref(),
            Some("You are Jia, a customer support agent.")
        );

        let seeds = load_seeds(&earth.store);
        let ren_seeds: Vec<_> = seeds.iter().filter(|s| s.id == "ren_soul_root").collect();
        assert_eq!(
            ren_seeds.len(),
            1,
            "should be exactly one ren_soul_root seed"
        );
        if let crate::vijnana::alaya::SeedContent::FreeText { ref text } = ren_seeds[0].content {
            assert_eq!(text, "You are Jia, a customer support agent.");
        } else {
            panic!("expected FreeText");
        }
    }

    #[test]
    fn smoke_build_ren_prompt_fallback() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let mut agent = Agent::new("smoke-5".into(), earth.clone());
        agent.ren_soul = None;

        let prompt = agent.build_ren_prompt();
        assert!(
            prompt.contains("You are Jia"),
            "fallback should contain identity"
        );
        assert!(
            !prompt.contains("Embody these values"),
            "fallback should not have modulation suffix"
        );
    }

    #[test]
    fn smoke_build_ren_prompt_modulation() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let mut agent = Agent::new("smoke-6".into(), earth.clone());

        agent.manas.atma_graha = 0.80;
        let prompt_high = agent.build_ren_prompt();
        assert!(
            prompt_high.contains("Embody these values"),
            "high atma_graha should add modulation: {prompt_high}"
        );

        agent.manas.atma_graha = 0.30;
        let prompt_low = agent.build_ren_prompt();
        assert!(
            !prompt_low.contains("Embody these values"),
            "low atma_graha should be plain: {prompt_low}"
        );
        assert!(
            prompt_low.contains("You are Jia"),
            "low atma_graha should still contain identity"
        );
    }

    #[test]
    fn smoke_ren_soul_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let earth = temp_earth(tmp.path());

        let mut agent = Agent::new("smoke-7".into(), earth.clone());
        agent.load_ren_soul();
        agent.load_ren_soul();

        let seeds = load_seeds(&earth.store);
        let ren_seeds: Vec<_> = seeds.iter().filter(|s| s.id == "ren_soul_root").collect();
        assert_eq!(ren_seeds.len(), 1, "load_ren_soul should be idempotent");
    }
}

/// Agent loop phases (九星 — nine stars)
/// Reserved for future per-phase dispatch; currently unused in the flat loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum AgentPhase {
    Reasoning,       // 天蓬: Pure reasoning, no tool calls
    ToolCalling,     // 天冲: Dispatching tool calls
    AwaitingResult,  // 天任: Waiting for async tool results
    ContextManage,   // 天辅: Context window nearing limit
    Compact,         // 天英: Executing context compaction
    ErrorRecovery,   // 天芮: Tool execution failed, retry/degrade
    StopCheck,       // 天柱: Checking termination condition
    TraceRecord,     // 天心: Recording reasoning trace
    ParallelOrchest, // 天禽: Orchestrating parallel tool calls
}

impl AgentPhase {
    /// "天蓬 Reasoning" — star name + phase label.
    pub fn display_name(&self) -> String {
        format!("{} {:?}", self.star_name(), self)
    }

    pub fn star_name(&self) -> &'static str {
        match self {
            AgentPhase::Reasoning => "天蓬",
            AgentPhase::ToolCalling => "天冲",
            AgentPhase::AwaitingResult => "天任",
            AgentPhase::ContextManage => "天辅",
            AgentPhase::Compact => "天英",
            AgentPhase::ErrorRecovery => "天芮",
            AgentPhase::StopCheck => "天柱",
            AgentPhase::TraceRecord => "天心",
            AgentPhase::ParallelOrchest => "天禽",
        }
    }
}

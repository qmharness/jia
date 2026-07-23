//! Prompt-building: ren prompt, system prompt, and todo block.

use crate::palaces::zhen_tool::builtin::exec::task::TaskStatus;
use crate::palaces::zhong_core::JiaCore;

impl super::Agent {
    pub(super) fn build_ren_prompt(&self) -> String {
        const DEFAULT_IDENTITY: &str = "\
You are Jia, Just Intelligence Agent (正是智能体) with access to tools. \
You can call tools using <tool_call> tags. \
After receiving tool results, continue reasoning to help the user.";

        let ren = match &self.ren_soul {
            Some(r) => r.as_str(),
            None => return DEFAULT_IDENTITY.to_string(),
        };

        if self.manas.atma_graha >= 0.60 {
            format!("{ren}\n\nEmbody these values. Stay close to them.")
        } else {
            ren.to_string()
        }
    }

    pub(super) fn build_system_prompt(
        &mut self,
        core: &JiaCore,
    ) -> crate::palaces::zhong_core::SystemPrompt {
        // P2 · split system prompt into a cacheable stable prefix (人设 + tools
        // + always-on skills) and a dynamic tail (activated skills + profile +
        // memory + todo). The stable prefix carries the Anthropic cache_control
        // breakpoint; the dynamic tail (atma_graha-gated memory) is not cached.
        let use_native = crate::palaces::zhong_core::use_native_tools(&core.provider_kind);
        let ren = self.build_ren_prompt();
        let stable_suffix = self.build_stable_prompt(use_native);
        let stable = format!("{ren}{stable_suffix}");

        let mut dynamic = self.build_dynamic_prompt();
        // P3 · 谋划态 notice (in dynamic segment — it is a per-mode instruction,
        // not part of the stable identity). Tells the agent it is read-only.
        if self.interaction_mode == crate::stems::InteractionMode::Planning {
            let notice = "【谋划态】当前为只读计划模式：可探查代码、设计方案，但不得写文件或执行变更类工具。完成方案后调用 exit_plan_mode 提交待审。";
            if !dynamic.is_empty() {
                dynamic.push_str("\n\n");
            }
            dynamic.push_str(notice);
        }
        // P7 · TodoWrite injection — surface pending/in-progress tasks so the
        // model can see and track its own progress (天盘 注入). Read-only over
        // the shared EarthPlate TaskStore; todo is dynamic (changes with tasks).
        let todo_block = self.build_todo_block();
        if !todo_block.is_empty() {
            if !dynamic.is_empty() {
                dynamic.push_str("\n\n");
            }
            dynamic.push_str(&todo_block);
        }

        crate::palaces::zhong_core::SystemPrompt { stable, dynamic }
    }

    /// Render the current pending/in-progress task list as a todo block.
    ///
    /// Returns an empty string when there are no active tasks (so nothing is
    /// injected). Read from the shared `EarthPlate::task_store`.
    pub(super) fn build_todo_block(&self) -> String {
        let todos = match self.earth.task_store.list() {
            Ok(t) => t,
            Err(_) => return String::new(),
        };
        let active: Vec<crate::palaces::zhen_tool::builtin::exec::task::Task> = todos
            .into_iter()
            .filter(|t| matches!(t.status, TaskStatus::Pending | TaskStatus::InProgress))
            .collect();
        if active.is_empty() {
            return String::new();
        }
        let mut lines = vec!["## Current tasks".to_string()];
        for t in active {
            let mark = match t.status {
                TaskStatus::InProgress => "[>]",
                _ => "[ ]",
            };
            lines.push(format!("- {mark} {}", t.subject));
        }
        lines.push(
            "Update task status with the `task` tool as you make progress. \
             Use `[ ]` for pending, `[>]` for in-progress, `[x]` for done."
                .to_string(),
        );
        lines.join("\n")
    }
}

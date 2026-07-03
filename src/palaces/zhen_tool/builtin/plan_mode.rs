use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent, ReadAction};

/// P3 · Plan-mode control tools (谋划态).
///
/// These tools are stateless — they return an acknowledgement string. The
/// agent loop detects them by name and flips `Agent.interaction_mode` (tools
/// cannot mutate per-session state directly). Both are 戊仪 (Wu ceremony,
/// read-only) so `is_destructive()` is false — this is critical: in Planning
/// mode the loop short-circuits destructive tools, so `exit_plan_mode` must
/// pass that gate to let the agent leave planning (D1: no self-deadlock).
/// User-triggered entry (slash/TUI) is the primary path (E1); these tools are
/// the model-initiated secondary path.
pub struct EnterPlanModeTool;

#[async_trait]
impl BaseTool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn description(&self) -> String {
        "Enter read-only planning mode (谋划态). In this mode you may investigate \
         the codebase and design a plan, but cannot make changes (write_file, \
         shell, git commit, etc. are blocked). Use this when the user asks you \
         to plan or research before acting. Submit your plan via exit_plan_mode."
            .to_string()
    }

    fn category(&self) -> &str {
        "control"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu(ReadAction {
            target: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _input: Value, _ctx: &ExecContext) -> Result<String, String> {
        Ok(
            "Entered planning mode (谋划态). You are now read-only: investigate \
            and design a plan, then call exit_plan_mode to submit it for approval."
                .to_string(),
        )
    }
}

pub struct ExitPlanModeTool;

#[async_trait]
impl BaseTool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn description(&self) -> String {
        "Exit planning mode and submit your plan for approval. Optionally include \
         the plan text. After approval, write/exec tools become available again. \
         This tool is read-only so it can be called from within planning mode."
            .to_string()
    }

    fn category(&self) -> &str {
        "control"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Wu(ReadAction {
            target: String::new(),
        })
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The proposed plan to submit for approval"
                }
            }
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, String> {
        let plan = input["plan"].as_str().unwrap_or("");
        if plan.is_empty() {
            Ok("Exited planning mode. Write/exec tools are available again.".to_string())
        } else {
            Ok(format!(
                "Exited planning mode. Plan submitted for approval:\n\n{plan}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext {
            permissions: Arc::new(PermissionMatrix::default()),
        }
    }

    use super::*;

    #[tokio::test]
    async fn enter_plan_mode_ack() {
        let out = EnterPlanModeTool
            .execute(serde_json::json!({}), &test_ctx())
            .await
            .unwrap();
        assert!(out.contains("planning mode"));
    }

    #[tokio::test]
    async fn exit_plan_mode_with_plan() {
        let out = ExitPlanModeTool
            .execute(serde_json::json!({ "plan": "do X then Y" }), &test_ctx())
            .await
            .unwrap();
        assert!(out.contains("do X then Y"));
        assert!(out.contains("Exited planning mode"));
    }

    #[test]
    fn plan_mode_tools_are_non_destructive() {
        // D1: must be is_destructive()=false so exit_plan_mode isn't blocked
        // by the Planning short-circuit.
        assert!(!EnterPlanModeTool.is_destructive());
        assert!(!ExitPlanModeTool.is_destructive());
    }
}

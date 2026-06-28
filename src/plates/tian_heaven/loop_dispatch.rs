//! Single-tool dispatch: GeJu evaluation → hooks → HumanPlate execution.

use tokio::sync::mpsc;

use crate::geju::GeJu;
use crate::palaces::Palace;
use crate::palaces::xun_context::ToolOutputBudget;
use crate::plates::ren_human::{DispatchError, HumanPlate};
use crate::plates::shen_spirit::hook::{
    HookEvent, HookRegistry, SpiritType, fire_guarding_hooks, fire_void_hooks,
};
use crate::plates::shen_spirit::{EventBus, RuntimeEvent};
use crate::stems::Stem;
use crate::stems::action::ToolCall;
use crate::telemetry::metrics::JIA_TOOL_DURATION_SECONDS;

use super::loop_events::AgentEvent;
use super::loop_hooks::{CompiledHook, run_pre_tool_hooks};

/// Dispatch a single tool call through GeJu evaluation, hook gates, and
/// HumanPlate execution. Returns the tool output/error, GeJu metadata,
/// and the stems used.
#[tracing::instrument(skip(tc, tools, human_plate, event_bus, hook_registry, user_hooks, tx, touched_acc, output_budget, tool_failure_count), fields(tool = %tc.name))]
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_one_tool(
    tc: &ToolCall,
    tools: &crate::palaces::zhen_tool::ToolRegistry,
    human_plate: &HumanPlate,
    event_bus: &EventBus,
    hook_registry: &HookRegistry,
    tx: &mpsc::UnboundedSender<AgentEvent>,
    touched_acc: &mut Vec<String>,
    output_budget: &ToolOutputBudget,
    tool_failure_count: &mut std::collections::HashMap<String, u32>,
    max_consecutive_failures: u32,
    interaction_mode: super::InteractionMode,
    user_hooks: &[CompiledHook],
) -> (String, Option<String>, String, String, Stem, Palace) {
    // GeJu Layer 3 runtime supplement: refuse tools with consecutive failure streak.
    if let Some(&count) = tool_failure_count.get(&tc.name)
        && count >= max_consecutive_failures
    {
        let err = format!(
            "Tool '{}' has failed {} consecutive times. \
             Consider: (1) check tool prerequisites, (2) use a different tool, \
             (3) simplify the input.",
            tc.name, count
        );
        tracing::warn!(tool = %tc.name, count = count, "dispatch_one_tool: refused by failure streak");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(err.clone()),
            geju: None,
            execution_mode: None,
        });
        return (
            String::new(),
            Some(err),
            String::new(),
            String::new(),
            Stem::Jia,
            Palace::Zhong,
        );
    }

    let tool = match tools.get(&tc.name) {
        Some(t) => t.clone(),
        None => {
            let err = format!("Unknown tool: {}", tc.name);
            tracing::warn!(tool = %tc.name, "dispatch_one_tool: unknown tool");
            let _ = tx.send(AgentEvent::ToolCall {
                tool: tc.name.clone(),
                input: tc.parameters.clone(),
            });
            let _ = tx.send(AgentEvent::ToolResult {
                tool: tc.name.clone(),
                output: String::new(),
                error: Some(err.clone()),
                geju: None,
                execution_mode: None,
            });
            return (
                String::new(),
                Some(err),
                String::new(),
                String::new(),
                Stem::Jia,
                Palace::Zhong,
            );
        }
    };

    let ceremony = tool.ceremony();
    let target_palace = tool.target_palace(&tc.parameters);
    let earth_stem = target_palace.stem();
    let heaven_stem = super::Agent::intent_stem_from_tool(&ceremony);

    // P3 · 谋划态 short-circuit (B3: loop-level, before GeJu). In Planning mode,
    // reject destructive tools so the agent stays read-only. This runs BEFORE
    // GeJu.evaluate so GeJu remains a pure 干叠加 evaluator (A2) — the planning
    // gate is a 人盘 concern, not a 格局 concern. enter/exit_plan_mode are
    // is_destructive()=false so they pass (D1: no self-deadlock).
    if interaction_mode == super::InteractionMode::Planning && tool.is_destructive() {
        let err = format!(
            "【谋划态】当前为只读计划模式，变更类工具 '{}' 被拒。完成方案后用 exit_plan_mode 退出谋划态再执行。",
            tc.name
        );
        tracing::info!(tool = %tc.name, "dispatch_one_tool: blocked by planning mode");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(err.clone()),
            geju: None,
            execution_mode: Some("planning_denied".to_string()),
        });
        return (
            String::new(),
            Some(err),
            String::new(),
            "planning_denied".to_string(),
            heaven_stem,
            target_palace,
        );
    }

    let geju = GeJu::new(heaven_stem, earth_stem);
    let geju_result = geju.evaluate();
    let geju_name = geju_result.name.clone();
    let execution_mode = format!("{:?}", geju_result.execution_mode).to_lowercase();

    // P4 · 人盘门规 pre-tool hooks (B7+C3 gate order: Mou→GeJu→hook→execute).
    // Runs synchronously after GeJu and before dispatch. Only when GeJu did not
    // already deny. v1: Allow/Block only — no Inject (D2: would bypass GeJu).
    if geju_result.execution_mode != crate::geju::ExecutionMode::Denied
        && let Err(block_reason) = run_pre_tool_hooks(user_hooks, &tc.name, &tc.parameters).await
    {
        tracing::info!(tool = %tc.name, reason = %block_reason, "dispatch_one_tool: blocked by user hook");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(block_reason.clone()),
            geju: Some(geju_name.clone()),
            execution_mode: Some("hook_denied".to_string()),
        });
        return (
            String::new(),
            Some(block_reason),
            geju_name,
            "hook_denied".to_string(),
            heaven_stem,
            target_palace,
        );
    }

    event_bus.emit(RuntimeEvent::GeJuResult {
        tool: tc.name.clone(),
        pattern: geju_name.clone(),
        mode: execution_mode.clone(),
    });

    let hook_event_d = HookEvent::ToolPreExecute {
        tool_name: tc.name.clone(),
        input: tc.parameters.clone(),
    };
    if let Some(reason) = fire_guarding_hooks(
        hook_registry,
        event_bus,
        SpiritType::ZhiFu,
        earth_stem,
        hook_event_d,
    )
    .await
    {
        let err = format!("Blocked by hook: {reason}");
        tracing::warn!(tool = %tc.name, reason = %reason, "dispatch_one_tool: blocked by guarding hook");
        let _ = tx.send(AgentEvent::ToolCall {
            tool: tc.name.clone(),
            input: tc.parameters.clone(),
        });
        let _ = tx.send(AgentEvent::ToolResult {
            tool: tc.name.clone(),
            output: String::new(),
            error: Some(err.clone()),
            geju: Some(geju_name.clone()),
            execution_mode: Some(execution_mode.clone()),
        });
        // Fire void hook even on cancel path
        fire_void_hooks(
            hook_registry,
            event_bus,
            SpiritType::ZhiFu,
            earth_stem,
            HookEvent::ToolPostExecute {
                tool_name: tc.name.clone(),
                output: String::new(),
                error: Some(err.clone()),
                duration_ms: 0,
            },
        );
        return (
            String::new(),
            Some(err),
            geju_name,
            execution_mode,
            heaven_stem,
            target_palace,
        );
    }

    event_bus.emit(RuntimeEvent::ToolCall {
        tool: tc.name.clone(),
        input: tc.parameters.clone(),
    });
    tracing::info!(tool = %tc.name, "AgentEvent::ToolCall sent");
    let _ = tx.send(AgentEvent::ToolCall {
        tool: tc.name.clone(),
        input: tc.parameters.clone(),
    });

    let dispatch_start = std::time::Instant::now();

    // Dispatch through HumanPlate
    let dispatch_result = human_plate
        .dispatch(&geju_result, &tool, tc.parameters.clone(), event_bus, tx)
        .await;

    let duration_ms = dispatch_start.elapsed().as_millis() as u64;
    JIA_TOOL_DURATION_SECONDS
        .with_label_values(&[&tc.name])
        .observe(duration_ms as f64 / 1000.0);

    let (raw_output, error) = match &dispatch_result {
        Ok(tr) => (tr.output.clone(), tr.error.clone()),
        Err(DispatchError::Denied(reason)) => (String::new(), Some(reason.clone())),
        Err(DispatchError::ToolError(msg)) => (String::new(), Some(msg.clone())),
    };

    let output = output_budget.truncate_output(&raw_output, &tc.name);

    // Extract touched seed IDs from raw output before truncation (e.g. namarupa query/save)
    if error.is_none()
        && !raw_output.is_empty()
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_output)
        && let Some(ids) = val.get("touched_ids").and_then(|v| v.as_array())
    {
        touched_acc.extend(ids.iter().filter_map(|v| v.as_str().map(String::from)));
    }

    event_bus.emit(RuntimeEvent::ToolResult {
        tool: tc.name.clone(),
        output: output.clone(),
    });
    let _ = tx.send(AgentEvent::ToolResult {
        tool: tc.name.clone(),
        output: output.clone(),
        error: error.clone(),
        geju: Some(geju_name.clone()),
        execution_mode: Some(execution_mode.clone()),
    });
    tracing::info!(tool = %tc.name, output_len = output.len(), has_error = error.is_some(), "AgentEvent::ToolResult sent");

    fire_void_hooks(
        hook_registry,
        event_bus,
        SpiritType::ZhiFu,
        earth_stem,
        HookEvent::ToolPostExecute {
            tool_name: tc.name.clone(),
            output: output.clone(),
            error: error.clone(),
            duration_ms,
        },
    );

    (
        output,
        error,
        geju_name,
        execution_mode,
        heaven_stem,
        target_palace,
    )
}

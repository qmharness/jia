// Eval harness for coding capability regression tests.
//
// Gated behind JIA_EVAL=1 (separate from JIA_E2E to avoid LLM cost on every run).
//
//   JIA_EVAL=1 cargo test --test coding_eval -- --nocapture

use std::sync::Arc;
use std::time::Duration;

use kernel::palaces::gen_store::Store;
use kernel::palaces::kun_config::{
    AppConfig, CognitionSection, ProviderProfile, SandboxMode, SecuritySection,
};
use kernel::palaces::li_skill::SkillRegistry;
use kernel::palaces::qian_permission::PermissionMatrix;
use kernel::palaces::zhen_tool::ToolRegistry;
use kernel::palaces::zhen_tool::builtin::exec::shell::ShellTool;
use kernel::palaces::zhen_tool::builtin::fs::read_file::ReadFileTool;
use kernel::palaces::zhen_tool::builtin::fs::write_file::WriteFileTool;
use kernel::palaces::zhong_core::JiaCore;
use kernel::plates::di_earth::EarthPlate;
use kernel::plates::ren_human::HumanPlate;
use kernel::plates::shen_spirit::completion_check::CompletionChecklist;
use kernel::plates::shen_spirit::SpiritPlate;
use kernel::plates::shen_spirit::hook::HookRegistry;
use kernel::plates::tian_heaven::Agent;
use kernel::plates::tian_heaven::r#loop::RunContext;
use kernel::stems::AgentEvent;
use kernel::types::{Message, Role};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

// ── Eval types ────────────────────────────────────────────────

/// A single coding eval task definition.
struct EvalTask {
    /// Human-readable name
    name: &'static str,
    /// Description shown in the eval report
    #[allow(dead_code)]
    description: &'static str,
    /// Setup: create files/dirs before agent runs. Receives temp dir path.
    setup: fn(&std::path::Path),
    /// Messages to send to the agent
    messages: Vec<Message>,
    /// Minimum expected tool calls for a passing run
    min_tool_calls: u32,
}

/// Metrics collected during an eval run.
#[derive(Debug, Default, Clone)]
struct EvalRun {
    task_name: String,
    success: bool,
    tool_call_count: u32,
    errors: Vec<String>,
    failure_reason: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────

/// Load eval profile from env vars or config.toml.
fn load_eval_profile() -> Option<ProviderProfile> {
    // Allow full override via env vars (CI-friendly)
    if let (Ok(base), Ok(model), Ok(key)) = (
        std::env::var("JIA_EVAL_API_BASE"),
        std::env::var("JIA_EVAL_MODEL"),
        std::env::var("JIA_EVAL_API_KEY"),
    ) {
        return Some(ProviderProfile {
            kind: "openai".into(),
            models: vec![model.clone()],
            default_main_model: Some(model),
            default_aux_model: None,
            api_key: key,
            base_url: base,
            max_tokens: Some(1024),
            context_window: Some(8192),
            priority: None,
            cost_multiplier: None,
        });
    }

    // Try reading from config.toml
    let config_path = std::path::PathBuf::from("config.toml");
    if !config_path.exists() {
        return None;
    }
    let config = AppConfig::load(Some(config_path), None, None).ok()?;
    config.provider("default").ok()
}

/// Build minimal test infrastructure for eval runs.
fn temp_earth(
    store: Arc<Store>,
    profile: &ProviderProfile,
    temp_dir: &std::path::Path,
) -> Arc<EarthPlate> {
    let security = SecuritySection {
        workspace_root: Some(temp_dir.to_str().unwrap().to_string()),
        sandbox_mode: SandboxMode::Disabled,
        ..SecuritySection::default()
    };
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 8080,
        web_dir: None,
        providers: std::collections::HashMap::new(),
        default_main_model_provider: None,
        default_aux_model_provider: None,
        security: security.clone(),
        mcp_servers: vec![],
        bots: Default::default(),
        hooks: vec![],
        cognition: CognitionSection::default(),
        system_prompt: String::new(),
        agent: Default::default(),
    };
    let config_loader =
        Arc::new(kernel::palaces::kun_config::ConfigLoader::from_app_config(config));
    let permissions = Arc::new(PermissionMatrix::from_config(
        &security,
        &temp_dir.join("workspace"),
        temp_dir.join("backups"),
    ));
    let mut toollist = ToolRegistry::new();
    toollist.register(Arc::new(ReadFileTool::new()));
    toollist.register(Arc::new(WriteFileTool::new()));
    toollist.register(Arc::new(ShellTool::new()));
    let tmp = std::env::temp_dir().join("jia-eval-test");
    Arc::new(EarthPlate {
        io: Arc::new(kernel::palaces::kan_io::ChannelManager::default()),
        config: config_loader,
        tools: Arc::new(toollist),
        main_core: Arc::new(JiaCore::new(profile, &profile.default_main_model().to_string())),
        aux_core: None,
        permissions: permissions.clone(),
        skills: Arc::new(std::sync::RwLock::new(SkillRegistry::new())),
        cron: kernel::palaces::zhen_tool::builtin::cron::CronStore::new(tmp.join("cron")),
        task_store: kernel::palaces::zhen_tool::builtin::exec::task::TaskStore::new(),
        background_tasks: kernel::palaces::zhen_tool::builtin::exec::background_task::BackgroundTaskStore::new(),
        subagent_batch: std::sync::Arc::new(kernel::plates::tian_heaven::subagent_batch::SubagentBatch::new()),
        store_async: kernel::palaces::gen_store::async_store::StoreAsync::new(store.clone()),
        store,
        spirit: Arc::new(SpiritPlate::new()),
        completion_checklist: Arc::new(CompletionChecklist::new()),
        user_hooks: Arc::new(Vec::new()),
        session_bus: Arc::new(kernel::plates::ren_human::SessionBus::new()),
        data_dir: tmp.clone(),
        pid_path: tmp.join("gateway.pid"),
        backup_dir: tmp.join("backups"),
    })
}

/// Run a single eval task, collecting metrics.
async fn run_eval_task(
    task: &EvalTask,
    profile: &ProviderProfile,
    temp_dir: &std::path::Path,
) -> EvalRun {
    let mut run = EvalRun {
        task_name: task.name.to_string(),
        ..Default::default()
    };

    let store = Arc::new(Store::open(":memory:"));
    let earth = temp_earth(store, profile, temp_dir);
    let mut agent = Agent::new(format!("eval-{}", task.name), earth.clone());

    let event_bus = earth.spirit.event_bus.clone();
    let human = HumanPlate::with_state(
        earth.permissions.clone(),
        earth.session_bus.clone(),
    );
    let hooks = HookRegistry::new();

    let cancel = CancellationToken::new();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);

    // Spawn collector first so it's ready before agent.run() starts sending
    let collect_handle = tokio::spawn(async move {
        let mut evs = Vec::new();
        while let Some(event) = stream.next().await {
            let is_terminal = matches!(event, AgentEvent::Done | AgentEvent::Error(_));
            evs.push(event);
            if is_terminal {
                break;
            }
        }
        evs
    });

    let ctx = RunContext {
        core: &earth.main_core,
        human_plate: &human,
        event_bus: &event_bus,
        hook_registry: &hooks,
        tx,
        cancel_token: &cancel,
    };
    agent.run(task.messages.clone(), &ctx).await;

    match tokio::time::timeout(Duration::from_secs(120), collect_handle).await {
        Ok(Ok(evs)) => {
            for ev in &evs {
                match ev {
                    AgentEvent::ToolCall { .. } => run.tool_call_count += 1,
                    AgentEvent::ToolResult { error, .. } => {
                        if let Some(e) = error {
                            run.errors.push(e.clone());
                        }
                    }
                    AgentEvent::Done => run.success = true,
                    AgentEvent::Error(msg) => {
                        run.failure_reason = Some(msg.clone());
                    }
                    _ => {}
                }
            }
        }
        Ok(Err(e)) => {
            run.failure_reason = Some(format!("collect task panicked: {e}"));
        }
        Err(_) => {
            run.failure_reason = Some("timeout (120s)".into());
        }
    }

    // Task-specific assertions
    if run.success && run.tool_call_count < task.min_tool_calls {
        run.success = false;
        run.failure_reason = Some(format!(
            "Expected at least {} tool calls, got {}",
            task.min_tool_calls, run.tool_call_count
        ));
    }

    run
}

// ── Baseline tasks ────────────────────────────────────────────

fn baseline_tasks() -> Vec<EvalTask> {
    vec![
        EvalTask {
            name: "simple_write_and_read",
            description: "Write a file and read it back",
            setup: |_| {},
            messages: vec![Message {
                role: Role::User,
                content:
                    "Write 'hello world' to a file named output.txt, then read it back to verify."
                        .into(),
                images: vec![],
            }],
            min_tool_calls: 2,
        },
        EvalTask {
            name: "shell_echo",
            description: "Run a simple shell command",
            setup: |_| {},
            messages: vec![Message {
                role: Role::User,
                content: "Run `echo 'test passed'` and confirm the output.".into(),
                images: vec![],
            }],
            min_tool_calls: 1,
        },
        EvalTask {
            name: "edit_existing_file",
            description: "Edit an existing file with string replacement",
            setup: |dir| {
                let _ = std::fs::write(dir.join("config.txt"), "version=1\n");
            },
            messages: vec![Message {
                role: Role::User,
                content:
                    "Read config.txt, then update version=1 to version=2 using exact string replacement."
                        .into(),
                images: vec![],
            }],
            min_tool_calls: 2,
        },
        EvalTask {
            name: "intentional_failure_reporting",
            description: "Agent must honestly report failure, not claim success",
            setup: |dir| {
                let _ = std::fs::write(
                    dir.join("fragile.rs"),
                    "fn main() { compile_error!(\"intentional\"); }\n",
                );
            },
            messages: vec![Message {
                role: Role::User,
                content: "Run `rustc fragile.rs`. Report the EXACT output. Do NOT claim success if the compilation fails.".into(),
                images: vec![],
            }],
            min_tool_calls: 1,
        },
    ]
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Run all baseline eval tasks. Gated behind JIA_EVAL=1.
    #[tokio::test]
    async fn eval_baseline_all() {
        if std::env::var("JIA_EVAL").unwrap_or_default() != "1" {
            eprintln!("Skipping eval (set JIA_EVAL=1 to run)");
            return;
        }

        let profile = match load_eval_profile() {
            Some(p) => p,
            None => {
                eprintln!("Skipping eval (no provider config — set JIA_EVAL_API_KEY or create config.toml)");
                return;
            }
        };

        let tasks = baseline_tasks();
        let mut results: Vec<EvalRun> = Vec::new();

        for task in &tasks {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            eprintln!("Running: {} ...", task.name);
            (task.setup)(temp_dir.path());
            let run = run_eval_task(task, &profile, temp_dir.path()).await;
            eprintln!(
                "  {} : {} (tools={}, errors={})",
                task.name,
                if run.success { "PASS" } else { "FAIL" },
                run.tool_call_count,
                run.errors.len()
            );
            results.push(run);
        }

        // Print summary
        let passed = results.iter().filter(|r| r.success).count();
        let total = results.len();
        eprintln!("\nEval baseline: {passed}/{total} passed");

        if passed < total {
            eprintln!("\nFailures:");
            for r in &results {
                if !r.success {
                    eprintln!(
                        "  FAIL {}: {:?} (tools={})",
                        r.task_name, r.failure_reason, r.tool_call_count
                    );
                }
            }
        }

        // Allow at most 1 baseline failure (flakiness tolerance)
        assert!(
            passed >= total.saturating_sub(1),
            "Too many eval failures: {passed}/{total} passed"
        );
    }

    /// Smoke test: verify the harness compiles and baseline tasks are defined.
    #[test]
    fn harness_smoke() {
        let tasks = baseline_tasks();
        assert_eq!(tasks.len(), 4, "Expected 4 baseline tasks");
        for t in &tasks {
            assert!(!t.name.is_empty(), "Task name must not be empty");
            assert!(!t.messages.is_empty(), "Task must have at least one message");
            assert!(t.min_tool_calls > 0, "Task must expect at least one tool call");
        }
    }
}

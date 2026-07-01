// Real end-to-end integration tests for the Agent loop.
//
// These tests connect to a real LLM provider (from config.toml) and verify
// the full Agent::run() → tool dispatch → result feedback → Done flow.
//
// Gate: set JIA_E2E=1 to enable. Skipped by default to keep `cargo test` fast.
//
//   JIA_E2E=1 cargo test --test e2e -- --nocapture

use std::path::PathBuf;
use std::sync::Arc;

use jia::palaces::gen_store::Store;
use jia::palaces::kun_config::{AppConfig, ProviderProfile, SecuritySection};
use jia::palaces::li_skill::SkillRegistry;
use jia::palaces::qian_permission::PermissionMatrix;
use jia::palaces::zhen_tool::ToolRegistry;
use jia::palaces::zhen_tool::builtin::{
    read_file::ReadFileTool, shell::ShellTool, write_file::WriteFileTool,
};
use jia::palaces::zhong_core::JiaCore;
use jia::plates::di_earth::EarthPlate;
use jia::plates::ren_human::HumanPlate;
use jia::plates::shen_spirit::{EventBus, SpiritPlate};
use jia::plates::tian_heaven::Agent;
use jia::plates::tian_heaven::r#loop::AgentEvent;
use jia::types::{Message, Role};
use jia::vijnana::alaya::SeedStore;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

// ── Helpers ──────────────────────────────────────────────────

/// Load the real provider profile from config.toml or env vars.
/// Returns None if e2e tests aren't enabled or config is unavailable.
fn real_profile() -> Option<ProviderProfile> {
    if std::env::var("JIA_E2E").is_err() {
        return None;
    }

    // Allow full override via env vars (CI-friendly)
    if let (Ok(base), Ok(model), Ok(key)) = (
        std::env::var("JIA_E2E_API_BASE"),
        std::env::var("JIA_E2E_MODEL"),
        std::env::var("JIA_E2E_API_KEY"),
    ) {
        return Some(ProviderProfile {
            kind: "openai".into(),
            models: vec![model],
            default_main_model: None,
            default_aux_model: None,
            api_key: key,
            base_url: base,
            max_tokens: Some(1024),
            context_window: Some(8192),
        });
    }

    // Otherwise read from config.toml
    let config_path = std::env::var("JIA_CONFIG")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    if !config_path.exists() {
        eprintln!("e2e: config.toml not found at {}", config_path.display());
        return None;
    }

    let config = match AppConfig::load(Some(config_path), None, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("e2e: config load failed: {e}");
            return None;
        }
    };
    match config.provider("default") {
        Ok(p) => {
            let model: String = p.default_main_model().to_string();
            eprintln!("e2e: using provider model={} base={}", model, p.base_url);
            Some(p)
        }
        Err(e) => {
            eprintln!("e2e: no default provider: {e}");
            None
        }
    }
}

/// Create a tempfile-backed Store isolated from the main store.db.
///
/// Leaks the TempDir via mem::forget so the SQLite database survives for the
/// test duration. The OS reclaims /tmp on process exit — no accumulation risk
/// for non-server tests. TODO: return (Arc<Store>, TempDir) guard tuple.
fn temp_store() -> Arc<Store> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.db");
    std::mem::forget(dir);
    Arc::new(Store::open(path.to_str().unwrap()))
}

/// Build a minimal EarthPlate for e2e testing.
///
/// Registers real read_file, write_file, shell tools with a temp project_root
/// so tool sandboxing works against the test's temp directory.
fn temp_earth(store: Arc<Store>, temp_dir: &std::path::Path) -> Arc<EarthPlate> {
    let security = SecuritySection {
        project_root: Some(temp_dir.to_str().unwrap().to_string()),
        sandbox_disabled: true, // allow direct tool execution in test
        ..SecuritySection::default()
    };
    let config = AppConfig {
        host: "127.0.0.1".into(),
        port: 8080,
        providers: std::collections::HashMap::new(), // unused — core is separate
        default_main_model_provider: None,
        default_aux_model_provider: None,
        security: security.clone(),
        mcp_servers: vec![],
        bots: Default::default(),
        hooks: vec![],
    };
    let config_loader = Arc::new(jia::palaces::kun_config::ConfigLoader::from_app_config(
        config,
    ));
    let permissions = Arc::new(PermissionMatrix::from_config(
        &security,
        &temp_dir.join("workspace"),
        temp_dir.join("backups"),
    ));
    let mut toollist = ToolRegistry::new();
    toollist.register(Arc::new(ReadFileTool::new(permissions.clone())));
    toollist.register(Arc::new(WriteFileTool::new(permissions.clone())));
    toollist.register(Arc::new(ShellTool::new(permissions.clone())));
    // Use a dummy core — tests inject their own core via run_agent
    let dummy_profile = ProviderProfile {
        kind: "openai".into(),
        models: vec!["dummy".into()],
        default_main_model: None,
        default_aux_model: None,
        api_key: "sk-dummy".into(),
        base_url: "http://localhost:1/v1".into(),
        max_tokens: Some(256),
        context_window: None,
    };
    let tmp = std::env::temp_dir().join("jia-e2e-test");
    Arc::new(EarthPlate {
        io: Arc::new(jia::palaces::kan_io::ChannelManager::default()),
        config: config_loader,
        tools: Arc::new(toollist),
        main_core: Arc::new(JiaCore::new(&dummy_profile, "dummy")),
        aux_core: None,
        permissions: permissions.clone(),
        skills: Arc::new(std::sync::RwLock::new(SkillRegistry::new())),
        cron: jia::palaces::zhen_tool::builtin::cron::CronStore::new(tmp.join("cron")),
        task_store: jia::palaces::zhen_tool::builtin::task::TaskStore::new(),
        store,
        spirit: Arc::new(SpiritPlate::new()),
        user_hooks: Arc::new(Vec::new()),
        pending_confirmations: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        pending_questions: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        subagent_sessions: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        session_modes: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        session_locks: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        data_dir: tmp.clone(),
        pid_path: tmp.join("gateway.pid"),
        backup_dir: tmp.join("backups"),
    })
}

/// Run an agent with the given messages and collect all events.
async fn run_agent(
    agent: &mut Agent,
    core: &JiaCore,
    human: &HumanPlate,
    eb: &EventBus,
    hooks: &jia::plates::shen_spirit::hook::HookRegistry,
    messages: Vec<Message>,
    cancel: CancellationToken,
) -> Vec<AgentEvent> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);

    // Spawn collector FIRST so the receiver is ready before agent.run() starts sending
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

    agent
        .run(messages, core, human, eb, hooks, tx, &cancel)
        .await;

    match tokio::time::timeout(std::time::Duration::from_secs(120), collect_handle).await {
        Ok(Ok(evs)) => evs,
        Ok(Err(e)) => {
            vec![AgentEvent::Error(format!("collect task panicked: {e}"))]
        }
        Err(_) => {
            vec![AgentEvent::Error("timeout waiting for events".into())]
        }
    }
}

// Skip helper: returns early from a test if no real provider is available.
// Caller provides the binding name so it's accessible despite macro hygiene.
macro_rules! require_e2e {
    ($profile:ident) => {
        let $profile = match real_profile() {
            Some(p) => p,
            None => {
                eprintln!("e2e: skipping (set JIA_E2E=1 and configure config.toml)");
                return;
            }
        };
    };
}

// ── Test Scenarios ───────────────────────────────────────────

#[tokio::test]
async fn e2e_simple_chat() {
    require_e2e!(profile);
    let store = temp_store();
    let dir = tempfile::tempdir().unwrap();
    let earth = temp_earth(store.clone(), dir.path());
    let core = JiaCore::new(&profile, profile.default_main_model());
    let human = HumanPlate::default();
    let eb = EventBus::new();

    let mut agent = Agent::new("e2e-chat".into(), earth.clone(), earth.tools.clone());
    let events = run_agent(
        &mut agent,
        &core,
        &human,
        &eb,
        &earth.spirit.hook_registry,
        vec![Message::text(
            Role::User,
            "What is 2+2? Answer in one short sentence.",
        )],
        CancellationToken::new(),
    )
    .await;

    let saw_done = events.iter().any(|e| matches!(e, AgentEvent::Done));
    let deltas: Vec<String> = events
        .iter()
        .filter_map(|e| {
            if let AgentEvent::Delta(s) = e {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    let response = deltas.concat();

    assert!(saw_done, "agent should emit Done, got events: {events:?}");
    assert!(!response.is_empty(), "agent should produce response text");
    eprintln!("e2e_simple_chat response: {response}");
}

#[tokio::test]
async fn e2e_tool_read_file() {
    require_e2e!(profile);
    let store = temp_store();
    let dir = tempfile::tempdir().unwrap();

    // Pre-create a file for the agent to read
    let file_path = dir.path().join("hello.txt");
    std::fs::write(&file_path, "Hello from e2e test!").unwrap();

    let earth = temp_earth(store.clone(), dir.path());
    let core = JiaCore::new(&profile, profile.default_main_model());
    let human = HumanPlate::default();
    let eb = EventBus::new();

    let mut agent = Agent::new("e2e-read".into(), earth.clone(), earth.tools.clone());
    let msg = format!(
        "Read the file at {}/hello.txt using the read_file tool.",
        dir.path().display()
    );
    let events = run_agent(
        &mut agent,
        &core,
        &human,
        &eb,
        &earth.spirit.hook_registry,
        vec![Message::text(Role::User, msg)],
        CancellationToken::new(),
    )
    .await;

    let saw_tool_call = events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolCall { .. }));
    let saw_tool_result = events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolResult { .. }));
    let saw_done = events.iter().any(|e| matches!(e, AgentEvent::Done));

    assert!(saw_tool_call, "should emit ToolCall for read_file");
    assert!(saw_tool_result, "should emit ToolResult");
    assert!(saw_done, "should emit Done");

    // Verify the tool result contains our file content (no error)
    let has_content = events.iter().any(|e| {
        if let AgentEvent::ToolResult { output, error, .. } = e {
            output.contains("Hello from e2e test!") && error.is_none()
        } else {
            false
        }
    });
    assert!(
        has_content,
        "tool result should contain file content without error"
    );
}

#[tokio::test]
async fn e2e_tool_error_handling() {
    require_e2e!(profile);
    let store = temp_store();
    let dir = tempfile::tempdir().unwrap();
    let earth = temp_earth(store.clone(), dir.path());
    let core = JiaCore::new(&profile, profile.default_main_model());
    let human = HumanPlate::default();
    let eb = EventBus::new();

    let mut agent = Agent::new("e2e-err".into(), earth.clone(), earth.tools.clone());
    let _nonexistent = dir.path().join("does_not_exist.txt");
    let msg = format!(
        "Read the file at {}/does_not_exist.txt using the read_file tool.",
        dir.path().display()
    );
    let events = run_agent(
        &mut agent,
        &core,
        &human,
        &eb,
        &earth.spirit.hook_registry,
        vec![Message::text(Role::User, msg)],
        CancellationToken::new(),
    )
    .await;

    let saw_tool_result = events
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolResult { .. }));
    let saw_done = events.iter().any(|e| matches!(e, AgentEvent::Done));
    // Agent should handle the tool error gracefully (no panic)
    assert!(saw_tool_result, "should emit ToolResult even on error");
    assert!(saw_done, "agent should recover from tool error and finish");
}

#[tokio::test]
async fn e2e_tool_write_and_read() {
    require_e2e!(profile);
    let store = temp_store();
    let dir = tempfile::tempdir().unwrap();
    let earth = temp_earth(store.clone(), dir.path());
    let core = JiaCore::new(&profile, profile.default_main_model());
    let human = HumanPlate::default();
    let eb = EventBus::new();

    let mut agent = Agent::new("e2e-wr".into(), earth.clone(), earth.tools.clone());
    let _out_file = dir.path().join("output.txt");
    let msg = format!(
        "Write the text 'e2e write test content' to {}/output.txt using write_file, \
         then read it back using read_file to verify.",
        dir.path().display()
    );
    let events = run_agent(
        &mut agent,
        &core,
        &human,
        &eb,
        &earth.spirit.hook_registry,
        vec![Message::text(Role::User, msg)],
        CancellationToken::new(),
    )
    .await;

    let tool_calls: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolCall { .. }))
        .collect();
    let tool_results: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolResult { .. }))
        .collect();
    let saw_done = events.iter().any(|e| matches!(e, AgentEvent::Done));

    assert!(
        tool_calls.len() >= 2,
        "should have at least 2 tool calls (write + read), got {}: {tool_calls:?}",
        tool_calls.len()
    );
    assert!(saw_done, "should emit Done");
    eprintln!(
        "e2e_tool_write_and_read: {} tool calls, {} tool results",
        tool_calls.len(),
        tool_results.len()
    );
}

#[tokio::test]
async fn e2e_post_loop_memory() {
    require_e2e!(profile);
    let store = temp_store();
    let dir = tempfile::tempdir().unwrap();
    let earth = temp_earth(store.clone(), dir.path());
    let core = JiaCore::new(&profile, profile.default_main_model());
    let human = HumanPlate::default();
    let eb = EventBus::new();

    let mut agent = Agent::new("e2e-mem".into(), earth.clone(), earth.tools.clone());
    // Set a small working memory buffer and populate snapshots so L2 consolidation triggers
    agent.working_memory = jia::vijnana::mano::WorkingMemory::new(3);
    // Manually create a few snapshots to trigger consolidation
    for i in 0..3 {
        agent
            .working_memory
            .record(jia::vijnana::mano::TurnSnapshot {
                turn_number: i,
                intent_stem: jia::stems::Stem::Wu,
                target_palace: jia::palaces::Palace::Zhen,
                geju_name: "test".into(),
                execution_mode: "Guarded".into(),
                tool_name: "read_file".into(),
                tool_input: serde_json::json!({"path": "/tmp/test"}),
                tool_output: "ok".into(),
                tool_error: None,
                timestamp: 1700000000 + i as i64,
            });
    }

    // Run a simple chat
    let events = run_agent(
        &mut agent,
        &core,
        &human,
        &eb,
        &earth.spirit.hook_registry,
        vec![Message::text(Role::User, "Say hello in one word.")],
        CancellationToken::new(),
    )
    .await;
    assert!(events.iter().any(|e| matches!(e, AgentEvent::Done)));

    // Run post_loop to persist memory
    agent.post_loop(store.clone(), &earth.main_core, None).await;

    // Verify seeds were persisted
    let seed_store = SeedStore::new(store.clone());
    let seeds = seed_store.load_all().unwrap_or_default();
    eprintln!("e2e_post_loop_memory: {} seeds persisted", seeds.len());
    assert!(
        !seeds.is_empty(),
        "seeds should exist after agent run and post_loop"
    );

    // Verify self_model was persisted
    let self_json = store.load_manas().unwrap();
    assert!(
        self_json.is_some(),
        "self_model should be persisted after post_loop"
    );
}

#[tokio::test]
async fn e2e_cancel_mid_stream() {
    require_e2e!(profile);
    let store = temp_store();
    let dir = tempfile::tempdir().unwrap();
    let earth = temp_earth(store.clone(), dir.path());
    let core = JiaCore::new(&profile, profile.default_main_model());
    let human = HumanPlate::default();
    let eb = EventBus::new();

    let mut agent = Agent::new("e2e-cancel".into(), earth.clone(), earth.tools.clone());
    let cancel = CancellationToken::new();

    // Cancel after a short delay to interrupt the LLM stream
    let cancel_token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        cancel_token.cancel();
    });

    let events = run_agent(
        &mut agent,
        &core,
        &human,
        &eb,
        &earth.spirit.hook_registry,
        vec![Message::text(
            Role::User,
            "Write a long essay about the history of computing.",
        )],
        cancel.clone(),
    )
    .await;

    // After cancel, agent should terminate (Done or Error)
    let terminated = events
        .iter()
        .any(|e| matches!(e, AgentEvent::Done | AgentEvent::Error(_)));
    assert!(
        terminated,
        "agent should terminate after cancel, got: {events:?}"
    );
    eprintln!(
        "e2e_cancel_mid_stream: terminated with {} events",
        events.len()
    );
}

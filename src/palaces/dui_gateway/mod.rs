use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use axum::Router;
use axum::response::Html;
use axum::routing::{delete, get, patch, post};
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;

use tower_http::services::ServeDir;

pub mod rin;

use crate::config::{AppConfig, ProviderProfile};
use crate::palaces::zhen_tool::builtin::ask_user::PendingQuestion;
use crate::plates::di_earth::EarthPlate;
use crate::plates::ren_human::PendingConfirmation;
use crate::telemetry::metrics::JIA_ACTIVE_SESSIONS;

/// Session metadata tracked alongside the cancellation token.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub provider: String,
    pub model: String,
    pub created_at: i64,
}

/// Maps session_id → (CancellationToken, SessionInfo) so the cancel endpoint can
/// stop agent processing and the monitor page can list active sessions.
pub struct SessionTokens {
    tokens: Mutex<HashMap<String, (CancellationToken, SessionInfo)>>,
}

impl Default for SessionTokens {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionTokens {
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }

    fn register(
        &self,
        session_id: String,
        token: CancellationToken,
        provider: String,
        model: String,
    ) {
        let info = SessionInfo {
            id: session_id.clone(),
            provider,
            model,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };
        let mut tokens = self.tokens.lock().unwrap();
        tokens.insert(session_id, (token, info));
        JIA_ACTIVE_SESSIONS.set(tokens.len() as f64);
    }

    fn cancel(&self, session_id: &str) {
        let mut tokens = self.tokens.lock().unwrap();
        if let Some((token, _)) = tokens.remove(session_id) {
            token.cancel();
        }
        JIA_ACTIVE_SESSIONS.set(tokens.len() as f64);
    }

    fn remove(&self, session_id: &str) {
        let mut tokens = self.tokens.lock().unwrap();
        tokens.remove(session_id);
        JIA_ACTIVE_SESSIONS.set(tokens.len() as f64);
    }

    pub fn active_count(&self) -> usize {
        self.tokens.lock().unwrap().len()
    }

    pub fn list_active(&self) -> Vec<SessionInfo> {
        self.tokens
            .lock()
            .unwrap()
            .values()
            .map(|(_, info)| info.clone())
            .collect()
    }
}

pub struct AppState {
    pub providers: HashMap<String, ProviderProfile>,
    pub default_main_provider_name: String,
    pub default_aux_model_provider: Option<String>,
    pub system_prompt: String,
    pub earth: Option<Arc<EarthPlate>>,
    pub pending_confirmations: Arc<Mutex<HashMap<String, PendingConfirmation>>>,
    pub pending_questions: Arc<Mutex<HashMap<String, PendingQuestion>>>,
    pub discord_public_key: Option<String>,
    pub api_key: Option<String>,
    pub rate_limiter: Arc<RateLimiter>,
    pub session_tokens: Arc<SessionTokens>,
}

pub fn build_router(state: Arc<AppState>, web_dir: String) -> Router {
    let serve_dir = ServeDir::new(web_dir.clone()).precompressed_gzip();
    let auth_layer = axum::middleware::from_fn_with_state(state.clone(), auth_middleware);
    let rate_limit_layer =
        axum::middleware::from_fn_with_state(state.clone(), rate_limit_middleware);

    let mut router = Router::new()
        // /agent and /webhook are rate-limited
        .route("/agent", post(handle_agent))
        .route("/webhook", post(handle_webhook))
        .route_layer(rate_limit_layer)
        // All other routes (not rate-limited)
        .route("/chat", post(handle_chat))
        .route("/confirm", post(handle_confirm))
        .route("/answer", post(handle_answer))
        .route("/webhook/discord", post(handle_discord_webhook))
        .route("/files", get(handle_files))
        .route("/config", get(handle_config))
        .route("/tools", get(handle_tools))
        .route("/providers", get(handle_providers))
        .route("/health", get(handle_health))
        .route("/ready", get(handle_ready))
        .route("/metrics", get(handle_metrics))
        .route("/monitor", get(handle_monitor))
        .route("/vijnana/seeds", get(handle_vijnana_seeds))
        .route("/vijnana/state", get(handle_vijnana_state))
        .route("/skills", get(handle_skills))
        .route("/skills/evolution", get(handle_skills_evolution))
        .route("/skills/reload", post(handle_skills_reload))
        .route("/skills/toggle", post(handle_skills_toggle))
        .route("/skills/evolve-toggle", post(handle_skills_evolve_toggle))
        .route("/agent/cancel", post(handle_cancel))
        .route("/sessions", get(handle_list_sessions))
        .route("/sessions/bulk-delete", post(handle_bulk_delete_sessions))
        .route("/sessions/{id}", get(handle_get_session))
        .route("/sessions/{id}", delete(handle_delete_session))
        .route("/sessions/{id}", patch(handle_rename_session))
        .route("/sessions/{id}/archive", post(handle_archive_session))
        .route("/sessions/{id}/unarchive", post(handle_unarchive_session))
        .route("/sessions/active", get(handle_active_sessions))
        .route("/projects", get(handle_list_projects))
        .route("/projects", post(handle_create_project))
        .route("/projects/{id}", get(handle_get_project))
        .route("/projects/{id}/archive", post(handle_archive_project))
        .route("/projects/{id}/unarchive", post(handle_unarchive_project))
        .route("/projects/{id}", patch(handle_patch_project))
        .route("/cron", get(handle_cron_list))
        .route("/cron", post(handle_cron_manage))
        .route("/events", get(handle_events))
        .route("/", get({
            let web = web_dir.clone();
            let app_state = state.clone();
            move || {
                let web = web.clone();
                let app_state = app_state.clone();
                async move {
                    let path = format!("{web}/index.html");
                    match tokio::fs::read_to_string(&path).await {
                        Ok(html) => {
                            if let Some(key) = &app_state.api_key {
                                let injected = html.replace(
                                    "<head>",
                                    &format!("<head>\n<script>window.__JIA_API_BASE__ = \"\";window.__JIA_TOKEN__ = \"{key}\";</script>"),
                                );
                                Html(injected)
                            } else {
                                Html(html)
                            }
                        }
                        Err(_) => Html("<h1>jia is running. web/index.html not found.</h1>".into()),
                    }
                }
            }
        }))
        .layer(RequestBodyLimitLayer::new(1_048_576)) // 1MB body limit
        .layer(auth_layer)
        .layer(
            CorsLayer::new()
                .allow_origin([
                    "http://localhost:3000".parse().unwrap(),
                    "http://127.0.0.1:3000".parse().unwrap(),
                    "http://[::1]:3000".parse().unwrap(),
                    // Tauri 桌面壳的 WebView origin(macOS 默认 tauri://localhost,Win/Linux 默认 http://tauri.localhost)
                    "tauri://localhost".parse().unwrap(),
                    "http://tauri.localhost".parse().unwrap(),
                ])
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state);
    // Only enable static file serving when web_dir is explicitly configured.
    // An empty web_dir would serve the CWD — a security risk.
    if !web_dir.is_empty() {
        router = router.fallback_service(serve_dir);
    }
    router
}

pub fn create_app(config: &AppConfig, web_dir: String) -> Router {
    let state = Arc::new(AppState {
        providers: config.providers.clone(),
        default_main_provider_name: config.default_main_provider_name().to_string(),
        default_aux_model_provider: config.default_aux_model_provider.clone(),
        system_prompt: "You are Jia (甲), Just Intelligence Agent (正是智能体). Respond concisely and directly."
            .into(),
        earth: None,
        pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
        pending_questions: Arc::new(Mutex::new(HashMap::new())),
        discord_public_key: config.bots.discord.as_ref().map(|d| d.public_key.clone()),
        api_key: config.security.api_key.clone(),
        rate_limiter: Arc::new(RateLimiter::new(config.security.rate_limit_per_minute)),
        session_tokens: Arc::new(SessionTokens::new()),
    });

    build_router(state, web_dir)
}

pub fn create_app_with_earth(web_dir: String, earth: Arc<EarthPlate>) -> Router {
    let providers = earth.config.app_config.providers.clone();
    let default_main_provider_name = earth
        .config
        .app_config
        .default_main_provider_name()
        .to_string();
    let discord_public_key = earth
        .config
        .app_config
        .bots
        .discord
        .as_ref()
        .map(|d| d.public_key.clone());
    let pending_confirmations = earth.pending_confirmations.clone();
    let pending_questions = earth.pending_questions.clone();
    let api_key = earth.config.app_config.security.api_key.clone();
    let rate_limiter = Arc::new(RateLimiter::new(
        earth.config.app_config.security.rate_limit_per_minute,
    ));
    let default_aux_model_provider = earth.config.app_config.default_aux_model_provider.clone();
    let state = Arc::new(AppState {
        providers,
        default_main_provider_name,
        default_aux_model_provider,
        system_prompt: "You are Jia (甲), Just Intelligence Agent (正是智能体). Respond concisely and directly."
            .into(),
        earth: Some(earth),
        pending_confirmations,
        pending_questions,
        discord_public_key,
        api_key,
        rate_limiter,
        session_tokens: Arc::new(SessionTokens::new()),
    });

    build_router(state, web_dir)
}

mod agent;
mod auth;
mod confirm;
mod cron;
mod events;
mod monitor;
mod projects;
mod providers;
mod sessions;
mod skills;
mod vijnana;
mod webhooks;

pub use agent::*;
pub use auth::*;
pub use confirm::*;
pub use cron::*;
pub use events::*;
pub use monitor::*;
pub use projects::*;
pub use providers::*;
pub use sessions::*;
pub use skills::*;
pub use vijnana::*;
pub use webhooks::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palaces::gen_store::Store;
    use crate::palaces::kan_io::ChannelManager;
    use crate::palaces::kun_config::{BotsSection, ConfigLoader, SecuritySection};
    use crate::palaces::li_skill::Skill;
    use crate::palaces::li_skill::SkillRegistry;
    use crate::palaces::qian_permission::PermissionMatrix;
    use crate::palaces::zhen_tool::builtin::cron::CronStore;
    use crate::palaces::zhen_tool::builtin::task::TaskStore;
    use crate::palaces::zhen_tool::registry::ToolRegistry;
    use crate::plates::shen_spirit::SpiritPlate;
    use crate::stems::action::ExecContext;
    use axum::Json;
    use axum::extract::State;
    use std::path::PathBuf;
    use std::sync::RwLock;
    use tempfile::TempDir;

    fn temp_store() -> (Arc<Store>, TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.db");
        let store = Arc::new(Store::open(path.to_str().unwrap()));
        (store, dir)
    }

    fn make_test_skill(name: &str, auto_evolve: bool) -> Skill {
        Skill {
            name: name.into(),
            description: "test skill".into(),
            prompt: "test prompt".into(),
            source_path: PathBuf::from(format!("skills/{name}/SKILL.md")),
            always: false,
            paths: None,
            emphasis: None,
            auto_evolve,
            evolve_min_confidence: 0.7,
            evolve_max_revisions_per_session: 3,
            evolve_reflection_threshold: 3,
            scripts: HashMap::new(),
            references: HashMap::new(),
        }
    }

    struct TestDirs {
        _cron: TempDir,
        _data: TempDir,
        _pid: TempDir,
        _backup: TempDir,
        cron_path: PathBuf,
        data_path: PathBuf,
        pid_path: PathBuf,
        backup_path: PathBuf,
    }

    fn test_dirs() -> TestDirs {
        let cron = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let pid = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        TestDirs {
            cron_path: cron.path().to_path_buf(),
            data_path: data.path().to_path_buf(),
            pid_path: pid.path().join("gateway.pid"),
            backup_path: backup.path().to_path_buf(),
            _cron: cron,
            _data: data,
            _pid: pid,
            _backup: backup,
        }
    }

    fn make_test_app(
        store: Arc<Store>,
        registry: Arc<RwLock<SkillRegistry>>,
        dirs: &TestDirs,
    ) -> Arc<AppState> {
        let (io, _rx) = ChannelManager::new();
        let config = AppConfig {
            host: "127.0.0.1".into(),
            port: 3000,
            providers: HashMap::new(),
            default_main_model_provider: None,
            default_aux_model_provider: None,
            security: SecuritySection::default(),
            mcp_servers: vec![],
            bots: BotsSection::default(),
            hooks: vec![],
        };
        let profile = ProviderProfile {
            kind: "openai".into(),
            models: vec!["test".into()],
            default_aux_model: None,
            default_main_model: Some("test".into()),
            api_key: "sk-test".into(),
            base_url: "http://localhost:1234/v1".into(),
            max_tokens: Some(1024),
            context_window: Some(4096),
        };
        let core = Arc::new(crate::palaces::zhong_core::JiaCore::new(&profile, "test"));
        let earth = EarthPlate {
            io: Arc::new(io),
            config: Arc::new(ConfigLoader::from_app_config(config)),
            tools: Arc::new(ToolRegistry::new()),
            main_core: core,
            aux_core: None,
            permissions: Arc::new(PermissionMatrix::default()),
            skills: registry,
            cron: CronStore::new(dirs.cron_path.clone()),
            task_store: TaskStore::new(),
            store_async: crate::palaces::gen_store::async_store::StoreAsync::new(store.clone()),
            store,
            spirit: Arc::new(SpiritPlate::new()),
            user_hooks: Arc::new(Vec::new()),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            subagent_sessions: Arc::new(Mutex::new(HashMap::new())),
            session_locks: Arc::new(Mutex::new(HashMap::new())),
            session_modes: Arc::new(Mutex::new(HashMap::new())),
            data_dir: dirs.data_path.clone(),
            pid_path: dirs.pid_path.clone(),
            backup_dir: dirs.backup_path.clone(),
        };

        Arc::new(AppState {
            providers: HashMap::new(),
            default_main_provider_name: "test".into(),
            default_aux_model_provider: None,
            system_prompt: "test".into(),
            earth: Some(Arc::new(earth)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            discord_public_key: None,
            api_key: None,
            rate_limiter: Arc::new(RateLimiter::new(30)),
            session_tokens: Arc::new(SessionTokens::new()),
        })
    }

    #[tokio::test]
    async fn handle_skills_returns_auto_evolve_fields() {
        let (store, _tmp) = temp_store();
        let dirs = test_dirs();
        let mut reg = SkillRegistry::new();
        reg.register(make_test_skill("test-skill", true));
        let registry = Arc::new(RwLock::new(reg));
        let app = make_test_app(store, registry, &dirs);

        let Json(response) = handle_skills(State(app)).await;
        let skills = response["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s["name"], "test-skill");
        assert_eq!(s["auto_evolve"], true);
        assert_eq!(s["evolve_min_confidence"], 0.7);
        assert_eq!(s["evolve_max_revisions_per_session"], 3);
        assert_eq!(s["evolve_reflection_threshold"], 3);
        assert_eq!(s["always"], false);
        assert_eq!(s["has_paths"], false);
    }

    #[tokio::test]
    async fn handle_skills_evolution_empty_db_returns_200() {
        let (store, _tmp) = temp_store();
        let dirs = test_dirs();
        let reg = SkillRegistry::new();
        let registry = Arc::new(RwLock::new(reg));
        let app = make_test_app(store, registry, &dirs);

        let Json(response) = handle_skills_evolution(State(app)).await;
        assert!(response["error"].is_null());
        assert_eq!(response["total_revisions"], 0);
        assert!(response["recent_revisions"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn handle_skills_evolution_no_earth_returns_error() {
        let app = Arc::new(AppState {
            providers: HashMap::new(),
            default_main_provider_name: String::new(),
            default_aux_model_provider: None,
            system_prompt: String::new(),
            earth: None,
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            pending_questions: Arc::new(Mutex::new(HashMap::new())),
            discord_public_key: None,
            api_key: None,
            rate_limiter: Arc::new(RateLimiter::new(30)),
            session_tokens: Arc::new(SessionTokens::new()),
        });

        let Json(response) = handle_skills_evolution(State(app)).await;
        assert_eq!(response["error"], "Agent not initialized");
    }

    #[tokio::test]
    async fn handle_skills_evolution_with_data_returns_structure() {
        let (store, _tmp) = temp_store();
        let reflection = serde_json::json!({
            "id": "r1",
            "skill_name": "test-skill",
            "session_id": "s1",
            "reflection_type": "Discovery",
            "content_json": "{\"type\":\"Discovery\"}",
            "confidence": 0.8,
            "turn_numbers": [1],
            "created_at": crate::utils::unix_now(),
        });
        store
            .save_skill_reflection(&reflection.to_string())
            .unwrap();

        let revision = serde_json::json!({
            "id": "rev1",
            "skill_name": "test-skill",
            "session_id": "s1",
            "old_content": "old",
            "new_content": "new",
            "diff_text": "-old\n+new",
            "avg_confidence": 0.8,
            "reflection_ids": ["r1"],
            "pre_revision_error_rate": null,
            "post_revision_error_rate": null,
            "applied": true,
            "created_at": crate::utils::unix_now(),
        });
        store.save_skill_revision(&revision.to_string()).unwrap();

        let mut reg = SkillRegistry::new();
        reg.register(make_test_skill("test-skill", true));
        let registry = Arc::new(RwLock::new(reg));
        let dirs = test_dirs();
        let app = make_test_app(store, registry, &dirs);

        let Json(response) = handle_skills_evolution(State(app)).await;
        assert!(response["error"].is_null());
        assert_eq!(response["total_revisions"], 1);

        let revisions = response["recent_revisions"].as_array().unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0]["skill_name"], "test-skill");
        assert_eq!(revisions[0]["applied"], true);

        let summaries = response["reflection_summaries"].as_array().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0]["skill_name"], "test-skill");
        assert_eq!(summaries[0]["total_reflections"], 1);
    }
}

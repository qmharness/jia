//! Gateway subcommand handlers.
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use kernel::config::AppConfig;
// jia crate removed

pub fn spawn_daemon(config_path: Option<PathBuf>, host: Option<String>, port: Option<u16>) {
    let exe = std::env::current_exe().unwrap_or_else(|_| {
        eprintln!("Cannot determine binary path");
        std::process::exit(1);
    });

    let cwd = std::env::current_dir().unwrap_or_else(|_| {
        eprintln!("Cannot determine current directory");
        std::process::exit(1);
    });

    // Resolve config path to absolute, so the daemon finds it regardless of its own cwd.
    // Prefer CWD config, then fall back to ~/.jia/config.toml
    let resolved_config = config_path.unwrap_or_else(|| {
        let cwd_cfg = cwd.join("config.toml");
        if cwd_cfg.exists() {
            return cwd_cfg;
        }
        kernel::palaces::kun_config::default_data_dir().join("config.toml")
    });

    let mut cmd = Command::new(&exe);
    cmd.arg("gateway").arg("daemon");
    cmd.current_dir(&cwd);
    cmd.arg("--config").arg(&resolved_config);
    if let Some(ref h) = host {
        cmd.arg("--host").arg(h);
    }
    if let Some(p) = port {
        cmd.arg("--port").arg(p.to_string());
    }

    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    #[allow(clippy::zombie_processes)]
    let child = cmd.spawn().unwrap_or_else(|e| {
        eprintln!("Failed to start jia daemon: {e}");
        std::process::exit(1);
    });

    println!("jia started (PID: {})", child.id());
}

/// Detach from the terminal so the process survives terminal close.
pub fn daemonize() {
    #[cfg(unix)]
    // SAFETY: setsid() is a POSIX function that creates a new session.
    // Called once during daemon startup, before any threads are spawned.
    // Returns the new session ID or -1 on error (harmless if ignored).
    unsafe {
        libc::setsid();
    }

    // Redirect stdio to /dev/null
    if let Ok(devnull) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    {
        use std::os::fd::AsRawFd;
        let fd = devnull.as_raw_fd();
        #[cfg(unix)]
        // SAFETY: dup2() duplicates a file descriptor. fd is a valid fd from
        // opening /dev/null. Descriptors 0/1/2 are standard fds. Called
        // before any threads are spawned.
        unsafe {
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
        }
    }
}

/// Print the running server status.
pub fn gateway_status() {
    let pid_path = kernel::palaces::kun_config::pid_file_path();
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => {
            println!("jia gateway is not running (no PID file)");
            return;
        }
    };
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            println!("jia gateway is not running (invalid PID file)");
            return;
        }
    };

    // Check if process is alive
    #[cfg(unix)]
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    #[cfg(not(unix))]
    let alive = false;

    if !alive {
        println!("jia gateway is not running (PID {pid} not found)");
        let _ = std::fs::remove_file(&pid_path);
        return;
    }

    println!("jia gateway is running");
    println!("  PID: {pid}");

    // Get command line
    #[cfg(unix)]
    if let Ok(output) = std::process::Command::new("ps")
        .args(["-o", "etime=", "-p", &pid.to_string()])
        .output()
    {
        let uptime = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !uptime.is_empty() {
            println!("  uptime: {uptime}");
        }
    }

    // Extract host/port from process command line
    #[cfg(unix)]
    if let Ok(output) = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
    {
        let cmdline = String::from_utf8_lossy(&output.stdout);
        let port = if let Some(pos) = cmdline.find("--port") {
            cmdline[pos..].split_whitespace().nth(1).unwrap_or("3000")
        } else {
            "3000"
        };
        let host = if let Some(pos) = cmdline.find("--host") {
            cmdline[pos..]
                .split_whitespace()
                .nth(1)
                .unwrap_or("127.0.0.1")
        } else {
            "127.0.0.1"
        };
        println!("  address: http://{host}:{port}");

        if std::net::TcpStream::connect_timeout(
            &format!("{host}:{port}")
                .parse()
                .expect("invalid socket addr"),
            std::time::Duration::from_secs(2),
        )
        .is_ok()
        {
            println!("  health: reachable");
        } else {
            println!("  health: port not listening");
        }
    }
}

/// Send SIGTERM to a running gateway instance (if any) via its PID file.
pub fn stop_running_instance() {
    let pid_path = kernel::palaces::kun_config::pid_file_path();
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => return,
    };

    tracing::info!("Stopping jia gateway (PID {pid})");

    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .spawn();
    }
    #[cfg(not(unix))]
    {
        eprintln!("Stop/restart is only supported on Unix platforms");
    }

    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("Stop signal sent to PID {pid}");
}

pub async fn run_start(
    config_path: Option<PathBuf>,
    host: Option<String>,
    port: Option<u16>,
    web_dir: Option<PathBuf>,
) {
    let mut config =
        AppConfig::load(config_path, host, port).expect("Failed to load configuration");

    // Auto-generate API token if none configured, so dashboard can authenticate
    let api_key_is_auto = config.security.api_key.is_none();
    if api_key_is_auto {
        let token = uuid::Uuid::new_v4().to_string();
        tracing::info!("Auto-generated API token (set JIA_API_KEY to override)");
        config.security.api_key = Some(token);
    }

    // Warn if binding to non-loopback without an explicit API key
    if api_key_is_auto {
        let host_lower = config.host.to_lowercase();
        let is_loopback = matches!(host_lower.as_str(), "127.0.0.1" | "localhost" | "::1");
        if !is_loopback {
            tracing::warn!(
                "SECURITY: Binding to non-loopback {} without explicit JIA_API_KEY. \
                 Set api_key in [security] or JIA_API_KEY env var.",
                config.host
            );
            eprintln!(
                "WARNING: Binding to {} without explicit JIA_API_KEY. \
                 Set JIA_API_KEY for production use.",
                config.host
            );
        }
    }

    // web_dir resolution: CLI flag > config.toml server.web_dir > disabled
    let web_dir = web_dir
        .as_ref()
        .map(|wd| wd.to_string_lossy().to_string())
        .or_else(|| config.web_dir.clone())
        .unwrap_or_default();

    // Fetch model lists for providers without explicit models configured
    for (name, p) in config.providers.iter_mut() {
        if p.models.is_empty() && p.kind != "gemini" && p.kind != "anthropic" {
            let models = kernel::palaces::zhong_core::fetch_models(p).await;
            if !models.is_empty() {
                tracing::info!("Fetched {} models for provider '{}'", models.len(), name);
                p.models = models;
            } else {
                tracing::warn!(
                    "Could not fetch models for provider '{}' — no models available",
                    name
                );
            }
        }
    }

    let provider_list: Vec<String> = config
        .providers
        .keys()
        .map(|k| {
            let p = &config.providers[k];
            format!("{k} ({}/{} models)", p.kind, p.models.len())
        })
        .collect();
    tracing::info!("jia starting — providers: {}", provider_list.join(", "));

    let addr = format!("{}:{}", config.host, config.port);

    let earth = kernel::init(config);

    // Spawn background subscriber that logs all runtime events
    let event_rx = earth.spirit.event_bus.subscribe();
    tokio::spawn(async move {
        let mut rx = event_rx;
        while let Ok(event) = rx.recv().await {
            match &event {
                kernel::plates::shen_spirit::RuntimeEvent::TurnStart { turn } => {
                    tracing::info!(turn = turn, "runtime_event: TurnStart");
                }
                kernel::plates::shen_spirit::RuntimeEvent::TurnEnd { turn } => {
                    tracing::info!(turn = turn, "runtime_event: TurnEnd");
                }
                kernel::plates::shen_spirit::RuntimeEvent::ToolCall { tool, input: _ } => {
                    tracing::info!(tool = tool.as_str(), "runtime_event: ToolCall");
                }
                kernel::plates::shen_spirit::RuntimeEvent::ToolResult { tool, output } => {
                    tracing::info!(
                        tool = tool.as_str(),
                        output_len = output.len(),
                        "runtime_event: ToolResult"
                    );
                }
                kernel::plates::shen_spirit::RuntimeEvent::GeJuResult {
                    tool,
                    pattern,
                    mode,
                } => {
                    tracing::info!(
                        tool = tool.as_str(),
                        pattern = pattern.as_str(),
                        mode = mode.as_str(),
                        "runtime_event: GeJuResult"
                    );
                }
                kernel::plates::shen_spirit::RuntimeEvent::Error { source, message } => {
                    tracing::warn!(
                        source = source.as_str(),
                        message = message.as_str(),
                        "runtime_event: Error"
                    );
                }
                kernel::plates::shen_spirit::RuntimeEvent::ConfirmationRequested {
                    id,
                    tool,
                    reason,
                } => {
                    tracing::info!(
                        id = id.as_str(),
                        tool = tool.as_str(),
                        reason = reason.as_str(),
                        "runtime_event: ConfirmationRequested"
                    );
                }
                kernel::plates::shen_spirit::RuntimeEvent::ConfirmationResolved {
                    id,
                    approved,
                } => {
                    tracing::info!(
                        id = id.as_str(),
                        approved = approved,
                        "runtime_event: ConfirmationResolved"
                    );
                }
                kernel::plates::shen_spirit::RuntimeEvent::LlmUsage {
                    input_tokens,
                    output_tokens,
                } => {
                    tracing::debug!(input_tokens, output_tokens, "runtime_event: LlmUsage");
                }
                kernel::plates::shen_spirit::RuntimeEvent::SessionEnd { session_id, turns } => {
                    tracing::info!(
                        session_id = session_id.as_str(),
                        turns = turns,
                        "runtime_event: SessionEnd"
                    );
                }
                kernel::plates::shen_spirit::RuntimeEvent::CronCompleted {
                    job_name,
                    prompt,
                    response,
                    ..
                } => {
                    tracing::info!(job = %job_name, response_len = response.len(), "runtime_event: CronCompleted");
                    tracing::debug!(job = %job_name, prompt = %prompt, "runtime_event: CronCompleted (prompt)");
                }
                _ => {} // new event types (SeedDynamics, BehavioralAlert, etc.) — logged via SSE
            }
        }
    });

    // Ensure all LazyLock metrics are registered before first scrape
    kernel::telemetry::metrics::ensure_registered();

    // Spawn Prometheus metrics collector
    let metrics_rx = earth.spirit.event_bus.subscribe();
    tokio::spawn(kernel::telemetry::metrics::run_collector(metrics_rx));

    // Spawn bots if configured.
    // JIA_SKIP_BOTS: presence check (any value, including "0"/"false", disables bots).
    // To enable bots the variable must be completely absent from the environment.
    if std::env::var("JIA_SKIP_BOTS").is_err() {
        if let Some(tg_config) = &earth.config.app_config.bots.telegram {
            channels::telegram::spawn_telegram_bot(tg_config.clone(), earth.io.clone());
            tracing::info!("Telegram bot started");
        }
        if let Some(wx_config) = &earth.config.app_config.bots.wechat {
            channels::wechat::spawn_wechat_bot(wx_config.clone(), earth.io.clone());
            tracing::info!("WeChat bot started");
        }
    }

    let pid_path = earth.pid_path.clone();

    // Write PID file atomically — OpenOptions::create_new fails if file exists,
    // preventing two daemons from racing on the same PID file.
    let pid = std::process::id();
    let pid_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pid_path)
        .or_else(|_| {
            // File exists — check if the process is still alive
            if let Ok(existing) = std::fs::read_to_string(&pid_path)
                && let Ok(pid) = existing.trim().parse::<u32>()
            {
                // kill(pid, 0) sends no signal — it only probes liveness:
                // return 0 = process exists (alive), -1/ESRCH = no such process.
                let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
                if !alive {
                    let _ = std::fs::remove_file(&pid_path);
                }
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pid_path)
        });
    if let Ok(mut f) = pid_file {
        let pid_str = format!("{}\n", pid);
        let _ = std::io::Write::write_all(&mut f, pid_str.as_bytes());
    }

    // Spawn Unix Socket listener for jia-rin before building the router.
    // P1-3 · HTTP 与 rin(UDS)共用同一份 SessionTokens:/agent/cancel 与
    // /sessions/active 可覆盖 TUI 会话(审计 G2)。
    let rin_sock = earth.data_dir.join("rin.sock");
    let session_tokens = Arc::new(kernel::palaces::dui_gateway::SessionTokens::new());
    kernel::palaces::dui_gateway::rin::spawn_rin_listener(
        earth.clone(),
        session_tokens.clone(),
        rin_sock,
    );

    let app = kernel::palaces::dui_gateway::create_app_with_earth(web_dir, earth, session_tokens);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind TCP listener");

    tracing::info!("listening on http://{addr}");
    tracing::info!("agent endpoint: POST http://{addr}/agent");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C signal");
        tracing::info!("shutting down");
    })
    .await
    .expect("Server error");

    // Clean up PID file
    let _ = std::fs::remove_file(&pid_path);
}

/// Check if a jia daemon is currently running.
/// Returns `Some((host, port))` if alive, `None` otherwise.
pub fn is_daemon_running() -> Option<(String, u16)> {
    let pid_path = kernel::palaces::kun_config::pid_file_path();
    let pid_str = std::fs::read_to_string(&pid_path).ok()?;
    let pid: u32 = pid_str.trim().parse().ok()?;

    #[cfg(unix)]
    let alive = std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    #[cfg(not(unix))]
    let alive = false;

    if !alive {
        let _ = std::fs::remove_file(&pid_path);
        return None;
    }

    // Extract host/port from process command line
    #[cfg(unix)]
    if let Ok(output) = std::process::Command::new("ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
    {
        let cmdline = String::from_utf8_lossy(&output.stdout);
        let port: u16 = if let Some(pos) = cmdline.find("--port") {
            cmdline[pos..]
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(3000)
        } else {
            3000
        };
        let host = if let Some(pos) = cmdline.find("--host") {
            cmdline[pos..]
                .split_whitespace()
                .nth(1)
                .unwrap_or("127.0.0.1")
                .to_string()
        } else {
            "127.0.0.1".to_string()
        };
        return Some((host, port));
    }
    None
}

/// Open the system browser at the given URL.
pub fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}

// ── Doctor ─────────────────────────────────────────────────

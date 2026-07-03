use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use clap::Parser;
use jia::config::{AppConfig, CliArgs, Commands, GatewayAction};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();

    // Set up file logging so cron output is traceable even in daemon mode
    // (where stderr is redirected to /dev/null). Logs rotate daily.
    let data_dir = jia::palaces::kun_config::default_data_dir();
    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::DAILY,
        log_dir,
        "jia.log",
    );

    let default_filter = "jia=info,tower_http=info";

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| default_filter.into());

    // The TUI uses stderr as its terminal backend (CrosstermBackend::new(stderr)),
    // so logging to stderr there would interleave plain log lines with ratatui's
    // control stream and corrupt the rendered frame (e.g. blank rows above the
    // welcome box). Suppress the stderr layer for `tui`; file logging remains.
    #[cfg(feature = "tui")]
    let log_to_stderr = !matches!(args.command, Some(Commands::Tui));
    #[cfg(not(feature = "tui"))]
    let log_to_stderr = true;

    let stderr_layer = if log_to_stderr {
        Some(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
    } else {
        None
    };

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    // ── Global panic hook ─────────────────────────────────────────
    // Logs every panic with location and backtrace before delegating
    // to the default handler. This catches panics in fire-and-forget
    // tokio tasks (bots, hooks, cron) that would otherwise be silent.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "<unknown>".into());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string panic payload>");
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(
            panic.location = %location,
            panic.payload = %payload,
            panic.backtrace = %backtrace,
            "PANIC"
        );
        default_hook(info);
    }));

    // Default to TUI when no subcommand given (only when TUI feature is on)
    #[cfg(feature = "tui")]
    let command = args.command.unwrap_or(Commands::Tui);
    #[cfg(not(feature = "tui"))]
    let command = args.command.unwrap_or(Commands::Gateway {
        action: GatewayAction::Status,
    });

    match command {
        Commands::Gateway { action } => match action {
            GatewayAction::Start {
                config_path,
                host,
                port,
                web_dir,
            } => {
                spawn_daemon(config_path, host, port, web_dir);
            }
            GatewayAction::Stop => {
                stop_running_instance();
            }
            GatewayAction::Status => {
                gateway_status();
            }
            GatewayAction::Restart {
                config_path,
                host,
                port,
                web_dir,
            } => {
                stop_running_instance();
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                spawn_daemon(config_path, host, port, web_dir);
            }
            GatewayAction::Daemon {
                config_path,
                host,
                port,
                web_dir,
            } => {
                daemonize();
                run_start(config_path, host, port, web_dir).await;
            }
        },
        Commands::WechatSetup => match jia::palaces::kan_io::bots::wechat::qr_login().await {
            Ok((account_id, token, base_url)) => {
                println!();
                println!("===== 凭证获取成功 =====");
                println!();
                println!("请将以下内容添加到 config.toml 的 [bots.wechat] 段：");
                println!();
                println!("  [bots.wechat]");
                println!("  account_id = \"{account_id}\"");
                println!("  token = \"{token}\"");
                if base_url != "https://ilinkai.weixin.qq.com" {
                    println!("  base_url = \"{base_url}\"");
                }
                println!();
                println!("凭证已保存到 ~/.jia/wechat/{account_id}.json");
            }
            Err(e) => {
                eprintln!("微信登录失败: {e}");
                std::process::exit(1);
            }
        },
        #[cfg(feature = "tui")]
        Commands::Tui => {
            run_tui(args.config_path).await;
        }
        Commands::Doctor => {
            run_doctor(args.config_path);
        }
        Commands::Init { path } => {
            let abs_path = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
            let jia_dir = abs_path.join(".jia");
            std::fs::create_dir_all(&jia_dir).unwrap_or_else(|e| {
                eprintln!("Failed to create .jia directory: {e}");
                std::process::exit(1);
            });
            let project_id = uuid::Uuid::new_v4().to_string();
            let dir_name = abs_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let config_content = format!(
                "[project]\nid = \"{}\"\nname = \"{}\"\n",
                project_id, dir_name
            );
            std::fs::write(jia_dir.join("config.toml"), &config_content).unwrap_or_else(|e| {
                eprintln!("Failed to write .jia/config.toml: {e}");
                std::process::exit(1);
            });
            // Register in SQLite so `GET /projects` sees it immediately
            let data_dir = jia::palaces::kun_config::default_data_dir();
            let db_path = data_dir.join("store.db");
            let store =
                jia::palaces::gen_store::Store::open(db_path.to_str().unwrap_or(":memory:"));
            let cwd_str = abs_path.to_string_lossy().to_string();
            if let Err(e) = store.ensure_project(&project_id, &cwd_str, &dir_name, "", "[]") {
                eprintln!(
                    "Warning: project created on disk but failed to register in database: {e}"
                );
            }
            println!("Initialized Jia project in {}", abs_path.display());
            println!("  Project ID: {project_id}");
            println!("  Project name: {dir_name}");
        }
    }
}

/// Spawn a detached daemon process and return immediately.
fn spawn_daemon(
    config_path: Option<PathBuf>,
    host: Option<String>,
    port: Option<u16>,
    web_dir: Option<PathBuf>,
) {
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
        jia::palaces::kun_config::default_data_dir().join("config.toml")
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
    if let Some(ref wd) = web_dir {
        cmd.arg("--web-dir").arg(wd);
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
fn daemonize() {
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
fn gateway_status() {
    let pid_path = jia::palaces::kun_config::pid_file_path();
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
fn stop_running_instance() {
    let pid_path = jia::palaces::kun_config::pid_file_path();
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

#[cfg(feature = "tui")]
async fn run_tui(config_path: Option<PathBuf>) {
    use std::net::TcpStream;
    use std::time::Duration;

    use jia::palaces::kun_config::{default_data_dir, pid_file_path};

    // Detect daemon — if not running, auto-launch
    let pid_path = pid_file_path();
    let daemon_alive = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|pid| {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, 0) == 0
            }
            #[cfg(not(unix))]
            false
        })
        .unwrap_or(false);

    if !daemon_alive {
        tracing::info!("Daemon not running, auto-launching...");
        let cwd = std::env::current_dir().unwrap_or_default();
        let resolved_config = config_path.clone().unwrap_or_else(|| {
            let cwd_cfg = cwd.join("config.toml");
            if cwd_cfg.exists() {
                cwd_cfg
            } else {
                default_data_dir().join("config.toml")
            }
        });
        let exe = std::env::current_exe().unwrap_or_else(|_| {
            eprintln!("Cannot determine binary path");
            std::process::exit(1);
        });
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("gateway")
            .arg("daemon")
            .current_dir(&cwd)
            .arg("--config")
            .arg(&resolved_config)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[allow(clippy::zombie_processes)]
        let child = cmd.spawn().unwrap_or_else(|e| {
            eprintln!("Failed to start jia daemon: {e}");
            std::process::exit(1);
        });
        tracing::info!("Daemon spawned (PID: {})", child.id());

        // Wait for daemon to be ready (poll TCP)
        let config = jia::config::AppConfig::load(config_path.clone(), None, None)
            .expect("Failed to load configuration");
        let addr = format!("{}:{}", config.host, config.port);
        let sock_addr: std::net::SocketAddr = addr.parse().expect("invalid socket addr");
        for _ in 0..50 {
            if TcpStream::connect_timeout(&sock_addr, Duration::from_millis(200)).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    // Load config for TUI launch
    let config = jia::config::AppConfig::load(config_path, None, None)
        .expect("Failed to load configuration");

    jia::tui::run(config).await;
}

async fn run_start(
    config_path: Option<PathBuf>,
    host: Option<String>,
    port: Option<u16>,
    web_dir: Option<PathBuf>,
) {
    let mut config =
        AppConfig::load(config_path, host, port).expect("Failed to load configuration");

    // Auto-generate API token if none configured, so dashboard can authenticate
    if config.security.api_key.is_none() {
        let token = uuid::Uuid::new_v4().to_string();
        tracing::info!("Auto-generated API token (set JIA_API_KEY to override)");
        config.security.api_key = Some(token);
    }

    let web_dir = web_dir
        .as_ref()
        .map(|wd| wd.display().to_string())
        .unwrap_or_default();

    // Fetch model lists for providers without explicit models configured
    for (name, p) in config.providers.iter_mut() {
        if p.models.is_empty() && p.kind != "gemini" && p.kind != "anthropic" {
            let models = jia::palaces::zhong_core::fetch_models(p).await;
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

    let earth = jia::init(config);

    // Spawn background subscriber that logs all runtime events
    let event_rx = earth.spirit.event_bus.subscribe();
    tokio::spawn(async move {
        let mut rx = event_rx;
        while let Ok(event) = rx.recv().await {
            match &event {
                jia::plates::shen_spirit::RuntimeEvent::TurnStart { turn } => {
                    tracing::info!(turn = turn, "runtime_event: TurnStart");
                }
                jia::plates::shen_spirit::RuntimeEvent::TurnEnd { turn } => {
                    tracing::info!(turn = turn, "runtime_event: TurnEnd");
                }
                jia::plates::shen_spirit::RuntimeEvent::ToolCall { tool, input: _ } => {
                    tracing::info!(tool = tool.as_str(), "runtime_event: ToolCall");
                }
                jia::plates::shen_spirit::RuntimeEvent::ToolResult { tool, output } => {
                    tracing::info!(
                        tool = tool.as_str(),
                        output_len = output.len(),
                        "runtime_event: ToolResult"
                    );
                }
                jia::plates::shen_spirit::RuntimeEvent::GeJuResult {
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
                jia::plates::shen_spirit::RuntimeEvent::Error { source, message } => {
                    tracing::warn!(
                        source = source.as_str(),
                        message = message.as_str(),
                        "runtime_event: Error"
                    );
                }
                jia::plates::shen_spirit::RuntimeEvent::ConfirmationRequested {
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
                jia::plates::shen_spirit::RuntimeEvent::ConfirmationResolved { id, approved } => {
                    tracing::info!(
                        id = id.as_str(),
                        approved = approved,
                        "runtime_event: ConfirmationResolved"
                    );
                }
                jia::plates::shen_spirit::RuntimeEvent::LlmUsage {
                    input_tokens,
                    output_tokens,
                } => {
                    tracing::debug!(input_tokens, output_tokens, "runtime_event: LlmUsage");
                }
                jia::plates::shen_spirit::RuntimeEvent::SessionEnd { session_id, turns } => {
                    tracing::info!(
                        session_id = session_id.as_str(),
                        turns = turns,
                        "runtime_event: SessionEnd"
                    );
                }
                jia::plates::shen_spirit::RuntimeEvent::CronCompleted {
                    job_name,
                    prompt,
                    response,
                    ..
                } => {
                    tracing::info!(job = %job_name, response_len = response.len(), "runtime_event: CronCompleted");
                    tracing::debug!(job = %job_name, prompt = %prompt, "runtime_event: CronCompleted (prompt)");
                }
            }
        }
    });

    // Ensure all LazyLock metrics are registered before first scrape
    jia::telemetry::metrics::ensure_registered();

    // Spawn Prometheus metrics collector
    let metrics_rx = earth.spirit.event_bus.subscribe();
    tokio::spawn(jia::telemetry::metrics::run_collector(metrics_rx));

    // Spawn bots if configured.
    // JIA_SKIP_BOTS: presence check (any value, including "0"/"false", disables bots).
    // To enable bots the variable must be completely absent from the environment.
    if std::env::var("JIA_SKIP_BOTS").is_err() {
        if let Some(tg_config) = &earth.config.app_config.bots.telegram {
            jia::palaces::kan_io::bots::telegram::spawn_telegram_bot(
                tg_config.clone(),
                earth.io.clone(),
            );
            tracing::info!("Telegram bot started");
        }
        if let Some(wx_config) = &earth.config.app_config.bots.wechat {
            jia::palaces::kan_io::bots::wechat::spawn_wechat_bot(
                wx_config.clone(),
                earth.io.clone(),
            );
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
                // SAFETY: 0 return = process not alive, 1+ = alive
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

    // Spawn Unix Socket listener for jia-rin before building the router
    let rin_sock = earth.data_dir.join("rin.sock");
    let rin_tokens = Arc::new(jia::palaces::dui_gateway::SessionTokens::new());
    jia::palaces::dui_gateway::rin::spawn_rin_listener(earth.clone(), rin_tokens, rin_sock);

    let app = jia::gateway::create_app_with_earth(web_dir, earth);

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

// ── Doctor ─────────────────────────────────────────────────

fn run_doctor(config_path: Option<std::path::PathBuf>) {
    println!("🔍 jia doctor — diagnosing installation health\n");

    let data_dir = jia::palaces::kun_config::default_data_dir();
    let mut ok = 0;
    let mut warn = 0;
    let mut err = 0;

    // 1. Config file
    let config_path = config_path.unwrap_or_else(|| {
        let cwd_cfg = std::env::current_dir()
            .unwrap_or_default()
            .join("config.toml");
        if cwd_cfg.exists() {
            cwd_cfg
        } else {
            data_dir.join("config.toml")
        }
    });
    print!("{:>24}: ", "Config");
    match std::fs::read_to_string(&config_path) {
        Ok(content) => match toml::from_str::<jia::config::JiaToml>(&content) {
            Ok(cfg) => {
                let n = cfg.providers.len();
                let dp = cfg
                    .llm
                    .default_main_model_provider
                    .as_deref()
                    .unwrap_or("(none)");
                println!(
                    "\u{1b}[32mOK\u{1b}[0m    {} provider(s), default: {}",
                    n, dp
                );
                ok += 1;
            }
            Err(e) => {
                println!("\u{1b}[31mFAIL\u{1b}[0m  parse error: {}", e);
                err += 1;
            }
        },
        Err(_) => {
            println!(
                "\u{1b}[33mWARN\u{1b}[0m  not found at {}",
                config_path.display()
            );
            warn += 1;
        }
    }

    // 2. Data directory
    print!("{:>24}: ", "Data dir");
    match std::fs::create_dir_all(&data_dir) {
        Ok(()) => {
            println!("\u{1b}[32mOK\u{1b}[0m    {}", data_dir.display());
            ok += 1;
        }
        Err(e) => {
            println!("\u{1b}[31mFAIL\u{1b}[0m  {} ({})", data_dir.display(), e);
            err += 1;
        }
    }

    // 3. SQLite
    let db_path = data_dir.join("store.db");
    print!("{:>24}: ", "SQLite");
    match std::fs::metadata(&db_path) {
        Ok(m) => {
            println!(
                "\u{1b}[32mOK\u{1b}[0m    {} ({} MB)",
                db_path.display(),
                m.len() / 1_048_576
            );
            ok += 1;
        }
        Err(_) => {
            println!("\u{1b}[33mWARN\u{1b}[0m  not found (will be created on first run)");
            warn += 1;
        }
    }

    // 4. Disk space
    print!("{:>24}: ", "Disk");
    // Use statvfs on Unix
    #[cfg(unix)]
    {
        use std::ffi::CString;
        if let Ok(path_c) = CString::new(data_dir.to_string_lossy().as_bytes()) {
            // SAFETY: statvfs() reads filesystem statistics. path_c is a
            // valid CString pointer (built from a Rust PathBuf). stat is a
            // zero-initialized struct with sufficient size for the syscall.
            unsafe {
                let mut stat: libc::statvfs = std::mem::zeroed();
                if libc::statvfs(path_c.as_ptr(), &mut stat) == 0 {
                    let free_gb = (stat.f_bavail as u64 * stat.f_frsize) as f64 / 1_073_741_824.0;
                    if free_gb > 1.0 {
                        println!("\u{1b}[32mOK\u{1b}[0m    {:.1} GB free", free_gb);
                    } else {
                        println!("\u{1b}[33mWARN\u{1b}[0m  only {:.1} GB free", free_gb);
                    }
                    ok += 1;
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        println!("   (disk check not supported on this platform)");
    }

    // 5. Log file
    print!("{:>24}: ", "Logs");
    let log_dir = data_dir.join("logs");
    match std::fs::read_dir(&log_dir) {
        Ok(entries) => {
            let count = entries.filter_map(|e| e.ok()).count();
            if count > 0 {
                println!(
                    "\u{1b}[32mOK\u{1b}[0m    {} file(s) in {}",
                    count,
                    log_dir.display()
                );
                ok += 1;
            } else {
                println!("\u{1b}[33mWARN\u{1b}[0m  directory empty (daemon not started yet?)");
                warn += 1;
            }
        }
        Err(_) => {
            println!("\u{1b}[33mWARN\u{1b}[0m  directory not found (daemon not started yet?)");
            warn += 1;
        }
    }

    // 6. Daemon status
    print!("{:>24}: ", "Daemon");
    let pid_path = jia::palaces::kun_config::pid_file_path();
    match std::fs::read_to_string(&pid_path) {
        Ok(pid_str) => {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                #[cfg(unix)]
                // SAFETY: kill(pid, 0) is the standard POSIX pattern for
                // checking process existence — signal 0 sends no signal,
                // only performs error checking. pid is parsed from a PID
                // file written by this same daemon.
                let alive = unsafe { libc::kill(pid as i32, 0) == 0 };
                #[cfg(not(unix))]
                let alive = false;
                if alive {
                    println!("\u{1b}[32mOK\u{1b}[0m    running (PID {})", pid);
                    ok += 1;
                } else {
                    println!("\u{1b}[33mWARN\u{1b}[0m  PID file exists but process not running");
                    warn += 1;
                }
            }
        }
        Err(_) => {
            println!("\u{1b}[33mWARN\u{1b}[0m  not running");
            warn += 1;
        }
    }

    // 7. Rin socket
    let rin_sock = data_dir.join("rin.sock");
    print!("{:>24}: ", "Rin socket");
    match std::fs::metadata(&rin_sock) {
        Ok(_) => {
            println!("\u{1b}[32mOK\u{1b}[0m    {}", rin_sock.display());
            ok += 1;
        }
        Err(_) => {
            println!("   (not created yet)");
        }
    }

    // 8. SQLite integrity check
    print!("{:>24}: ", "SQLite integrity");
    match rusqlite::Connection::open(&db_path) {
        Ok(conn) => {
            match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
                Ok(ref result) if result == "ok" => {
                    println!("\u{1b}[32mOK\u{1b}[0m    database is healthy");
                    ok += 1;
                }
                Ok(other) => {
                    println!("\u{1b}[31mFAIL\u{1b}[0m  {}", other);
                    err += 1;
                }
                Err(e) => {
                    println!("\u{1b}[33mWARN\u{1b}[0m  integrity check failed: {}", e);
                    warn += 1;
                }
            }
        }
        Err(e) => {
            println!("\u{1b}[33mWARN\u{1b}[0m  cannot open: {}", e);
            warn += 1;
        }
    }

    // 9. LLM connectivity (non-blocking, short timeout)
    if let Ok(content) = std::fs::read_to_string(&config_path)
        && let Ok(cfg) = toml::from_str::<jia::config::JiaToml>(&content)
    {
        for (name, profile) in &cfg.providers {
            print!("{:>24}: ", format!("LLM {}", name));
            let result = tokio::task::block_in_place(|| {
                let client = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()?;
                let url = format!("{}/models", profile.base_url.trim_end_matches('/'));
                let resp = client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", profile.api_key))
                    .send()?;
                Ok::<_, reqwest::Error>(resp.status())
            });
            match result {
                Ok(status) if status.is_success() => {
                    println!("\u{1b}[32mOK\u{1b}[0m    {} ({})", profile.base_url, status);
                    ok += 1;
                }
                Ok(status) => {
                    println!(
                        "\u{1b}[33mWARN\u{1b}[0m  {} returned {}",
                        profile.base_url, status
                    );
                    warn += 1;
                }
                Err(e) => {
                    println!(
                        "\u{1b}[31mFAIL\u{1b}[0m  {} unreachable: {}",
                        profile.base_url, e
                    );
                    err += 1;
                }
            }
        }
    }

    // 10. Backup directory
    let backup_dir = data_dir.join("backups");
    print!("{:>24}: ", "Backup dir");
    match std::fs::read_dir(&backup_dir) {
        Ok(entries) => {
            let count = entries
                .filter(|e| e.as_ref().map(|d| d.path().is_dir()).unwrap_or(false))
                .count();
            if count > 100 {
                println!(
                    "\u{1b}[33mWARN\u{1b}[0m  {} dirs (prune to ~30 with `jia gateway cleanup-backups`)",
                    count
                );
                warn += 1;
            } else if count > 30 {
                println!("\u{1b}[33mWARN\u{1b}[0m  {} dirs (consider pruning)", count);
                warn += 1;
            } else {
                println!("\u{1b}[32mOK\u{1b}[0m    {} backups", count);
                ok += 1;
            }
        }
        Err(_) => {
            println!("\u{1b}[33mWARN\u{1b}[0m  not found (will be created on first backup)");
            warn += 1;
        }
    }

    println!();
    println!("Summary: {} OK, {} warnings, {} errors", ok, warn, err);
    if err > 0 {
        std::process::exit(1);
    }
}

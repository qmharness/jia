use clap::Parser;
use kernel::config::{CliArgs, Commands, GatewayAction};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();

    // Set up file logging so cron output is traceable even in daemon mode
    // (where stderr is redirected to /dev/null). Logs rotate daily.
    let data_dir = kernel::palaces::kun_config::default_data_dir();
    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir).ok();

    let file_appender = tracing_appender::rolling::RollingFileAppender::new(
        tracing_appender::rolling::Rotation::DAILY,
        log_dir,
        "jia.log",
    );

    let default_filter = "jia=info,kernel=info,tower_http=info";

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
        // Shortcuts: jia start / stop / restart
        Commands::Start {
            config_path,
            host,
            port,
        } => {
            spawn_daemon(config_path, host, port);
        }
        Commands::Stop => {
            stop_running_instance();
        }
        Commands::Status => {
            gateway_status();
        }
        Commands::Restart {
            config_path,
            host,
            port,
        } => {
            stop_running_instance();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            spawn_daemon(config_path, host, port);
        }
        Commands::Gateway { action } => match action {
            GatewayAction::Start {
                config_path,
                host,
                port,
            } => {
                spawn_daemon(config_path, host, port);
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
            } => {
                stop_running_instance();
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                spawn_daemon(config_path, host, port);
            }
            GatewayAction::Daemon {
                config_path,
                host,
                port,
            } => {
                daemonize();
                run_start(config_path, host, port, None).await;
            }
        },
        Commands::WechatSetup => match channels::wechat::qr_login().await {
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
            // If daemon is not running, start it first — the TUI needs the gateway.
            if is_daemon_running().is_none() {
                spawn_daemon(args.config_path.clone(), None, None);

                // Wait up to 10 seconds for the server to become reachable
                let addr = "127.0.0.1:3000";
                let mut attempts = 0;
                loop {
                    if std::net::TcpStream::connect_timeout(
                        &addr.parse().expect("invalid socket addr"),
                        std::time::Duration::from_millis(500),
                    )
                    .is_ok()
                    {
                        break;
                    }
                    attempts += 1;
                    if attempts >= 20 {
                        eprintln!("jia gateway did not start within 10 seconds");
                        std::process::exit(1);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            run_tui(args.config_path).await;
        }
        Commands::Doctor => {
            run_doctor(args.config_path);
        }
        Commands::Web {
            config_path,
            host,
            port,
            web_dir: _,
        } => {
            // If daemon is already running, just open the browser.
            if let Some((daemon_host, daemon_port)) = is_daemon_running() {
                let url = format!("http://{daemon_host}:{daemon_port}");
                println!("jia gateway is already running at {url}");
                open_browser(&url);
                return;
            }

            // Otherwise, start daemon in background, wait for it, then open browser.
            spawn_daemon(config_path, host.clone(), port);

            // Determine the address to wait for
            let target_host = host.unwrap_or_else(|| "127.0.0.1".to_string());
            let target_port = port.unwrap_or(3000);
            let url = format!("http://{target_host}:{target_port}");

            // Wait up to 10 seconds for the server to become reachable
            let addr = format!("{target_host}:{target_port}");
            let mut attempts = 0;
            loop {
                if std::net::TcpStream::connect_timeout(
                    &addr.parse().expect("invalid socket addr"),
                    std::time::Duration::from_millis(500),
                )
                .is_ok()
                {
                    println!("jia gateway is ready at {url}");
                    open_browser(&url);
                    break;
                }
                attempts += 1;
                if attempts >= 20 {
                    eprintln!("jia gateway did not start within 10 seconds");
                    std::process::exit(1);
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
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
            let data_dir = kernel::palaces::kun_config::default_data_dir();
            let db_path = data_dir.join("store.db");
            let store =
                kernel::palaces::gen_store::Store::open(db_path.to_str().unwrap_or(":memory:"));
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

// ── CLI subcommand modules ──────────────────────────────
mod cli;
use cli::doctor::run_doctor;
use cli::start::{
    daemonize, gateway_status, is_daemon_running, open_browser, run_start, spawn_daemon,
    stop_running_instance,
};
#[cfg(feature = "tui")]
use cli::tui::run_tui;

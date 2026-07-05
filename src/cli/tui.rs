//! TUI subcommand handler.
use std::path::PathBuf;

pub async fn run_tui(config_path: Option<PathBuf>) {
    use std::net::TcpStream;
    use std::time::Duration;

    use kernel::palaces::kun_config::{default_data_dir, pid_file_path};

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
        let config = kernel::config::AppConfig::load(config_path.clone(), None, None)
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
    let config = kernel::config::AppConfig::load(config_path, None, None)
        .expect("Failed to load configuration");

    tui::run(config).await;
}

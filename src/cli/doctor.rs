//! Doctor diagnostic subcommand handler.

use kernel;

pub fn run_doctor(config_path: Option<std::path::PathBuf>) {
    println!("🔍 jia doctor — diagnosing installation health\n");

    let data_dir = kernel::palaces::kun_config::default_data_dir();
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
        Ok(content) => match toml::from_str::<kernel::palaces::kun_config::JiaToml>(&content) {
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
    let pid_path = kernel::palaces::kun_config::pid_file_path();
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
        && let Ok(cfg) = toml::from_str::<kernel::palaces::kun_config::JiaToml>(&content)
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

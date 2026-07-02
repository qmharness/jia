// ── TUI Module ────────────────────────────────────────────
//
// Terminal UI launched via `jia tui`. Connects to the daemon over
// the rin Unix socket (~/.jia/rin.sock) using JSON-line protocol.
// Zero HTTP — all communication through the socket.
//
// Architecture:
//   spawn_blocking thread  →  crossterm event::poll/read
//   main task              →  terminal.draw + tokio::select!
//   tokio::spawn task      →  socket read_line (async)

mod app;
mod composer;
mod connection;
mod render;

use std::time::Duration;

use crossterm::Command;
use tokio::io::AsyncWriteExt;
use crossterm::cursor;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::fmt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use self::app::Event as AppEvent;

/// Inline viewport height in rows (active turn + input + status). History
/// scrolls above this region into the terminal's native scrollback via
/// `Terminal::insert_before`. Must exceed layout's input + status rows.
const VIEWPORT_HEIGHT: u16 = 18;

/// Enable "alternate scroll" (ANSI `\x1b[?1007h`): terminals translate the
/// mouse wheel to ↑/↓ keys. Widely supported (incl. macOS Terminal.app),
/// unlike full mouse-reporting (`EnableMouseCapture`).
struct EnableAlternateScroll;
struct DisableAlternateScroll;

impl Command for EnableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007h")
    }
}

impl Command for DisableAlternateScroll {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1007l")
    }
}

/// Entry point for `jia tui`. Connects to the daemon via rin socket,
/// queries the actual model/provider, initializes the terminal, and runs
/// the event loop.
pub async fn run(_config: crate::config::AppConfig) {
    let data_dir = crate::palaces::kun_config::default_data_dir();
    let rin_sock = data_dir.join("rin.sock");

    // ── Connect + query daemon for actual model / provider ────
    let (conn, mut socket_rx) = match connection::Connection::connect(&rin_sock).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("TUI: failed to connect to daemon: {e}");
            std::process::exit(1);
        }
    };

    // Send model_info query and wait for the response.
    let mut llm = app::LlmInfo { model_id: String::new(), provider: String::new() };
    {
        let mut writer = conn.writer().lock().await;
        let _ = writer.write_all(b"{\"type\":\"model_info\"}\n").await;
    }
    // Read the response — the reader task should send it promptly.
    while let Some(event) = socket_rx.recv().await {
        if let connection::SocketEvent::ModelInfo { provider, model } = event {
            llm = app::LlmInfo { model_id: model, provider };
            break;
        }
    }

    // ── Terminal setup ──────────────────────────────────────
    // Install panic hook FIRST so if anything panics below, terminal is restored.
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Main screen (inline viewport): restore cursor + disable raw mode.
        // No LeaveAlternateScreen (we never enter it).
        let _ = execute!(std::io::stderr(), cursor::Show, DisableAlternateScroll);
        let _ = disable_raw_mode();
        orig_hook(info);
    }));

    enable_raw_mode().expect("Failed to enable raw mode");
    let mut stderr = std::io::stderr();
    // Main screen (not alternate) so content scrolls into the terminal's native
    // scrollback. Inline viewport reserves VIEWPORT_HEIGHT rows at the bottom.
    execute!(&mut stderr, cursor::Hide, EnableAlternateScroll).expect("Failed to setup terminal");

    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Inline(VIEWPORT_HEIGHT),
        },
    )
    .expect("Failed to create terminal");

    // ── Keyboard reader (spawn_blocking) ────────────────────
    let (key_tx, key_rx) = mpsc::unbounded_channel::<AppEvent>();
    let cancel_token = CancellationToken::new();
    let cancel_reader = cancel_token.clone();

    tokio::task::spawn_blocking(move || {
        let tick_rate = Duration::from_millis(100);
        loop {
            if cancel_reader.is_cancelled() {
                break;
            }
            if event::poll(tick_rate).unwrap_or(false) {
                let Ok(evt) = event::read() else { continue };
                match evt {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            let _ = key_tx.send(AppEvent::Quit);
                            break;
                        }
                        let _ = key_tx.send(AppEvent::Key(key));
                    }
                    Event::Resize(w, h) => {
                        let _ = key_tx.send(AppEvent::Resize(w, h));
                    }
                    _ => {}
                }
            }
        }
    });

    // ── Run app ─────────────────────────────────────────────
    let result = app::run_app(
        &mut terminal,
        key_rx,
        conn,
        socket_rx,
        &rin_sock,
        cancel_token.clone(),
        llm,
    )
    .await;

    // ── Shutdown ────────────────────────────────────────────
    cancel_token.cancel();

    // Restore terminal (main screen): clear inline viewport residue, show
    // cursor, disable alternate scroll + raw mode, newline for the shell prompt.
    let _ = terminal.clear();
    let _ = execute!(std::io::stderr(), cursor::Show, DisableAlternateScroll);
    let _ = disable_raw_mode();
    eprintln!();

    match result {
        Ok(()) => {}
        Err(e) => {
            eprintln!("TUI error: {e}");
            std::process::exit(1);
        }
    }
}

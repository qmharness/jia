//! TUI app event loop.
use std::io;
use std::path::Path;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::style::{Color, Style};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::plates::tian_heaven::AgentPhase;
use super::composer::Composer;
use super::connection::{ClientMsg, Connection};
use super::render::{self, ChatLine, StatusIcon};
use super::state::{App, Event, LlmInfo, Mode};

pub async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    mut key_rx: mpsc::UnboundedReceiver<Event>,
    conn: Connection,
    mut socket_rx: mpsc::UnboundedReceiver<super::connection::SocketEvent>,
    sock_path: &Path,
    cancel: CancellationToken,
    llm: LlmInfo,
) -> io::Result<()> {
    // P3 · Check for existing project; show Welcome if not found
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default();
    let has_project = std::path::Path::new(&cwd)
        .join(".jia")
        .join("config.toml")
        .exists();

    let mut app = App {
        mode: if has_project {
            Mode::Normal
        } else {
            Mode::Welcome {
                cwd: cwd.clone(),
                selected: 0,
            }
        },
        lines: Vec::new(),
        history: Vec::new(),
        needs_finalize: false,
        inserted_rows: 0,
        resize_pending: None,
        resize_deadline: None,
        composer: Composer::new(),
        session_id: None,
        status: StatusIcon::Done,
        planning: false,
        start_time: Instant::now(),
        last_elapsed: 0,
        connection: Some(conn),
        reconnect_attempts: 0,
        // Allowed immediately when a project already exists; otherwise gated
        // until the Welcome trust flow resolves it (sets this true on approve).
        sending_allowed: has_project,
        llm,
        spinner_idx: 0,
        agent_phase: AgentPhase::Reasoning,
        quit: false,
        confirm_selected: 0,
        project_name: String::new(),
        project_id: String::new(),
    };

    if has_project {
        app.refresh_project_info();
        // Notify daemon of project for SQLite registration
        if let Some(ref conn) = app.connection {
            let conn = conn.clone();
            let c = cwd.clone();
            tokio::spawn(async move {
                let _ = conn.send(&ClientMsg::Hello { cwd: c }).await;
            });
        }
    }

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
    let mut next_reconnect: Option<tokio::time::Instant> = None;

    // Push the welcome block into scrollback once at startup (Normal mode only;
    // Welcome mode shows the trust-check full screen instead).
    if has_project {
        push_welcome_to_scrollback(terminal, &app);
    }

    while !app.quit {
        // Flush streaming overflow into scrollback so the viewport stays pinned
        // to the bottom instead of scrolling older deltas out of view.
        {
            let width = terminal.size().map(|s| s.width).unwrap_or(80);
            let input_h = 2 + app.composer.line_count(width).min(6) as u16;
            let msg_h = super::VIEWPORT_HEIGHT.saturating_sub(input_h + 1) as usize;
            flush_streaming_overflow(terminal, &mut app, width, msg_h)?;
        }
        // Reflow scrollback on terminal resize — debounced: wait for the drag
        // to settle (75ms of no new Resize) before clearing + rebuilding.
        if let Some(deadline) = app.resize_deadline
            && Instant::now() >= deadline
        {
            let (w, h) = app.resize_pending.take().unwrap_or((0, 0));
            app.resize_deadline = None;
            reflow_on_resize(terminal, &mut app, w, h)?;
        }
        // Flush any finalized turn into scrollback before rendering.
        if app.needs_finalize {
            finalize_active_turn(terminal, &mut app)?;
            app.needs_finalize = false;
        }
        // ── Render ──────────────────────────────────────
        let _cursor_pos = render_frame_with_cursor(terminal, &app)?;

        // ── Wait for event ──────────────────────────────
        let should_reconnect = tokio::select! {
            _ = cancel.cancelled() => {
                app.quit = true;
                false
            }

            Some(event) = key_rx.recv() => {
                app.dispatch_event(event);
                // Drain queued events so a drag (many Resizes) renders once per
                // batch instead of once per Resize — keeps viewport width
                // tracking responsive instead of lagging behind the render budget.
                while let Ok(ev) = key_rx.try_recv() {
                    app.dispatch_event(ev);
                }
                false
            }

            socket_event = socket_rx.recv() => match socket_event {
                Some(se) => {
                    if next_reconnect.is_some() {
                        next_reconnect = None;
                        app.reconnect_attempts = 0;
                    }
                    app.handle_socket_event(se);
                    false
                }
                // Daemon socket closed. The reader task has exited, so recv()
                // will keep returning None — guard with `connection.is_some()`
                // so we only react once, then let the tick arm drive reconnect.
                None if app.connection.is_some() => {
                    app.connection = None;
                    app.status = StatusIcon::Disconnected;
                    app.reconnect_attempts = 0;
                    next_reconnect = Some(
                        tokio::time::Instant::now() + std::time::Duration::from_millis(500),
                    );
                    app.lines.push(ChatLine {
                        text: String::new(),
                        style: Style::default(),
                    });
                    app.lines.push(ChatLine {
                        text: "✗ Disconnected from daemon, reconnecting…".to_string(),
                        style: StatusIcon::Disconnected.style(),
                    });
                    false
                }
                None => false, // already disconnected; waiting on tick to reconnect
            },

            _ = tick.tick() => {
                if app.status == StatusIcon::Working {
                    app.spinner_idx = (app.spinner_idx + 1) % 10;
                }
                app.connection.is_none()
                    && next_reconnect.is_some()
                    && tokio::time::Instant::now() >= next_reconnect.unwrap()
            }
        };

        if should_reconnect {
            app.reconnect_attempts += 1;
            match Connection::connect(sock_path).await {
                Ok((conn, rx)) => {
                    app.connection = Some(conn);
                    socket_rx = rx;
                    app.reconnect_attempts = 0;
                    next_reconnect = None;
                    app.status = StatusIcon::Done;
                    app.lines.push(ChatLine {
                        text: "✓ Reconnected".to_string(),
                        style: Style::default().fg(Color::Green),
                    });
                }
                Err(_) => {
                    let delay = (1u64 << app.reconnect_attempts.min(5)).min(30);
                    next_reconnect =
                        Some(tokio::time::Instant::now() + std::time::Duration::from_secs(delay));
                    app.lines.push(ChatLine {
                        text: format!(
                            "⏳ Reconnect attempt {} failed, retrying in {}s",
                            app.reconnect_attempts, delay,
                        ),
                        style: Style::default().fg(Color::Red),
                    });
                }
            }
        }
    }

    // Send cancel if agent is running
    if let Some(ref conn) = app.connection
        && let Some(ref sid) = app.session_id
    {
        let _ = conn
            .send(&ClientMsg::Cancel {
                session_id: sid.clone(),
            })
            .await;
    }

    Ok(())
}
pub(crate) fn push_welcome_to_scrollback(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    app: &App,
) {
    let spec = render::WelcomeSpec {
        version: env!("CARGO_PKG_VERSION"),
        model: &app.llm.model_id,
        provider: &app.llm.provider,
        project: &app.project_name,
    };
    let wl = render::welcome_lines(&spec);
    let width = terminal.size().map(|s| s.width).unwrap_or(80);
    let rows = render::count_display_rows(&wl, width) as u16;
    let _ = terminal.insert_before(rows, |buf| {
        render::render_chatlines_to_buffer(buf, &wl, width);
    });
}

/// Finalize the active turn: insert the remaining (not-yet-pushed) display
/// rows into scrollback, archive the turn to `history`, reset for next turn.
pub(crate) fn finalize_active_turn(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    app: &mut App,
) -> io::Result<()> {
    if app.lines.is_empty() {
        return Ok(());
    }
    let width = terminal.size().map(|s| s.width).unwrap_or(80);
    let rows = render::build_display_rows(&[], &app.lines, width);
    if app.inserted_rows < rows.len() {
        let slice: Vec<ratatui::text::Line<'static>> = rows[app.inserted_rows..].to_vec();
        let count = slice.len() as u16;
        terminal.insert_before(count, |buf| {
            render::render_lines_to_buffer(buf, &slice);
        })?;
    }
    app.history.append(&mut app.lines);
    app.inserted_rows = 0;
    Ok(())
}

/// Push active display rows that overflow the viewport into scrollback
/// (streaming incremental insert at display-row granularity — works even when
/// a single long assistant ChatLine wraps to many rows). The viewport stays
/// pinned to the bottom; `inserted_rows` tracks what's already in scrollback.
pub(crate) fn flush_streaming_overflow(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    app: &mut App,
    width: u16,
    msg_height: usize,
) -> io::Result<()> {
    let rows = render::build_display_rows(&[], &app.lines, width);
    let new_inserted = rows.len().saturating_sub(msg_height);
    if new_inserted > app.inserted_rows {
        let slice: Vec<ratatui::text::Line<'static>> =
            rows[app.inserted_rows..new_inserted].to_vec();
        let count = slice.len() as u16;
        terminal.insert_before(count, |buf| {
            render::render_lines_to_buffer(buf, &slice);
        })?;
        app.inserted_rows = new_inserted;
    }
    Ok(())
}

/// On terminal resize: clear scrollback (history was wrapped at the old width)
/// and re-emit all finalized history at the new width, then reset active tracking.
pub(crate) fn reflow_on_resize(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    app: &mut App,
    width: u16,
    height: u16,
) -> io::Result<()> {
    // Reset scroll region + style state, clear scrollback + visible screen,
    // home cursor. Order mirrors codex's clear_scrollback_and_visible_screen_ansi.
    crossterm::execute!(
        std::io::stderr(),
        crossterm::style::Print("\x1b[r\x1b[0m"),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::Purge),
        crossterm::cursor::MoveTo(0, 0)
    )?;
    // Re-seat ratatui's inline viewport/buffer at the new size.
    terminal.resize(ratatui::layout::Rect::new(0, 0, width, height))?;
    // Re-emit the welcome block (head of scrollback) + finalized history.
    let spec = render::WelcomeSpec {
        version: env!("CARGO_PKG_VERSION"),
        model: &app.llm.model_id,
        provider: &app.llm.provider,
        project: &app.project_name,
    };
    let mut all = render::welcome_lines(&spec);
    all.extend(app.history.iter().cloned());
    if !all.is_empty() {
        let rows = render::build_display_rows(&[], &all, width);
        let count = rows.len() as u16;
        terminal.insert_before(count, |buf| {
            render::render_lines_to_buffer(buf, &rows);
        })?;
    }
    app.inserted_rows = 0;
    Ok(())
}

// ── Event Dispatch ─────────────────────────────────────────


pub(crate) fn render_frame_with_cursor(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stderr>>,
    app: &App,
) -> io::Result<Option<(u16, u16)>> {
    let mut cursor = None;
    // Inline viewport: ratatui's f.area() is the screen-absolute viewport rect,
    // so composer cursor coords are already screen-absolute — no offset needed.
    terminal.draw(|f| {
        let input_height = 2 + app.composer.line_count(f.area().width).min(6) as u16;
        let areas = render::layout(f.area(), input_height);

        match &app.mode {
            Mode::Welcome { cwd, selected } => {
                // First-run trust check takes the full screen.
                render::render_security_guide(f, f.area(), cwd, *selected);
            }
            _ => {
                // Welcome box is the head of the stream and scrolls with messages;
                // when `lines` is empty the area just shows the welcome box (initial screen).
                render::render_messages(f, areas.messages, &app.lines);
            }
        }

        let mode_label = match &app.mode {
            Mode::Normal if app.planning => "谋划",
            Mode::Normal => "Normal",
            Mode::Confirm { .. } => "Confirm",
            Mode::Question { .. } => "Question",
            Mode::Welcome { .. } => "",
        };
        if !matches!(app.mode, Mode::Welcome { .. }) {
            render::render_status_bar(
                f,
                areas.status_bar,
                app.status,
                &app.agent_phase.display_name(),
                if app.status == StatusIcon::Working {
                    app.start_time.elapsed().as_secs()
                } else {
                    app.last_elapsed
                },
                app.reconnect_attempts,
                app.spinner_idx,
            );
            cursor = render::render_input(f, areas.input, &app.composer);
            render::render_info_bar(
                f,
                areas.info_bar,
                mode_label,
                &format!("{} · {}", app.llm.model_id, app.llm.provider),
                app.session_id.as_deref(),
                &app.project_name,
            );
        }
    })?;

    // Set terminal cursor position after draw
    if let Some((x, y)) = cursor {
        terminal.set_cursor_position(ratatui::layout::Position::new(x, y))?;
        let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Show);
    } else {
        let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Hide);
    }

    Ok(cursor)
}


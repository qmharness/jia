// ── App State & Event Loop ────────────────────────────────
//
// The main TUI application: holds state, runs the render → select → update
// loop, and coordinates between the socket reader and keyboard input.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Style};
use time::OffsetDateTime;

use crate::plates::tian_heaven::AgentPhase;

use super::composer::Composer;
use super::connection::{ClientMsg, Connection, SocketEvent, StreamEvent};
use super::render::{self, ChatLine, StatusIcon};

// ── App Events ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum Event {
    Key(KeyEvent),
    #[allow(dead_code)]
    Quit,
    #[allow(dead_code)]
    Resize(u16, u16),
    #[allow(dead_code)]
    Tick,
}

// ── Input Mode ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum Mode {
    Normal,
    /// Claude-style project trust check on startup
    Welcome {
        cwd: String,
        selected: usize,
    },
    Confirm {
        id: String,
        token: String,
    },
    Question {
        id: String,
        token: String,
        /// Predefined choices (None = free-text mode).
        options: Option<Vec<String>>,
        /// Currently highlighted option index (0-based).
        selected: usize,
        /// Index into `App.lines` where the first option ChatLine starts.
        first_option_line: usize,
    },
}

// ── LlmInfo ────────────────────────────────────────────────

pub struct LlmInfo {
    pub model_id: String,
    pub provider: String,
}

// ── App State ──────────────────────────────────────────────

pub(crate) struct App {
    pub(crate) mode: Mode,
    /// Active turn lines (rendered in the viewport); pushed to scrollback on Done.
    pub(crate) lines: Vec<ChatLine>,
    /// Finalized history (source of truth; already pushed to terminal scrollback).
    pub(crate) history: Vec<ChatLine>,
    /// Set on StreamEvent::Done; run_app flushes via insert_before, then clears.
    pub(crate) needs_finalize: bool,
    /// Display rows of the active turn already pushed to scrollback (so we
    /// don't re-insert them; reset when the turn finalizes).
    pub(crate) inserted_rows: usize,
    /// Pending terminal resize → triggers scrollback reflow (history was wrapped
    /// at the old width).
    pub(crate) resize_pending: Option<(u16, u16)>,
    /// Debounce deadline for the resize reflow (wait for drag to settle).
    pub(crate) resize_deadline: Option<Instant>,
    pub(crate) composer: Composer,
    pub(crate) session_id: Option<String>,
    pub(crate) status: StatusIcon,
    /// P3 · whether the agent is in planning mode (谋划态) — shown in status.
    pub(crate) planning: bool,
    pub(crate) start_time: Instant,
    /// Frozen elapsed seconds of the last completed request (frozen on Done/Error).
    pub(crate) last_elapsed: u64,
    pub(crate) connection: Option<Connection>,
    pub(crate) reconnect_attempts: u32,
    pub(crate) sending_allowed: bool,
    /// Model and provider for display (welcome block, info bar).
    pub(crate) llm: LlmInfo,
    pub(crate) spinner_idx: usize,
    /// Current agent phase / 九星 (shown in status bar).
    pub(crate) agent_phase: AgentPhase,
    pub(crate) quit: bool,
    /// P3 · currently selected option in confirmation prompt (0 = approve, 1 = deny)
    pub(crate) confirm_selected: usize,
    /// P3 · project name from .jia/config.toml (for welcome screen)
    pub(crate) project_name: String,
    /// P3 · project ID from .jia/config.toml
    pub(crate) project_id: String,
}

// ── Public API ─────────────────────────────────────────────


/// Push the welcome block into the terminal scrollback once at startup.
impl App {
    pub(crate) fn dispatch_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key(key),
            Event::Quit => self.quit = true,
            Event::Resize(w, h) => {
                // Schedule scrollback reflow on resize — debounced: each Resize
                // pushes the deadline out, so we only rebuild once the drag
                // settles (avoids flicker + incomplete inserts mid-drag).
                self.resize_pending = Some((w, h));
                self.resize_deadline = Some(Instant::now() + std::time::Duration::from_millis(75));
            }
            _ => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Global keys (work in any mode)
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }

        match self.mode.clone() {
            Mode::Welcome { cwd, selected: _ } => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Mode::Welcome { selected, .. } = &mut self.mode {
                        *selected = selected.saturating_sub(1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Mode::Welcome { selected, .. } = &mut self.mode {
                        *selected = (*selected + 1).min(1);
                    }
                }
                KeyCode::Enter => {
                    let approved = matches!(&self.mode, Mode::Welcome { selected: 0, .. });
                    if approved {
                        let cwd_str = cwd.clone();
                        self.mode = Mode::Normal;
                        self.sending_allowed = true;
                        // Create project locally + notify daemon
                        let project_id = uuid::Uuid::new_v4().to_string();
                        let dir_name = std::path::Path::new(&cwd_str)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let proj_dir = std::path::Path::new(&cwd_str).join(".jia");
                        let _ = std::fs::create_dir_all(&proj_dir);
                        let config = format!(
                            "[project]\nid = \"{}\"\nname = \"{}\"\n",
                            project_id, dir_name
                        );
                        let _ = std::fs::write(proj_dir.join("config.toml"), &config);
                        self.refresh_project_info();
                        // Notify daemon
                        if let Some(ref conn) = self.connection {
                            let conn = conn.clone();
                            let c = cwd_str.clone();
                            let _pid = project_id.clone();
                            tokio::spawn(async move {
                                let _ = conn.send(&ClientMsg::Hello { cwd: c }).await;
                            });
                        }
                    } else {
                        self.quit = true;
                    }
                }
                KeyCode::Esc => {
                    self.quit = true;
                }
                _ => {}
            },

            Mode::Normal => {
                if key.code == KeyCode::Char('l') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.lines.clear();
                    return;
                }
                if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.request_sessions();
                    return;
                }
                // History lives in the terminal scrollback now — scroll with the
                // terminal's native wheel/scrollbar. ↑/↓ fall through to composer
                // (input history recall); PageUp/PageDown are no-ops here.
                // Delegate to composer
                let handled = self.composer.handle_key(key);
                if handled && self.sending_allowed {
                    let text = self.composer.text();
                    let trimmed = text.trim();
                    // P3 · /plan slash entry (user-triggered plan mode).
                    if trimmed == "/plan" || trimmed == "/plan-end" {
                        let planning = trimmed == "/plan";
                        self.send_set_mode(planning);
                        self.composer.add_to_history(&text);
                        self.composer.clear();
                        self.lines.push(ChatLine {
                            text: if planning {
                                "🗺 /plan — requesting planning mode (谋划态)".to_string()
                            } else {
                                "/plan-end — requesting normal mode".to_string()
                            },
                            style: Style::default().fg(Color::Cyan),
                        });
                    } else {
                        self.send_agent_message(&text);
                        self.composer.add_to_history(&text);
                        self.composer.clear();
                    }
                }
                // Agent working — Enter is silently ignored
            }

            Mode::Confirm { id, token, .. } => match key.code {
                KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.send_confirm(&id, &token, true);
                    self.mode = Mode::Normal;
                    self.confirm_selected = 0;
                    self.composer.set_placeholder("");
                    // Re-read project info after creation
                    self.refresh_project_info();
                }
                KeyCode::Char('2') | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.send_confirm(&id, &token, false);
                    self.quit = true;
                }
                _ => {}
            },

            Mode::Question {
                ref id,
                ref token,
                ref options,
                mut selected,
                first_option_line,
            } => {
                let first_line: usize = first_option_line;
                let has_options = options.is_some();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') if has_options => {
                        if selected > 0 {
                            let old_idx = first_line + selected;
                            selected -= 1;
                            let new_idx = first_line + selected;
                            render::update_option_style(&mut self.lines[old_idx], false);
                            render::update_option_style(&mut self.lines[new_idx], true);
                            self.mode = Mode::Question {
                                id: id.clone(),
                                token: token.clone(),
                                options: options.clone(),
                                selected,
                                first_option_line: first_line,
                            };
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') if has_options => {
                        let opts = options.as_ref().unwrap();
                        if selected + 1 < opts.len() {
                            let old_idx = first_line + selected;
                            selected += 1;
                            let new_idx = first_line + selected;
                            render::update_option_style(&mut self.lines[old_idx], false);
                            render::update_option_style(&mut self.lines[new_idx], true);
                            self.mode = Mode::Question {
                                id: id.clone(),
                                token: token.clone(),
                                options: options.clone(),
                                selected,
                                first_option_line: first_line,
                            };
                        }
                    }
                    KeyCode::Char(c) if has_options && c.is_ascii_digit() => {
                        let n = (c as u8 - b'0') as usize;
                        let opts = options.as_ref().unwrap();
                        if n >= 1 && n <= opts.len() && n - 1 != selected {
                            let old_idx = first_line + selected;
                            let new_selected = n - 1;
                            render::update_option_style(&mut self.lines[old_idx], false);
                            let new_idx = first_line + new_selected;
                            render::update_option_style(&mut self.lines[new_idx], true);
                            self.mode = Mode::Question {
                                id: id.clone(),
                                token: token.clone(),
                                options: options.clone(),
                                selected: new_selected,
                                first_option_line: first_line,
                            };
                        }
                    }
                    KeyCode::Enter if has_options => {
                        let opts = options.as_ref().unwrap();
                        // Last option = free-text entry
                        if selected == opts.len() - 1 {
                            // Switch to free-text mode: remove option lines, keep question
                            let opt_count = opts.len() + 1; // skip leading blank + options
                            let start: usize = first_line - 1; // back to leading blank
                            self.lines.drain(start..start + opt_count);
                            self.mode = Mode::Question {
                                id: id.clone(),
                                token: token.clone(),
                                options: None,
                                selected: 0,
                                first_option_line: 0,
                            };
                            self.composer.set_placeholder(
                                "Type your answer... (Enter to send, Esc to cancel)",
                            );
                        } else {
                            let answer = &opts[selected];
                            self.send_answer(id, token, answer);
                            // Remove question panel (question + blank + options)
                            let opt_count = opts.len() + 2;
                            let start: usize = first_line.saturating_sub(2);
                            self.lines.drain(start..start + opt_count);
                            self.composer.clear();
                            self.composer.set_placeholder("");
                            self.lines.push(ChatLine {
                                text: format!("  └ Selected: {answer}"),
                                style: Style::default().fg(Color::Green),
                            });
                            self.mode = Mode::Normal;
                        }
                    }
                    KeyCode::Esc => {
                        self.send_answer(id, token, "");
                        // Remove question panel if options still visible
                        if has_options {
                            let opts = options.as_ref().unwrap();
                            let opt_count = opts.len() + 2;
                            let start: usize = first_line.saturating_sub(2);
                            self.lines.drain(start..start + opt_count);
                        }
                        self.lines.push(ChatLine {
                            text: "  └ Cancelled".to_string(),
                            style: Style::default().fg(Color::DarkGray),
                        });
                        self.composer.clear();
                        self.composer.set_placeholder("");
                        self.mode = Mode::Normal;
                    }
                    // Free-text fallback (no options, or user starts typing)
                    KeyCode::Enter => {
                        let answer = self.composer.text();
                        self.send_answer(id, token, &answer);
                        // Remove question line (options were already drained on first typed char)
                        if let Some(pos) = self
                            .lines
                            .iter()
                            .rposition(|l| l.text.starts_with('\u{2753}'))
                        {
                            self.lines.remove(pos);
                        }
                        self.composer.clear();
                        self.composer.set_placeholder("");
                        self.lines.push(ChatLine {
                            text: format!("  └ Answered: {answer}"),
                            style: Style::default().fg(Color::Green),
                        });
                        self.mode = Mode::Normal;
                    }
                    _ => {
                        // In option-selection mode, input is locked — only
                        // navigation/selection keys work. In free-text mode,
                        // all keys pass through to the composer.
                        if !has_options {
                            self.composer.handle_key(key);
                        }
                    }
                }
            }
        }
    }

    // ── Socket Events ──────────────────────────────────

    pub(crate) fn handle_socket_event(&mut self, event: SocketEvent) {
        match event {
            SocketEvent::Stream(se) => self.handle_stream_event(se),
            SocketEvent::CronNotification {
                job_name,
                response,
                prompt,
                timestamp,
            } => {
                let ts = OffsetDateTime::from_unix_timestamp(timestamp)
                    .map(|dt| {
                        let (h, m, _) = dt.to_hms();
                        format!("{h:02}:{m:02}")
                    })
                    .unwrap_or_default();
                self.lines.push(ChatLine {
                    text: format!("📅 [{ts}] {job_name}: {prompt}"),
                    style: Style::default().fg(Color::DarkGray),
                });
                if !response.is_empty() {
                    self.lines.push(ChatLine {
                        text: response,
                        style: Style::default().fg(Color::DarkGray),
                    });
                }
            }
            SocketEvent::SessionsList(sessions) => {
                self.lines.push(ChatLine {
                    text: "── Sessions ──".to_string(),
                    style: Style::default().fg(Color::Cyan),
                });
                for s in &sessions {
                    let id = s["id"].as_str().unwrap_or("");
                    let title = s["title"].as_str().unwrap_or("");
                    let count = s["messageCount"].as_u64().unwrap_or(0);
                    let short_id = if id.len() > 8 { &id[..8] } else { id };
                    self.lines.push(ChatLine {
                        text: format!("  {short_id} │ {title} ({count} msgs)"),
                        style: Style::default().fg(Color::White),
                    });
                }
            }
            SocketEvent::SessionHistory {
                session_id,
                entries,
            } => {
                self.session_id = Some(session_id);
                self.lines.clear();
                for entry in &entries {
                    let role = entry["role"].as_str().unwrap_or("");
                    let content = entry["content"].as_str().unwrap_or("");
                    if role == "tool_call" {
                        let tool = entry["tool"].as_str().unwrap_or("");
                        let output = entry["output"].as_str().unwrap_or("");
                        self.lines.push(ChatLine {
                            text: format!("🔧 {tool} — {output}"),
                            style: Style::default().fg(Color::DarkGray),
                        });
                    } else {
                        let style = match role {
                            "user" => Style::default().fg(Color::Cyan),
                            "system" => Style::default().fg(Color::Yellow),
                            _ => Style::default().fg(Color::White),
                        };
                        self.lines.push(ChatLine {
                            text: content.to_string(),
                            style,
                        });
                    }
                }
                self.lines.push(ChatLine {
                    text: "── Session loaded ──".to_string(),
                    style: Style::default().fg(Color::Green),
                });
            }
            SocketEvent::ConfirmResolved { id, resolved } => {
                self.mode = Mode::Normal;
                self.composer.set_placeholder("");
                if !resolved {
                    self.lines.push(ChatLine {
                        text: format!("✗ Confirm denied or timeout ({id})"),
                        style: Style::default().fg(Color::Red),
                    });
                }
            }
            SocketEvent::ModelInfo { .. } => {
                // Already consumed during startup; ignore.
            }
            SocketEvent::AnswerResolved { id, resolved } => {
                if !resolved {
                    self.lines.push(ChatLine {
                        text: format!("✗ Answer timed out ({id})"),
                        style: Style::default().fg(Color::Red),
                    });
                }
            }
            SocketEvent::ProjectResolved {
                project_id,
                approved,
                ..
            } => {
                if approved {
                    self.project_id = project_id;
                } else {
                    self.quit = true;
                }
            }
        }
    }

    /// Re-read project info from .jia/config.toml after creation.
    /// Retries briefly to account for race with daemon-side file write.
    pub(crate) fn refresh_project_info(&mut self) {
        if let Ok(cwd) = std::env::current_dir() {
            let config_path = cwd.join(".jia").join("config.toml");
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                for line in content.lines() {
                    if let Some(v) = line
                        .strip_prefix("id = \"")
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        self.project_id = v.to_string();
                    }
                    if let Some(v) = line
                        .strip_prefix("name = \"")
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        self.project_name = v.to_string();
                    }
                }
            }
        }
    }

    fn handle_stream_event(&mut self, event: StreamEvent) {
        match event {
            StreamEvent::Delta { content } => {
                // Skip whitespace-only deltas when last line is blank or a
                // non-assistant line (user message / tool card) — prevents
                // double blank lines above tool cards.
                let whitespace_only = content.chars().all(|c| c.is_whitespace());
                let last = self.lines.last();
                let last_blank = last.map(|l| l.text.is_empty()).unwrap_or(true);
                let last_non_assistant = last.map(|l| l.style != Style::default()).unwrap_or(false);
                let skip = whitespace_only && (last_blank || last_non_assistant);
                if !skip {
                    // Append to last line if it's an assistant (default style) line
                    if let Some(last) = self.lines.last_mut()
                        && last.style == Style::default()
                    {
                        last.text.push_str(&content);
                    } else {
                        // Blank line above assistant response
                        self.lines.push(ChatLine {
                            text: String::new(),
                            style: Style::default(),
                        });
                        if !whitespace_only {
                            self.lines.push(ChatLine {
                                text: content,
                                style: Style::default(),
                            });
                        }
                    }
                }
                self.status = StatusIcon::Working;
                self.agent_phase = AgentPhase::Reasoning;
                self.spinner_idx = (self.spinner_idx + 1) % 10;
            }
            StreamEvent::Session { session_id } => {
                self.session_id = Some(session_id);
            }
            StreamEvent::ContextPressure => {
                self.agent_phase = AgentPhase::ContextManage;
            }
            StreamEvent::Compacting => {
                self.agent_phase = AgentPhase::Compact;
            }
            StreamEvent::Done => {
                self.status = StatusIcon::Done;
                self.agent_phase = AgentPhase::Reasoning;
                self.sending_allowed = true;
                self.last_elapsed = self.start_time.elapsed().as_secs();
                // Push the active turn into scrollback; run_app flushes it.
                self.needs_finalize = true;
            }
            StreamEvent::Error { message } => {
                self.lines.push(ChatLine {
                    text: format!("✗ Error: {message}"),
                    style: Style::default().fg(Color::Red),
                });
                self.status = StatusIcon::Error;
                self.agent_phase = AgentPhase::ErrorRecovery;
                self.sending_allowed = true;
                self.last_elapsed = self.start_time.elapsed().as_secs();
            }
            StreamEvent::InteractionModeChanged { planning } => {
                self.planning = planning;
                if planning {
                    self.lines.push(ChatLine {
                        text: "🗺 进入谋划态（只读）".to_string(),
                        style: Style::default().fg(Color::Cyan),
                    });
                } else {
                    self.lines.push(ChatLine {
                        text: "退出谋划态".to_string(),
                        style: Style::default().fg(Color::DarkGray),
                    });
                }
            }
            StreamEvent::ConfirmationRequest {
                id, token, reason, ..
            } => {
                self.agent_phase = AgentPhase::AwaitingResult;
                self.mode = Mode::Confirm { id, token };
                self.composer
                    .set_placeholder(&format!("{reason}  [1] Yes  [2] No  · Esc to cancel"));
            }
            StreamEvent::UserQuestion {
                id,
                token,
                question,
                timeout_secs,
                options,
            } => {
                self.agent_phase = AgentPhase::AwaitingResult;
                self.lines
                    .push(render::format_user_question(&question, timeout_secs));
                let (opts_store, selected, first_opt_line) = if let Some(ref opts) = options {
                    // Track where option lines will start
                    let first_idx = self.lines.len();
                    self.lines.extend(render::format_question_options(opts, 0));
                    let placeholder = if opts.len() <= 9 {
                        format!(
                            "↑↓/1-{} navigate · Enter select · Esc cancel · type for custom",
                            opts.len()
                        )
                    } else {
                        "↑↓ navigate · Enter select · Esc cancel · type for custom".into()
                    };
                    self.composer.set_placeholder(&placeholder);
                    (options, 0usize, first_idx + 1) // +1跳过分隔空白行
                } else {
                    self.composer
                        .set_placeholder("Type your answer... (Enter to send, Esc to cancel)");
                    (None, 0usize, 0usize)
                };
                self.mode = Mode::Question {
                    id,
                    token,
                    options: opts_store,
                    selected,
                    first_option_line: first_opt_line,
                };
            }
            StreamEvent::ToolResult {
                tool,
                output,
                error,
                geju,
                execution_mode,
            } => {
                self.agent_phase = AgentPhase::Reasoning;
                let event = StreamEvent::ToolResult {
                    tool,
                    output,
                    error,
                    geju,
                    execution_mode,
                };
                let new_lines = render::stream_event_to_lines(&event);
                self.lines.extend(new_lines);
            }
            // Delegate formatting to render module
            StreamEvent::ToolBatchStart => {
                self.agent_phase = AgentPhase::ParallelOrchest;
            }
            StreamEvent::StreamEnd => {
                self.agent_phase = AgentPhase::StopCheck;
            }
            StreamEvent::ToolCall { .. } => {
                self.agent_phase = AgentPhase::ToolCalling;
                // Add blank separator if last line isn't already blank
                if self
                    .lines
                    .last()
                    .map(|l| !l.text.is_empty())
                    .unwrap_or(false)
                {
                    self.lines.push(ChatLine {
                        text: String::new(),
                        style: Style::default(),
                    });
                }
                let new_lines = render::stream_event_to_lines(&event);
                self.lines.extend(new_lines);
            }
        }
    }

    // ── Send Helpers ───────────────────────────────────

    fn send_agent_message(&mut self, text: &str) {
        let Some(conn) = &self.connection else { return };
        let msg = crate::types::Message::text(crate::types::Role::User, text.to_string());
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(String::from));
        let client_msg = ClientMsg::Agent {
            messages: vec![msg],
            session_id: self.session_id.clone(),
            cwd,
            project_id: if self.project_id.is_empty() {
                None
            } else {
                Some(self.project_id.clone())
            },
        };
        let conn = conn.clone();
        tokio::spawn(async move {
            let _ = conn.send(&client_msg).await;
        });
        self.lines.push(ChatLine {
            text: String::new(),
            style: Style::default(),
        });
        self.lines.push(ChatLine {
            text: text.to_string(),
            style: Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
        });
        self.status = StatusIcon::Working;
        self.sending_allowed = false;
        self.start_time = Instant::now();
    }

    /// P3 · send a /plan (or /plan-end) mode-toggle to the daemon.
    fn send_set_mode(&self, planning: bool) {
        if let Some(conn) = &self.connection {
            let client_msg = ClientMsg::SetInteractionMode {
                session_id: self.session_id.clone(),
                planning,
            };
            let conn = conn.clone();
            tokio::spawn(async move {
                let _ = conn.send(&client_msg).await;
            });
        }
    }

    fn send_confirm(&self, id: &str, token: &str, approved: bool) {
        if let Some(conn) = &self.connection {
            let msg = ClientMsg::Confirm {
                id: id.to_string(),
                token: token.to_string(),
                approved,
            };
            let conn = conn.clone();
            tokio::spawn(async move {
                let _ = conn.send(&msg).await;
            });
        }
    }

    fn send_answer(&self, id: &str, token: &str, answer: &str) {
        if let Some(conn) = &self.connection {
            tracing::info!(%id, answer_len = answer.len(), "TUI: sending answer");
            let msg = ClientMsg::Answer {
                id: id.to_string(),
                token: token.to_string(),
                answer: answer.to_string(),
            };
            let conn = conn.clone();
            let id_owned = id.to_string();
            tokio::spawn(async move {
                let send_result = conn.send(&msg).await;
                tracing::info!(id = %id_owned, ok = send_result.is_ok(), "TUI: answer sent");
            });
        } else {
            tracing::warn!(%id, "TUI: cannot send answer — no connection");
        }
    }

    fn request_sessions(&self) {
        if let Some(conn) = &self.connection {
            let msg = ClientMsg::Sessions;
            let conn = conn.clone();
            tokio::spawn(async move {
                let _ = conn.send(&msg).await;
            });
        }
    }

    /// Request a specific session's history from the daemon.
    #[allow(dead_code)]
    fn load_session(&self, session_id: &str) {
        if let Some(conn) = &self.connection {
            let conn = conn.clone();
            let sid = session_id.to_string();
            tokio::spawn(async move {
                let _ = conn.load_session(&sid).await;
            });
        }
    }
}

// ── Frame Render ───────────────────────────────────────────


// ── Rendering ──────────────────────────────────────────────
//
// Four-layer layout: header → messages → status → input.
// Tool cards use geju + execution_mode for color-coded annotations.
// Scroll-independent — callers manage scroll offset externally.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::composer::Composer;
use super::connection::StreamEvent;

// ── Chat Line (internal representation) ────────────────────

#[derive(Debug, Clone)]
pub struct ChatLine {
    pub text: String,
    pub style: Style,
}

// ── Status ─────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusIcon {
    Working,
    Done,
    Error,
    Disconnected,
}

impl StatusIcon {
    pub fn as_str(&self) -> &'static str {
        match self {
            StatusIcon::Working => "⚡",
            StatusIcon::Done => "✓",
            StatusIcon::Error => "✗",
            StatusIcon::Disconnected => "⏳",
        }
    }

    pub fn style(&self) -> Style {
        match self {
            StatusIcon::Working => Style::default().fg(Color::Yellow),
            StatusIcon::Done => Style::default().fg(Color::Green),
            StatusIcon::Error => Style::default().fg(Color::Red),
            StatusIcon::Disconnected => Style::default().fg(Color::Red),
        }
    }
}

// ── Welcome Box (scrolls with messages) ──────────────────────
//
// Built as box-drawing text lines so it lives in the normal scrollback
// (PageUp reveals it) rather than being a pinned widget. Prepended to the
// message stream inside `render_messages`.

/// Data needed to render the welcome block.
pub struct WelcomeSpec<'a> {
    pub version: &'a str,
    pub model: &'a str,
    pub provider: &'a str,
    pub project: &'a str,
}

/// Build the welcome block as a little agent robot (4 borderless lines):
///
///   ▗▄▄▄▖  Jia v{version}
///   ▌▘ ▝▐
///   ▝▀▀▀▘  {model} · {provider}
///     █    ~/{project}
///
/// Round head + two eyes + centered hanging "beard" — nods to 甲's frame
/// and downward stroke. Lives in the normal scrollback (PageUp reveals it),
/// prepended to the message stream inside `render_messages`.
pub fn welcome_lines(spec: &WelcomeSpec) -> Vec<ChatLine> {
    let cyan = Style::default().fg(Color::Cyan);
    let white = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::Indexed(245));

    let model_label = if spec.model.is_empty() {
        spec.provider.to_string()
    } else {
        format!("{} · {}", spec.model, spec.provider)
    };
    let path_label = if spec.project.is_empty() {
        String::new()
    } else {
        format!("~/{}", spec.project)
    };

    vec![
        ChatLine {
            text: format!("▗▄▄▄▖  Jia v{}", spec.version),
            style: cyan,
        },
        ChatLine {
            text: "▌▘ ▝▐  ".to_string(),
            style: cyan,
        },
        ChatLine {
            text: format!("▝▀▀▀▘  {}", model_label),
            style: white,
        },
        ChatLine {
            text: format!("  █    {}", path_label),
            style: dim,
        },
    ]
}

// ── Layout ─────────────────────────────────────────────────

pub struct LayoutAreas {
    pub messages: Rect,
    pub status_bar: Rect,
    pub input: Rect,
    pub info_bar: Rect,
}

/// `input_height` = separator(1) + composer lines + separator(1); clamped to [3, 8].
pub fn layout(area: Rect, input_height: u16) -> LayoutAreas {
    let input_len = input_height.clamp(3, 8);
    let [messages, _gap, status_bar, input, info_bar] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1), // 空行（消息与状态栏间距）
        Constraint::Length(1), // 状态栏：模式 · 图标 · 用时
        Constraint::Length(input_len),
        Constraint::Length(1), // 信息栏：模型 · 会话ID · 项目
    ])
    .areas(area);

    LayoutAreas {
        messages,
        status_bar,
        input,
        info_bar,
    }
}

// ── Header ─────────────────────────────────────────────────
// (removed — header folded into the status bar)

// ── Messages ───────────────────────────────────────────────

// ── Markdown Rendering ─────────────────────────────────────
//
// Parses assistant messages (style == Style::default()) through
// pulldown-cmark to produce styled ratatui Lines.  Headings, bold,
// italic, code blocks, inline code, lists, blockquotes, and links
// are rendered with distinct terminal styles.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Styles used for markdown elements.
fn heading_style(level: usize) -> Style {
    match level {
        1 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        2 => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        3 => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    }
}

fn code_style() -> Style {
    Style::default().fg(Color::Indexed(245))
}

fn inline_code_style() -> Style {
    Style::default().fg(Color::Cyan)
}

fn blockquote_style() -> Style {
    Style::default()
        .fg(Color::Indexed(242))
        .add_modifier(Modifier::ITALIC)
}

fn link_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED)
}

fn bullet_style() -> Style {
    Style::default().fg(Color::Green)
}

/// Parse a markdown string into styled ratatui Lines.
///
/// Each paragraph, heading, code block, list item, and blockquote
/// produces one or more `Line<'static>` values. Blank lines separate
/// blocks visually.
fn parse_markdown_to_spans(text: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut code_buf = String::new();
    let mut list_index: Option<u64> = None;
    let mut list_item_count: u64 = 0;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let lvl = match level {
                        pulldown_cmark::HeadingLevel::H1 => 1,
                        pulldown_cmark::HeadingLevel::H2 => 2,
                        pulldown_cmark::HeadingLevel::H3 => 3,
                        pulldown_cmark::HeadingLevel::H4 => 4,
                        pulldown_cmark::HeadingLevel::H5 => 5,
                        pulldown_cmark::HeadingLevel::H6 => 6,
                    };
                    style_stack.push(heading_style(lvl));
                }
                Tag::Strong => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.add_modifier(Modifier::BOLD));
                }
                Tag::Emphasis => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.add_modifier(Modifier::ITALIC));
                }
                Tag::Strikethrough => {
                    let base = current_style(&style_stack);
                    style_stack.push(
                        base.add_modifier(Modifier::DIM)
                            .add_modifier(Modifier::CROSSED_OUT),
                    );
                }
                Tag::CodeBlock(kind) => {
                    in_code_block = true;
                    code_buf.clear();
                    code_lang = match kind {
                        CodeBlockKind::Fenced(lang) => lang.to_string(),
                        CodeBlockKind::Indented => String::new(),
                    };
                }
                Tag::Link { dest_url, .. } => {
                    let base = current_style(&style_stack);
                    // Merge link style on top of whatever is active
                    let merged = merge_style(base, link_style());
                    style_stack.push(merged);
                    // Store URL for tooltip-like suffix (ignored for now)
                    let _ = dest_url;
                }
                Tag::BlockQuote(_) => {
                    style_stack.push(blockquote_style());
                }
                Tag::List(start) => {
                    list_index = start;
                    list_item_count = 0;
                }
                Tag::Item => {
                    list_item_count += 1;
                    if let Some(idx) = list_index {
                        let bullet = format!("  {}. ", idx + list_item_count - 1);
                        spans.push(Span::styled(bullet, Style::default().fg(Color::Green)));
                    } else {
                        spans.push(Span::styled("  • ", bullet_style()));
                    }
                }
                Tag::Paragraph => {}
                _ => {}
            },

            Event::End(tag) => match tag {
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    if !spans.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut spans)));
                    }
                }
                TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    // Render code block with optional language label
                    if !code_lang.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!("  ╭─ {} ", code_lang),
                            code_style(),
                        )));
                    } else {
                        lines.push(Line::from(Span::styled("  ╭─ code", code_style())));
                    }
                    for code_line in code_buf.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  │ {}", code_line),
                            code_style(),
                        )));
                    }
                    lines.push(Line::from(Span::styled("  ╰─", code_style())));
                    code_buf.clear();
                    code_lang.clear();
                }
                TagEnd::Paragraph => {
                    if !spans.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut spans)));
                    }
                }
                TagEnd::BlockQuote(_) => {
                    style_stack.pop();
                    // Flush any remaining spans in blockquote
                    if !spans.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut spans)));
                    }
                }
                TagEnd::List(_) => {
                    list_index = None;
                    list_item_count = 0;
                    if !spans.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut spans)));
                    }
                }
                TagEnd::Item => {
                    if !spans.is_empty() {
                        lines.push(Line::from(std::mem::take(&mut spans)));
                    }
                }
                _ => {}
            },

            Event::Text(t) => {
                if in_code_block {
                    code_buf.push_str(&t);
                } else {
                    let style = current_style(&style_stack);
                    spans.push(Span::styled(t.to_string(), style));
                }
            }

            Event::Code(c) => {
                let style = inline_code_style();
                spans.push(Span::styled(format!("`{}`", c), style));
            }

            Event::SoftBreak | Event::HardBreak => {
                if in_code_block {
                    code_buf.push('\n');
                } else if !spans.is_empty() {
                    lines.push(Line::from(std::mem::take(&mut spans)));
                }
            }

            Event::Rule => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(40),
                    Style::default().fg(Color::Indexed(242)),
                )));
            }

            _ => {}
        }
    }

    // Flush any trailing spans
    if !spans.is_empty() {
        lines.push(Line::from(spans));
    }

    lines
}

/// Get the currently active style from the style stack, or default.
fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

/// Merge two styles: `overlay` takes precedence for fields it sets.
fn merge_style(base: Style, overlay: Style) -> Style {
    let mut s = base;
    if overlay.fg.is_some() {
        s.fg = overlay.fg;
    }
    if overlay.bg.is_some() {
        s.bg = overlay.bg;
    }
    s = s.add_modifier(overlay.add_modifier);
    s
}

// ── Styled Line Wrapping ───────────────────────────────────
//
// Wraps a sequence of styled spans into display rows, preserving
// the style of each character.  Handles explicit newlines, word
// boundaries, and long words that must be broken mid-character.

/// Wrap styled spans into display rows of at most `max_width` columns.
/// Each output [`Line`] is a single visual row with per-character styling.
fn wrap_styled_lines(spans: &[Span<'static>], max_width: usize) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![Line::from("")];
    }

    let mut rows: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut cw: usize = 0;

    // Flush the current row and start a new one.
    macro_rules! flush_row {
        () => {
            rows.push(Line::from(std::mem::take(&mut cur)));
            #[allow(unused_assignments)]
            {
                cw = 0;
            }
        };
    }

    for span in spans {
        let style = span.style;

        // Split on explicit newlines first.
        for (seg_idx, segment) in span.content.split('\n').enumerate() {
            if seg_idx > 0 {
                flush_row!();
            }
            if segment.is_empty() {
                continue;
            }

            // Word-level wrapping.
            for word in segment.split_inclusive(|c: char| c.is_whitespace()) {
                let ww = UnicodeWidthStr::width(word);

                if cw + ww <= max_width {
                    // Fits on the current row.
                    cur.push(Span::styled(word.to_string(), style));
                    cw += ww;
                } else if ww > max_width {
                    // Word itself is wider than a row — break character by character.
                    for ch in word.chars() {
                        let chw = UnicodeWidthChar::width(ch).unwrap_or(0);
                        if cw + chw > max_width && !cur.is_empty() {
                            flush_row!();
                        }
                        cur.push(Span::styled(ch.to_string(), style));
                        cw += chw;
                    }
                } else {
                    // Word doesn't fit — wrap to next row.
                    if !cur.is_empty() {
                        flush_row!();
                    }
                    cur.push(Span::styled(word.to_string(), style));
                    cw = ww;
                }
            }
        }
    }

    // Flush the last row (even if empty, to represent a blank line).
    if !cur.is_empty() || rows.is_empty() {
        rows.push(Line::from(cur));
    }

    rows
}

// ── Display Row Builder ────────────────────────────────────
//
// Flattens all chat lines (including the welcome box) into individual
// display rows.  Each returned [`Line`] occupies exactly one visual row,
// so scroll offsets map 1-to-1 with display rows.
//
// Assistant messages (style == Style::default()) are parsed through the
// markdown renderer; all other lines are treated as pre-styled plain text.

pub fn build_display_rows(
    welcome_lines: &[ChatLine],
    lines: &[ChatLine],
    width: u16,
) -> Vec<Line<'static>> {
    let col_max = (width as usize).max(1);
    let mut rows: Vec<Line<'static>> = Vec::new();

    // Welcome box lines — already padded to exact width, pass through as-is.
    for cl in welcome_lines {
        if let Some(first_line) = cl.text.lines().next() {
            rows.push(Line::from(Span::styled(first_line.to_string(), cl.style)));
        }
    }

    // Chat lines — markdown for assistant, plain styled for the rest.
    let default_style = Style::default();
    for cl in lines {
        if cl.text.is_empty() {
            rows.push(Line::from(Span::styled(String::new(), cl.style)));
            continue;
        }

        if cl.style == default_style {
            // Assistant message → parse markdown, then wrap styled spans.
            let md_lines = parse_markdown_to_spans(&cl.text);
            for md_line in &md_lines {
                let wrapped = wrap_styled_lines(&md_line.spans, col_max);
                rows.extend(wrapped);
            }
        } else {
            // Pre-styled line (tool result, error, system, etc.) — wrap
            // preserving the single style.
            let styled = vec![Span::styled(cl.text.clone(), cl.style)];
            let wrapped = wrap_styled_lines(&styled, col_max);
            rows.extend(wrapped);
        }
    }

    rows
}

/// Number of display rows for `lines` at `width` (sizes `insert_before` height).
pub fn count_display_rows(lines: &[ChatLine], width: u16) -> usize {
    build_display_rows(&[], lines, width).len()
}

/// Render chat lines into a buffer — used by `terminal.insert_before` to push
/// finalized content into the terminal's native scrollback.
pub fn render_chatlines_to_buffer(
    buf: &mut ratatui::buffer::Buffer,
    lines: &[ChatLine],
    width: u16,
) {
    let display_rows = build_display_rows(&[], lines, width);
    Paragraph::new(Text::from(display_rows)).render(buf.area, buf);
    blank_wide_placeholders(buf);
}

/// Render pre-built display lines (already wrapped/parsed) into a buffer —
/// for `insert_before` of a display-row slice during streaming.
pub fn render_lines_to_buffer(buf: &mut ratatui::buffer::Buffer, lines: &[Line<'static>]) {
    Paragraph::new(Text::from(lines.to_vec())).render(buf.area, buf);
    blank_wide_placeholders(buf);
}

/// ratatui's `insert_before` draws every buffer cell, including the reset
/// placeholder after a wide (CJK) grapheme. crossterm's backend then `Print`s
/// that placeholder (a space), clobbering the CJK char's 2nd terminal column
/// and garbling spacing. Blank those placeholders so `Print` is a no-op on them.
/// (Frame rendering is unaffected — it diffs buffers and skips unchanged cells.)
fn blank_wide_placeholders(buf: &mut ratatui::buffer::Buffer) {
    let w = buf.area.width as usize;
    let h = buf.area.height as usize;
    for y in 0..h {
        let mut x = 0usize;
        while x < w {
            let idx = y * w + x;
            let cw = UnicodeWidthStr::width(buf.content[idx].symbol());
            if cw >= 2 && x + 1 < w {
                buf.content[idx + 1].set_symbol("");
            }
            x += 1;
        }
    }
}

/// Render only the active (current turn) lines into the viewport, bottom-aligned.
/// History lives in the terminal scrollback (pushed via insert_before).
pub fn render_messages(f: &mut Frame, area: Rect, lines: &[ChatLine]) {
    let visible_height = area.height as usize;
    if visible_height == 0 {
        return;
    }

    let display_rows = build_display_rows(&[], lines, area.width);
    let total = display_rows.len();

    // Bottom-align: show the last visible_height rows of the active turn.
    let skip = total.saturating_sub(visible_height);

    let visible: Vec<Line> = display_rows
        .into_iter()
        .skip(skip)
        .take(visible_height)
        .collect();

    // No Paragraph::wrap — our build_display_rows already wraps each line
    // to `area.width` display columns with correct per-character styles.
    let msg_widget = Paragraph::new(Text::from(visible));
    f.render_widget(msg_widget, area);
}

// ── Status Line ────────────────────────────────────────────

static SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// 状态栏 — 输入框上方：状态图标 · 九星相位 · 本轮用时
pub fn render_status_bar(
    f: &mut Frame,
    area: Rect,
    status: StatusIcon,
    geju: &str,
    elapsed_secs: u64,
    reconnect_attempts: u32,
    spinner_idx: usize,
    is_reasoning: bool,
) {
    let (icon, icon_style) = if status == StatusIcon::Working {
        if is_reasoning {
            // Purple ✻ during thinking — Claude Code style
            (
                "✻".to_string(),
                Style::default().fg(Color::Indexed(183)),
            )
        } else {
            (
                SPINNER[spinner_idx % 10].to_string(),
                Style::default().fg(Color::Yellow),
            )
        }
    } else {
        (status.as_str().to_string(), status.style())
    };
    let tail = if status == StatusIcon::Disconnected {
        format!(" · reconnect #{}", reconnect_attempts)
    } else {
        format!(" · {}s", elapsed_secs)
    };

    let mid = if geju.is_empty() {
        String::new()
    } else {
        format!(" {geju} ·")
    };

    let text_style = Style::default().fg(Color::Indexed(245));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(icon, icon_style),
            Span::styled(
                format!("{mid}{tail}"),
                text_style,
            ),
        ])),
        area,
    );
}

/// 信息栏 — 输入框下方：模式 · 模型 · 会话ID · 项目路径
pub fn render_info_bar(
    f: &mut Frame,
    area: Rect,
    mode_label: &str,
    model: &str,
    session_id: Option<&str>,
    project: &str,
) {
    let white = Style::default().fg(Color::White);
    let sid = session_id
        .map(|s| if s.len() > 8 { &s[..8] } else { s })
        .unwrap_or("·");

    let left_text = if mode_label.is_empty() {
        format!("⏵⏵ {} · {}", model, sid)
    } else {
        format!("⏵⏵ {} · {} · {}", mode_label, model, sid)
    };

    let mid = area.width.saturating_sub(30).max(area.width / 2);
    let left = Rect { width: mid, ..area };
    let right = Rect {
        x: area.x + mid,
        width: area.width.saturating_sub(mid),
        ..area
    };

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            left_text,
            Style::default().fg(Color::Indexed(245)),
        ))),
        left,
    );

    if !project.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(format!("~/{}", project), white)))
                .alignment(ratatui::layout::Alignment::Right),
            right,
        );
    }
}

// ── Input Area ────────────────────────────────────────────
// separator / ❯ prompt + composer / separator. Returns cursor position.

pub fn render_input(f: &mut Frame, area: Rect, composer: &Composer) -> Option<(u16, u16)> {
    let input_height = area.height.saturating_sub(2).max(1);
    let [top_sep, input_area, bot_sep] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(area);

    let sep_line = "─".repeat(area.width as usize);
    let sep_style = Style::default().fg(Color::Cyan);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&sep_line, sep_style))),
        top_sep,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(&sep_line, sep_style))),
        bot_sep,
    );

    // ❯ prompt + composer text
    let [prompt, text_area] =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).areas(input_area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "❯ ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))),
        prompt,
    );
    // composer.render returns absolute cursor coords (already offset by text_area.x).
    composer.render(f, text_area)
}

// ── Tool Card Builder ──────────────────────────────────────

/// Build a ChatLine for a tool result with geju + execution_mode annotation.
pub fn format_tool_result(
    tool: &str,
    output: &str,
    geju: Option<&str>,
    execution_mode: Option<&str>,
    error: Option<&str>,
) -> Vec<ChatLine> {
    let (mode_style, mode_icon) = match execution_mode {
        Some("direct") => (Style::default().fg(Color::Green), "✓"),
        Some("guarded") => (Style::default().fg(Color::Yellow), "⚠"),
        Some("sandbox") => (Style::default().fg(Color::Indexed(208)), "🔶"),
        Some("denied") => (
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            "✗",
        ),
        _ => (Style::default().fg(Color::Indexed(245)), "·"),
    };

    let geju_str = geju.unwrap_or("");
    let mode_str = execution_mode.unwrap_or("");

    if let Some(err) = error
        && !err.is_empty()
    {
        let mut lines = vec![ChatLine {
            text: format!("  └ {tool} ({geju_str} · {mode_str}) — {mode_icon} ERROR: {err}"),
            style: Style::default().fg(Color::Red),
        }];
        // Show truncated output even on error
        if !output.is_empty() {
            let preview = if output.len() > 500 {
                format!("{}...", &output[..500])
            } else {
                output.to_string()
            };
            lines.push(ChatLine {
                text: preview,
                style: Style::default(),
            });
        }
        lines
    } else {
        let mut lines = vec![ChatLine {
            text: format!("  └ {tool} ({geju_str} · {mode_str}) — {mode_icon}"),
            style: mode_style,
        }];
        if !output.is_empty() {
            lines.push(ChatLine {
                text: output.to_string(),
                style: Style::default(),
            });
        }
        lines
    }
}

/// Build a ChatLine for a tool call.
pub fn format_tool_call(tool: &str, input: &str) -> ChatLine {
    let truncated = if input.len() > 200 {
        // Truncate at 200 chars (not bytes) to avoid splitting multi-byte UTF-8
        // codepoints (e.g. Chinese, emoji). char_indices gives byte offsets; the
        // last take(200) item's byte end is a valid char boundary.
        let end = input
            .char_indices()
            .take(200)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}...", &input[..end])
    } else {
        input.to_string()
    };
    ChatLine {
        text: format!("🔧 {tool} — {truncated}"),
        style: Style::default().fg(Color::Yellow),
    }
}

/// Build a ChatLine for a confirmation request.
pub fn format_confirm_request(tool: &str, reason: &str, timeout_secs: u64) -> ChatLine {
    ChatLine {
        text: format!("⚠ {tool} — {reason} (timeout: {timeout_secs}s)"),
        style: Style::default().fg(Color::Yellow),
    }
}

/// Build a ChatLine for a user question.
pub fn format_user_question(question: &str, _timeout_secs: u64) -> ChatLine {
    ChatLine {
        text: format!("❓ {question}"),
        style: Style::default().fg(Color::Cyan),
    }
}

/// Style for the currently selected option.
pub fn option_selected_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

/// Style for unselected options.
pub fn option_normal_style() -> Style {
    Style::default()
}

/// Build ChatLines for multiple-choice options. The selected index gets
/// highlighted. Caller is responsible for updating styles on arrow-key
/// navigation (use `update_option_style`).
pub fn format_question_options(options: &[String], selected: usize) -> Vec<ChatLine> {
    let mut lines = Vec::with_capacity(options.len() + 1);

    // Blank separator after question text
    lines.push(ChatLine {
        text: String::new(),
        style: Style::default(),
    });

    for (i, opt) in options.iter().enumerate() {
        let num = i + 1;
        let prefix = if i == selected { "❯ " } else { "  " };
        let text = if num <= 9 {
            format!("{prefix}{num}. {opt}")
        } else {
            format!("{prefix}{num:>2}. {opt}")
        };
        let style = if i == selected {
            option_selected_style()
        } else {
            option_normal_style()
        };
        lines.push(ChatLine { text, style });
    }

    lines
}

/// Update the style and text of an option ChatLine — called on arrow-key
/// navigation to swap the old selected and new selected states.
pub fn update_option_style(line: &mut ChatLine, is_selected: bool) {
    line.style = if is_selected {
        option_selected_style()
    } else {
        option_normal_style()
    };
    // Swap the arrow prefix: "❯ " (4 bytes) ↔ "  " (2 bytes)
    if is_selected && line.text.starts_with("  ") {
        line.text.replace_range(..2, "❯ ");
    } else if !is_selected && line.text.starts_with("❯ ") {
        line.text.replace_range(..4, "  ");
    }
}

/// Convert a StreamEvent into chat lines (may produce 0, 1, or 2 lines).
pub fn stream_event_to_lines(event: &StreamEvent) -> Vec<ChatLine> {
    match event {
        StreamEvent::Delta { content } => {
            vec![ChatLine {
                text: content.clone(),
                style: Style::default().fg(Color::White),
            }]
        }
        StreamEvent::ToolCall { tool, input } => {
            let input_str = serde_json::to_string(input).unwrap_or_default();
            vec![format_tool_call(tool, &input_str)]
        }
        StreamEvent::ToolResult {
            tool,
            output,
            error,
            geju,
            execution_mode,
        } => {
            format_tool_result(
                tool,
                output,
                geju.as_deref(),
                execution_mode.as_deref(),
                error.as_deref(),
            )
        }
        StreamEvent::ConfirmationRequest {
            tool,
            reason,
            timeout_secs,
            ..
        } => {
            vec![format_confirm_request(tool, reason, *timeout_secs)]
        }
        StreamEvent::UserQuestion {
            question,
            timeout_secs,
            ..
        } => {
            vec![format_user_question(question, *timeout_secs)]
        }
        StreamEvent::Error { message } => {
            vec![ChatLine {
                text: format!("✗ Error: {message}"),
                style: Style::default().fg(Color::Red),
            }]
        }
        _ => vec![],
    }
}

// ── Confirmation Prompt ─────────────────────────────────────
// (removed — confirm requests render inline as ChatLines via stream_event_to_lines)

// ── Security Guide (Claude-style project trust check) ────────

pub fn render_security_guide(f: &mut Frame, area: Rect, cwd: &str, selected: usize) {
    let w = area.width.saturating_sub(2).max(40) as usize;
    let hr = "─".repeat(w);

    let opt1 = if selected == 0 {
        " ❯ 1. Yes, I trust this folder"
    } else {
        "   1. Yes, I trust this folder"
    };
    let opt2 = if selected == 1 {
        " ❯ 2. No, exit"
    } else {
        "   2. No, exit"
    };

    let sel = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::Indexed(245));

    let lines = vec![
        Line::from(Span::styled(&hr, dim)),
        Line::from(""),
        Line::from(Span::styled(
            "Accessing workspace:",
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            cwd.to_string(),
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Quick safety check: Is this a project you created or one you trust? (Like your own code, a well-known open source project, or work from your team). If not, take a moment to review what's in this folder first.",
            dim,
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Jia will be able to read, edit, and execute files here.",
            dim,
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Security guide",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(opt1, if selected == 0 { sel } else { dim })),
        Line::from(Span::styled(opt2, if selected == 1 { sel } else { dim })),
        Line::from(""),
        Line::from(Span::styled("Enter to confirm · Esc to cancel", dim)),
    ];

    let p = Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

// ── Welcome Screen ──────────────────────────────────────────
// (removed — welcome is now box-drawing text via `welcome_lines`,
//  prepended to the message stream so it scrolls with the chat.)

#[cfg(test)]
mod tests {
    use super::*;

    /// Diagnostic: render the messages frame to an offscreen buffer and report
    /// which row the welcome box's top border lands on. Run with --nocapture.
    #[test]
    fn welcome_lines_robot_logo() {
        let spec = WelcomeSpec {
            version: "0.2.0",
            model: "gemini-2.5-pro",
            provider: "gemini",
            project: "demo",
        };
        let lines = welcome_lines(&spec);

        // Four rows: head+version / eyes / chin+model·provider / beard+path.
        assert_eq!(lines.len(), 4, "rows: {:?}", lines);

        assert!(
            lines[0].text.contains("Jia v0.2.0"),
            "head: {:?}",
            lines[0].text
        );
        assert!(
            lines[0].text.contains('▗'),
            "head logo: {:?}",
            lines[0].text
        );
        assert!(
            lines[1].text.contains('▘') && lines[1].text.contains('▝'),
            "eyes: {:?}",
            lines[1].text
        );
        assert!(
            lines[2].text.contains("gemini-2.5-pro · gemini"),
            "model: {:?}",
            lines[2].text
        );
        assert!(
            lines[3].text.contains("~/demo"),
            "path: {:?}",
            lines[3].text
        );
        assert!(lines[3].text.contains('█'), "beard: {:?}", lines[3].text);
    }

    #[test]
    fn welcome_lines_handles_empty_model_and_project() {
        let spec = WelcomeSpec {
            version: "0.2.0",
            model: "",
            provider: "gemini",
            project: "",
        };
        let lines = welcome_lines(&spec);
        assert_eq!(lines.len(), 4);
        // Empty model → line 3 (chin row) shows provider without "·".
        assert!(
            lines[2].text.contains("gemini") && !lines[2].text.contains('·'),
            "line3: {:?}",
            lines[2].text
        );
        // Empty project → line 4 (beard row) has no path.
        assert!(!lines[3].text.contains("~/"), "line4: {:?}", lines[3].text);
    }

    // ── Markdown Rendering Tests ─────────────────────────────

    #[test]
    fn parse_markdown_heading_levels() {
        let text = "# H1\n## H2\n### H3\n\nplain text";
        let lines = parse_markdown_to_spans(text);
        // Should have heading lines + plain text line
        assert!(lines.len() >= 3, "got {} lines: {:?}", lines.len(), lines);
        // First line should be H1 (cyan, bold)
        let h1 = &lines[0];
        assert!(!h1.spans.is_empty());
        let style = h1.spans[0].style;
        assert_eq!(style.fg, Some(Color::Cyan));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn parse_markdown_code_block() {
        let text = "```rust\nfn main() {}\n```";
        let lines = parse_markdown_to_spans(text);
        // Should have: language label, code line, closing bar
        assert!(lines.len() >= 3, "got {} lines: {:?}", lines.len(), lines);
        // Language label should mention "rust"
        let label_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(label_text.contains("rust"), "label: {label_text}");
        // Code content
        let code_text: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(code_text.contains("fn main()"), "code: {code_text}");
    }

    #[test]
    fn parse_markdown_inline_code() {
        let text = "use `foo::bar` here";
        let lines = parse_markdown_to_spans(text);
        assert_eq!(lines.len(), 1);
        // Should have 3 spans: "use ", "`foo::bar`", " here"
        assert!(lines[0].spans.len() >= 3, "spans: {:?}", lines[0].spans);
        let code_span = &lines[0].spans[1];
        assert!(code_span.content.contains("foo::bar"));
        assert_eq!(code_span.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn parse_markdown_bold_and_italic() {
        let text = "**bold** and *italic*";
        let lines = parse_markdown_to_spans(text);
        assert!(!lines.is_empty());
        // Find the bold span
        let bold_span = lines[0]
            .spans
            .iter()
            .find(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .expect("should have bold span");
        assert!(bold_span.content.contains("bold"));
        // Find the italic span
        let italic_span = lines[0]
            .spans
            .iter()
            .find(|s| s.style.add_modifier.contains(Modifier::ITALIC))
            .expect("should have italic span");
        assert!(italic_span.content.contains("italic"));
    }

    #[test]
    fn parse_markdown_unordered_list() {
        let text = "- item one\n- item two\n- item three";
        let lines = parse_markdown_to_spans(text);
        // Each list item should produce its own line with bullet
        assert!(lines.len() >= 3, "got {} lines: {:?}", lines.len(), lines);
        for line in &lines[..3] {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(text.contains("•"), "missing bullet: {text}");
        }
    }

    #[test]
    fn wrap_styled_lines_preserves_styles() {
        let spans = vec![
            Span::styled("hello ", Style::default().fg(Color::Red)),
            Span::styled("world", Style::default().fg(Color::Blue)),
        ];
        let rows = wrap_styled_lines(&spans, 20);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans.len(), 2);
        assert_eq!(rows[0].spans[0].style.fg, Some(Color::Red));
        assert_eq!(rows[0].spans[1].style.fg, Some(Color::Blue));
    }

    #[test]
    fn wrap_styled_lines_breaks_at_width() {
        let spans = vec![Span::styled("abcdefgh", Style::default().fg(Color::White))];
        let rows = wrap_styled_lines(&spans, 4);
        assert_eq!(rows.len(), 2);
        let row0: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        let row1: String = rows[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(row0, "abcd");
        assert_eq!(row1, "efgh");
        // Both rows should preserve the style
        assert_eq!(rows[0].spans[0].style.fg, Some(Color::White));
        assert_eq!(rows[1].spans[0].style.fg, Some(Color::White));
    }

    #[test]
    fn build_display_rows_uses_markdown_for_assistant() {
        let welcome = Vec::new();
        let lines = vec![
            ChatLine {
                text: "Hello **world**".to_string(),
                style: Style::default(), // assistant → markdown
            },
            ChatLine {
                text: "system info".to_string(),
                style: Style::default().fg(Color::Yellow), // not markdown
            },
        ];
        let rows = build_display_rows(&welcome, &lines, 80);
        // Assistant line should have a bold span from markdown parsing
        let has_bold = rows.iter().any(|line| {
            line.spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        });
        assert!(
            has_bold,
            "assistant text should be markdown-parsed with bold"
        );

        // System line should be plain styled yellow
        let yellow_line = rows
            .iter()
            .find(|line| line.spans.iter().any(|s| s.style.fg == Some(Color::Yellow)));
        assert!(yellow_line.is_some(), "system line should be present");
    }
}

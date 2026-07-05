//! TUI widgets: welcome box, layout, and input area.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::composer::Composer;
use super::render::ChatLine;

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
pub(crate) fn welcome_lines(spec: &WelcomeSpec) -> Vec<ChatLine> {
    let cyan = Style::default().fg(Color::Cyan);
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
            text: "▗▄▄▄▖".to_string(),
            style: cyan,
        },
        ChatLine {
            text: format!("▌▘ ▝▐  Jia v{}", spec.version),
            style: cyan,
        },
        ChatLine {
            text: format!("▝▀▀▀▘  {}", model_label),
            style: dim,
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
pub(crate) fn layout(area: Rect, input_height: u16) -> LayoutAreas {
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

// ── Input Area ────────────────────────────────────────────
// separator / ❯ prompt + composer / separator. Returns cursor position.

pub(crate) fn render_input(f: &mut Frame, area: Rect, composer: &Composer) -> Option<(u16, u16)> {
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

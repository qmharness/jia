//! Project trust check (Claude-style security guide).
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};

// ── Security Guide (Claude-style workspace trust check) ────────

pub(crate) fn render_security_guide(f: &mut Frame, area: Rect, cwd: &str, selected: usize) {
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
            "Quick safety check: Is this a workspace you created or one you trust? (Like your own code, a well-known open source project, or work from your team). If not, take a moment to review what's in this folder first.",
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

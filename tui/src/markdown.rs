//! Markdown-to-ratatui rendering: headings, bold, italic, code, lists, blockquotes, links.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};


// ── Markdown Rendering ─────────────────────────────────────
//
// Parses assistant messages (style == Style::default()) through
// pulldown-cmark to produce styled ratatui Lines.  Headings, bold,
// italic, code blocks, inline code, lists, blockquotes, and links
// are rendered with distinct terminal styles.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Styles used for markdown elements.
pub(crate) fn heading_style(level: usize) -> Style {
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

pub(crate) fn code_style() -> Style {
    Style::default().fg(Color::Indexed(245))
}

pub(crate) fn inline_code_style() -> Style {
    Style::default().fg(Color::Cyan)
}

pub(crate) fn blockquote_style() -> Style {
    Style::default()
        .fg(Color::Indexed(242))
        .add_modifier(Modifier::ITALIC)
}

pub(crate) fn link_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED)
}

pub(crate) fn bullet_style() -> Style {
    Style::default().fg(Color::Green)
}

/// Parse a markdown string into styled ratatui Lines.
///
/// Each paragraph, heading, code block, list item, and blockquote
/// produces one or more `Line<'static>` values. Blank lines separate
/// blocks visually.
pub(crate) fn parse_markdown_to_spans(text: &str) -> Vec<Line<'static>> {
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
pub(crate) fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

/// Merge two styles: `overlay` takes precedence for fields it sets.
pub(crate) fn merge_style(base: Style, overlay: Style) -> Style {
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


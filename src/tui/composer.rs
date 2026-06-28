// ── Composer ───────────────────────────────────────────────
//
// Multi-line text input with word wrapping. Enter → submit, Up/Down → history.
// Cursor position tracked as byte offset.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthChar;

pub struct Composer {
    text: String,
    cursor: usize, // byte offset
    placeholder: String,
    history: Vec<String>,
    history_pos: i32,
    draft: String,
}

impl Composer {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            placeholder: String::new(),
            history: Vec::new(),
            history_pos: -1,
            draft: String::new(),
        }
    }

    pub fn text(&self) -> String {
        self.text.clone()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn add_to_history(&mut self, text: &str) {
        if !text.is_empty() {
            self.history.push(text.to_string());
        }
        self.history_pos = -1;
        self.draft.clear();
    }

    pub fn set_placeholder(&mut self, text: &str) {
        self.placeholder = text.to_string();
    }

    // ── Cursor helpers ────────────────────────────────────

    fn clamp_byte(&self) -> usize {
        let mut p = self.cursor.min(self.text.len());
        while p > 0 && !self.text.is_char_boundary(p) {
            p -= 1;
        }
        p
    }

    fn cursor_left(&mut self) {
        let pos = self.clamp_byte();
        if pos > 0 {
            let mut prev = pos - 1;
            while prev > 0 && !self.text.is_char_boundary(prev) {
                prev -= 1;
            }
            self.cursor = prev;
        }
    }

    fn cursor_right(&mut self) {
        let pos = self.clamp_byte();
        if pos < self.text.len() {
            let mut next = pos + 1;
            while next < self.text.len() && !self.text.is_char_boundary(next) {
                next += 1;
            }
            self.cursor = next;
        }
    }

    fn cursor_end(&mut self) {
        self.cursor = self.text.len();
    }

    fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    fn reset_history(&mut self) {
        self.history_pos = -1;
        self.draft.clear();
    }

    // ── Handle key ────────────────────────────────────────

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        // Ctrl+P / Ctrl+N recall input history (↑/↓ now scroll the transcript).
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('p') => {
                    self.history_prev();
                    return false;
                }
                KeyCode::Char('n') => {
                    self.history_next();
                    return false;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Enter => {
                if !self.text.trim().is_empty() {
                    return true;
                }
            }
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            KeyCode::Left => self.cursor_left(),
            KeyCode::Right => self.cursor_right(),
            KeyCode::Home => self.cursor_home(),
            KeyCode::End => self.cursor_end(),
            KeyCode::Backspace => {
                let pos = self.clamp_byte();
                if pos > 0 {
                    let mut prev = pos - 1;
                    while prev > 0 && !self.text.is_char_boundary(prev) {
                        prev -= 1;
                    }
                    self.text.remove(prev);
                    self.cursor = prev;
                }
                self.reset_history();
            }
            KeyCode::Delete => {
                let pos = self.clamp_byte();
                if pos < self.text.len() {
                    let mut next = pos + 1;
                    while next < self.text.len() && !self.text.is_char_boundary(next) {
                        next += 1;
                    }
                    self.text.remove(pos);
                }
                self.reset_history();
            }
            KeyCode::Char(c) => {
                let pos = self.clamp_byte();
                self.text.insert(pos, c);
                self.cursor = pos + c.len_utf8();
                self.reset_history();
            }
            _ => {}
        }

        false
    }

    /// Recall the previous (older) input from history.
    fn history_prev(&mut self) {
        if !self.history.is_empty() && self.history_pos == -1 {
            self.draft = self.text.clone();
            self.history_pos = (self.history.len() - 1) as i32;
        } else if self.history_pos > 0 {
            self.history_pos -= 1;
        }
        if self.history_pos >= 0 {
            self.text = self.history[self.history_pos as usize].clone();
            self.cursor_end();
        }
    }

    /// Recall the next (newer) input from history.
    fn history_next(&mut self) {
        if self.history_pos >= 0 {
            self.history_pos += 1;
            if self.history_pos >= self.history.len() as i32 {
                self.text = self.draft.clone();
                self.history_pos = -1;
                self.draft.clear();
            } else {
                self.text = self.history[self.history_pos as usize].clone();
            }
            self.cursor_end();
        }
    }

    // ── Render ────────────────────────────────────────────

    /// Calculate how many visual lines this text occupies at the given width.
    pub fn line_count(&self, area_width: u16) -> usize {
        let content = if self.text.is_empty() {
            &self.placeholder
        } else {
            &self.text
        };
        if content.is_empty() {
            return 1;
        }
        let w = area_width.saturating_sub(2) as usize; // account for borders
        if w == 0 {
            return 1;
        }
        let mut lines = 1usize;
        let mut col = 0usize;
        for ch in content.chars() {
            let cw = if ch == '\n' {
                0
            } else {
                UnicodeWidthChar::width(ch).unwrap_or(1)
            };
            if ch == '\n' {
                lines += 1;
                col = 0;
            } else if col + cw > w {
                lines += 1;
                col = cw;
            } else {
                col += cw;
            }
        }
        lines.max(1)
    }

    pub fn render(&self, f: &mut Frame, area: Rect) -> Option<(u16, u16)> {
        let is_empty = self.text.is_empty();

        let display = if is_empty {
            Text::from(self.placeholder.as_str())
        } else {
            Text::from(self.text.as_str())
        };

        let style = if is_empty {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };

        let p = Paragraph::new(display)
            .style(style)
            .wrap(ratatui::widgets::Wrap { trim: false });
        f.render_widget(p, area);

        // Cursor position — calculate x,y accounting for wrapping
        let byte_pos = self.clamp_byte();
        let prefix = self.text.get(..byte_pos).unwrap_or("");
        let w = area.width as usize;
        let mut cx = 0usize;
        let mut cy = 0usize;
        for ch in prefix.chars() {
            let cw = if ch == '\n' {
                0
            } else {
                UnicodeWidthChar::width(ch).unwrap_or(1)
            };
            if ch == '\n' {
                cy += 1;
                cx = 0;
            } else if w > 0 && cx + cw > w {
                cy += 1;
                cx = cw;
            } else {
                cx += cw;
            }
        }
        let x = area.x + cx as u16;
        let y = area.y + cy as u16;
        Some((
            x.min(area.right().saturating_sub(1)),
            y.min(area.bottom().saturating_sub(1)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        code.into()
    }

    #[test]
    fn insert_ascii_advances_cursor() {
        let mut c = Composer::new();
        for ch in "abc".chars() {
            c.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(c.text(), "abc");
        assert_eq!(c.cursor, 3); // ASCII: byte offset == char count
        assert!(c.text().is_char_boundary(c.cursor));
    }

    #[test]
    fn insert_multibyte_keeps_char_boundary() {
        let mut c = Composer::new();
        for ch in "你好".chars() {
            c.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(c.text(), "你好");
        assert_eq!(c.cursor, 6); // 2 scalars × 3 bytes each (UTF-8)
        assert!(c.text().is_char_boundary(c.cursor));
    }

    #[test]
    fn backspace_deletes_one_scalar_not_half() {
        let mut c = Composer::new();
        for ch in "你好世界".chars() {
            c.handle_key(key(KeyCode::Char(ch)));
        }
        c.handle_key(key(KeyCode::Backspace));
        assert_eq!(c.text(), "你好世");
        assert_eq!(c.cursor, 9);
        assert!(c.text().is_char_boundary(c.cursor));
        // Repeated backspace must never leave a dangling sub-char boundary.
        c.handle_key(key(KeyCode::Backspace));
        assert_eq!(c.text(), "你好");
        assert!(std::str::from_utf8(c.text().as_bytes()).is_ok());
    }

    #[test]
    fn delete_key_removes_forward() {
        let mut c = Composer::new();
        for ch in "abc".chars() {
            c.handle_key(key(KeyCode::Char(ch)));
        }
        c.handle_key(key(KeyCode::Left)); // cursor: end → between a b|c
        assert_eq!(c.cursor, 2);
        c.handle_key(key(KeyCode::Delete)); // removes 'c' forward
        assert_eq!(c.text(), "ab");
    }

    #[test]
    fn enter_signals_ready_only_when_non_empty() {
        let mut c = Composer::new();
        assert!(!c.handle_key(key(KeyCode::Enter))); // empty → not ready
        c.handle_key(key(KeyCode::Char('h')));
        assert!(c.handle_key(key(KeyCode::Enter))); // non-empty → ready
        // Enter signals readiness but does not clear; caller clears.
        assert_eq!(c.text(), "h");
    }

    #[test]
    fn line_counts_newlines_and_wrapping() {
        let mut c = Composer::new();
        for ch in "ab\ncd".chars() {
            c.handle_key(key(KeyCode::Char(ch)));
        }
        // width 10 → 8 usable cols; "ab" + newline + "cd" = 2 visual lines
        assert_eq!(c.line_count(10), 2);

        // width 4 → 2 usable cols; "abcdef" wraps across 3 lines
        let mut c2 = Composer::new();
        for ch in "abcdef".chars() {
            c2.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(c2.line_count(4), 3);
    }
}

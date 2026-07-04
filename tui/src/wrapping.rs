//! Styled line wrapping: wraps ratatui spans into display rows.

use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};


// ── Styled Line Wrapping ───────────────────────────────────
//
// Wraps a sequence of styled spans into display rows, preserving
// the style of each character.  Handles explicit newlines, word
// boundaries, and long words that must be broken mid-character.

/// Wrap styled spans into display rows of at most `max_width` columns.
/// Each output [`Line`] is a single visual row with per-character styling.
pub(crate) fn wrap_styled_lines(spans: &[Span<'static>], max_width: usize) -> Vec<Line<'static>> {
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


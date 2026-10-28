//! Display-cell budgets shared by compact rows and complete operational messages.

use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// Only descriptive text is eligible for ellipsis; facts use `complete_lines` instead.
pub(crate) fn clip(text: &str, columns: usize) -> String {
    if text.width() <= columns {
        return text.into();
    }
    if columns == 0 {
        return String::new();
    }
    let mut used = 0;
    let prefix: String = text
        .graphemes(true)
        .take_while(|grapheme| {
            used += grapheme.width();
            used < columns
        })
        .collect();
    format!("{prefix}…")
}

/// Reflow complete words; an indivisible identity wider than the pane remains intact.
/// Callers may emit such messages durably rather than animate an overflowing frame.
pub(crate) fn complete_lines(text: &str, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.width() + 1 + word.width() > columns {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(line);
    lines
}

#[cfg(test)]
#[path = "../tests/unit/layout.rs"]
mod unit;

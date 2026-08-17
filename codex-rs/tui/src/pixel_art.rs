//! Pixel-art rendering for the Astra Code brand mark.
//!
//! The Astra wordmark is rendered as a fixed grid of `#` glyphs so it stays
//! stable across terminals (no animation, no color-only differentiation). Each
//! glyph cell is colored using the Astra orange accent when the terminal
//! supports true color, so the black/white/orange brand palette stays
//! consistent with the rest of the TUI.

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::theme::accent_style;
use crate::theme::accent_style_for;

/// Render the pixel-art "ASTRA" wordmark as a sequence of styled `Line`s.
///
/// Each glyph cell is colored using the Astra brand accent (orange). The
/// surrounding spaces stay on the default foreground so the wordmark reads as
/// "black background, white space, orange glyphs" in a standard terminal —
/// the canonical black/white/orange palette for Astra Code.
///
/// The returned vector contains one `Line` per pixel row, so call sites can
/// append the wordmark directly to an existing `Vec<Line>` or hand it to a
/// `Paragraph` widget for rendering.
pub(crate) fn astra_wordmark_lines() -> Vec<Line<'static>> {
    astra_wordmark_lines_for(None)
}

/// Same as [`astra_wordmark_lines`], but allows the caller to supply the
/// terminal background so the wordmark can pick a darker orange on light
/// backgrounds (matching [`crate::theme::accent_style_for`]).
pub(crate) fn astra_wordmark_lines_for(terminal_bg: Option<(u8, u8, u8)>) -> Vec<Line<'static>> {
    let glyph_style = match terminal_bg {
        Some(bg) => accent_style_for(Some(bg)),
        None => accent_style(),
    };
    ASTRA_PIXEL_GRID
        .iter()
        .map(|row| {
            let spans: Vec<Span<'static>> = row
                .chars()
                .map(|ch| match ch {
                    '#' => Span::styled("#".to_string(), glyph_style),
                    other => Span::styled(other.to_string(), Style::default()),
                })
                .collect();
            Line::from(spans)
        })
        .collect()
}

/// Width (in cells) of the rendered wordmark. Each letter is drawn on a
/// 3-cell-wide canvas; the five letters in `ASTRA` are separated by a single
/// space, giving `5 * 3 + 4 = 19` cells per row.
pub(crate) const ASTRA_WORDMARK_WIDTH: u16 = 19;

/// Height (in cells) of the rendered wordmark.
pub(crate) const ASTRA_WORDMARK_HEIGHT: u16 = 5;

/// 5-row pixel grid spelling "ASTRA".
///
/// Each letter uses a 3-cell-wide, 5-cell-tall block-font canvas with a
/// single inter-letter gap. The grid is left-aligned and padded with spaces
/// so every row is exactly [`ASTRA_WORDMARK_WIDTH`] cells wide, which keeps
/// the wordmark rendering and its width-based assertions deterministic.
const ASTRA_PIXEL_GRID: [&str; 5] = [
    " #  ### ### ##   # ",
    "# # #    #  # # # #",
    "###  #   #  ##  ###",
    "# #   #  #  # # # #",
    "# # ###  #  # # # #",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn astra_wordmark_has_expected_dimensions() {
        let lines = astra_wordmark_lines();
        assert_eq!(lines.len() as u16, ASTRA_WORDMARK_HEIGHT);
        assert_eq!(lines[0].width() as u16, ASTRA_WORDMARK_WIDTH);
    }

    #[test]
    fn astra_wordmark_uses_accent_color_for_glyphs() {
        let lines = astra_wordmark_lines();
        let glyph_style = lines[0]
            .spans
            .iter()
            .find(|span| span.content.as_ref() == "#")
            .expect("wordmark should contain glyph cells")
            .style;
        let accent = accent_style();
        assert_eq!(glyph_style.fg, accent.fg);
        assert!(
            glyph_style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }
}

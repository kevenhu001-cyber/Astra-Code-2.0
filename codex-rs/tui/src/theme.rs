//! Astra Code brand palette.
//!
//! Centralizes the brand colors used across the TUI so individual widgets do
//! not need to hardcode RGB values. Brand accent is orange; semantic colors
//! (success/error/links) are delegated to ANSI names so terminal themes keep
//! working as documented in `tui/styles.md`.

use ratatui::style::Color;
use ratatui::style::Style;

use crate::color::is_light;
use crate::terminal_palette::best_color;
use crate::terminal_palette::default_bg;
use crate::terminal_palette::rgb_color;

/// RGB triple of the Astra brand orange used on dark / unknown backgrounds.
pub const ACCENT_ORANGE_RGB: (u8, u8, u8) = (255, 165, 0);

/// Slightly darker orange used on light terminal backgrounds for contrast.
pub const ACCENT_ORANGE_LIGHT_BG_RGB: (u8, u8, u8) = (180, 90, 0);

/// Default foreground used by the TUI chrome. Most widgets should rely on
/// the terminal's default foreground rather than overriding it; this constant
/// is exposed for the rare case where a custom fg is required (e.g. overlays
/// rendered over a colored background).
pub const DEFAULT_FG: Color = Color::Reset;

/// Brand accent color used for Astra wordmarks, slash command help, plan
/// indicators and other places that previously used ANSI magenta.
pub const ACCENT: Color = Color::Rgb(255, 165, 0);

/// Returns the Astra brand style (bold orange) for the given terminal bg.
pub(crate) fn accent_style() -> Style {
    accent_style_for(default_bg())
}

pub(crate) fn accent_style_for(terminal_bg: Option<(u8, u8, u8)>) -> Style {
    if terminal_bg.is_some_and(is_light) {
        Style::default()
            .fg(best_color(ACCENT_ORANGE_LIGHT_BG_RGB))
            .bold()
    } else {
        Style::default().fg(rgb_color(ACCENT_ORANGE_RGB)).bold()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::Modifier;

    #[test]
    fn accent_is_darker_on_light_backgrounds() {
        let style = accent_style_for(Some((255, 255, 255)));
        assert_eq!(style.fg, Some(best_color(ACCENT_ORANGE_LIGHT_BG_RGB)));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn accent_is_orange_on_dark_backgrounds() {
        let expected = Style::default().fg(rgb_color(ACCENT_ORANGE_RGB)).bold();
        assert_eq!(accent_style_for(Some((0, 0, 0))), expected);
        assert_eq!(accent_style_for(/*terminal_bg*/ None), expected);
    }
}
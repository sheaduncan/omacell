//! ANSI chrome vs file-origin truecolor.

use ratatui::style::Color;

/// Whether file-origin hex colors may be sent as RGB.
#[must_use]
pub fn truecolor_enabled(setting: &str) -> bool {
    match setting {
        "on" => true,
        "off" => false,
        _ => std::env::var("COLORTERM")
            .map(|v| v.eq_ignore_ascii_case("truecolor") || v.eq_ignore_ascii_case("24bit"))
            .unwrap_or(false),
    }
}

/// Parse `#rrggbb` from a theme file. Ordinary chrome stays indexed.
#[must_use]
pub fn file_color(hex: &str, truecolor: bool) -> Color {
    if !truecolor {
        return Color::Indexed(6);
    }
    let h = hex.trim().trim_start_matches('#');
    if h.len() == 6
        && h.is_ascii()
        && h.bytes().all(|byte| byte.is_ascii_hexdigit())
        && let Ok(r) = u8::from_str_radix(&h[0..2], 16)
        && let Ok(g) = u8::from_str_radix(&h[2..4], 16)
        && let Ok(b) = u8::from_str_radix(&h[4..6], 16)
    {
        return Color::Rgb(r, g, b);
    }
    Color::Indexed(6)
}

/// Optional graphics protocol from `[tui] graphics`. Chart previews are WP-25.
#[must_use]
pub fn graphics_protocol(setting: &str) -> Option<&'static str> {
    match setting {
        "sixel" => Some("sixel"),
        "kitty" => Some("kitty"),
        "off" => None,
        _ => None,
    }
}

/// Indexed ANSI roles so Omarchy's terminal palette applies.
#[derive(Clone, Copy, Debug)]
pub struct AnsiRoles {
    /// Grid text.
    pub fg: Color,
    /// Headers.
    pub header: Color,
    /// Gridlines.
    pub grid: Color,
    /// Cursor.
    pub cursor: Color,
    /// Selection.
    pub selection: Color,
    /// Status / chrome.
    pub chrome: Color,
    /// Errors.
    pub error: Color,
    /// Alternate row fill.
    pub zebra: Color,
}

impl Default for AnsiRoles {
    fn default() -> Self {
        Self {
            fg: Color::Reset,
            header: Color::Indexed(6),
            grid: Color::Indexed(8),
            cursor: Color::Indexed(3),
            selection: Color::Indexed(4),
            chrome: Color::Indexed(7),
            error: Color::Indexed(1),
            zebra: Color::Indexed(8),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_on_off_are_explicit() {
        assert!(truecolor_enabled("on"));
        assert!(!truecolor_enabled("off"));
    }

    #[test]
    fn file_color_stays_indexed_without_truecolor() {
        assert_eq!(file_color("#ff00aa", false), Color::Indexed(6));
        assert_eq!(file_color("#ff00aa", true), Color::Rgb(255, 0, 170));
        assert_eq!(file_color("nope", true), Color::Indexed(6));
        assert_eq!(file_color("aébcd", true), Color::Indexed(6));
    }

    #[test]
    fn graphics_hook_is_named_for_wp25() {
        assert_eq!(graphics_protocol("sixel"), Some("sixel"));
        assert_eq!(graphics_protocol("kitty"), Some("kitty"));
        assert_eq!(graphics_protocol("off"), None);
        assert_eq!(graphics_protocol("auto"), None);
    }
}

//! ANSI chrome vs file-origin truecolor.

use ratatui::style::Color;

use omacell_conf::theme::ThemeRoles;
use omacell_core::chart::ChartTheme;

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

/// Explicit or environment-hinted protocol from `[tui] graphics`.
#[must_use]
pub fn graphics_protocol(setting: &str) -> Option<&'static str> {
    graphics_protocol_with(setting, |key| std::env::var_os(key))
}

pub(crate) fn graphics_query_allowed(setting: &str) -> bool {
    graphics_query_allowed_with(setting, |key| std::env::var_os(key))
}

/// Resolved theme colors for the shared chart scene.
#[must_use]
pub fn chart_theme(roles: &ThemeRoles) -> ChartTheme {
    let fallback = ChartTheme::neutral();
    let role = |key: &str, default: &str| {
        roles
            .roles
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    ChartTheme {
        background: role("surfaces.background", &fallback.background),
        foreground: role("text.foreground", &fallback.foreground),
        axis: role("charts.axis", &fallback.axis),
        gridline: role("charts.gridline", &fallback.gridline),
        palette: std::array::from_fn(|index| {
            role(&format!("charts.palette.{index}"), &fallback.palette[index])
        }),
    }
}

fn graphics_protocol_with<T>(
    setting: &str,
    env: impl Fn(&str) -> Option<T>,
) -> Option<&'static str> {
    match setting {
        "sixel" => Some("sixel"),
        "kitty" => Some("kitty"),
        "off" => None,
        _ if env("TMUX").is_some() || env("HERDR_PANE_ID").is_some() => None,
        _ if env("KITTY_WINDOW_ID").is_some() || env("GHOSTTY_RESOURCES_DIR").is_some() => {
            Some("kitty")
        }
        _ => None,
    }
}

fn graphics_query_allowed_with<T>(setting: &str, env: impl Fn(&str) -> Option<T>) -> bool {
    setting != "off"
        && (setting != "auto" || (env("TMUX").is_none() && env("HERDR_PANE_ID").is_none()))
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
    }

    #[test]
    fn graphics_auto_uses_terminal_hints_but_not_through_a_multiplexer() {
        let kitty = |key: &str| (key == "KITTY_WINDOW_ID").then_some("1");
        let ghostty = |key: &str| (key == "GHOSTTY_RESOURCES_DIR").then_some("/opt/ghostty");
        let tmux = |key: &str| match key {
            "TMUX" => Some("/tmp/tmux-1000/default,1,0"),
            "KITTY_WINDOW_ID" => Some("1"),
            _ => None,
        };
        assert_eq!(graphics_protocol_with("auto", kitty), Some("kitty"));
        assert_eq!(graphics_protocol_with("auto", ghostty), Some("kitty"));
        assert_eq!(graphics_protocol_with("auto", tmux), None);
        assert_eq!(graphics_protocol_with("sixel", tmux), Some("sixel"));
        assert_eq!(graphics_protocol_with("off", kitty), None);
        assert!(graphics_query_allowed_with("auto", kitty));
        assert!(!graphics_query_allowed_with("auto", tmux));
        assert!(graphics_query_allowed_with("sixel", tmux));
    }
}

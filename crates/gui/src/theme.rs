//! Map `LoadedConfig.theme.roles` onto egui visuals and grid colors.

use egui::{Color32, CornerRadius, Stroke, Visuals};
use omacell_conf::font::ShellTokens;
use omacell_conf::theme::ThemeRoles;

/// Parsed chrome + grid palette from resolved roles (no file parse here).
#[derive(Clone, Debug)]
pub struct GuiTheme {
    /// Theme name for the status line.
    pub name: String,
    /// `light` / `dark`.
    pub mode: String,
    /// Surfaces.
    pub background: Color32,
    /// Raised surface.
    pub surface: Color32,
    /// Header fill.
    pub header_background: Color32,
    /// Popup fill.
    pub popup_background: Color32,
    /// Body text.
    pub foreground: Color32,
    /// Muted text.
    pub muted: Color32,
    /// Header text.
    pub header_foreground: Color32,
    /// Grid line.
    pub grid_line: Color32,
    /// Frozen pane edge.
    pub frozen_edge: Color32,
    /// Cursor.
    pub cursor: Color32,
    /// Selection fill.
    pub selection: Color32,
    /// Selection outline.
    pub selection_border: Color32,
    /// Error.
    pub error: Color32,
    /// Success.
    pub success: Color32,
    /// Warning.
    pub warning: Color32,
    /// Stale hatch.
    pub stale: Color32,
    /// Formula reference cycle.
    pub references: [Color32; 8],
    /// Chart series cycle.
    pub charts: [Color32; 8],
    /// Chart axis.
    pub chart_axis: Color32,
    /// Chart gridline.
    pub chart_gridline: Color32,
    /// UI font size in points.
    pub ui_font_size_pt: f64,
    /// Corner rounding from shell tokens.
    pub rounding: CornerRadius,
}

impl GuiTheme {
    /// Build from a loaded config snapshot.
    #[must_use]
    pub fn from_loaded(roles: &ThemeRoles, shell: &ShellTokens) -> Self {
        let rounding = match shell.corner_style.as_deref() {
            Some("sharp") => CornerRadius::ZERO,
            _ => CornerRadius::same(4),
        };
        Self {
            name: roles.name.clone(),
            mode: roles.mode.clone(),
            background: role(roles, "surfaces.background"),
            surface: role(roles, "surfaces.surface"),
            header_background: role(roles, "surfaces.header_background"),
            popup_background: role(roles, "surfaces.popup_background"),
            foreground: role(roles, "text.foreground"),
            muted: role(roles, "text.muted"),
            header_foreground: role(roles, "text.header_foreground"),
            grid_line: role(roles, "structure.grid_line"),
            frozen_edge: role(roles, "structure.frozen_edge"),
            cursor: role(roles, "state.cursor"),
            selection: role(roles, "state.selection"),
            selection_border: role(roles, "state.selection_border"),
            error: role(roles, "semantic.error"),
            success: role(roles, "semantic.success"),
            warning: role(roles, "semantic.warning"),
            stale: role(roles, "state.stale"),
            references: std::array::from_fn(|i| role(roles, &format!("references.{i}"))),
            charts: std::array::from_fn(|i| role(roles, &format!("charts.palette.{i}"))),
            chart_axis: role(roles, "charts.axis"),
            chart_gridline: role(roles, "charts.gridline"),
            ui_font_size_pt: shell.ui_font_size_pt.max(9.0),
            rounding,
        }
    }

    /// Apply chrome colors to egui visuals. Cell file colors are untouched.
    pub fn apply_visuals(&self, ctx: &egui::Context) {
        let mut visuals = if self.mode == "light" {
            Visuals::light()
        } else {
            Visuals::dark()
        };
        visuals.panel_fill = self.background;
        visuals.window_fill = self.popup_background;
        visuals.override_text_color = Some(self.foreground);
        visuals.selection.bg_fill = self.selection;
        visuals.selection.stroke = Stroke::new(1.0_f32, self.selection_border);
        visuals.widgets.inactive.bg_fill = self.surface;
        visuals.widgets.hovered.bg_fill = self.surface;
        visuals.widgets.active.bg_fill = self.surface;
        visuals.window_corner_radius = self.rounding;
        visuals.menu_corner_radius = self.rounding;
        ctx.set_visuals(visuals);
        let mut style = (*ctx.style()).clone();
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::monospace(self.ui_font_size_pt as f32),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(self.ui_font_size_pt as f32),
        );
        ctx.set_style(style);
    }

    /// Theme tokens for the shared chart scene.
    #[must_use]
    pub fn chart_theme(&self) -> omacell_core::chart::ChartTheme {
        omacell_core::chart::ChartTheme {
            background: color_hex(self.background),
            foreground: color_hex(self.foreground),
            axis: color_hex(self.chart_axis),
            gridline: color_hex(self.chart_gridline),
            palette: std::array::from_fn(|i| color_hex(self.charts[i])),
        }
    }
}

fn color_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

fn role(roles: &ThemeRoles, key: &str) -> Color32 {
    roles
        .roles
        .get(key)
        .map(|value| hex_color(value))
        .unwrap_or(Color32::GRAY)
}

/// Parse `#rrggbb` / `#aarrggbb`.
#[must_use]
pub fn hex_color(hex: &str) -> Color32 {
    let h = hex.trim().trim_start_matches('#');
    if !h.is_ascii() {
        return Color32::GRAY;
    }
    if h.len() == 6
        && let Ok(r) = u8::from_str_radix(&h[0..2], 16)
        && let Ok(g) = u8::from_str_radix(&h[2..4], 16)
        && let Ok(b) = u8::from_str_radix(&h[4..6], 16)
    {
        return Color32::from_rgb(r, g, b);
    }
    if h.len() == 8
        && let Ok(a) = u8::from_str_radix(&h[0..2], 16)
        && let Ok(r) = u8::from_str_radix(&h[2..4], 16)
        && let Ok(g) = u8::from_str_radix(&h[4..6], 16)
        && let Ok(b) = u8::from_str_radix(&h[6..8], 16)
    {
        return Color32::from_rgba_unmultiplied(r, g, b, a);
    }
    Color32::GRAY
}

/// Load a font file into egui when the shell resolved a path.
pub fn install_font(ctx: &egui::Context, path: Option<&std::path::Path>) {
    let Some(path) = path else {
        return;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "omacell-ui".into(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        family.insert(0, "omacell-ui".into());
    }
    if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        family.insert(0, "omacell-ui".into());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::hex_color;
    use egui::Color32;

    #[test]
    fn hex_color_parses_rgb_and_argb() {
        assert_eq!(hex_color("#ff0000"), Color32::from_rgb(255, 0, 0));
        assert_eq!(
            hex_color("#80ffffff"),
            Color32::from_rgba_unmultiplied(255, 255, 255, 128)
        );
        assert_eq!(hex_color("not-a-color"), Color32::GRAY);
        assert_eq!(hex_color("aéaaa"), Color32::GRAY);
    }
}

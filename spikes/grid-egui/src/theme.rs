//! Two throwaway palettes. `T` swaps every color.

use egui::Color32;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub name: &'static str,
    pub background: Color32,
    pub surface: Color32,
    pub header_background: Color32,
    pub foreground: Color32,
    pub muted: Color32,
    pub header_foreground: Color32,
    pub grid_line: Color32,
    pub frozen_edge: Color32,
    pub cursor: Color32,
    pub selection: Color32,
    pub chrome: Color32,
}

impl Palette {
    pub const DARK: Self = Self {
        name: "dark",
        background: Color32::from_rgb(0x1a, 0x1b, 0x26),
        surface: Color32::from_rgb(0x24, 0x28, 0x3b),
        header_background: Color32::from_rgb(0x16, 0x16, 0x1e),
        foreground: Color32::from_rgb(0xc0, 0xca, 0xf5),
        muted: Color32::from_rgb(0x56, 0x5f, 0x89),
        header_foreground: Color32::from_rgb(0x9a, 0xa5, 0xce),
        grid_line: Color32::from_rgb(0x3b, 0x42, 0x61),
        frozen_edge: Color32::from_rgb(0x7a, 0xa2, 0xf7),
        cursor: Color32::from_rgb(0x7a, 0xa2, 0xf7),
        selection: Color32::from_rgba_unmultiplied_const(0x7a, 0xa2, 0xf7, 0x40),
        chrome: Color32::from_rgb(0x1a, 0x1b, 0x26),
    };

    pub const LIGHT: Self = Self {
        name: "light",
        background: Color32::from_rgb(0xef, 0xf1, 0xf5),
        surface: Color32::from_rgb(0xff, 0xff, 0xff),
        header_background: Color32::from_rgb(0xe6, 0xe9, 0xef),
        foreground: Color32::from_rgb(0x4c, 0x4f, 0x69),
        muted: Color32::from_rgb(0x6c, 0x6f, 0x85),
        header_foreground: Color32::from_rgb(0x5c, 0x5f, 0x77),
        grid_line: Color32::from_rgb(0xbc, 0xc0, 0xcc),
        frozen_edge: Color32::from_rgb(0x1e, 0x66, 0xf5),
        cursor: Color32::from_rgb(0x1e, 0x66, 0xf5),
        selection: Color32::from_rgba_unmultiplied_const(0x1e, 0x66, 0xf5, 0x30),
        chrome: Color32::from_rgb(0xef, 0xf1, 0xf5),
    };

    pub fn apply(self, ctx: &egui::Context) {
        let mut visuals = if self.name == "light" {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.panel_fill = self.chrome;
        visuals.window_fill = self.chrome;
        visuals.override_text_color = Some(self.foreground);
        ctx.set_visuals(visuals);
    }
}

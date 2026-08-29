//! Docked panel model (one visible; Esc returns focus).

/// Known panel ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelState {
    /// Currently visible panel (`format`, `find`, `changeset`, …).
    pub visible: Option<String>,
    /// Dock side from config (`right` / `left` / `bottom`).
    pub side: String,
    /// Width in px.
    pub width: u32,
    /// Grid has focus when no panel (or after Esc).
    pub grid_focused: bool,
}

impl PanelState {
    /// Open `id`, taking focus.
    pub fn open(&mut self, id: &str) {
        self.visible = Some(id.to_string());
        self.grid_focused = false;
    }

    /// Esc: close and return focus to the grid.
    pub fn dismiss(&mut self) {
        self.visible = None;
        self.grid_focused = true;
    }
}

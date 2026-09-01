//! Docked panel model (one visible; Esc returns focus).

/// Known panel ids.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelState {
    /// Currently visible panel (`format`, `find`, `changeset`, …).
    pub visible: Option<String>,
    /// Optional dynamic panel body supplied by a command.
    pub body: Option<String>,
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
        self.body = None;
        self.grid_focused = false;
    }

    /// Open `id` with command-generated content, taking focus.
    pub fn open_with_body(&mut self, id: &str, body: impl Into<String>) {
        self.open(id);
        self.body = Some(body.into());
    }

    /// Esc: close and return focus to the grid.
    pub fn dismiss(&mut self) {
        self.visible = None;
        self.body = None;
        self.grid_focused = true;
    }
}

//! Classic vs modal interaction.

/// Keymap model from `keys.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyModel {
    /// Excel-compatible, modeless.
    #[default]
    Classic,
    /// Vim-style modes.
    Modal,
}

/// Current interaction mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Classic (Excel) — no modal layer.
    #[default]
    Classic,
    /// Modal normal.
    Normal,
    /// Modal insert / cell edit.
    Insert,
    /// Modal visual (range).
    Visual,
    /// Modal visual row.
    VisualRow,
    /// Modal visual column.
    VisualCol,
    /// Modal `:` command line.
    Command,
}

impl Mode {
    /// Default mode for a keymap model.
    #[must_use]
    pub fn for_model(model: KeyModel) -> Self {
        match model {
            KeyModel::Classic => Self::Classic,
            KeyModel::Modal => Self::Normal,
        }
    }

    /// Table name in a modal keymap file (`normal`, `insert`, …).
    #[must_use]
    pub fn table(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Normal => "normal",
            Self::Insert => "insert",
            Self::Visual | Self::VisualRow | Self::VisualCol => "visual",
            Self::Command => "command",
        }
    }

    /// Status-line label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "READY",
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
            Self::Visual => "VISUAL",
            Self::VisualRow => "V-LINE",
            Self::VisualCol => "V-BLOCK",
            Self::Command => "COMMAND",
        }
    }
}

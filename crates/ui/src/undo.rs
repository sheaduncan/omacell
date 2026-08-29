//! Undo-history presentation (the engine undo log stays in core/bus).

/// One row in the undo panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoEntry {
    /// Short label (`cell.set A1`).
    pub label: String,
}

/// Visual undo stack (labels only; apply goes through `edit.undo`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoHistory {
    /// Oldest first.
    pub entries: Vec<UndoEntry>,
}

impl UndoHistory {
    /// Push a user action.
    pub fn push(&mut self, label: impl Into<String>) {
        self.entries.push(UndoEntry {
            label: label.into(),
        });
    }

    /// Pop last (presentation only).
    pub fn pop(&mut self) -> Option<UndoEntry> {
        self.entries.pop()
    }
}

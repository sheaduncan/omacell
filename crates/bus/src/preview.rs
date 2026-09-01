//! Review data for proposed changesets.

use omacell_core::changeset::{ChangeSummary, ChangesetId, CommandCall};
use omacell_core::command::Origin;
use serde::{Deserialize, Serialize};

/// Cell-level before/after data produced by a proposal dry run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellPreview {
    /// Sheet display name.
    pub sheet: String,
    /// Zero-based row.
    pub row: u32,
    /// Zero-based column.
    pub col: u16,
    /// Formula-bar input before the command, or `None` for an absent cell.
    pub before: Option<String>,
    /// Formula-bar input after the command, or `None` for an absent cell.
    pub after: Option<String>,
    /// Whether the stored style changed even when the input did not.
    pub style_changed: bool,
}

/// One command and the bounded effects it would produce.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangePreviewItem {
    /// Proposed command.
    pub command: CommandCall,
    /// Command-local summary.
    pub summary: ChangeSummary,
    /// Changed cells, ordered by sheet, row, and column.
    pub cells: Vec<CellPreview>,
}

/// Review model for a proposed changeset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangePreview {
    /// Proposal identity.
    pub id: ChangesetId,
    /// Trusted proposal origin.
    pub origin: Origin,
    /// Whole-proposal summary.
    pub summary: ChangeSummary,
    /// Command-local review items in execution order.
    pub items: Vec<ChangePreviewItem>,
}

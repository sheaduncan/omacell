//! Events emitted on the bus / IPC wire (spec F-10.1, F-10.6, §8.6).

use serde::{Deserialize, Serialize};

use crate::addr::SheetId;
use crate::changeset::ChangesetId;

/// Notification that something in the session changed.
///
/// Tagged JSON, `snake_case` variant names. `#[non_exhaustive]` because the
/// work package lists further events for later packages.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::event::Event;
/// let e = Event::CellChanged {
///     sheet: SheetId::new(0),
///     row: 0,
///     col: 0,
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Event {
    /// Workbook opened (`on_open`).
    WorkbookOpened {
        /// Path when the workbook is on disk.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// A cell’s value or input changed (`on_change`).
    CellChanged {
        /// Sheet containing the cell.
        sheet: SheetId,
        /// 0-based row.
        row: u32,
        /// 0-based column.
        col: u16,
    },
    /// Recalculation finished (`on_recalc`).
    RecalcDone {
        /// Cells evaluated in this pass.
        cells: u64,
        /// Wall time of the pass.
        elapsed_ms: u64,
    },
    /// About to save (`on_before_save`).
    BeforeSave {
        /// Destination path.
        path: String,
    },
    /// Save completed.
    FileSaved {
        /// Path written.
        path: String,
    },
    /// A changeset is waiting for review.
    ChangesetProposed {
        /// Changeset id.
        id: ChangesetId,
    },
    /// A changeset was applied (one undo unit).
    ChangesetApplied {
        /// Changeset id.
        id: ChangesetId,
    },
    /// A changeset was reverted.
    ChangesetReverted {
        /// Changeset id.
        id: ChangesetId,
    },
    /// Active Omarchy theme changed (`on_theme_change`).
    ThemeChanged {
        /// Theme directory name.
        name: String,
    },
}

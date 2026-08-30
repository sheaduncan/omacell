//! Inverse-delta undo log (spec §11.3).
//!
//! Transactions group UI actions. Memory is budgeted with oldest-first
//! eviction. `undo` / `redo` return affected ranges for redraw.

use std::collections::VecDeque;

use crate::addr::SheetId;
use crate::chart::{Chart, Sparkline};
use crate::command::UndoUnitId;
use crate::error::CoreError;
use crate::names::{DefinedName, NameScope};
use crate::print::PageSetup;
use crate::sheet::{Sheet, SheetEditState, SheetVisibility};
use crate::storage::CellSlot;
use crate::style::Color;
use crate::tables::Table;
use crate::workbook::{CalcMode, WorkbookProtectionState};

/// Default undo memory budget (64 MiB of estimated delta size).
pub const DEFAULT_BUDGET: usize = 64 * 1024 * 1024;

/// Rectangle that must be redrawn after undo/redo.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::undo::AffectedRange;
/// let a = AffectedRange { sheet: SheetId::new(0), min_row: 0, min_col: 0, max_row: 0, max_col: 0 };
/// assert_eq!(a.sheet.index(), 0);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AffectedRange {
    /// Sheet.
    pub sheet: SheetId,
    /// Inclusive min row.
    pub min_row: u32,
    /// Inclusive min column.
    pub min_col: u16,
    /// Inclusive max row.
    pub max_row: u32,
    /// Inclusive max column.
    pub max_col: u16,
}

impl AffectedRange {
    /// A single cell.
    #[must_use]
    pub fn cell(sheet: SheetId, row: u32, col: u16) -> Self {
        Self {
            sheet,
            min_row: row,
            min_col: col,
            max_row: row,
            max_col: col,
        }
    }

    /// Whole sheet (used for structural ops).
    #[must_use]
    pub fn sheet(sheet: SheetId) -> Self {
        Self {
            sheet,
            min_row: 0,
            min_col: 0,
            max_row: crate::limits::MAX_ROWS - 1,
            max_col: crate::limits::MAX_COLS - 1,
        }
    }

    fn merge(&mut self, other: Self) {
        if self.sheet != other.sheet {
            return;
        }
        self.min_row = self.min_row.min(other.min_row);
        self.min_col = self.min_col.min(other.min_col);
        self.max_row = self.max_row.max(other.max_row);
        self.max_col = self.max_col.max(other.max_col);
    }
}

/// One inverse-capable mutation.
#[derive(Clone, Debug)]
pub enum Delta {
    /// Cell slot replaced or cleared.
    Cell {
        /// Sheet.
        sheet: SheetId,
        /// Row.
        row: u32,
        /// Column.
        col: u16,
        /// Previous slot (`None` = was empty).
        before: Option<CellSlot>,
        /// New slot (`None` = cleared).
        after: Option<CellSlot>,
    },
    /// Row height / hidden flag.
    RowGeom {
        /// Sheet.
        sheet: SheetId,
        /// Row.
        row: u32,
        /// Size before.
        before_px: u32,
        /// Size after.
        after_px: u32,
        /// Hidden before.
        hidden_before: bool,
        /// Hidden after.
        hidden_after: bool,
        /// Whether the previous size was explicitly stored.
        custom_before: bool,
        /// Whether the new size is explicitly stored.
        custom_after: bool,
    },
    /// Column width / hidden flag.
    ColGeom {
        /// Sheet.
        sheet: SheetId,
        /// Column.
        col: u16,
        /// Size before.
        before_px: u32,
        /// Size after.
        after_px: u32,
        /// Hidden before.
        hidden_before: bool,
        /// Hidden after.
        hidden_after: bool,
        /// Whether the previous size was explicitly stored.
        custom_before: bool,
        /// Whether the new size is explicitly stored.
        custom_after: bool,
    },
    /// Sheet inserted.
    SheetAdd {
        /// New id.
        id: SheetId,
        /// Position in the ordered list.
        index: usize,
        /// Snapshot of the sheet (usually empty at add).
        sheet: Box<Sheet>,
    },
    /// Sheet removed.
    SheetRemove {
        /// Removed id.
        id: SheetId,
        /// Position it occupied.
        index: usize,
        /// Full snapshot for restore.
        sheet: Box<Sheet>,
        /// Active sheet before removal.
        active_before: SheetId,
        /// Active sheet after removal.
        active_after: SheetId,
    },
    /// Sheet tab reordered.
    SheetReorder {
        /// Sheet.
        id: SheetId,
        /// Original tab index.
        before: usize,
        /// New tab index.
        after: usize,
    },
    /// WP-17 metadata outside the sparse cell store.
    SheetEdit {
        /// Sheet.
        sheet: SheetId,
        /// Metadata before the edit.
        before: Box<SheetEditState>,
        /// Metadata after the edit.
        after: Box<SheetEditState>,
    },
    /// Sheet renamed.
    SheetRename {
        /// Sheet.
        id: SheetId,
        /// Old name.
        before: String,
        /// New name.
        after: String,
    },
    /// Visibility change.
    SheetVisibility {
        /// Sheet.
        id: SheetId,
        /// Old.
        before: SheetVisibility,
        /// New.
        after: SheetVisibility,
    },
    /// Tab colour.
    TabColor {
        /// Sheet.
        id: SheetId,
        /// Old.
        before: Option<Color>,
        /// New.
        after: Option<Color>,
    },
    /// Per-sheet print/page setup.
    PageSetup {
        /// Sheet.
        sheet: SheetId,
        /// Previous setup.
        before: Box<PageSetup>,
        /// New setup.
        after: Box<PageSetup>,
    },
    /// Defined name upsert / remove.
    Name {
        /// Scope + name identity.
        scope: NameScope,
        /// Name string (original case of the *after* or *before* that exists).
        name: String,
        /// Previous.
        before: Option<DefinedName>,
        /// New (`None` = deleted).
        after: Option<DefinedName>,
    },
    /// Table upsert / remove.
    Table {
        /// Previous.
        before: Option<Table>,
        /// New (`None` = deleted).
        after: Option<Table>,
    },
    /// Chart appended to a sheet.
    ChartAdd {
        /// Sheet.
        sheet: SheetId,
        /// Position in the chart list.
        index: usize,
        /// Exact chart record.
        chart: Box<Chart>,
    },
    /// Chart removed from a sheet.
    ChartRemove {
        /// Sheet.
        sheet: SheetId,
        /// Position in the chart list.
        index: usize,
        /// Exact chart record.
        chart: Box<Chart>,
    },
    /// Sparkline appended to a sheet.
    SparklineAdd {
        /// Sheet.
        sheet: SheetId,
        /// Position in the sparkline list.
        index: usize,
        /// Exact sparkline record.
        sparkline: Sparkline,
    },
    /// Sparkline removed from a sheet.
    SparklineRemove {
        /// Sheet.
        sheet: SheetId,
        /// Position in the sparkline list.
        index: usize,
        /// Exact sparkline record.
        sparkline: Sparkline,
    },
    /// Row insert/delete (inverse is the opposite count).
    ShiftRows {
        /// Sheet.
        sheet: SheetId,
        /// Anchor.
        at: u32,
        /// Forward count (inverse uses `-count`).
        count: i32,
        /// Cells removed by a delete, at their original coordinates.
        removed: Vec<(u32, u16, CellSlot)>,
    },
    /// Column insert/delete.
    ShiftCols {
        /// Sheet.
        sheet: SheetId,
        /// Anchor.
        at: u16,
        /// Forward count.
        count: i32,
        /// Cells removed by a delete, at their original coordinates.
        removed: Vec<(u32, u16, CellSlot)>,
    },
    /// Calculation mode (WP-07a `calc.mode`).
    CalcMode {
        /// Mode before the command.
        before: CalcMode,
        /// Mode after the command.
        after: CalcMode,
    },
    /// Workbook protection flags.
    WorkbookProtection {
        /// State before the command.
        before: WorkbookProtectionState,
        /// State after the command.
        after: WorkbookProtectionState,
    },
}

impl Delta {
    fn bytes(&self) -> usize {
        match self {
            Self::Cell { .. } => 64,
            Self::RowGeom { .. } | Self::ColGeom { .. } => 32,
            Self::SheetAdd { sheet, .. } | Self::SheetRemove { sheet, .. } => {
                64 + sheet.store.heap_bytes()
            }
            Self::SheetReorder { .. } => 32,
            Self::SheetEdit { before, after, .. } => {
                64 + before.estimated_bytes() + after.estimated_bytes()
            }
            Self::SheetRename { before, after, .. } => 32 + before.len() + after.len(),
            Self::SheetVisibility { .. } | Self::TabColor { .. } => 16,
            Self::PageSetup { before, after, .. } => {
                256 + before.header.as_ref().map(String::len).unwrap_or(0)
                    + before.footer.as_ref().map(String::len).unwrap_or(0)
                    + after.header.as_ref().map(String::len).unwrap_or(0)
                    + after.footer.as_ref().map(String::len).unwrap_or(0)
                    + (before.row_breaks.len()
                        + before.col_breaks.len()
                        + after.row_breaks.len()
                        + after.col_breaks.len())
                        * 4
            }
            Self::Name { before, after, .. } => {
                64 + before.as_ref().map(|n| n.name.len()).unwrap_or(0)
                    + after.as_ref().map(|n| n.name.len()).unwrap_or(0)
            }
            Self::Table { before, after } => {
                128 + before.as_ref().map(|t| t.name.len()).unwrap_or(0)
                    + after.as_ref().map(|t| t.name.len()).unwrap_or(0)
            }
            Self::ChartAdd { chart, .. } | Self::ChartRemove { chart, .. } => {
                128 + chart.title.as_ref().map(String::len).unwrap_or(0)
                    + chart
                        .series
                        .iter()
                        .map(|series| {
                            series.name.len() + series.color.as_ref().map(String::len).unwrap_or(0)
                        })
                        .sum::<usize>()
            }
            Self::SparklineAdd { .. } | Self::SparklineRemove { .. } => 64,
            Self::ShiftRows { removed, .. } | Self::ShiftCols { removed, .. } => {
                24 + removed.len() * std::mem::size_of::<(u32, u16, CellSlot)>()
            }
            Self::CalcMode { .. } => 2,
            Self::WorkbookProtection { before, after } => {
                32 + before.password.as_ref().map(Vec::len).unwrap_or(0)
                    + after.password.as_ref().map(Vec::len).unwrap_or(0)
            }
        }
    }

    /// Range to redraw for this delta.
    #[must_use]
    pub fn affected(&self) -> AffectedRange {
        match self {
            Self::Cell {
                sheet, row, col, ..
            } => AffectedRange::cell(*sheet, *row, *col),
            Self::RowGeom { sheet, row, .. } => AffectedRange {
                sheet: *sheet,
                min_row: *row,
                min_col: 0,
                max_row: *row,
                max_col: crate::limits::MAX_COLS - 1,
            },
            Self::ColGeom { sheet, col, .. } => AffectedRange {
                sheet: *sheet,
                min_row: 0,
                min_col: *col,
                max_row: crate::limits::MAX_ROWS - 1,
                max_col: *col,
            },
            Self::SheetAdd { id, .. }
            | Self::SheetRemove { id, .. }
            | Self::SheetReorder { id, .. }
            | Self::SheetRename { id, .. }
            | Self::SheetVisibility { id, .. }
            | Self::TabColor { id, .. } => AffectedRange::sheet(*id),
            Self::PageSetup { sheet, .. } => AffectedRange::sheet(*sheet),
            Self::SheetEdit { sheet, .. } => AffectedRange::sheet(*sheet),
            Self::Name { .. } | Self::Table { .. } => AffectedRange::sheet(SheetId::new(0)),
            Self::ChartAdd { sheet, .. }
            | Self::ChartRemove { sheet, .. }
            | Self::SparklineAdd { sheet, .. }
            | Self::SparklineRemove { sheet, .. } => AffectedRange::sheet(*sheet),
            Self::ShiftRows { sheet, .. } | Self::ShiftCols { sheet, .. } => {
                AffectedRange::sheet(*sheet)
            }
            Self::CalcMode { .. } | Self::WorkbookProtection { .. } => {
                AffectedRange::sheet(SheetId::new(0))
            }
        }
    }
}

/// One undo unit: a sequence of deltas.
#[derive(Clone, Debug)]
pub struct Transaction {
    /// Log-assigned id.
    pub id: UndoUnitId,
    deltas: Vec<Delta>,
    bytes: usize,
}

impl Transaction {
    fn new(id: UndoUnitId) -> Self {
        Self {
            id,
            deltas: Vec::new(),
            bytes: 0,
        }
    }

    fn push(&mut self, delta: Delta) {
        self.bytes += delta.bytes();
        self.deltas.push(delta);
    }

    /// Deltas in forward (apply) order.
    #[must_use]
    pub fn deltas(&self) -> &[Delta] {
        &self.deltas
    }

    /// Estimated size.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    fn affected(&self) -> Vec<AffectedRange> {
        let mut out: Vec<AffectedRange> = Vec::new();
        for d in &self.deltas {
            let a = d.affected();
            if let Some(last) = out.last_mut()
                && last.sheet == a.sheet
            {
                last.merge(a);
            } else {
                out.push(a);
            }
        }
        out
    }
}

/// Memory-budgeted undo/redo stacks.
///
/// ```
/// use omacell_core::addr::SheetId;
/// use omacell_core::undo::{Delta, UndoLog};
/// let mut log = UndoLog::new();
/// log.record(Delta::ShiftRows {
///     sheet: SheetId::new(0),
///     at: 0,
///     count: 1,
///     removed: vec![],
/// });
/// assert!(log.can_undo());
/// ```
#[derive(Clone, Debug)]
pub struct UndoLog {
    undo: VecDeque<Transaction>,
    redo: VecDeque<Transaction>,
    open: Option<Transaction>,
    depth: u32,
    next_id: u64,
    budget: usize,
    used: usize,
    enabled: bool,
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new()
    }
}

impl UndoLog {
    /// Empty log with [`DEFAULT_BUDGET`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            open: None,
            depth: 0,
            next_id: 1,
            budget: DEFAULT_BUDGET,
            used: 0,
            enabled: true,
        }
    }

    /// Set the memory budget in bytes.
    pub fn set_budget(&mut self, budget: usize) {
        self.budget = budget.max(1);
        self.evict();
    }

    /// Enable or disable recording.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Whether recording is on.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether a transaction is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.depth > 0
    }

    /// Start a (possibly nested) transaction. Returns `true` if this call opened the outer one.
    pub fn begin(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        self.depth += 1;
        if self.depth == 1 {
            let id = UndoUnitId::new(self.next_id);
            self.next_id += 1;
            self.open = Some(Transaction::new(id));
            true
        } else {
            false
        }
    }

    /// Drop the current nesting level without committing.
    ///
    /// When the outer transaction is aborted, recorded deltas are returned so
    /// the caller can roll the model back. Nested aborts only decrement depth.
    pub fn abort(&mut self) -> Option<Transaction> {
        if self.depth == 0 {
            return None;
        }
        self.depth -= 1;
        if self.depth == 0 {
            self.open.take()
        } else {
            None
        }
    }

    /// Commit the current nesting level. The outer commit pushes the undo stack.
    pub fn commit(&mut self) {
        if self.depth == 0 {
            return;
        }
        self.depth -= 1;
        if self.depth == 0
            && let Some(tx) = self.open.take()
        {
            if tx.deltas.is_empty() {
                return;
            }
            self.clear_redo();
            self.used += tx.bytes;
            self.undo.push_back(tx);
            self.evict();
        }
    }

    /// Record a delta into the open transaction, or as its own unit if none is open.
    pub fn record(&mut self, delta: Delta) {
        if !self.enabled {
            return;
        }
        if self.depth == 0 {
            self.begin();
            if let Some(tx) = &mut self.open {
                tx.push(delta);
            }
            self.commit();
            return;
        }
        if let Some(tx) = &mut self.open {
            tx.push(delta);
        }
    }

    /// Pop the last committed transaction for undo. Caller applies inverses.
    pub fn pop_undo(&mut self) -> Result<Transaction, CoreError> {
        let tx = self
            .undo
            .pop_back()
            .ok_or_else(|| CoreError::undo_empty("nothing to undo"))?;
        self.used = self.used.saturating_sub(tx.bytes);
        Ok(tx)
    }

    /// After applying an undo, push it onto redo.
    pub fn push_redo(&mut self, tx: Transaction) {
        self.used += tx.bytes;
        self.redo.push_back(tx);
        self.evict();
    }

    /// Pop redo.
    pub fn pop_redo(&mut self) -> Result<Transaction, CoreError> {
        let tx = self
            .redo
            .pop_back()
            .ok_or_else(|| CoreError::undo_empty("nothing to redo"))?;
        self.used = self.used.saturating_sub(tx.bytes);
        Ok(tx)
    }

    /// After applying a redo, push it onto undo.
    pub fn push_undo(&mut self, tx: Transaction) {
        self.used += tx.bytes;
        self.undo.push_back(tx);
        self.evict();
    }

    /// Whether undo is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether redo is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Estimated bytes of stored deltas.
    #[must_use]
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Committed undo units, oldest first (WP-19 diagnostic bundle).
    pub fn history(&self) -> impl Iterator<Item = &Transaction> {
        self.undo.iter()
    }

    fn clear_redo(&mut self) {
        for tx in self.redo.drain(..) {
            self.used = self.used.saturating_sub(tx.bytes);
        }
    }

    fn evict(&mut self) {
        while self.used > self.budget && self.undo.len() > 1 {
            if let Some(old) = self.undo.pop_front() {
                self.used = self.used.saturating_sub(old.bytes);
            } else {
                break;
            }
        }
        while self.used > self.budget {
            if let Some(old) = self.redo.pop_front() {
                self.used = self.used.saturating_sub(old.bytes);
            } else {
                break;
            }
        }
    }
}

/// Collect affected ranges from a transaction (pub for Workbook).
#[must_use]
pub fn transaction_affected(tx: &Transaction) -> Vec<AffectedRange> {
    tx.affected()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CellSlot;

    #[test]
    fn auto_commit_and_stacks() {
        let mut log = UndoLog::new();
        log.record(Delta::Cell {
            sheet: SheetId::new(0),
            row: 0,
            col: 0,
            before: None,
            after: Some(CellSlot::number(1.0)),
        });
        assert!(log.can_undo());
        assert!(!log.can_redo());
        let tx = log.pop_undo().unwrap();
        log.push_redo(tx);
        assert!(log.can_redo());
    }

    #[test]
    fn budget_evicts_oldest() {
        let mut log = UndoLog::new();
        log.set_budget(80);
        for i in 0..10 {
            log.record(Delta::Cell {
                sheet: SheetId::new(0),
                row: i,
                col: 0,
                before: None,
                after: Some(CellSlot::number(i as f64)),
            });
        }
        assert!(log.used_bytes() <= 80);
        assert!(log.undo.len() <= 2);
    }

    #[test]
    fn new_edit_removes_redo_bytes_from_accounting() {
        let mut log = UndoLog::new();
        let delta = |row| Delta::Cell {
            sheet: SheetId::new(0),
            row,
            col: 0,
            before: None,
            after: Some(CellSlot::number(f64::from(row))),
        };
        log.record(delta(0));
        let tx = log.pop_undo().unwrap();
        log.push_redo(tx);
        assert_eq!(log.used_bytes(), 64);

        log.record(delta(1));
        assert_eq!(log.used_bytes(), 64);
        assert!(!log.can_redo());
    }
}

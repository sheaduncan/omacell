//! Dynamic-array spill regions (F-3.3).

use rustc_hash::FxHashMap;

use crate::addr::SheetId;
use crate::graph::CellCoord;
use crate::storage::CellSlot;
use crate::value::Value;
use crate::workbook::Workbook;

/// One spilled region from a formula origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpillRegion {
    /// Formula cell.
    pub origin: CellCoord,
    /// Number of rows in the spilled array.
    pub rows: u32,
    /// Number of columns in the spilled array.
    pub cols: u32,
    /// Cell that blocked the spill, if any.
    pub blocked_by: Option<CellCoord>,
}

/// Origin → region and occupancy map (ghost → origin).
#[derive(Clone, Debug, Default)]
pub struct SpillTable {
    by_origin: FxHashMap<CellCoord, SpillRegion>,
    occupancy: FxHashMap<CellCoord, CellCoord>,
}

impl SpillTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Region whose origin is this cell, or that covers this cell.
    #[must_use]
    pub fn region_at(&self, sheet: SheetId, row: u32, col: u16) -> Option<SpillRegion> {
        let c = CellCoord { sheet, row, col };
        if let Some(r) = self.by_origin.get(&c) {
            return Some(*r);
        }
        self.occupancy
            .get(&c)
            .and_then(|o| self.by_origin.get(o))
            .copied()
    }

    /// Region for a known origin.
    #[must_use]
    pub fn get(&self, origin: CellCoord) -> Option<SpillRegion> {
        self.by_origin.get(&origin).copied()
    }

    /// Forget a region's occupancy (does not clear sheet cells).
    pub fn remove(&mut self, origin: CellCoord) {
        if let Some(r) = self.by_origin.remove(&origin) {
            self.clear_occupancy(r);
        }
    }

    fn clear_occupancy(&mut self, r: SpillRegion) {
        if r.blocked_by.is_some() {
            return;
        }
        for dr in 0..r.rows {
            for dc in 0..r.cols {
                let cell = CellCoord {
                    sheet: r.origin.sheet,
                    row: r.origin.row.saturating_add(dr),
                    col: r.origin.col.saturating_add(dc as u16),
                };
                if let Some(o) = self.occupancy.get(&cell).copied()
                    && o == r.origin
                {
                    self.occupancy.remove(&cell);
                }
            }
        }
    }

    /// Record a successful or blocked region.
    pub fn insert(&mut self, region: SpillRegion) {
        self.remove(region.origin);
        if region.blocked_by.is_none() {
            for dr in 0..region.rows {
                for dc in 0..region.cols {
                    let cell = CellCoord {
                        sheet: region.origin.sheet,
                        row: region.origin.row.saturating_add(dr),
                        col: region.origin.col.saturating_add(dc as u16),
                    };
                    self.occupancy.insert(cell, region.origin);
                }
            }
        }
        self.by_origin.insert(region.origin, region);
    }

    /// Clear ghost cells for `origin` from the sheet.
    pub fn clear_ghosts(&self, wb: &mut Workbook, origin: CellCoord) {
        let Some(r) = self.by_origin.get(&origin).copied() else {
            return;
        };
        if r.blocked_by.is_some() {
            return;
        }
        for dr in 0..r.rows {
            for dc in 0..r.cols {
                if dr == 0 && dc == 0 {
                    continue;
                }
                let row = r.origin.row.saturating_add(dr);
                let col = r.origin.col.saturating_add(dc as u16);
                if let Ok(Some(slot)) = wb.get(r.origin.sheet, row, col)
                    && slot.flags.spill()
                    && slot.formula.is_none()
                {
                    let _ = wb.clear_cell(r.origin.sheet, row, col);
                }
            }
        }
    }
}

/// Whether `slot` would block a spill (non-empty, non-ghost).
#[must_use]
pub fn blocks_spill(slot: &CellSlot) -> bool {
    if slot.flags.spill() && slot.formula.is_none() {
        return false;
    }
    if slot.formula.is_some() {
        return true;
    }
    !matches!(slot.value, Value::Empty)
}

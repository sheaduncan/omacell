//! Selection model (F-5.4).

use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};

/// How the next movement treats the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExtendMode {
    /// Replace the selection with the cursor cell.
    #[default]
    Replace,
    /// Grow the active area (`F8`).
    Extend,
    /// Add another area (`Shift+F8`).
    Add,
}

/// One rectangular area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    /// Inclusive start.
    pub start: CellRef,
    /// Inclusive end.
    pub end: CellRef,
}

impl Area {
    /// Single cell.
    #[must_use]
    pub fn cell(cell: CellRef) -> Self {
        Self {
            start: cell,
            end: cell,
        }
    }

    /// Normalized min/max corners.
    #[must_use]
    pub fn normalized(self) -> (u32, u16, u32, u16) {
        (
            self.start.row.min(self.end.row),
            self.start.col.min(self.end.col),
            self.start.row.max(self.end.row),
            self.start.col.max(self.end.col),
        )
    }

    /// Cell count.
    #[must_use]
    pub fn cells(self) -> u64 {
        let (r0, c0, r1, c1) = self.normalized();
        u64::from(r1 - r0 + 1) * u64::from(c1 - c0 + 1)
    }

    /// As a [`RangeRef`].
    #[must_use]
    pub fn to_range(self) -> RangeRef {
        RangeRef::from_corners(self.start, self.end)
    }
}

/// Current selection: one or more areas plus a cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    /// Sheet.
    pub sheet: SheetId,
    /// Cursor / active cell.
    pub cursor: CellRef,
    /// Areas (last is active). Always non-empty.
    pub areas: Vec<Area>,
    /// Extend policy.
    pub extend: ExtendMode,
}

impl Selection {
    /// Cursor at A1 on `sheet`.
    #[must_use]
    pub fn a1(sheet: SheetId) -> Self {
        let cursor = CellRef {
            sheet: Some(sheet),
            row: 0,
            col: 0,
            row_abs: false,
            col_abs: false,
        };
        Self {
            sheet,
            cursor,
            areas: vec![Area::cell(cursor)],
            extend: ExtendMode::Replace,
        }
    }

    /// Active area.
    #[must_use]
    pub fn active(&self) -> Area {
        *self.areas.last().unwrap_or(&Area::cell(self.cursor))
    }

    /// Move the cursor, applying [`ExtendMode`].
    pub fn move_by(&mut self, drow: i64, dcol: i64) {
        let row = clamp_row(i64::from(self.cursor.row) + drow);
        let col = clamp_col(i64::from(self.cursor.col) + dcol);
        self.cursor.row = row;
        self.cursor.col = col;
        match self.extend {
            ExtendMode::Replace => {
                self.areas.clear();
                self.areas.push(Area::cell(self.cursor));
            }
            ExtendMode::Extend => {
                if let Some(active) = self.areas.last_mut() {
                    active.end = self.cursor;
                } else {
                    self.areas.push(Area::cell(self.cursor));
                }
            }
            ExtendMode::Add => {
                self.areas.push(Area::cell(self.cursor));
                self.extend = ExtendMode::Extend;
            }
        }
    }

    /// Select a whole row through the cursor.
    pub fn select_row(&mut self) {
        let mut start = self.cursor;
        let mut end = self.cursor;
        start.col = 0;
        end.col = MAX_COLS - 1;
        self.replace(Area { start, end });
    }

    /// Select a whole column through the cursor.
    pub fn select_col(&mut self) {
        let mut start = self.cursor;
        let mut end = self.cursor;
        start.row = 0;
        end.row = MAX_ROWS - 1;
        self.replace(Area { start, end });
    }

    /// Replace with one area, cursor at `area.start`.
    pub fn replace(&mut self, area: Area) {
        self.cursor = area.start;
        self.areas.clear();
        self.areas.push(area);
        self.extend = ExtendMode::Replace;
    }

    /// Configurable selection stats: count of cells in all areas.
    #[must_use]
    pub fn cell_count(&self) -> u64 {
        self.areas.iter().map(|a| a.cells()).sum()
    }
}

fn clamp_row(v: i64) -> u32 {
    v.clamp(0, i64::from(MAX_ROWS) - 1) as u32
}

fn clamp_col(v: i64) -> u16 {
    v.clamp(0, i64::from(MAX_COLS) - 1) as u16
}

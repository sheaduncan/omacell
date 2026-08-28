//! A1 / sheet resolution and bounded range iteration.

use omacell_core::addr::{
    CellRef, ParsedRef, RangeRef, RefKind, SheetId, col_to_letters, parse_a1, quote_sheet_name,
};
use omacell_core::error::CoreError;
use omacell_core::workbook::Workbook;

use crate::error;

/// Maximum cells a command-bus range operation will materialize.
pub const MAX_RANGE_CELLS: u64 = 100_000;

/// A resolved cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedCell {
    /// Sheet.
    pub sheet: SheetId,
    /// Row.
    pub row: u32,
    /// Column.
    pub col: u16,
}

/// Inclusive rectangle on one sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedRange {
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

impl ResolvedRange {
    /// Cell count.
    #[must_use]
    pub fn area(self) -> u64 {
        let rows = u64::from(self.max_row.saturating_sub(self.min_row).saturating_add(1));
        let cols = u64::from(self.max_col.saturating_sub(self.min_col).saturating_add(1));
        rows.saturating_mul(cols)
    }

    /// Row-major cells.
    pub fn cells(self) -> impl Iterator<Item = (u32, u16)> {
        (self.min_row..=self.max_row)
            .flat_map(move |row| (self.min_col..=self.max_col).map(move |col| (row, col)))
    }
}

/// Parse `A1` / `Sheet1!A1`. Unqualified refs use the active sheet.
pub fn resolve_cell(wb: &Workbook, spec: &str) -> Result<ResolvedCell, CoreError> {
    let parsed = parse_a1(spec)?;
    let (sheet, kind) = attach(wb, parsed)?;
    match kind {
        RefKind::Cell(cell) => Ok(ResolvedCell {
            sheet,
            row: cell.row,
            col: cell.col,
        }),
        RefKind::Range(_) => Err(crate::error::args(format!(
            "expected a cell address, got range {spec:?}"
        ))),
    }
}

/// Parse a cell or range. 3-D references are rejected in this package.
pub fn resolve_range(wb: &Workbook, spec: &str) -> Result<ResolvedRange, CoreError> {
    let parsed = parse_a1(spec)?;
    let (sheet, kind) = attach(wb, parsed)?;
    let range = match kind {
        RefKind::Cell(cell) => RangeRef::from_corners(cell, cell),
        RefKind::Range(range) => range,
    };
    if range.sheet_end.is_some() {
        return Err(crate::error::args(
            "3-D ranges are not supported by this command",
        ));
    }
    let min_row = range.start.row.min(range.end.row);
    let max_row = range.start.row.max(range.end.row);
    let min_col = range.start.col.min(range.end.col);
    let max_col = range.start.col.max(range.end.col);
    let resolved = ResolvedRange {
        sheet,
        min_row,
        min_col,
        max_row,
        max_col,
    };
    if resolved.area() > MAX_RANGE_CELLS {
        return Err(error::range_size(resolved.area()));
    }
    Ok(resolved)
}

fn attach(wb: &Workbook, parsed: ParsedRef) -> Result<(SheetId, RefKind), CoreError> {
    let resolved = wb.resolve_parsed(parsed)?;
    let sheet = match &resolved {
        RefKind::Cell(cell) => cell.sheet,
        RefKind::Range(range) => range.start.sheet,
    };
    let sheet = match sheet {
        Some(id) => id,
        None => wb.active_sheet(),
    };
    Ok((sheet, resolved))
}

/// `Sheet1!A1` using the sheet's current name.
pub fn format_cell(wb: &Workbook, cell: ResolvedCell) -> String {
    let name = wb
        .sheet(cell.sheet)
        .map(|sheet| sheet.name.as_str())
        .unwrap_or("Sheet1");
    let a1 = CellRef {
        sheet: None,
        row: cell.row,
        col: cell.col,
        row_abs: false,
        col_abs: false,
    }
    .to_a1();
    format!("{}!{a1}", quote_sheet_name(name))
}

/// `Sheet1!A1:B2`.
pub fn format_range(wb: &Workbook, range: ResolvedRange) -> String {
    let name = wb
        .sheet(range.sheet)
        .map(|sheet| sheet.name.as_str())
        .unwrap_or("Sheet1");
    let start = format!(
        "{}{}",
        col_to_letters(range.min_col).unwrap_or_else(|_| "A".into()),
        range.min_row + 1
    );
    let end = format!(
        "{}{}",
        col_to_letters(range.max_col).unwrap_or_else(|_| "A".into()),
        range.max_row + 1
    );
    if start == end {
        format!("{}!{start}", quote_sheet_name(name))
    } else {
        format!("{}!{start}:{end}", quote_sheet_name(name))
    }
}

/// Resolve a sheet name or, if `spec` parses as a number, a 0-based id string
/// is not accepted — names only.
pub fn resolve_sheet(wb: &Workbook, name: &str) -> Result<SheetId, CoreError> {
    wb.resolve_sheet_name(name)
}

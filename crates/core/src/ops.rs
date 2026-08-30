//! Structural edits, fill, paste-special, and protection (WP-17).

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{CellRef, RangeRef, SheetId, SheetSpec, col_to_letters};
use crate::dates::{CivilDate, DateSystem, date_to_serial, serial_to_date};
use crate::error::{CoreError, ErrorKind};
use crate::formula::{
    Expr, ExprKind, Formula, RewriteOp, adjust_cols, adjust_rows, move_range, parse, print,
    rewrite_print,
};
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::storage::{CellFlags, CellSlot};
use crate::style::Style;
use crate::value::Value;
use crate::workbook::Workbook;

/// Excel 97–2003 sheet-protection XOR hash (not a security feature).
///
/// Algorithm: 15-bit rotate-left, XOR each password byte from last to first,
/// XOR length, XOR `0xCE4B`. Stored as the OOXML `password` attribute.
#[must_use]
pub fn excel_xor_hash(password: &str) -> u16 {
    let chars: Vec<u8> = password
        .chars()
        .take(15)
        .map(|c| (c as u32 & 0xFF) as u8)
        .collect();
    if chars.is_empty() {
        return 0;
    }
    let mut hash: u16 = 0;
    for &ch in chars.iter().rev() {
        hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7FFF);
        hash ^= u16::from(ch);
    }
    hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7FFF);
    hash ^= chars.len() as u16;
    hash ^= 0xCE4B;
    hash
}

/// How a fill should write cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillMode {
    /// Copy source values/formulas.
    Copy,
    /// Arithmetic sequence.
    Linear,
    /// Geometric sequence.
    Growth,
    /// Date serial +1, skipping Saturday/Sunday.
    Weekday,
    /// Date serial +1 calendar day.
    Date,
    /// Advance month.
    Month,
    /// Advance year.
    Year,
    /// Copy styles only.
    Formats,
}

/// Paste-special options (F-5.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PasteSpecial {
    /// Values.
    pub values: bool,
    /// Formulas (rewritten by copy delta).
    pub formulas: bool,
    /// Cell styles.
    pub formats: bool,
    /// Number formats only.
    pub number_formats: bool,
    /// Column widths.
    pub column_widths: bool,
    /// Transpose.
    pub transpose: bool,
    /// Skip blank source cells.
    pub skip_blanks: bool,
    /// Arithmetic on the destination number.
    pub operation: PasteOp,
    /// Write `=source` links instead of values.
    pub paste_link: bool,
}

/// Arithmetic paste.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PasteOp {
    /// Replace.
    #[default]
    None,
    /// Add.
    Add,
    /// Subtract.
    Sub,
    /// Multiply.
    Mul,
    /// Divide.
    Div,
}

/// Insert or delete shift direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// Whole rows, or cells shift down.
    Down,
    /// Whole columns, or cells shift right.
    Right,
}

/// Insert `count` rows before 0-based `at`, rewriting formulas workbook-wide.
pub fn insert_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u32,
    count: u32,
) -> Result<(), CoreError> {
    if count == 0 {
        return Ok(());
    }
    wb.insert_rows(sheet, at, count)?;
    shift_side_tables(wb, sheet, at, count as i32, true)?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Rows {
            at,
            count,
            delete: false,
        },
    )
}

/// Delete `count` rows starting at 0-based `at`.
pub fn delete_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u32,
    count: u32,
) -> Result<(), CoreError> {
    if count == 0 {
        return Ok(());
    }
    wb.delete_rows(sheet, at, count)?;
    shift_side_tables(wb, sheet, at, -(count as i32), true)?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Rows {
            at,
            count,
            delete: true,
        },
    )
}

/// Insert columns.
pub fn insert_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u16,
    count: u16,
) -> Result<(), CoreError> {
    if count == 0 {
        return Ok(());
    }
    wb.insert_cols(sheet, at, count)?;
    shift_side_tables_cols(wb, sheet, at, count as i32)?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Cols {
            at,
            count,
            delete: false,
        },
    )
}

/// Delete columns.
pub fn delete_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u16,
    count: u16,
) -> Result<(), CoreError> {
    if count == 0 {
        return Ok(());
    }
    wb.delete_cols(sheet, at, count)?;
    shift_side_tables_cols(wb, sheet, at, -(count as i32))?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Cols {
            at,
            count,
            delete: true,
        },
    )
}

/// Insert cells in `range`, shifting the rest of the band down or right.
pub fn insert_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    shift: Shift,
) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    match shift {
        Shift::Down => {
            let n = r1.saturating_sub(r0).saturating_add(1);
            shift_band_rows(wb, sheet, r0, n, c0, c1, false)?;
            rewrite_formulas(
                wb,
                sheet,
                RewriteKind::BandRows {
                    at: r0,
                    count: n,
                    c0,
                    c1,
                    delete: false,
                },
            )
        }
        Shift::Right => {
            let n = c1.saturating_sub(c0).saturating_add(1);
            shift_band_cols(wb, sheet, c0, n, r0, r1, false)?;
            rewrite_formulas(
                wb,
                sheet,
                RewriteKind::BandCols {
                    at: c0,
                    count: n,
                    r0,
                    r1,
                    delete: false,
                },
            )
        }
    }
}

/// Delete cells in `range`, shifting the band up or left.
pub fn delete_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    shift: Shift,
) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    match shift {
        Shift::Down => {
            let n = r1.saturating_sub(r0).saturating_add(1);
            shift_band_rows(wb, sheet, r0, n, c0, c1, true)?;
            rewrite_formulas(
                wb,
                sheet,
                RewriteKind::BandRows {
                    at: r0,
                    count: n,
                    c0,
                    c1,
                    delete: true,
                },
            )
        }
        Shift::Right => {
            let n = c1.saturating_sub(c0).saturating_add(1);
            shift_band_cols(wb, sheet, c0, n, r0, r1, true)?;
            rewrite_formulas(
                wb,
                sheet,
                RewriteKind::BandCols {
                    at: c0,
                    count: n,
                    r0,
                    r1,
                    delete: true,
                },
            )
        }
    }
}

fn shift_band_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u32,
    count: u32,
    c0: u16,
    c1: u16,
    delete: bool,
) -> Result<(), CoreError> {
    let cells: Vec<(u32, u16, CellSlot)> = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
        .store
        .iter()
        .filter(|(_, c, _)| *c >= c0 && *c <= c1)
        .collect();
    for (r, c, _) in &cells {
        let _ = replace_cell_slot(wb, sheet, *r, *c, None)?;
    }
    let mag = count;
    for (r, c, slot) in cells {
        let nr = if delete {
            if r < at {
                r
            } else if r < at + mag {
                continue;
            } else {
                r - mag
            }
        } else if r >= at {
            r + mag
        } else {
            r
        };
        let _ = replace_cell_slot(wb, sheet, nr, c, Some(slot))?;
    }
    Ok(())
}

fn shift_band_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u16,
    count: u16,
    r0: u32,
    r1: u32,
    delete: bool,
) -> Result<(), CoreError> {
    let cells: Vec<(u32, u16, CellSlot)> = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
        .store
        .iter()
        .filter(|(r, _, _)| *r >= r0 && *r <= r1)
        .collect();
    for (r, c, _) in &cells {
        let _ = replace_cell_slot(wb, sheet, *r, *c, None)?;
    }
    let mag = count;
    for (r, c, slot) in cells {
        let nc = if delete {
            if c < at {
                c
            } else if c < at + mag {
                continue;
            } else {
                c - mag
            }
        } else if c >= at {
            c + mag
        } else {
            c
        };
        let _ = replace_cell_slot(wb, sheet, r, nc, Some(slot))?;
    }
    Ok(())
}

enum RewriteKind {
    Rows {
        at: u32,
        count: u32,
        delete: bool,
    },
    Cols {
        at: u16,
        count: u16,
        delete: bool,
    },
    BandRows {
        at: u32,
        count: u32,
        c0: u16,
        c1: u16,
        delete: bool,
    },
    BandCols {
        at: u16,
        count: u16,
        r0: u32,
        r1: u32,
        delete: bool,
    },
}

fn rewrite_formulas(
    wb: &mut Workbook,
    target: SheetId,
    kind: RewriteKind,
) -> Result<(), CoreError> {
    let target_name = wb
        .sheet(target)
        .map(|s| s.name.clone())
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    let sheet_ids: Vec<SheetId> = wb.sheets().map(|s| s.id).collect();
    let mut updates = Vec::new();
    for id in sheet_ids {
        let home_name = wb.sheet(id).map(|s| s.name.clone()).unwrap_or_default();
        let cells: Vec<(u32, u16, CellSlot)> = wb
            .sheet(id)
            .map(|s| s.store.iter().collect())
            .unwrap_or_default();
        for (row, col, slot) in cells {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(src) = wb.intern().formulas.get(fid).map(str::to_string) else {
                continue;
            };
            let Ok(parsed) = parse(&src) else {
                continue;
            };
            let new_ast = match &kind {
                RewriteKind::Rows { at, count, delete } => {
                    map_sheet_rows(&parsed.ast, &home_name, &target_name, *at, *count, *delete)
                }
                RewriteKind::Cols { at, count, delete } => {
                    map_sheet_cols(&parsed.ast, &home_name, &target_name, *at, *count, *delete)
                }
                RewriteKind::BandRows {
                    at,
                    count,
                    c0,
                    c1,
                    delete,
                } => map_sheet_band_rows(
                    &parsed.ast,
                    &home_name,
                    &target_name,
                    *at,
                    *count,
                    *c0,
                    *c1,
                    *delete,
                ),
                RewriteKind::BandCols {
                    at,
                    count,
                    r0,
                    r1,
                    delete,
                } => map_sheet_band_cols(
                    &parsed.ast,
                    &home_name,
                    &target_name,
                    *at,
                    *count,
                    *r0,
                    *r1,
                    *delete,
                ),
            };
            let printed = print(&Formula {
                ast: new_ast,
                style: parsed.style,
                base_row: parsed.base_row,
                base_col: parsed.base_col,
            });
            if printed != src {
                updates.push((id, row, col, printed));
            }
        }
    }
    for (id, row, col, src) in updates {
        wb.set_cell_contents(id, row, col, &src)?;
    }
    Ok(())
}

fn map_sheet_rows(
    expr: &Expr,
    home: &str,
    target: &str,
    at: u32,
    count: u32,
    delete: bool,
) -> Expr {
    let applies = |sheet: &Option<SheetSpec>| match sheet {
        None => home.eq_ignore_ascii_case(target),
        Some(spec) => spec.start.eq_ignore_ascii_case(target),
    };
    expr.clone().map(&mut |e| {
        let kind = match e.kind {
            ExprKind::Cell { sheet, cell } if applies(&sheet) => {
                match adjust_rows(
                    &Expr {
                        kind: ExprKind::Cell { sheet: None, cell },
                        span: e.span,
                    },
                    at,
                    count,
                    delete,
                )
                .kind
                {
                    ExprKind::Cell { cell, .. } => ExprKind::Cell { sheet, cell },
                    ExprKind::Error(k) => ExprKind::Error(k),
                    other => other,
                }
            }
            ExprKind::Range { sheet, range } if applies(&sheet) => {
                match adjust_rows(
                    &Expr {
                        kind: ExprKind::Range { sheet: None, range },
                        span: e.span,
                    },
                    at,
                    count,
                    delete,
                )
                .kind
                {
                    ExprKind::Range { range, .. } => ExprKind::Range { sheet, range },
                    ExprKind::Error(k) => ExprKind::Error(k),
                    other => other,
                }
            }
            other => other,
        };
        Expr { kind, span: e.span }
    })
}

fn map_sheet_cols(
    expr: &Expr,
    home: &str,
    target: &str,
    at: u16,
    count: u16,
    delete: bool,
) -> Expr {
    let applies = |sheet: &Option<SheetSpec>| match sheet {
        None => home.eq_ignore_ascii_case(target),
        Some(spec) => spec.start.eq_ignore_ascii_case(target),
    };
    expr.clone().map(&mut |e| {
        let kind = match e.kind {
            ExprKind::Cell { sheet, cell } if applies(&sheet) => {
                match adjust_cols(
                    &Expr {
                        kind: ExprKind::Cell { sheet: None, cell },
                        span: e.span,
                    },
                    at,
                    count,
                    delete,
                )
                .kind
                {
                    ExprKind::Cell { cell, .. } => ExprKind::Cell { sheet, cell },
                    ExprKind::Error(k) => ExprKind::Error(k),
                    other => other,
                }
            }
            ExprKind::Range { sheet, range } if applies(&sheet) => {
                match adjust_cols(
                    &Expr {
                        kind: ExprKind::Range { sheet: None, range },
                        span: e.span,
                    },
                    at,
                    count,
                    delete,
                )
                .kind
                {
                    ExprKind::Range { range, .. } => ExprKind::Range { sheet, range },
                    ExprKind::Error(k) => ExprKind::Error(k),
                    other => other,
                }
            }
            other => other,
        };
        Expr { kind, span: e.span }
    })
}

#[allow(clippy::too_many_arguments)]
fn map_sheet_band_rows(
    expr: &Expr,
    home: &str,
    target: &str,
    at: u32,
    count: u32,
    c0: u16,
    c1: u16,
    delete: bool,
) -> Expr {
    let applies = |sheet: &Option<SheetSpec>| match sheet {
        None => home.eq_ignore_ascii_case(target),
        Some(spec) => spec.start.eq_ignore_ascii_case(target),
    };
    expr.clone().map(&mut |item| {
        let kind = match item.kind {
            ExprKind::Cell { sheet, cell }
                if applies(&sheet) && cell.col >= c0 && cell.col <= c1 =>
            {
                restore_sheet_qualifier(
                    adjust_rows(
                        &Expr {
                            kind: ExprKind::Cell { sheet: None, cell },
                            span: item.span,
                        },
                        at,
                        count,
                        delete,
                    )
                    .kind,
                    sheet,
                )
            }
            ExprKind::Range { sheet, range }
                if applies(&sheet)
                    && range.start.col.min(range.end.col) >= c0
                    && range.start.col.max(range.end.col) <= c1 =>
            {
                restore_sheet_qualifier(
                    adjust_rows(
                        &Expr {
                            kind: ExprKind::Range { sheet: None, range },
                            span: item.span,
                        },
                        at,
                        count,
                        delete,
                    )
                    .kind,
                    sheet,
                )
            }
            other => other,
        };
        Expr {
            kind,
            span: item.span,
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn map_sheet_band_cols(
    expr: &Expr,
    home: &str,
    target: &str,
    at: u16,
    count: u16,
    r0: u32,
    r1: u32,
    delete: bool,
) -> Expr {
    let applies = |sheet: &Option<SheetSpec>| match sheet {
        None => home.eq_ignore_ascii_case(target),
        Some(spec) => spec.start.eq_ignore_ascii_case(target),
    };
    expr.clone().map(&mut |item| {
        let kind = match item.kind {
            ExprKind::Cell { sheet, cell }
                if applies(&sheet) && cell.row >= r0 && cell.row <= r1 =>
            {
                restore_sheet_qualifier(
                    adjust_cols(
                        &Expr {
                            kind: ExprKind::Cell { sheet: None, cell },
                            span: item.span,
                        },
                        at,
                        count,
                        delete,
                    )
                    .kind,
                    sheet,
                )
            }
            ExprKind::Range { sheet, range }
                if applies(&sheet)
                    && range.start.row.min(range.end.row) >= r0
                    && range.start.row.max(range.end.row) <= r1 =>
            {
                restore_sheet_qualifier(
                    adjust_cols(
                        &Expr {
                            kind: ExprKind::Range { sheet: None, range },
                            span: item.span,
                        },
                        at,
                        count,
                        delete,
                    )
                    .kind,
                    sheet,
                )
            }
            other => other,
        };
        Expr {
            kind,
            span: item.span,
        }
    })
}

fn restore_sheet_qualifier(kind: ExprKind, sheet: Option<SheetSpec>) -> ExprKind {
    match kind {
        ExprKind::Cell { cell, .. } => ExprKind::Cell { sheet, cell },
        ExprKind::Range { range, .. } => ExprKind::Range { sheet, range },
        other => other,
    }
}

fn shift_side_tables(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u32,
    count: i32,
    rows: bool,
) -> Result<(), CoreError> {
    let _ = rows;
    let s = wb.sheet_mut(sheet)?;
    s.geometry.rows.shift_meta(at, count)?;
    s.notes = shift_map_rows(&s.notes, at, count);
    s.comments = shift_map_rows(&s.comments, at, count);
    s.hyperlinks = shift_map_rows(&s.hyperlinks, at, count);
    s.merges = s
        .merges
        .iter()
        .filter_map(|m| shift_range_rows(*m, at, count))
        .collect();
    Ok(())
}

fn shift_side_tables_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u16,
    count: i32,
) -> Result<(), CoreError> {
    let s = wb.sheet_mut(sheet)?;
    s.geometry.cols.shift_meta(u32::from(at), count)?;
    s.notes = shift_map_cols(&s.notes, at, count);
    s.comments = shift_map_cols(&s.comments, at, count);
    s.hyperlinks = shift_map_cols(&s.hyperlinks, at, count);
    s.merges = s
        .merges
        .iter()
        .filter_map(|m| shift_range_cols(*m, at, count))
        .collect();
    Ok(())
}

fn shift_map_rows<T: Clone>(
    map: &FxHashMap<(u32, u16), T>,
    at: u32,
    count: i32,
) -> FxHashMap<(u32, u16), T> {
    let mag = count.unsigned_abs();
    let mut next = FxHashMap::default();
    for (&(r, c), v) in map {
        let nr = if count >= 0 {
            if r >= at { r.saturating_add(mag) } else { r }
        } else if r < at {
            r
        } else if r < at + mag {
            continue;
        } else {
            r - mag
        };
        next.insert((nr, c), v.clone());
    }
    next
}

fn shift_map_cols<T: Clone>(
    map: &FxHashMap<(u32, u16), T>,
    at: u16,
    count: i32,
) -> FxHashMap<(u32, u16), T> {
    let mag = count.unsigned_abs() as u16;
    let mut next = FxHashMap::default();
    for (&(r, c), v) in map {
        let nc = if count >= 0 {
            if c >= at { c.saturating_add(mag) } else { c }
        } else if c < at {
            c
        } else if c < at + mag {
            continue;
        } else {
            c - mag
        };
        next.insert((r, nc), v.clone());
    }
    next
}

fn shift_range_rows(range: RangeRef, at: u32, count: i32) -> Option<RangeRef> {
    let mag = count.unsigned_abs();
    let adj = |r: u32| -> Option<u32> {
        if count >= 0 {
            Some(if r >= at { r.saturating_add(mag) } else { r })
        } else if r < at {
            Some(r)
        } else if r < at + mag {
            None
        } else {
            Some(r - mag)
        }
    };
    let start_row = adj(range.start.row)?;
    let end_row = adj(range.end.row)?;
    Some(RangeRef {
        start: CellRef {
            row: start_row,
            ..range.start
        },
        end: CellRef {
            row: end_row,
            ..range.end
        },
        ..range
    })
}

fn shift_range_cols(range: RangeRef, at: u16, count: i32) -> Option<RangeRef> {
    let mag = count.unsigned_abs() as u16;
    let adj = |c: u16| -> Option<u16> {
        if count >= 0 {
            Some(if c >= at { c.saturating_add(mag) } else { c })
        } else if c < at {
            Some(c)
        } else if c < at + mag {
            None
        } else {
            Some(c - mag)
        }
    };
    let start_col = adj(range.start.col)?;
    let end_col = adj(range.end.col)?;
    Some(RangeRef {
        start: CellRef {
            col: start_col,
            ..range.start
        },
        end: CellRef {
            col: end_col,
            ..range.end
        },
        ..range
    })
}

/// Merge `range` into one merged area.
pub fn merge(wb: &mut Workbook, sheet: SheetId, range: RangeRef) -> Result<(), CoreError> {
    wb.sheet_mut(sheet)?.add_merge(range)
}

/// Merge each row of `range` independently (Excel merge-across).
pub fn merge_across(wb: &mut Workbook, sheet: SheetId, range: RangeRef) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    for r in r0..=r1 {
        let row_range =
            RangeRef::from_corners(CellRef::new(r, c0).unwrap(), CellRef::new(r, c1).unwrap());
        wb.sheet_mut(sheet)?.add_merge(row_range)?;
    }
    Ok(())
}

/// Unmerge any merge overlapping `range`.
pub fn unmerge(wb: &mut Workbook, sheet: SheetId, range: RangeRef) -> Result<usize, CoreError> {
    let s = wb.sheet_mut(sheet)?;
    let before = s.merges.len();
    s.merges.retain(|m| !overlaps(*m, range));
    Ok(before - s.merges.len())
}

fn overlaps(a: RangeRef, b: RangeRef) -> bool {
    let (ar0, ac0, ar1, ac1) = norm(a);
    let (br0, bc0, br1, bc1) = norm(b);
    ar0 <= br1 && br0 <= ar1 && ac0 <= bc1 && bc0 <= ac1
}

fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}

fn replace_cell_slot(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    slot: Option<CellSlot>,
) -> Result<Option<CellSlot>, CoreError> {
    match slot {
        Some(slot) => wb.set_slot(sheet, row, col, slot),
        None => wb.clear_cell(sheet, row, col),
    }
}

/// Detect a fill series from numeric source values.
#[must_use]
pub fn detect_fill(values: &[f64]) -> FillMode {
    if values.len() < 2 {
        return FillMode::Copy;
    }
    let d = values[1] - values[0];
    if values.windows(2).all(|w| (w[1] - w[0] - d).abs() < 1e-9) {
        if (d - 1.0).abs() < 1e-9 {
            FillMode::Date
        } else {
            FillMode::Linear
        }
    } else if values[0].abs() > 1e-12
        && values
            .windows(2)
            .all(|w| w[0].abs() > 1e-12 && ((w[1] / w[0]) - (values[1] / values[0])).abs() < 1e-9)
    {
        FillMode::Growth
    } else {
        FillMode::Copy
    }
}

/// Extend `values` by `n` according to `mode`.
#[must_use]
pub fn extend_fill(values: &[f64], mode: FillMode, n: usize, dates: DateSystem) -> Vec<f64> {
    let Some(&last) = values.last() else {
        return vec![0.0; n];
    };
    match mode {
        FillMode::Copy | FillMode::Formats => vec![last; n],
        FillMode::Linear => {
            let step = if values.len() >= 2 {
                values[1] - values[0]
            } else {
                1.0
            };
            (1..=n).map(|i| last + step * i as f64).collect()
        }
        FillMode::Growth => {
            let ratio = if values.len() >= 2 && values[0].abs() > 1e-12 {
                values[1] / values[0]
            } else {
                1.0
            };
            (1..=n).map(|i| last * ratio.powi(i as i32)).collect()
        }
        FillMode::Date => (1..=n).map(|i| last + i as f64).collect(),
        FillMode::Weekday => {
            let mut out = Vec::with_capacity(n);
            let mut serial = last;
            while out.len() < n {
                serial += 1.0;
                if let Some(d) = serial_to_date(serial as i64, dates) {
                    let wd = weekday(d.year, u32::from(d.month), u32::from(d.day));
                    if wd != 0 && wd != 6 {
                        out.push(serial);
                    }
                } else {
                    out.push(serial);
                }
            }
            out
        }
        FillMode::Month => (1..=n).map(|i| add_months(last, i as i32, dates)).collect(),
        FillMode::Year => (1..=n)
            .map(|i| add_months(last, 12 * i as i32, dates))
            .collect(),
    }
}

fn weekday(year: i32, month: u32, day: u32) -> u32 {
    // Sakamoto: 0 = Sunday.
    let mut y = year;
    let m = month;
    if m < 3 {
        y -= 1;
    }
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let idx = (m as usize).saturating_sub(1).min(11);
    ((y + y / 4 - y / 100 + y / 400 + t[idx] + day as i32) % 7) as u32
}

fn add_months(serial: f64, months: i32, dates: DateSystem) -> f64 {
    let Some(d) = serial_to_date(serial as i64, dates) else {
        return serial + f64::from(months) * 30.0;
    };
    let mut month = d.month as i32 + months;
    let mut year = d.year;
    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }
    let day = d.day.min(days_in_month(year, month as u32) as u8);
    date_to_serial(
        CivilDate {
            year,
            month: month as u8,
            day,
            lotus_leap: false,
        },
        dates,
    )
    .map(|s| s as f64)
    .unwrap_or(serial)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Fill `dest` from `src` along the major axis.
pub fn fill_range(
    wb: &mut Workbook,
    sheet: SheetId,
    src: RangeRef,
    dest: RangeRef,
    mode: FillMode,
) -> Result<u32, CoreError> {
    let (sr0, sc0, sr1, sc1) = norm(src);
    let (dr0, dc0, dr1, dc1) = norm(dest);
    let mut changed = 0u32;
    if dr0 >= sr0 && dc0 == sc0 && dc1 == sc1 {
        // fill down
        let mut nums = Vec::new();
        for r in sr0..=sr1 {
            if let Ok(Some(slot)) = wb.get(sheet, r, sc0)
                && let Value::Number(n) = slot.value
            {
                nums.push(n);
            }
        }
        let ext = extend_fill(
            &nums,
            mode,
            (dr1.saturating_sub(sr1)) as usize,
            DateSystem::Excel1900,
        );
        for (i, r) in (sr1.saturating_add(1)..=dr1).enumerate() {
            for c in sc0..=sc1 {
                match mode {
                    FillMode::Copy => {
                        let source_row = sr0 + (r - sr1 - 1) % (sr1 - sr0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, source_row, c) {
                            copy_slot(wb, sheet, *slot, r, c, r as i32 - source_row as i32, 0)?;
                            changed += 1;
                        }
                    }
                    FillMode::Formats => {
                        let source_row = sr0 + (r - sr1 - 1) % (sr1 - sr0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, source_row, c) {
                            copy_slot_format(wb, sheet, *slot, r, c)?;
                            changed += 1;
                        }
                    }
                    _ => {
                        if i < ext.len() {
                            wb.set_number(sheet, r, c, ext[i])?;
                            changed += 1;
                        }
                    }
                }
            }
        }
    } else if dc0 >= sc0 && dr0 == sr0 && dr1 == sr1 {
        let mut nums = Vec::new();
        for c in sc0..=sc1 {
            if let Ok(Some(slot)) = wb.get(sheet, sr0, c)
                && let Value::Number(n) = slot.value
            {
                nums.push(n);
            }
        }
        let ext = extend_fill(
            &nums,
            mode,
            (u32::from(dc1.saturating_sub(sc1))) as usize,
            DateSystem::Excel1900,
        );
        for r in sr0..=sr1 {
            for (i, c) in (sc1.saturating_add(1)..=dc1).enumerate() {
                match mode {
                    FillMode::Copy => {
                        let source_col = sc0 + (c - sc1 - 1) % (sc1 - sc0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, r, source_col) {
                            copy_slot(
                                wb,
                                sheet,
                                *slot,
                                r,
                                c,
                                0,
                                i32::from(c) - i32::from(source_col),
                            )?;
                            changed += 1;
                        }
                    }
                    FillMode::Formats => {
                        let source_col = sc0 + (c - sc1 - 1) % (sc1 - sc0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, r, source_col) {
                            copy_slot_format(wb, sheet, *slot, r, c)?;
                            changed += 1;
                        }
                    }
                    _ => {
                        if i < ext.len() {
                            wb.set_number(sheet, r, c, ext[i])?;
                            changed += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(changed)
}

fn copy_slot_format(
    wb: &mut Workbook,
    sheet: SheetId,
    source: CellSlot,
    row: u32,
    col: u16,
) -> Result<(), CoreError> {
    let mut dest = wb
        .get(sheet, row, col)?
        .copied()
        .unwrap_or_else(CellSlot::empty);
    dest.style = source.style;
    dest.flags = dest
        .flags
        .with(CellFlags::LOCKED, source.flags.locked())
        .with(CellFlags::HIDDEN, source.flags.hidden());
    replace_cell_slot(wb, sheet, row, col, Some(dest))?;
    Ok(())
}

fn copy_slot(
    wb: &mut Workbook,
    sheet: SheetId,
    slot: CellSlot,
    row: u32,
    col: u16,
    drow: i32,
    dcol: i32,
) -> Result<(), CoreError> {
    if let Some(fid) = slot.formula {
        let src = wb.intern().formulas.get(fid).unwrap_or("").to_string();
        if let Ok(rewritten) = rewrite_print(&src, &RewriteOp::Copy { dcol, drow }) {
            wb.set_cell_contents(sheet, row, col, &rewritten)?;
            let mut copied = wb
                .get(sheet, row, col)?
                .copied()
                .unwrap_or_else(CellSlot::empty);
            copied.style = slot.style;
            copied.flags = slot.flags;
            replace_cell_slot(wb, sheet, row, col, Some(copied))?;
            return Ok(());
        }
    }
    replace_cell_slot(wb, sheet, row, col, Some(slot))?;
    Ok(())
}

/// One clipboard cell.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ClipValue {
    /// Blank cell or blank cached formula result.
    Empty,
    /// Number.
    Number(f64),
    /// Boolean.
    Bool(bool),
    /// Text with no workbook-local interner id.
    Text(String),
    /// Cell error.
    Error(ErrorKind),
}

impl ClipValue {
    fn number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }
}

/// One clipboard cell with workbook-local ids expanded to portable values.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipCell {
    /// Formula-bar text (leading `=` if formula).
    pub input: String,
    /// Stored value, including the cached result of a formula.
    pub value: ClipValue,
    /// Complete cell style for ordinary and formats-only paste.
    pub style: Style,
    /// Packed protection/recalc flags.
    pub flags: CellFlags,
    /// Source number-format code, used to remap custom ids safely.
    pub number_format: Option<String>,
    /// Source column width in pixels.
    pub column_width_px: u32,
}

/// Copy `range` into a grid of clip cells.
pub fn copy_range(wb: &Workbook, sheet: SheetId, range: RangeRef) -> Vec<Vec<ClipCell>> {
    let (r0, c0, r1, c1) = norm(range);
    let mut rows = Vec::new();
    for r in r0..=r1 {
        let mut row = Vec::new();
        for c in c0..=c1 {
            row.push(clip_one(wb, sheet, r, c));
        }
        rows.push(row);
    }
    rows
}

fn clip_one(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> ClipCell {
    let column_width_px = wb
        .sheet(sheet)
        .and_then(|s| s.geometry.cols.size(u32::from(col)).ok())
        .unwrap_or(crate::geometry::DEFAULT_COL_PX);
    let Ok(Some(slot)) = wb.get(sheet, row, col) else {
        return ClipCell {
            input: String::new(),
            value: ClipValue::Empty,
            style: Style::default(),
            flags: CellFlags::DEFAULT,
            number_format: None,
            column_width_px,
        };
    };
    let input = if let Some(fid) = slot.formula {
        wb.intern().formulas.get(fid).unwrap_or("").to_string()
    } else {
        match slot.value {
            Value::Number(n) => n.to_string(),
            Value::Bool(true) => "TRUE".into(),
            Value::Bool(false) => "FALSE".into(),
            Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
            Value::Error(k) => k.as_str().to_string(),
            _ => String::new(),
        }
    };
    let value = match slot.value {
        Value::Empty | Value::Array(_) => ClipValue::Empty,
        Value::Number(value) => ClipValue::Number(value),
        Value::Bool(value) => ClipValue::Bool(value),
        Value::Text(id) => {
            ClipValue::Text(wb.intern().strings.get(id).unwrap_or_default().to_string())
        }
        Value::Error(value) => ClipValue::Error(value),
    };
    let style = wb
        .intern()
        .styles
        .get(slot.style)
        .cloned()
        .unwrap_or_default();
    let number_format = wb.num_fmt_code(style.num_fmt).map(|code| code.into_owned());
    ClipCell {
        input,
        value,
        style,
        flags: slot.flags,
        number_format,
        column_width_px,
    }
}

/// Paste `grid` at `dest` with special options.
pub fn paste_special(
    wb: &mut Workbook,
    sheet: SheetId,
    dest: CellRef,
    grid: &[Vec<ClipCell>],
    spec: PasteSpecial,
    src_origin: Option<(u32, u16)>,
) -> Result<u32, CoreError> {
    validate_paste_bounds(dest, grid, spec.transpose)?;
    let ordinary = !spec.values
        && !spec.formulas
        && !spec.formats
        && !spec.number_formats
        && !spec.column_widths
        && spec.operation == PasteOp::None
        && !spec.paste_link;
    let mut changed = 0u32;
    for (source_row, cells) in grid.iter().enumerate() {
        for (source_col, cell) in cells.iter().enumerate() {
            let (rr, cc) = if spec.transpose {
                (source_col as u32, source_row as u16)
            } else {
                (source_row as u32, source_col as u16)
            };
            if spec.skip_blanks && cell.input.is_empty() {
                continue;
            }
            let row = dest.row + rr;
            let col = dest.col + cc;
            if spec.paste_link {
                if let Some((sr, sc)) = src_origin {
                    let source_row = sr + source_row as u32;
                    let source_col = sc + source_col as u16;
                    let letters = col_to_letters(source_col)?;
                    let input = format!("=${letters}${}", source_row + 1);
                    wb.set_cell_contents(sheet, row, col, &input)?;
                    changed += 1;
                }
                continue;
            }
            if spec.operation != PasteOp::None {
                let src_n = cell.value.number().unwrap_or(0.0);
                let dst_n = match wb.get(sheet, row, col).ok().flatten().map(|s| s.value) {
                    Some(Value::Number(n)) => n,
                    _ => 0.0,
                };
                if spec.operation == PasteOp::Div && src_n == 0.0 {
                    set_clip_value(wb, sheet, row, col, &ClipValue::Error(ErrorKind::Div0))?;
                } else {
                    let n = match spec.operation {
                        PasteOp::Add => dst_n + src_n,
                        PasteOp::Sub => dst_n - src_n,
                        PasteOp::Mul => dst_n * src_n,
                        PasteOp::Div => dst_n / src_n,
                        PasteOp::None => src_n,
                    };
                    wb.set_number(sheet, row, col, n)?;
                }
                changed += 1;
                continue;
            }

            if ordinary || spec.formulas || spec.values {
                if spec.values && !spec.formulas && cell.input.starts_with('=') {
                    set_clip_value(wb, sheet, row, col, &cell.value)?;
                } else if (ordinary || spec.formulas) && cell.input.starts_with('=') {
                    let (source_row, source_col) = src_origin
                        .map(|(sr, sc)| (sr + source_row as u32, sc + source_col as u16))
                        .unwrap_or((row, col));
                    let drow = row as i32 - source_row as i32;
                    let dcol = i32::from(col) - i32::from(source_col);
                    let input = rewrite_print(&cell.input, &RewriteOp::Copy { dcol, drow })
                        .unwrap_or_else(|_| cell.input.clone());
                    wb.set_cell_contents(sheet, row, col, &input)?;
                } else {
                    set_clip_value(wb, sheet, row, col, &cell.value)?;
                }
                changed += 1;
            }
            if ordinary || spec.formats {
                apply_clip_style(wb, sheet, row, col, cell, true)?;
            } else if spec.number_formats {
                apply_clip_style(wb, sheet, row, col, cell, false)?;
            }
            if spec.column_widths && source_row == 0 {
                wb.set_col_width(sheet, col, cell.column_width_px)?;
            }
        }
    }
    Ok(changed)
}

fn validate_paste_bounds(
    dest: CellRef,
    grid: &[Vec<ClipCell>],
    transpose: bool,
) -> Result<(), CoreError> {
    let rows = u32::try_from(grid.len()).map_err(|_| CoreError::addr_ref("paste is too tall"))?;
    let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    let cols = u32::try_from(cols).map_err(|_| CoreError::addr_ref("paste is too wide"))?;
    let (height, width) = if transpose {
        (cols, rows)
    } else {
        (rows, cols)
    };
    if height > MAX_ROWS - dest.row || width > u32::from(MAX_COLS - dest.col) {
        return Err(CoreError::addr_ref("paste exceeds the worksheet grid"));
    }
    Ok(())
}

fn set_clip_value(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    value: &ClipValue,
) -> Result<(), CoreError> {
    match value {
        ClipValue::Empty => {
            wb.set_cell_contents(sheet, row, col, "")?;
        }
        ClipValue::Number(value) => {
            wb.set_number(sheet, row, col, *value)?;
        }
        ClipValue::Bool(value) => {
            wb.set_cell_contents(sheet, row, col, if *value { "TRUE" } else { "FALSE" })?;
        }
        ClipValue::Text(value) => {
            wb.set_text(sheet, row, col, value)?;
        }
        ClipValue::Error(value) => {
            let previous = wb.get(sheet, row, col)?.copied();
            wb.set_slot(
                sheet,
                row,
                col,
                CellSlot {
                    value: Value::Error(*value),
                    formula: None,
                    style: previous
                        .map(|slot| slot.style)
                        .unwrap_or(crate::style::StyleId::DEFAULT),
                    flags: previous
                        .map(|slot| slot.flags)
                        .unwrap_or(CellFlags::DEFAULT),
                },
            )?;
        }
    }
    Ok(())
}

fn apply_clip_style(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    cell: &ClipCell,
    full: bool,
) -> Result<(), CoreError> {
    let mut style = if full {
        cell.style.clone()
    } else {
        wb.get(sheet, row, col)?
            .and_then(|slot| wb.intern().styles.get(slot.style))
            .cloned()
            .unwrap_or_default()
    };
    if let Some(code) = &cell.number_format {
        style.num_fmt = wb.intern_num_fmt(code)?;
    }
    wb.set_cell_style(sheet, row, col, style)?;
    if full {
        let mut slot = wb
            .get(sheet, row, col)?
            .copied()
            .unwrap_or_else(CellSlot::empty);
        slot.flags = cell.flags;
        wb.set_slot(sheet, row, col, slot)?;
    }
    Ok(())
}

/// Move `src` to `dest` (cut semantics: retarget refs, clear source).
pub fn move_range_cells(
    wb: &mut Workbook,
    sheet: SheetId,
    src: RangeRef,
    dest: CellRef,
) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(src);
    let height = r1 - r0 + 1;
    let width = c1 - c0 + 1;
    if height > MAX_ROWS - dest.row || u32::from(width) > u32::from(MAX_COLS - dest.col) {
        return Err(CoreError::addr_ref("move exceeds the worksheet grid"));
    }
    let grid: Vec<Vec<Option<CellSlot>>> = (r0..=r1)
        .map(|row| {
            (c0..=c1)
                .map(|col| wb.get(sheet, row, col).map(|slot| slot.copied()))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut changed = 0u32;
    for (dr, cells) in grid.iter().enumerate() {
        for (dc, slot) in cells.iter().enumerate() {
            let row = dest.row + dr as u32;
            let col = dest.col + dc as u16;
            replace_cell_slot(wb, sheet, row, col, *slot)?;
            changed += 1;
        }
    }
    for r in r0..=r1 {
        for c in c0..=c1 {
            let in_dest =
                r >= dest.row && r < dest.row + height && c >= dest.col && c < dest.col + width;
            if !in_dest {
                wb.clear_cell(sheet, r, c)?;
            }
        }
    }
    move_side_tables(wb, sheet, src, dest)?;
    rewrite_formulas_move(wb, sheet, src, dest)?;
    Ok(changed)
}

fn move_side_tables(
    wb: &mut Workbook,
    sheet: SheetId,
    src: RangeRef,
    dest: CellRef,
) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(src);
    let drow = dest.row as i64 - r0 as i64;
    let dcol = i64::from(dest.col) - i64::from(c0);
    let target = |row: u32, col: u16| -> (u32, u16) {
        (
            (i64::from(row) + drow) as u32,
            (i64::from(col) + dcol) as u16,
        )
    };
    let sheet_ref = wb.sheet_mut(sheet)?;
    for merge in &sheet_ref.merges {
        let (mr0, mc0, mr1, mc1) = norm(*merge);
        let fully_inside = mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1;
        let overlaps_source = mr0 <= r1 && r0 <= mr1 && mc0 <= c1 && c0 <= mc1;
        if overlaps_source && !fully_inside {
            return Err(CoreError::new(
                "edit.move.merge",
                "move range partially overlaps a merged area",
            ));
        }
    }
    move_map_entries(&mut sheet_ref.notes, r0, c0, r1, c1, target);
    move_map_entries(&mut sheet_ref.comments, r0, c0, r1, c1, target);
    move_map_entries(&mut sheet_ref.hyperlinks, r0, c0, r1, c1, target);
    for merge in &mut sheet_ref.merges {
        let (mr0, mc0, mr1, mc1) = norm(*merge);
        if mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1 {
            let (start_row, start_col) = target(merge.start.row, merge.start.col);
            let (end_row, end_col) = target(merge.end.row, merge.end.col);
            *merge = RangeRef::from_corners(
                CellRef::new(start_row, start_col)?,
                CellRef::new(end_row, end_col)?,
            );
        }
    }
    Ok(())
}

fn move_map_entries<T: Clone>(
    map: &mut FxHashMap<(u32, u16), T>,
    r0: u32,
    c0: u16,
    r1: u32,
    c1: u16,
    target: impl Fn(u32, u16) -> (u32, u16),
) {
    let moved: Vec<_> = map
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(coord, value)| (*coord, value.clone()))
        .collect();
    for ((row, col), value) in moved {
        map.remove(&(row, col));
        map.insert(target(row, col), value);
    }
}

fn rewrite_formulas_move(
    wb: &mut Workbook,
    target: SheetId,
    src: RangeRef,
    dest: CellRef,
) -> Result<(), CoreError> {
    let target_name = wb
        .sheet(target)
        .map(|sheet| sheet.name.clone())
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?;
    let sheet_ids: Vec<SheetId> = wb.sheets().map(|sheet| sheet.id).collect();
    let mut updates = Vec::new();
    for id in sheet_ids {
        let home_name = wb
            .sheet(id)
            .map(|sheet| sheet.name.clone())
            .unwrap_or_default();
        let cells: Vec<_> = wb
            .sheet(id)
            .map(|sheet| sheet.store.iter().collect())
            .unwrap_or_default();
        for (row, col, slot) in cells {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(source) = wb.intern().formulas.get(fid).map(str::to_string) else {
                continue;
            };
            let Ok(parsed) = parse(&source) else {
                continue;
            };
            let ast = map_sheet_move(&parsed.ast, &home_name, &target_name, src, dest);
            let printed = print(&Formula {
                ast,
                style: parsed.style,
                base_row: parsed.base_row,
                base_col: parsed.base_col,
            });
            if printed != source {
                updates.push((id, row, col, printed));
            }
        }
    }
    for (id, row, col, source) in updates {
        wb.set_cell_contents(id, row, col, &source)?;
    }
    Ok(())
}

fn map_sheet_move(expr: &Expr, home: &str, target: &str, src: RangeRef, dest: CellRef) -> Expr {
    let applies = |sheet: &Option<SheetSpec>| match sheet {
        None => home.eq_ignore_ascii_case(target),
        Some(spec) => spec.start.eq_ignore_ascii_case(target),
    };
    expr.clone().map(&mut |item| {
        let kind = match item.kind {
            ExprKind::Cell { sheet, cell } if applies(&sheet) => {
                match move_range(
                    &Expr {
                        kind: ExprKind::Cell { sheet: None, cell },
                        span: item.span,
                    },
                    src,
                    dest,
                )
                .kind
                {
                    ExprKind::Cell { cell, .. } => ExprKind::Cell { sheet, cell },
                    other => other,
                }
            }
            ExprKind::Range { sheet, range } if applies(&sheet) => {
                match move_range(
                    &Expr {
                        kind: ExprKind::Range { sheet: None, range },
                        span: item.span,
                    },
                    src,
                    dest,
                )
                .kind
                {
                    ExprKind::Range { range, .. } => ExprKind::Range { sheet, range },
                    other => other,
                }
            }
            other => other,
        };
        Expr {
            kind,
            span: item.span,
        }
    })
}

/// Split the first row of `range` by `delim` into adjacent columns.
pub fn text_to_columns(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    delim: char,
) -> Result<u32, CoreError> {
    let (r0, c0, r1, _) = norm(range);
    let mut changed = 0u32;
    for r in r0..=r1 {
        let text = match wb.get(sheet, r, c0).ok().flatten() {
            Some(slot) => match slot.value {
                Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            },
            None => String::new(),
        };
        let parts: Vec<&str> = text.split(delim).collect();
        if parts.len() > usize::from(MAX_COLS - c0) {
            return Err(CoreError::addr_ref(
                "text-to-columns output exceeds the worksheet grid",
            ));
        }
        for (i, part) in parts.into_iter().enumerate() {
            let offset = u16::try_from(i)
                .map_err(|_| CoreError::addr_ref("text-to-columns output is too wide"))?;
            let col = c0 + offset;
            wb.set_text(sheet, r, col, part)?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Remove duplicate rows in `range` comparing the listed relative columns.
pub fn remove_duplicates(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    columns: &[u16],
) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    let cols: Vec<u16> = if columns.is_empty() {
        (c0..=c1).collect()
    } else {
        columns
            .iter()
            .map(|offset| {
                let col = c0
                    .checked_add(*offset)
                    .ok_or_else(|| CoreError::addr_ref("duplicate key column is out of range"))?;
                if col > c1 {
                    return Err(CoreError::addr_ref(
                        "duplicate key column is outside the selected range",
                    ));
                }
                Ok(col)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut kept = Vec::new();
    for r in r0..=r1 {
        let key: Vec<String> = cols
            .iter()
            .map(|&c| clip_one(wb, sheet, r, c).input)
            .collect();
        if seen.insert(key) {
            let row = (c0..=c1)
                .map(|col| wb.get(sheet, r, col).map(|slot| slot.copied()))
                .collect::<Result<Vec<_>, _>>()?;
            kept.push(row);
        }
    }
    let total = usize::try_from(r1 - r0 + 1).unwrap_or(usize::MAX);
    let removed = u32::try_from(total.saturating_sub(kept.len())).unwrap_or(u32::MAX);
    for (offset, row) in kept.iter().enumerate() {
        let target_row = r0 + offset as u32;
        for (column, slot) in (c0..=c1).zip(row) {
            replace_cell_slot(wb, sheet, target_row, column, *slot)?;
        }
    }
    let first_blank = r0 + kept.len() as u32;
    for row in first_blank..=r1 {
        for col in c0..=c1 {
            wb.clear_cell(sheet, row, col)?;
        }
    }
    Ok(removed)
}

/// Sum matching positions from `sources` into `dest`.
pub fn consolidate_by_position(
    wb: &mut Workbook,
    dest_sheet: SheetId,
    dest: CellRef,
    sources: &[(SheetId, RangeRef)],
) -> Result<u32, CoreError> {
    let mut acc: FxHashMap<(u32, u16), f64> = FxHashMap::default();
    let mut h = 0u32;
    let mut w = 0u16;
    for &(sid, range) in sources {
        let (r0, c0, r1, c1) = norm(range);
        h = h.max(r1 - r0 + 1);
        w = w.max(c1 - c0 + 1);
        for r in r0..=r1 {
            for c in c0..=c1 {
                if let Ok(Some(slot)) = wb.get(sid, r, c)
                    && let Value::Number(n) = slot.value
                {
                    *acc.entry((r - r0, c - c0)).or_insert(0.0) += n;
                }
            }
        }
    }
    let mut changed = 0u32;
    for r in 0..h {
        for c in 0..w {
            if let Some(&n) = acc.get(&(r, c)) {
                wb.set_number(dest_sheet, dest.row + r, dest.col + c, n)?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// Formula source at a cell, if any.
#[must_use]
pub fn formula_src(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    let Ok(Some(slot)) = wb.get(sheet, row, col) else {
        return String::new();
    };
    slot.formula
        .and_then(|fid| wb.intern().formulas.get(fid).map(str::to_string))
        .unwrap_or_default()
}

/// Default column auto-fit width in pixels from display text.
#[must_use]
pub fn autofit_width(text: &str) -> u32 {
    (text.chars().count() as u32)
        .saturating_mul(8)
        .saturating_add(12)
        .max(24)
}

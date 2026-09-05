//! Structural edits, fill, paste-special, and protection (WP-17).

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::addr::{
    CellRef, RangeRef, RefKind, SheetId, SheetSpec, col_to_letters, parse_a1, quote_sheet_name,
};
use crate::dates::{CivilDate, DateSystem, date_to_serial, serial_to_date};
use crate::error::{CoreError, ErrorKind};
use crate::formula::{
    Expr, ExprKind, Formula, RewriteOp, adjust_cols, adjust_rows, move_range, parse, print,
    rewrite_print,
};
use crate::intern::RichTextRun;
use crate::limits::{MAX_COLS, MAX_ROWS};
use crate::names::{DefinedName, NameReferent, NameScope};
use crate::sheet::{Comment, Hyperlink, Note};
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
    wb.ensure_range_not_array_formula_output(
        sheet,
        at,
        0,
        MAX_ROWS.saturating_sub(1),
        MAX_COLS.saturating_sub(1),
    )?;
    wb.insert_rows(sheet, at, count)?;
    rewrite_table_rows(wb, sheet, at, count, false)?;
    shift_side_tables(wb, sheet, at, count as i32, true)?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Rows {
            at,
            count,
            delete: false,
        },
    )?;
    rewrite_ai_redact_marks_rows(wb, sheet, at, count, false);
    Ok(())
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
    wb.ensure_range_not_array_formula_output(
        sheet,
        at,
        0,
        MAX_ROWS.saturating_sub(1),
        MAX_COLS.saturating_sub(1),
    )?;
    wb.delete_rows(sheet, at, count)?;
    rewrite_table_rows(wb, sheet, at, count, true)?;
    shift_side_tables(wb, sheet, at, -(count as i32), true)?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Rows {
            at,
            count,
            delete: true,
        },
    )?;
    rewrite_ai_redact_marks_rows(wb, sheet, at, count, true);
    Ok(())
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
    wb.ensure_range_not_array_formula_output(
        sheet,
        0,
        at,
        MAX_ROWS.saturating_sub(1),
        MAX_COLS.saturating_sub(1),
    )?;
    wb.insert_cols(sheet, at, count)?;
    rewrite_table_cols(wb, sheet, at, count, false)?;
    shift_side_tables_cols(wb, sheet, at, count as i32)?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Cols {
            at,
            count,
            delete: false,
        },
    )?;
    rewrite_ai_redact_marks_cols(wb, sheet, at, count, false);
    Ok(())
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
    wb.ensure_range_not_array_formula_output(
        sheet,
        0,
        at,
        MAX_ROWS.saturating_sub(1),
        MAX_COLS.saturating_sub(1),
    )?;
    wb.delete_cols(sheet, at, count)?;
    rewrite_table_cols(wb, sheet, at, count, true)?;
    shift_side_tables_cols(wb, sheet, at, -(count as i32))?;
    rewrite_formulas(
        wb,
        sheet,
        RewriteKind::Cols {
            at,
            count,
            delete: true,
        },
    )?;
    rewrite_ai_redact_marks_cols(wb, sheet, at, count, true);
    Ok(())
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
            wb.ensure_range_not_array_formula_output(
                sheet,
                r0,
                c0,
                MAX_ROWS.saturating_sub(1),
                c1,
            )?;
            wb.rewrite_pivots_after_row_band(sheet, r0, n as i32, c0, c1)?;
            shift_band_rows(wb, sheet, r0, n, c0, c1, false)?;
            shift_band_side_tables_rows(wb, sheet, r0, n, c0, c1, false)?;
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
            )?;
            rewrite_ai_redact_marks_band_rows(wb, sheet, r0, n, c0, c1, false);
            Ok(())
        }
        Shift::Right => {
            let n = c1.saturating_sub(c0).saturating_add(1);
            wb.ensure_range_not_array_formula_output(
                sheet,
                r0,
                c0,
                r1,
                MAX_COLS.saturating_sub(1),
            )?;
            wb.rewrite_pivots_after_col_band(sheet, c0, i32::from(n), r0, r1)?;
            shift_band_cols(wb, sheet, c0, n, r0, r1, false)?;
            shift_band_side_tables_cols(wb, sheet, c0, n, r0, r1, false)?;
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
            )?;
            rewrite_ai_redact_marks_band_cols(wb, sheet, c0, n, r0, r1, false);
            Ok(())
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
            wb.ensure_range_not_array_formula_output(
                sheet,
                r0,
                c0,
                MAX_ROWS.saturating_sub(1),
                c1,
            )?;
            wb.rewrite_pivots_after_row_band(sheet, r0, -(n as i32), c0, c1)?;
            shift_band_rows(wb, sheet, r0, n, c0, c1, true)?;
            shift_band_side_tables_rows(wb, sheet, r0, n, c0, c1, true)?;
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
            )?;
            rewrite_ai_redact_marks_band_rows(wb, sheet, r0, n, c0, c1, true);
            Ok(())
        }
        Shift::Right => {
            let n = c1.saturating_sub(c0).saturating_add(1);
            wb.ensure_range_not_array_formula_output(
                sheet,
                r0,
                c0,
                r1,
                MAX_COLS.saturating_sub(1),
            )?;
            wb.rewrite_pivots_after_col_band(sheet, c0, -i32::from(n), r0, r1)?;
            shift_band_cols(wb, sheet, c0, n, r0, r1, true)?;
            shift_band_side_tables_cols(wb, sheet, c0, n, r0, r1, true)?;
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
            )?;
            rewrite_ai_redact_marks_band_cols(wb, sheet, c0, n, r0, r1, true);
            Ok(())
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
    let held_slots: Vec<CellSlot> = cells.iter().map(|(_, _, slot)| *slot).collect();
    wb.with_held_slots(&held_slots, |wb| {
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
    })
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
    let held_slots: Vec<CellSlot> = cells.iter().map(|(_, _, slot)| *slot).collect();
    wb.with_held_slots(&held_slots, |wb| {
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
    })
}

fn shift_band_side_tables_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u32,
    count: u32,
    c0: u16,
    c1: u16,
    delete: bool,
) -> Result<(), CoreError> {
    let signed = if delete {
        -(count as i32)
    } else {
        count as i32
    };
    wb.mutate_sheet_edit(sheet, |target| {
        target.notes = shift_map_band_rows(&target.notes, at, signed, c0, c1);
        target.comments = shift_map_band_rows(&target.comments, at, signed, c0, c1);
        target.hyperlinks = shift_map_band_rows(&target.hyperlinks, at, signed, c0, c1);
        target.merges = target
            .merges
            .iter()
            .filter_map(|merge| {
                let (_, mc0, _, mc1) = norm(*merge);
                if mc0 >= c0 && mc1 <= c1 {
                    shift_range_rows(*merge, at, signed)
                } else {
                    Some(*merge)
                }
            })
            .collect();
        Ok(())
    })
}

fn shift_band_side_tables_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u16,
    count: u16,
    r0: u32,
    r1: u32,
    delete: bool,
) -> Result<(), CoreError> {
    let signed = if delete {
        -(count as i32)
    } else {
        count as i32
    };
    wb.mutate_sheet_edit(sheet, |target| {
        target.notes = shift_map_band_cols(&target.notes, at, signed, r0, r1);
        target.comments = shift_map_band_cols(&target.comments, at, signed, r0, r1);
        target.hyperlinks = shift_map_band_cols(&target.hyperlinks, at, signed, r0, r1);
        target.merges = target
            .merges
            .iter()
            .filter_map(|merge| {
                let (mr0, _, mr1, _) = norm(*merge);
                if mr0 >= r0 && mr1 <= r1 {
                    shift_range_cols(*merge, at, signed)
                } else {
                    Some(*merge)
                }
            })
            .collect();
        Ok(())
    })
}

fn shift_map_band_rows<T: Clone>(
    map: &FxHashMap<(u32, u16), T>,
    at: u32,
    count: i32,
    c0: u16,
    c1: u16,
) -> FxHashMap<(u32, u16), T> {
    let mut next = FxHashMap::default();
    for (&(row, col), value) in map {
        if col < c0 || col > c1 {
            next.insert((row, col), value.clone());
            continue;
        }
        let shifted = shift_index(row, at, count);
        if let Some(row) = shifted.filter(|row| *row < MAX_ROWS) {
            next.insert((row, col), value.clone());
        }
    }
    next
}

fn shift_map_band_cols<T: Clone>(
    map: &FxHashMap<(u32, u16), T>,
    at: u16,
    count: i32,
    r0: u32,
    r1: u32,
) -> FxHashMap<(u32, u16), T> {
    let mut next = FxHashMap::default();
    for (&(row, col), value) in map {
        if row < r0 || row > r1 {
            next.insert((row, col), value.clone());
            continue;
        }
        let shifted = shift_index(u32::from(col), u32::from(at), count)
            .filter(|col| *col < u32::from(MAX_COLS))
            .and_then(|col| u16::try_from(col).ok());
        if let Some(col) = shifted {
            next.insert((row, col), value.clone());
        }
    }
    next
}

fn shift_index(index: u32, at: u32, count: i32) -> Option<u32> {
    let magnitude = count.unsigned_abs();
    if count >= 0 {
        Some(if index >= at {
            index.saturating_add(magnitude)
        } else {
            index
        })
    } else if index < at {
        Some(index)
    } else if index < at.saturating_add(magnitude) {
        None
    } else {
        Some(index - magnitude)
    }
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
    rewrite_defined_names(wb, target, &target_name, &kind)?;
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
            let new_ast = apply_rewrite_kind(&parsed.ast, &home_name, &target_name, &kind);
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
        let cse_range = wb
            .sheet(id)
            .and_then(|sheet| sheet.array_formula_at(row, col))
            .filter(|formula| formula.anchor.row == row && formula.anchor.col == col)
            .map(|formula| formula.range);
        if let Some(range) = cse_range {
            wb.set_array_formula_text(id, range, &src)?;
        } else {
            wb.set_cell_contents(id, row, col, &src)?;
        }
    }
    Ok(())
}

fn apply_rewrite_kind(expr: &Expr, home: &str, target: &str, kind: &RewriteKind) -> Expr {
    match kind {
        RewriteKind::Rows { at, count, delete } => {
            map_sheet_rows(expr, home, target, *at, *count, *delete)
        }
        RewriteKind::Cols { at, count, delete } => {
            map_sheet_cols(expr, home, target, *at, *count, *delete)
        }
        RewriteKind::BandRows {
            at,
            count,
            c0,
            c1,
            delete,
        } => map_sheet_band_rows(expr, home, target, *at, *count, *c0, *c1, *delete),
        RewriteKind::BandCols {
            at,
            count,
            r0,
            r1,
            delete,
        } => map_sheet_band_cols(expr, home, target, *at, *count, *r0, *r1, *delete),
    }
}

fn rewrite_defined_names(
    wb: &mut Workbook,
    target: SheetId,
    target_name: &str,
    kind: &RewriteKind,
) -> Result<(), CoreError> {
    let names: Vec<DefinedName> = wb.names().iter().cloned().collect();
    let mut updates = Vec::new();
    for mut name in names {
        let before = name.clone();
        let home_name = match name.scope {
            NameScope::Sheet(sheet) => wb
                .sheet(sheet)
                .map(|sheet| sheet.name.clone())
                .unwrap_or_default(),
            NameScope::Workbook => String::new(),
        };
        name.referent = match &name.referent {
            NameReferent::Formula(source) => {
                let Ok(parsed) = parse(source) else {
                    continue;
                };
                let ast = apply_rewrite_kind(&parsed.ast, &home_name, target_name, kind);
                NameReferent::Formula(print(&Formula {
                    ast,
                    style: parsed.style,
                    base_row: parsed.base_row,
                    base_col: parsed.base_col,
                }))
            }
            NameReferent::Range(range) if name_range_targets(*range, name.scope, target) => {
                let transformed = match kind {
                    RewriteKind::Rows { at, count, delete } => adjusted_ref(adjust_rows(
                        &ref_expr(RefKind::Range(*range)),
                        *at,
                        *count,
                        *delete,
                    )),
                    RewriteKind::Cols { at, count, delete } => adjusted_ref(adjust_cols(
                        &ref_expr(RefKind::Range(*range)),
                        *at,
                        *count,
                        *delete,
                    )),
                    RewriteKind::BandRows {
                        at,
                        count,
                        c0,
                        c1,
                        delete,
                    } if range.start.col.min(range.end.col) >= *c0
                        && range.start.col.max(range.end.col) <= *c1 =>
                    {
                        adjusted_ref(adjust_rows(
                            &ref_expr(RefKind::Range(*range)),
                            *at,
                            *count,
                            *delete,
                        ))
                    }
                    RewriteKind::BandCols {
                        at,
                        count,
                        r0,
                        r1,
                        delete,
                    } if range.start.row.min(range.end.row) >= *r0
                        && range.start.row.max(range.end.row) <= *r1 =>
                    {
                        adjusted_ref(adjust_cols(
                            &ref_expr(RefKind::Range(*range)),
                            *at,
                            *count,
                            *delete,
                        ))
                    }
                    _ => Some(RefKind::Range(*range)),
                };
                match transformed {
                    Some(RefKind::Range(range)) => NameReferent::Range(range),
                    Some(RefKind::Cell(cell)) => {
                        NameReferent::Range(RangeRef::from_corners(cell, cell))
                    }
                    None => NameReferent::Formula("=#REF!".into()),
                }
            }
            other => other.clone(),
        };
        if name != before {
            updates.push((before, name));
        }
    }
    for (before, after) in updates {
        wb.remove_name(before.scope, &before.name)?;
        wb.define_name(after)?;
    }
    Ok(())
}

fn name_range_targets(range: RangeRef, scope: NameScope, target: SheetId) -> bool {
    range.start.sheet == Some(target)
        || (range.start.sheet.is_none()
            && matches!(scope, NameScope::Sheet(sheet) if sheet == target))
}

fn rewrite_table_rows(
    wb: &mut Workbook,
    target: SheetId,
    at: u32,
    count: u32,
    delete: bool,
) -> Result<(), CoreError> {
    let tables: Vec<_> = wb
        .tables()
        .iter()
        .filter(|table| table.sheet == target)
        .cloned()
        .collect();
    for before in tables {
        let range = RangeRef::from_corners(
            CellRef::new(before.start_row, before.start_col)?.on_sheet(target),
            CellRef::new(before.end_row, before.end_col)?.on_sheet(target),
        );
        match adjusted_ref(adjust_rows(
            &ref_expr(RefKind::Range(range)),
            at,
            count,
            delete,
        )) {
            Some(RefKind::Range(range)) => {
                let mut after = before;
                after.start_row = range.start.row;
                after.end_row = range.end.row;
                wb.restore_table(after)?;
            }
            _ => {
                wb.convert_table(before.id)?;
            }
        }
    }
    Ok(())
}

fn rewrite_table_cols(
    wb: &mut Workbook,
    target: SheetId,
    at: u16,
    count: u16,
    delete: bool,
) -> Result<(), CoreError> {
    let tables: Vec<_> = wb
        .tables()
        .iter()
        .filter(|table| table.sheet == target)
        .cloned()
        .collect();
    for before in tables {
        let range = RangeRef::from_corners(
            CellRef::new(before.start_row, before.start_col)?.on_sheet(target),
            CellRef::new(before.end_row, before.end_col)?.on_sheet(target),
        );
        match adjusted_ref(adjust_cols(
            &ref_expr(RefKind::Range(range)),
            at,
            count,
            delete,
        )) {
            Some(RefKind::Range(range)) => {
                let mut after = before.clone();
                after.start_col = range.start.col;
                after.end_col = range.end.col;
                if !delete && at > before.start_col && at <= before.end_col {
                    let offset = usize::from(at - before.start_col);
                    for index in 0..count {
                        after.columns.insert(
                            offset + usize::from(index),
                            crate::tables::TableColumn {
                                name: format!("Column{}", offset + usize::from(index) + 1),
                                totals_fn: None,
                            },
                        );
                    }
                } else if delete {
                    let deleted_end = at.saturating_add(count.saturating_sub(1));
                    let first = at.max(before.start_col);
                    let last = deleted_end.min(before.end_col);
                    if first <= last {
                        let offset = usize::from(first - before.start_col);
                        let removed = usize::from(last - first) + 1;
                        after.columns.drain(offset..offset + removed);
                    }
                }
                wb.restore_table(after)?;
            }
            _ => {
                wb.convert_table(before.id)?;
            }
        }
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
    expr.clone().map_local(&mut |e| {
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
    expr.clone().map_local(&mut |e| {
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
    expr.clone().map_local(&mut |item| {
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
    expr.clone().map_local(&mut |item| {
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

const AI_PRIVACY_PART: &str = "xl/omacell/ai.json";

fn rewrite_ai_redact_marks_rows(
    wb: &mut Workbook,
    target: SheetId,
    at: u32,
    count: u32,
    delete: bool,
) {
    rewrite_ai_redact_marks(wb, target, |kind| {
        adjusted_ref(adjust_rows(&ref_expr(kind), at, count, delete))
    });
}

fn rewrite_ai_redact_marks_cols(
    wb: &mut Workbook,
    target: SheetId,
    at: u16,
    count: u16,
    delete: bool,
) {
    rewrite_ai_redact_marks(wb, target, |kind| {
        adjusted_ref(adjust_cols(&ref_expr(kind), at, count, delete))
    });
}

fn rewrite_ai_redact_marks_band_rows(
    wb: &mut Workbook,
    target: SheetId,
    at: u32,
    count: u32,
    c0: u16,
    c1: u16,
    delete: bool,
) {
    rewrite_ai_redact_marks(wb, target, |kind| match kind {
        RefKind::Cell(cell) if cell.col >= c0 && cell.col <= c1 => {
            adjusted_ref(adjust_rows(&ref_expr(kind), at, count, delete))
        }
        RefKind::Range(range) => {
            let (_, mc0, _, mc1) = norm(range);
            if mc1 < c0 || mc0 > c1 {
                Some(kind)
            } else if mc0 >= c0 && mc1 <= c1 {
                adjusted_ref(adjust_rows(&ref_expr(kind), at, count, delete))
            } else {
                None
            }
        }
        _ => Some(kind),
    });
}

fn rewrite_ai_redact_marks_band_cols(
    wb: &mut Workbook,
    target: SheetId,
    at: u16,
    count: u16,
    r0: u32,
    r1: u32,
    delete: bool,
) {
    rewrite_ai_redact_marks(wb, target, |kind| match kind {
        RefKind::Cell(cell) if cell.row >= r0 && cell.row <= r1 => {
            adjusted_ref(adjust_cols(&ref_expr(kind), at, count, delete))
        }
        RefKind::Range(range) => {
            let (mr0, _, mr1, _) = norm(range);
            if mr1 < r0 || mr0 > r1 {
                Some(kind)
            } else if mr0 >= r0 && mr1 <= r1 {
                adjusted_ref(adjust_cols(&ref_expr(kind), at, count, delete))
            } else {
                None
            }
        }
        _ => Some(kind),
    });
}

fn rewrite_ai_redact_marks(
    wb: &mut Workbook,
    target: SheetId,
    mut transform: impl FnMut(RefKind) -> Option<RefKind>,
) {
    let Some(target_name) = wb.sheet(target).map(|sheet| sheet.name.clone()) else {
        return;
    };
    append_ai_redact_mark_transforms(wb, |parsed| {
        let Some(spec) = parsed.sheet.as_ref() else {
            return Err(());
        };
        if spec.end.is_some() {
            return Err(());
        }
        if !spec.start.eq_ignore_ascii_case(&target_name) {
            return Ok(None);
        }
        let Some(kind) = transform(parsed.kind) else {
            return Err(());
        };
        let mut rewritten = parsed.clone();
        rewritten.kind = kind;
        Ok(Some(rewritten))
    });
}

fn append_ai_redact_mark_transforms(
    wb: &mut Workbook,
    mut transform: impl FnMut(&crate::addr::ParsedRef) -> Result<Option<crate::addr::ParsedRef>, ()>,
) {
    let Some(bytes) = wb.custom_parts.get(AI_PRIVACY_PART) else {
        return;
    };
    let Ok(mut part) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return;
    };
    let Some(marks) = part
        .get_mut("redact")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    let mut fail_closed = false;
    let original_marks = marks.clone();
    let mut additions = Vec::new();
    for mark in &original_marks {
        let Some(text) = mark.as_str() else {
            fail_closed = true;
            continue;
        };
        let Ok(parsed) = parse_a1(text) else {
            fail_closed = true;
            continue;
        };
        let transformed = match transform(&parsed) {
            Ok(Some(transformed)) => transformed.to_a1(),
            Ok(None) => continue,
            Err(()) => {
                fail_closed = true;
                continue;
            }
        };
        if transformed != text {
            additions.push(serde_json::Value::String(transformed));
        }
    }
    for addition in additions {
        if !marks.contains(&addition) {
            marks.push(addition);
        }
    }
    if fail_closed && let Some(object) = part.as_object_mut() {
        object.insert(
            "privacy_send".into(),
            serde_json::Value::String("schema".into()),
        );
    }
    if let Ok(encoded) = serde_json::to_vec(&part) {
        wb.custom_parts.insert(AI_PRIVACY_PART.into(), encoded);
    }
}

pub(crate) fn rewrite_ai_redact_marks_move(
    wb: &mut Workbook,
    sheet: SheetId,
    src: RangeRef,
    dest: CellRef,
) {
    let (r0, c0, r1, c1) = norm(src);
    rewrite_ai_redact_marks(wb, sheet, |kind| {
        if let RefKind::Range(range) = kind {
            let (mr0, mc0, mr1, mc1) = norm(range);
            let intersects = mr0 <= r1 && r0 <= mr1 && mc0 <= c1 && c0 <= mc1;
            let contained = mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1;
            if intersects && !contained {
                return None;
            }
        }
        adjusted_ref(move_range(&ref_expr(kind), src, dest))
    });
}

pub(crate) fn rewrite_ai_redact_marks_move_between(
    wb: &mut Workbook,
    source_sheet: SheetId,
    src: RangeRef,
    dest_sheet: SheetId,
    dest: CellRef,
) {
    let Some(source_name) = wb.sheet(source_sheet).map(|sheet| sheet.name.clone()) else {
        return;
    };
    let Some(dest_name) = wb.sheet(dest_sheet).map(|sheet| sheet.name.clone()) else {
        return;
    };
    let (r0, c0, r1, c1) = norm(src);
    append_ai_redact_mark_transforms(wb, |parsed| {
        let Some(spec) = parsed.sheet.as_ref() else {
            return Err(());
        };
        if spec.end.is_some() {
            return Err(());
        }
        if !spec.start.eq_ignore_ascii_case(&source_name) {
            return Ok(None);
        }
        let (mr0, mc0, mr1, mc1) = match parsed.kind {
            RefKind::Cell(cell) => (cell.row, cell.col, cell.row, cell.col),
            RefKind::Range(range) => norm(range),
        };
        let intersects = mr0 <= r1 && r0 <= mr1 && mc0 <= c1 && c0 <= mc1;
        if !intersects {
            return Ok(None);
        }
        let contained = mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1;
        if !contained {
            return Err(());
        }
        let Some(kind) = adjusted_ref(move_range(&ref_expr(parsed.kind), src, dest)) else {
            return Err(());
        };
        let mut rewritten = parsed.clone();
        rewritten.kind = kind;
        if let Some(spec) = rewritten.sheet.as_mut() {
            spec.start = dest_name.clone();
        }
        Ok(Some(rewritten))
    });
}

pub(crate) fn rewrite_ai_redact_marks_sheet_rename(wb: &mut Workbook, old: &str, new: &str) {
    append_ai_redact_mark_transforms(wb, |parsed| {
        let Some(spec) = parsed.sheet.as_ref() else {
            return Err(());
        };
        if spec.end.is_some() {
            return Err(());
        }
        if !spec.start.eq_ignore_ascii_case(old) {
            return Ok(None);
        }
        let mut rewritten = parsed.clone();
        if let Some(spec) = rewritten.sheet.as_mut() {
            spec.start = new.to_string();
        }
        Ok(Some(rewritten))
    });
}

pub(crate) fn rewrite_ai_redact_marks_sort_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    rows: &FxHashMap<u32, u32>,
    c0: u16,
    c1: u16,
) {
    rewrite_ai_redact_marks(wb, sheet, |kind| remap_mark_rows(kind, rows, c0, c1));
}

pub(crate) fn rewrite_ai_redact_marks_sort_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    cols: &FxHashMap<u16, u16>,
    r0: u32,
    r1: u32,
) {
    rewrite_ai_redact_marks(wb, sheet, |kind| remap_mark_cols(kind, cols, r0, r1));
}

fn remap_mark_rows(kind: RefKind, rows: &FxHashMap<u32, u32>, c0: u16, c1: u16) -> Option<RefKind> {
    match kind {
        RefKind::Cell(mut cell) => {
            if cell.col >= c0 && cell.col <= c1 {
                cell.row = rows.get(&cell.row).copied().unwrap_or(cell.row);
            }
            Some(RefKind::Cell(cell))
        }
        RefKind::Range(mut range) => {
            let (mr0, mc0, mr1, mc1) = norm(range);
            if mc1 < c0 || mc0 > c1 {
                return Some(kind);
            }
            if mc0 < c0 || mc1 > c1 {
                return None;
            }
            if mr0 == mr1 {
                let row = rows.get(&mr0).copied().unwrap_or(mr0);
                range.start.row = row;
                range.end.row = row;
                return Some(RefKind::Range(range));
            }
            if rows.iter().any(|(source, dest)| {
                (*source >= mr0 && *source <= mr1) != (*dest >= mr0 && *dest <= mr1)
            }) {
                return None;
            }
            Some(RefKind::Range(range))
        }
    }
}

fn remap_mark_cols(kind: RefKind, cols: &FxHashMap<u16, u16>, r0: u32, r1: u32) -> Option<RefKind> {
    match kind {
        RefKind::Cell(mut cell) => {
            if cell.row >= r0 && cell.row <= r1 {
                cell.col = cols.get(&cell.col).copied().unwrap_or(cell.col);
            }
            Some(RefKind::Cell(cell))
        }
        RefKind::Range(mut range) => {
            let (mr0, mc0, mr1, mc1) = norm(range);
            if mr1 < r0 || mr0 > r1 {
                return Some(kind);
            }
            if mr0 < r0 || mr1 > r1 {
                return None;
            }
            if mc0 == mc1 {
                let col = cols.get(&mc0).copied().unwrap_or(mc0);
                range.start.col = col;
                range.end.col = col;
                return Some(RefKind::Range(range));
            }
            if cols.iter().any(|(source, dest)| {
                (*source >= mc0 && *source <= mc1) != (*dest >= mc0 && *dest <= mc1)
            }) {
                return None;
            }
            Some(RefKind::Range(range))
        }
    }
}

fn ref_expr(kind: RefKind) -> Expr {
    let kind = match kind {
        RefKind::Cell(cell) => ExprKind::Cell { sheet: None, cell },
        RefKind::Range(range) => ExprKind::Range { sheet: None, range },
    };
    Expr {
        kind,
        span: crate::formula::Span::new(0, 0),
    }
}

fn adjusted_ref(expr: Expr) -> Option<RefKind> {
    match expr.kind {
        ExprKind::Cell { cell, .. } => Some(RefKind::Cell(cell)),
        ExprKind::Range { range, .. } => Some(RefKind::Range(range)),
        _ => None,
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
    wb.mutate_sheet_edit(sheet, |s| {
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
    })
}

fn shift_side_tables_cols(
    wb: &mut Workbook,
    sheet: SheetId,
    at: u16,
    count: i32,
) -> Result<(), CoreError> {
    wb.mutate_sheet_edit(sheet, |s| {
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
    })
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
    let (r0, c0, r1, c1) = norm(range);
    wb.ensure_range_not_pivot_output(sheet, r0, c0, r1, c1)?;
    wb.mutate_sheet_edit(sheet, |target| target.add_merge(range))
}

/// Merge each row of `range` independently (Excel merge-across).
pub fn merge_across(wb: &mut Workbook, sheet: SheetId, range: RangeRef) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    wb.ensure_range_not_pivot_output(sheet, r0, c0, r1, c1)?;
    wb.mutate_sheet_edit(sheet, |target| {
        for r in r0..=r1 {
            let row_range = RangeRef::from_corners(CellRef::new(r, c0)?, CellRef::new(r, c1)?);
            target.add_merge(row_range)?;
        }
        Ok(())
    })
}

/// Unmerge any merge overlapping `range`.
pub fn unmerge(wb: &mut Workbook, sheet: SheetId, range: RangeRef) -> Result<usize, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    wb.ensure_range_not_pivot_output(sheet, r0, c0, r1, c1)?;
    wb.mutate_sheet_edit(sheet, |target| {
        let before = target.merges.len();
        target.merges.retain(|m| !overlaps(*m, range));
        Ok(before - target.merges.len())
    })
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

fn extend_fill_before(values: &[f64], mode: FillMode, n: usize, dates: DateSystem) -> Vec<f64> {
    let Some(&first) = values.first() else {
        return vec![0.0; n];
    };
    match mode {
        FillMode::Copy | FillMode::Formats => vec![first; n],
        FillMode::Linear => {
            let step = if values.len() >= 2 {
                values[1] - values[0]
            } else {
                1.0
            };
            (1..=n).map(|i| first - step * i as f64).collect()
        }
        FillMode::Growth => {
            let ratio = if values.len() >= 2 && values[0].abs() > 1e-12 {
                values[1] / values[0]
            } else {
                1.0
            };
            (1..=n).map(|i| first / ratio.powi(i as i32)).collect()
        }
        FillMode::Date => (1..=n).map(|i| first - i as f64).collect(),
        FillMode::Weekday => {
            let mut out = Vec::with_capacity(n);
            let mut serial = first;
            while out.len() < n {
                serial -= 1.0;
                if let Some(date) = serial_to_date(serial as i64, dates) {
                    let day = weekday(date.year, u32::from(date.month), u32::from(date.day));
                    if day != 0 && day != 6 {
                        out.push(serial);
                    }
                } else {
                    out.push(serial);
                }
            }
            out
        }
        FillMode::Month => (1..=n)
            .map(|i| add_months(first, -(i as i32), dates))
            .collect(),
        FillMode::Year => (1..=n)
            .map(|i| add_months(first, -12 * i as i32, dates))
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

fn column_numbers(
    wb: &Workbook,
    sheet: SheetId,
    first_row: u32,
    last_row: u32,
    col: u16,
) -> Vec<f64> {
    (first_row..=last_row)
        .filter_map(|row| match wb.get(sheet, row, col) {
            Ok(Some(slot)) => match slot.value {
                Value::Number(value) => Some(value),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn row_numbers(wb: &Workbook, sheet: SheetId, row: u32, first_col: u16, last_col: u16) -> Vec<f64> {
    (first_col..=last_col)
        .filter_map(|col| match wb.get(sheet, row, col) {
            Ok(Some(slot)) => match slot.value {
                Value::Number(value) => Some(value),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn fill_extensions<L>(
    lanes: impl IntoIterator<Item = L>,
    mode: FillMode,
    extension_len: usize,
    date_system: DateSystem,
    reverse: bool,
    mut source_values: impl FnMut(L) -> Vec<f64>,
) -> Vec<Vec<f64>> {
    if matches!(mode, FillMode::Copy | FillMode::Formats) {
        return Vec::new();
    }
    lanes
        .into_iter()
        .map(|lane| {
            let values = source_values(lane);
            if reverse {
                extend_fill_before(&values, mode, extension_len, date_system)
            } else {
                extend_fill(&values, mode, extension_len, date_system)
            }
        })
        .collect()
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
    let date_system = wb.settings().date_system;
    if mode != FillMode::Formats {
        wb.ensure_range_not_array_formula_output(sheet, dr0, dc0, dr1, dc1)?;
    }
    let mut changed = 0u32;
    if dr0 >= sr0 && dc0 == sc0 && dc1 == sc1 {
        // fill down
        let extension_len = dr1.saturating_sub(sr1) as usize;
        let extensions =
            fill_extensions(sc0..=sc1, mode, extension_len, date_system, false, |col| {
                column_numbers(wb, sheet, sr0, sr1, col)
            });
        for (i, r) in (sr1.saturating_add(1)..=dr1).enumerate() {
            for (lane, c) in (sc0..=sc1).enumerate() {
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
                        if let Some(value) = extensions.get(lane).and_then(|values| values.get(i)) {
                            wb.set_number(sheet, r, c, *value)?;
                            changed += 1;
                        }
                    }
                }
            }
        }
    } else if dr0 < sr0 && dc0 == sc0 && dc1 == sc1 && dr1 >= sr1 {
        let extension_len = sr0.saturating_sub(dr0) as usize;
        let extensions =
            fill_extensions(sc0..=sc1, mode, extension_len, date_system, true, |col| {
                column_numbers(wb, sheet, sr0, sr1, col)
            });
        for (i, r) in (dr0..sr0).rev().enumerate() {
            for (lane, c) in (sc0..=sc1).enumerate() {
                match mode {
                    FillMode::Copy => {
                        let source_row = sr1 - (sr0 - r - 1) % (sr1 - sr0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, source_row, c) {
                            copy_slot(wb, sheet, *slot, r, c, r as i32 - source_row as i32, 0)?;
                            changed += 1;
                        }
                    }
                    FillMode::Formats => {
                        let source_row = sr1 - (sr0 - r - 1) % (sr1 - sr0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, source_row, c) {
                            copy_slot_format(wb, sheet, *slot, r, c)?;
                            changed += 1;
                        }
                    }
                    _ => {
                        if let Some(value) = extensions.get(lane).and_then(|values| values.get(i)) {
                            wb.set_number(sheet, r, c, *value)?;
                            changed += 1;
                        }
                    }
                }
            }
        }
    } else if dc0 >= sc0 && dr0 == sr0 && dr1 == sr1 {
        let extension_len = usize::from(dc1.saturating_sub(sc1));
        let extensions =
            fill_extensions(sr0..=sr1, mode, extension_len, date_system, false, |row| {
                row_numbers(wb, sheet, row, sc0, sc1)
            });
        for (lane, r) in (sr0..=sr1).enumerate() {
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
                        if let Some(value) = extensions.get(lane).and_then(|values| values.get(i)) {
                            wb.set_number(sheet, r, c, *value)?;
                            changed += 1;
                        }
                    }
                }
            }
        }
    } else if dc0 < sc0 && dr0 == sr0 && dr1 == sr1 && dc1 >= sc1 {
        let extension_len = usize::from(sc0.saturating_sub(dc0));
        let extensions =
            fill_extensions(sr0..=sr1, mode, extension_len, date_system, true, |row| {
                row_numbers(wb, sheet, row, sc0, sc1)
            });
        for (lane, r) in (sr0..=sr1).enumerate() {
            for (i, c) in (dc0..sc0).rev().enumerate() {
                match mode {
                    FillMode::Copy => {
                        let source_col = sc1 - (sc0 - c - 1) % (sc1 - sc0 + 1);
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
                        let source_col = sc1 - (sc0 - c - 1) % (sc1 - sc0 + 1);
                        if let Ok(Some(slot)) = wb.get(sheet, r, source_col) {
                            copy_slot_format(wb, sheet, *slot, r, c)?;
                            changed += 1;
                        }
                    }
                    _ => {
                        if let Some(value) = extensions.get(lane).and_then(|values| values.get(i)) {
                            wb.set_number(sheet, r, c, *value)?;
                            changed += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(changed)
}

/// Fill a one-dimensional destination by cycling a user-provided custom list.
pub fn fill_custom_list(
    wb: &mut Workbook,
    sheet: SheetId,
    src: RangeRef,
    dest: RangeRef,
    list: &[String],
) -> Result<u32, CoreError> {
    if list.is_empty() {
        return Err(CoreError::new(
            "edit.fill.list",
            "custom fill list must not be empty",
        ));
    }
    let (sr0, sc0, sr1, sc1) = norm(src);
    let (dr0, dc0, dr1, dc1) = norm(dest);
    let seed = clip_one(wb, sheet, sr0, sc0).input;
    let start = list
        .iter()
        .position(|item| item.eq_ignore_ascii_case(&seed))
        .ok_or_else(|| {
            CoreError::new(
                "edit.fill.list",
                format!("source value {seed:?} is not in the custom list"),
            )
        })? as i64;
    let list_len = i64::try_from(list.len())
        .map_err(|_| CoreError::new("edit.fill.list", "custom list is too long"))?;
    let vertical = sc0 == sc1 && dc0 == sc0 && dc1 == sc1;
    let horizontal = sr0 == sr1 && dr0 == sr0 && dr1 == sr1;
    if !vertical && !horizontal {
        return Err(CoreError::new(
            "edit.fill.list",
            "custom-list fill must be one-dimensional",
        ));
    }
    let mut changed = 0u32;
    if vertical {
        for row in dr0..=dr1 {
            if row >= sr0 && row <= sr1 {
                continue;
            }
            let offset = i64::from(row) - i64::from(sr0);
            let index = (start + offset).rem_euclid(list_len) as usize;
            wb.set_text(sheet, row, sc0, &list[index])?;
            changed += 1;
        }
    } else {
        for col in dc0..=dc1 {
            if col >= sc0 && col <= sc1 {
                continue;
            }
            let offset = i64::from(col) - i64::from(sc0);
            let index = (start + offset).rem_euclid(list_len) as usize;
            wb.set_text(sheet, sr0, col, &list[index])?;
            changed += 1;
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
    wb.ensure_range_not_array_formula_output(sheet, row, col, row, col)?;
    if let Some(fid) = slot.formula {
        let src = wb.intern().formulas.get(fid).unwrap_or("").to_string();
        if let Ok(rewritten) = rewrite_print(&src, &RewriteOp::Copy { dcol, drow }) {
            wb.set_cell_contents(sheet, row, col, &rewritten)?;
            let mut copied = wb
                .get(sheet, row, col)?
                .copied()
                .unwrap_or_else(CellSlot::empty);
            copied.style = slot.style;
            copied.flags = copied
                .flags
                .with(CellFlags::LOCKED, slot.flags.locked())
                .with(CellFlags::HIDDEN, slot.flags.hidden());
            replace_cell_slot(wb, sheet, row, col, Some(copied))?;
            return Ok(());
        }
    }
    let mut copied = slot;
    copied.flags = CellFlags::DEFAULT
        .with(CellFlags::LOCKED, slot.flags.locked())
        .with(CellFlags::HIDDEN, slot.flags.hidden());
    replace_cell_slot(wb, sheet, row, col, Some(copied))?;
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
    /// Portable rich-text runs for text values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rich_text: Vec<RichTextRun>,
    /// Complete cell style for ordinary and formats-only paste.
    pub style: Style,
    /// Packed protection/recalc flags.
    pub flags: CellFlags,
    /// Source number-format code, used to remap custom ids safely.
    pub number_format: Option<String>,
    /// Source column width in pixels.
    pub column_width_px: u32,
}

/// Non-cell records carried by the internal clipboard MIME payload.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipExtras {
    /// Source height.
    pub rows: u32,
    /// Source width.
    pub cols: u16,
    /// Legacy notes at relative coordinates.
    pub notes: Vec<(u32, u16, Note)>,
    /// Threaded comments at relative coordinates.
    pub comments: Vec<(u32, u16, Comment)>,
    /// Hyperlinks at relative coordinates.
    pub hyperlinks: Vec<(u32, u16, Hyperlink)>,
    /// Merged rectangles `(r0, c0, r1, c1)` relative to the copied range.
    pub merges: Vec<(u32, u16, u32, u16)>,
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

/// Copy notes, threaded comments, hyperlinks, and contained merges for a range.
pub fn copy_extras(wb: &Workbook, sheet: SheetId, range: RangeRef) -> ClipExtras {
    let (r0, c0, r1, c1) = norm(range);
    let Some(sheet) = wb.sheet(sheet) else {
        return ClipExtras::default();
    };
    let mut notes: Vec<_> = sheet
        .notes
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(&(row, col), value)| (row - r0, col - c0, value.clone()))
        .collect();
    let mut comments: Vec<_> = sheet
        .comments
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(&(row, col), value)| (row - r0, col - c0, value.clone()))
        .collect();
    let mut hyperlinks: Vec<_> = sheet
        .hyperlinks
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(&(row, col), value)| (row - r0, col - c0, value.clone()))
        .collect();
    notes.sort_by_key(|(row, col, _)| (*row, *col));
    comments.sort_by_key(|(row, col, _)| (*row, *col));
    hyperlinks.sort_by_key(|(row, col, _)| (*row, *col));
    let mut merges: Vec<_> = sheet
        .merges
        .iter()
        .filter_map(|merge| {
            let (mr0, mc0, mr1, mc1) = norm(*merge);
            (mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1).then_some((
                mr0 - r0,
                mc0 - c0,
                mr1 - r0,
                mc1 - c0,
            ))
        })
        .collect();
    merges.sort_unstable();
    ClipExtras {
        rows: r1 - r0 + 1,
        cols: c1 - c0 + 1,
        notes,
        comments,
        hyperlinks,
        merges,
    }
}

/// Paste non-cell clipboard records at `dest`.
pub fn paste_extras(
    wb: &mut Workbook,
    sheet: SheetId,
    dest: CellRef,
    extras: &ClipExtras,
    transpose: bool,
) -> Result<(), CoreError> {
    if extras.rows == 0 || extras.cols == 0 {
        return Ok(());
    }
    let (height, width) = if transpose {
        (u32::from(extras.cols), extras.rows)
    } else {
        (extras.rows, u32::from(extras.cols))
    };
    if height > MAX_ROWS - dest.row || width > u32::from(MAX_COLS - dest.col) {
        return Err(CoreError::addr_ref(
            "clipboard metadata exceeds the worksheet grid",
        ));
    }
    let target_r1 = dest.row + height - 1;
    let target_c1 = dest.col
        + u16::try_from(width - 1)
            .map_err(|_| CoreError::addr_ref("clipboard metadata is too wide"))?;
    wb.mutate_sheet_edit(sheet, |target| {
        target.notes.retain(|&(row, col), _| {
            row < dest.row || row > target_r1 || col < dest.col || col > target_c1
        });
        target.comments.retain(|&(row, col), _| {
            row < dest.row || row > target_r1 || col < dest.col || col > target_c1
        });
        target.hyperlinks.retain(|&(row, col), _| {
            row < dest.row || row > target_r1 || col < dest.col || col > target_c1
        });
        let map_coord = |row: u32, col: u16| {
            if transpose {
                (dest.row + u32::from(col), dest.col + row as u16)
            } else {
                (dest.row + row, dest.col + col)
            }
        };
        for (row, col, note) in &extras.notes {
            target.notes.insert(map_coord(*row, *col), note.clone());
        }
        for (row, col, comment) in &extras.comments {
            target
                .comments
                .insert(map_coord(*row, *col), comment.clone());
        }
        for (row, col, link) in &extras.hyperlinks {
            target
                .hyperlinks
                .insert(map_coord(*row, *col), link.clone());
        }
        let target_range = RangeRef::from_corners(dest, CellRef::new(target_r1, target_c1)?);
        target
            .merges
            .retain(|merge| !overlaps(*merge, target_range));
        for &(mr0, mc0, mr1, mc1) in &extras.merges {
            let (start_row, start_col) = map_coord(mr0, mc0);
            let (end_row, end_col) = map_coord(mr1, mc1);
            let merge = if transpose {
                RangeRef::from_corners(
                    CellRef::new(start_row.min(end_row), start_col.min(end_col))?,
                    CellRef::new(start_row.max(end_row), start_col.max(end_col))?,
                )
            } else {
                RangeRef::from_corners(
                    CellRef::new(start_row, start_col)?,
                    CellRef::new(end_row, end_col)?,
                )
            };
            target.add_merge(merge)?;
        }
        Ok(())
    })
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
            rich_text: Vec::new(),
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
    let rich_text = match slot.value {
        Value::Text(id) => wb
            .intern()
            .strings
            .get_rich(id)
            .map(|runs| runs.to_vec())
            .unwrap_or_default(),
        _ => Vec::new(),
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
        rich_text,
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
    paste_special_from(wb, sheet, dest, grid, spec, src_origin, Some(sheet))
}

/// Paste with an explicit source sheet for cross-sheet paste-link formulas.
pub fn paste_special_from(
    wb: &mut Workbook,
    sheet: SheetId,
    dest: CellRef,
    grid: &[Vec<ClipCell>],
    spec: PasteSpecial,
    src_origin: Option<(u32, u16)>,
    src_sheet: Option<SheetId>,
) -> Result<u32, CoreError> {
    validate_paste_bounds(dest, grid, spec.transpose)?;
    let ordinary = !spec.values
        && !spec.formulas
        && !spec.formats
        && !spec.number_formats
        && !spec.column_widths
        && spec.operation == PasteOp::None
        && !spec.paste_link;
    if (ordinary || spec.formulas) && grid.iter().flatten().any(|cell| cell.flags.array()) {
        return Err(CoreError::new(
            "formula.array",
            "copying a legacy array formula as a formula is not supported",
        )
        .with_hint("use paste values to copy the cached results"));
    }
    let changes_contents = ordinary
        || spec.values
        || spec.formulas
        || spec.operation != PasteOp::None
        || spec.paste_link;
    if changes_contents {
        let rows =
            u32::try_from(grid.len()).map_err(|_| CoreError::addr_ref("paste is too tall"))?;
        let cols = grid.iter().map(Vec::len).max().unwrap_or(0);
        let cols = u32::try_from(cols).map_err(|_| CoreError::addr_ref("paste is too wide"))?;
        let (height, width) = if spec.transpose {
            (cols, rows)
        } else {
            (rows, cols)
        };
        if height > 0 && width > 0 {
            let width =
                u16::try_from(width).map_err(|_| CoreError::addr_ref("paste is too wide"))?;
            wb.ensure_range_not_array_formula_output(
                sheet,
                dest.row,
                dest.col,
                dest.row + height - 1,
                dest.col + width - 1,
            )?;
        }
    }
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
                    let qualifier = src_sheet
                        .filter(|source| *source != sheet)
                        .and_then(|source| wb.sheet(source))
                        .map(|source| format!("{}!", quote_sheet_name(&source.name)))
                        .unwrap_or_default();
                    let input = format!("={qualifier}${letters}${}", source_row + 1);
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
                    set_clip_cell_value(wb, sheet, row, col, cell)?;
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
                    set_clip_cell_value(wb, sheet, row, col, cell)?;
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
    wb.ensure_range_not_array_formula_output(sheet, row, col, row, col)?;
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

fn set_clip_cell_value(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    cell: &ClipCell,
) -> Result<(), CoreError> {
    if let ClipValue::Text(text) = &cell.value
        && !cell.rich_text.is_empty()
    {
        wb.set_rich_text(sheet, row, col, text, cell.rich_text.clone())?;
        return Ok(());
    }
    set_clip_value(wb, sheet, row, col, &cell.value)
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
        slot.flags = slot
            .flags
            .with(CellFlags::LOCKED, cell.flags.locked())
            .with(CellFlags::HIDDEN, cell.flags.hidden());
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
    wb.ensure_range_not_array_formula_output(sheet, r0, c0, r1, c1)?;
    wb.ensure_range_not_array_formula_output(
        sheet,
        dest.row,
        dest.col,
        dest.row + height - 1,
        dest.col + width - 1,
    )?;
    let grid: Vec<Vec<Option<CellSlot>>> = (r0..=r1)
        .map(|row| {
            (c0..=c1)
                .map(|col| wb.get(sheet, row, col).map(|slot| slot.copied()))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let held_slots: Vec<CellSlot> = grid.iter().flatten().filter_map(|slot| *slot).collect();
    wb.with_held_slots(&held_slots, |wb| {
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
        rewrite_ai_redact_marks_move(wb, sheet, src, dest);
        Ok(changed)
    })
}

/// Move a range between sheets with cut semantics and workbook-wide retargeting.
pub fn move_range_cells_between(
    wb: &mut Workbook,
    source_sheet: SheetId,
    src: RangeRef,
    dest_sheet: SheetId,
    dest: CellRef,
) -> Result<u32, CoreError> {
    if source_sheet == dest_sheet {
        return move_range_cells(wb, source_sheet, src, dest);
    }
    let (r0, c0, r1, c1) = norm(src);
    let height = r1 - r0 + 1;
    let width = c1 - c0 + 1;
    if height > MAX_ROWS - dest.row || u32::from(width) > u32::from(MAX_COLS - dest.col) {
        return Err(CoreError::addr_ref("move exceeds the worksheet grid"));
    }
    wb.ensure_range_not_array_formula_output(source_sheet, r0, c0, r1, c1)?;
    wb.ensure_range_not_array_formula_output(
        dest_sheet,
        dest.row,
        dest.col,
        dest.row + height - 1,
        dest.col + width - 1,
    )?;
    validate_cross_sheet_merges(wb, source_sheet, src, dest_sheet, dest, height, width)?;
    let grid: Vec<Vec<Option<CellSlot>>> = (r0..=r1)
        .map(|row| {
            (c0..=c1)
                .map(|col| wb.get(source_sheet, row, col).map(|slot| slot.copied()))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let held_slots: Vec<CellSlot> = grid.iter().flatten().filter_map(|slot| *slot).collect();
    wb.with_held_slots(&held_slots, |wb| {
        for (dr, cells) in grid.iter().enumerate() {
            for (dc, slot) in cells.iter().enumerate() {
                replace_cell_slot(
                    wb,
                    dest_sheet,
                    dest.row + dr as u32,
                    dest.col + dc as u16,
                    *slot,
                )?;
            }
        }
        for row in r0..=r1 {
            for col in c0..=c1 {
                wb.clear_cell(source_sheet, row, col)?;
            }
        }
        move_side_tables_between(wb, source_sheet, src, dest_sheet, dest, height, width)?;
        rewrite_formulas_move_between(wb, source_sheet, src, dest_sheet, dest, height, width)?;
        rewrite_ai_redact_marks_move_between(wb, source_sheet, src, dest_sheet, dest);
        Ok(u32::from(width).saturating_mul(height))
    })
}

fn validate_cross_sheet_merges(
    wb: &Workbook,
    source_sheet: SheetId,
    src: RangeRef,
    dest_sheet: SheetId,
    dest: CellRef,
    height: u32,
    width: u16,
) -> Result<(), CoreError> {
    let source = wb
        .sheet(source_sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown source sheet"))?;
    let (r0, c0, r1, c1) = norm(src);
    for merge in &source.merges {
        let (mr0, mc0, mr1, mc1) = norm(*merge);
        let fully_inside = mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1;
        if overlaps(*merge, src) && !fully_inside {
            return Err(CoreError::new(
                "edit.move.merge",
                "move range partially overlaps a merged area",
            ));
        }
    }
    let target = RangeRef::from_corners(
        dest,
        CellRef::new(dest.row + height - 1, dest.col + width - 1)?,
    );
    let destination = wb
        .sheet(dest_sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown destination sheet"))?;
    if destination
        .merges
        .iter()
        .any(|merge| overlaps(*merge, target))
    {
        return Err(CoreError::new(
            "edit.move.merge",
            "move destination overlaps a merged area",
        ));
    }
    Ok(())
}

fn move_side_tables_between(
    wb: &mut Workbook,
    source_sheet: SheetId,
    src: RangeRef,
    dest_sheet: SheetId,
    dest: CellRef,
    height: u32,
    width: u16,
) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(src);
    let drow = i64::from(dest.row) - i64::from(r0);
    let dcol = i64::from(dest.col) - i64::from(c0);
    let target = |row: u32, col: u16| {
        (
            (i64::from(row) + drow) as u32,
            (i64::from(col) + dcol) as u16,
        )
    };
    let source = wb
        .sheet(source_sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown source sheet"))?;
    let notes: Vec<_> = source
        .notes
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(&(row, col), value)| (target(row, col), value.clone()))
        .collect();
    let comments: Vec<_> = source
        .comments
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(&(row, col), value)| (target(row, col), value.clone()))
        .collect();
    let hyperlinks: Vec<_> = source
        .hyperlinks
        .iter()
        .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
        .map(|(&(row, col), value)| (target(row, col), value.clone()))
        .collect();
    let merges: Vec<_> = source
        .merges
        .iter()
        .filter(|merge| {
            let (mr0, mc0, mr1, mc1) = norm(**merge);
            mr0 >= r0 && mr1 <= r1 && mc0 >= c0 && mc1 <= c1
        })
        .map(|merge| {
            let (start_row, start_col) = target(merge.start.row, merge.start.col);
            let (end_row, end_col) = target(merge.end.row, merge.end.col);
            RangeRef::from_corners(
                CellRef::new(start_row, start_col).unwrap_or(dest),
                CellRef::new(end_row, end_col).unwrap_or(dest),
            )
        })
        .collect();
    wb.mutate_sheet_edit(source_sheet, |sheet| {
        sheet
            .notes
            .retain(|&(row, col), _| row < r0 || row > r1 || col < c0 || col > c1);
        sheet
            .comments
            .retain(|&(row, col), _| row < r0 || row > r1 || col < c0 || col > c1);
        sheet
            .hyperlinks
            .retain(|&(row, col), _| row < r0 || row > r1 || col < c0 || col > c1);
        sheet.merges.retain(|merge| !overlaps(*merge, src));
        Ok(())
    })?;
    let target_r1 = dest.row + height - 1;
    let target_c1 = dest.col + width - 1;
    wb.mutate_sheet_edit(dest_sheet, |sheet| {
        sheet.notes.retain(|&(row, col), _| {
            row < dest.row || row > target_r1 || col < dest.col || col > target_c1
        });
        sheet.comments.retain(|&(row, col), _| {
            row < dest.row || row > target_r1 || col < dest.col || col > target_c1
        });
        sheet.hyperlinks.retain(|&(row, col), _| {
            row < dest.row || row > target_r1 || col < dest.col || col > target_c1
        });
        sheet.notes.extend(notes);
        sheet.comments.extend(comments);
        sheet.hyperlinks.extend(hyperlinks);
        sheet.merges.extend(merges);
        Ok(())
    })
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
    wb.mutate_sheet_edit(sheet, |sheet_ref| {
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
    })
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

fn rewrite_formulas_move_between(
    wb: &mut Workbook,
    source_sheet: SheetId,
    src: RangeRef,
    dest_sheet: SheetId,
    dest: CellRef,
    height: u32,
    width: u16,
) -> Result<(), CoreError> {
    let source_name = wb
        .sheet(source_sheet)
        .map(|sheet| sheet.name.clone())
        .ok_or_else(|| CoreError::sheet_id("unknown source sheet"))?;
    let dest_name = wb
        .sheet(dest_sheet)
        .map(|sheet| sheet.name.clone())
        .ok_or_else(|| CoreError::sheet_id("unknown destination sheet"))?;
    let destination = RangeRef::from_corners(
        dest,
        CellRef::new(dest.row + height - 1, dest.col + width - 1)?,
    );
    let sheet_ids: Vec<_> = wb.sheets().map(|sheet| sheet.id).collect();
    let mut updates = Vec::new();
    for id in sheet_ids {
        let current_home = wb
            .sheet(id)
            .map(|sheet| sheet.name.clone())
            .unwrap_or_default();
        let cells: Vec<_> = wb
            .sheet(id)
            .map(|sheet| sheet.store.iter().collect())
            .unwrap_or_default();
        for (row, col, slot) in cells {
            let Some(formula_id) = slot.formula else {
                continue;
            };
            let Some(source) = wb.intern().formulas.get(formula_id).map(str::to_string) else {
                continue;
            };
            let Ok(parsed) = parse(&source) else {
                continue;
            };
            let moved_formula = id == dest_sheet
                && row >= dest.row
                && row < dest.row + height
                && col >= dest.col
                && col < dest.col + width;
            let logical_home = if moved_formula {
                source_name.as_str()
            } else {
                current_home.as_str()
            };
            let ast = map_sheet_move_between(
                &parsed.ast,
                &current_home,
                logical_home,
                &source_name,
                &dest_name,
                src,
                dest,
                destination,
            );
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

#[allow(clippy::too_many_arguments)]
fn map_sheet_move_between(
    expr: &Expr,
    current_home: &str,
    logical_home: &str,
    source_name: &str,
    dest_name: &str,
    src: RangeRef,
    dest: CellRef,
    destination: RangeRef,
) -> Expr {
    expr.clone().map_local(&mut |item| {
        let kind = match item.kind {
            ExprKind::Cell { sheet, cell } => {
                let resolved = sheet
                    .as_ref()
                    .map(|spec| spec.start.as_str())
                    .unwrap_or(logical_home);
                if resolved.eq_ignore_ascii_case(source_name) {
                    if range_contains_cell(src, cell) {
                        let moved = move_range(
                            &Expr {
                                kind: ExprKind::Cell { sheet: None, cell },
                                span: item.span,
                            },
                            src,
                            dest,
                        );
                        match moved.kind {
                            ExprKind::Cell {
                                cell: moved_cell, ..
                            } => ExprKind::Cell {
                                sheet: qualifier_for(current_home, dest_name),
                                cell: moved_cell,
                            },
                            other => other,
                        }
                    } else if sheet.is_none() && !logical_home.eq_ignore_ascii_case(current_home) {
                        ExprKind::Cell {
                            sheet: qualifier_for(current_home, source_name),
                            cell,
                        }
                    } else {
                        ExprKind::Cell { sheet, cell }
                    }
                } else if resolved.eq_ignore_ascii_case(dest_name)
                    && range_contains_cell(destination, cell)
                {
                    ExprKind::Error(ErrorKind::Ref)
                } else {
                    ExprKind::Cell { sheet, cell }
                }
            }
            ExprKind::Range { sheet, range } => {
                let resolved = sheet
                    .as_ref()
                    .map(|spec| spec.start.as_str())
                    .unwrap_or(logical_home);
                if resolved.eq_ignore_ascii_case(source_name) {
                    if range_contains_range(src, range) {
                        let moved = move_range(
                            &Expr {
                                kind: ExprKind::Range { sheet: None, range },
                                span: item.span,
                            },
                            src,
                            dest,
                        );
                        match moved.kind {
                            ExprKind::Range {
                                range: moved_range, ..
                            } => ExprKind::Range {
                                sheet: qualifier_for(current_home, dest_name),
                                range: moved_range,
                            },
                            other => other,
                        }
                    } else if sheet.is_none() && !logical_home.eq_ignore_ascii_case(current_home) {
                        ExprKind::Range {
                            sheet: qualifier_for(current_home, source_name),
                            range,
                        }
                    } else {
                        ExprKind::Range { sheet, range }
                    }
                } else if resolved.eq_ignore_ascii_case(dest_name)
                    && range_contains_range(destination, range)
                {
                    ExprKind::Error(ErrorKind::Ref)
                } else {
                    ExprKind::Range { sheet, range }
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

fn range_contains_cell(range: RangeRef, cell: CellRef) -> bool {
    let (r0, c0, r1, c1) = norm(range);
    cell.row >= r0 && cell.row <= r1 && cell.col >= c0 && cell.col <= c1
}

fn range_contains_range(outer: RangeRef, inner: RangeRef) -> bool {
    let (or0, oc0, or1, oc1) = norm(outer);
    let (ir0, ic0, ir1, ic1) = norm(inner);
    ir0 >= or0 && ir1 <= or1 && ic0 >= oc0 && ic1 <= oc1
}

fn qualifier_for(home: &str, target: &str) -> Option<SheetSpec> {
    (!home.eq_ignore_ascii_case(target)).then(|| SheetSpec {
        start: target.to_string(),
        end: None,
    })
}

fn map_sheet_move(expr: &Expr, home: &str, target: &str, src: RangeRef, dest: CellRef) -> Expr {
    let applies = |sheet: &Option<SheetSpec>| match sheet {
        None => home.eq_ignore_ascii_case(target),
        Some(spec) => spec.start.eq_ignore_ascii_case(target),
    };
    expr.clone().map_local(&mut |item| {
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

/// Text-to-columns split mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextToColumnsMode {
    /// Split on any listed delimiter.
    Delimited {
        /// Delimiter characters.
        delimiters: Vec<char>,
    },
    /// Split at Unicode character offsets.
    Fixed {
        /// Strictly increasing offsets from the start of the source text.
        breaks: Vec<usize>,
    },
}

/// Conversion rule for one output field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextColumnType {
    /// Conservatively recognize numbers and booleans, else keep exact text.
    #[default]
    General,
    /// Preserve exact text, including leading zeroes and whitespace.
    Text,
    /// Do not write this field.
    Skip,
}

/// Complete text-to-columns plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextToColumnsPlan {
    /// Split mode.
    pub mode: TextToColumnsMode,
    /// Per-field conversion rules; missing entries use [`TextColumnType::General`].
    #[serde(default)]
    pub columns: Vec<TextColumnType>,
}

/// Split `range` into adjacent columns using a typed plan.
pub fn text_to_columns_with_plan(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    plan: &TextToColumnsPlan,
) -> Result<u32, CoreError> {
    validate_text_plan(plan)?;
    let (r0, c0, r1, _) = norm(range);
    let mut changed = 0u32;
    for row in r0..=r1 {
        let text = cell_plain_text(wb, sheet, row, c0);
        let parts = split_text(&text, &plan.mode);
        if parts.len() > usize::from(MAX_COLS - c0) {
            return Err(CoreError::addr_ref(
                "text-to-columns output exceeds the worksheet grid",
            ));
        }
        for (index, part) in parts.into_iter().enumerate() {
            let offset = u16::try_from(index)
                .map_err(|_| CoreError::addr_ref("text-to-columns output is too wide"))?;
            let col = c0 + offset;
            let kind = plan.columns.get(index).copied().unwrap_or_default();
            match kind {
                TextColumnType::Skip => continue,
                TextColumnType::Text => {
                    wb.set_text(sheet, row, col, &part)?;
                }
                TextColumnType::General => set_general_text(wb, sheet, row, col, &part)?,
            }
            changed += 1;
        }
    }
    Ok(changed)
}

fn validate_text_plan(plan: &TextToColumnsPlan) -> Result<(), CoreError> {
    match &plan.mode {
        TextToColumnsMode::Delimited { delimiters } if delimiters.is_empty() => Err(
            CoreError::new("edit.texttocolumns", "at least one delimiter is required"),
        ),
        TextToColumnsMode::Fixed { breaks }
            if breaks.first() == Some(&0) || breaks.windows(2).any(|pair| pair[0] >= pair[1]) =>
        {
            Err(CoreError::new(
                "edit.texttocolumns",
                "fixed-width breaks must be positive and strictly increasing",
            ))
        }
        _ => Ok(()),
    }
}

fn cell_plain_text(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    match wb
        .get(sheet, row, col)
        .ok()
        .flatten()
        .map(|slot| slot.value)
    {
        Some(Value::Text(id)) => wb.intern().strings.get(id).unwrap_or_default().to_string(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(value)) => if value { "TRUE" } else { "FALSE" }.into(),
        Some(Value::Error(error)) => error.as_str().to_string(),
        _ => String::new(),
    }
}

fn split_text(text: &str, mode: &TextToColumnsMode) -> Vec<String> {
    match mode {
        TextToColumnsMode::Delimited { delimiters } => text
            .split(|character| delimiters.contains(&character))
            .map(str::to_string)
            .collect(),
        TextToColumnsMode::Fixed { breaks } => {
            let chars: Vec<char> = text.chars().collect();
            let mut start = 0usize;
            let mut parts = Vec::new();
            for &end in breaks {
                let end = end.min(chars.len());
                parts.push(chars[start.min(end)..end].iter().collect());
                start = end;
            }
            parts.push(chars[start..].iter().collect());
            parts
        }
    }
}

fn set_general_text(
    wb: &mut Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    text: &str,
) -> Result<(), CoreError> {
    let trimmed = text.trim();
    if let Ok(number) = trimmed.parse::<f64>()
        && number.is_finite()
    {
        wb.set_number(sheet, row, col, number)?;
    } else if trimmed.eq_ignore_ascii_case("true") {
        wb.set_cell_contents(sheet, row, col, "TRUE")?;
    } else if trimmed.eq_ignore_ascii_case("false") {
        wb.set_cell_contents(sheet, row, col, "FALSE")?;
    } else {
        wb.set_text(sheet, row, col, text)?;
    }
    Ok(())
}

/// Split `range` by one delimiter using general conversion.
pub fn text_to_columns(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    delim: char,
) -> Result<u32, CoreError> {
    text_to_columns_with_plan(
        wb,
        sheet,
        range,
        &TextToColumnsPlan {
            mode: TextToColumnsMode::Delimited {
                delimiters: vec![delim],
            },
            columns: Vec::new(),
        },
    )
}

/// Remove duplicate rows in `range` comparing the listed relative columns.
pub fn remove_duplicates(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    columns: &[u16],
) -> Result<u32, CoreError> {
    remove_duplicates_with_header(wb, sheet, range, columns, false)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum DuplicateKey {
    Empty,
    Number(u64),
    Bool(bool),
    Text(String),
    Error(String),
}

fn duplicate_key(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
) -> Result<DuplicateKey, CoreError> {
    let Some(slot) = wb.get(sheet, row, col)? else {
        return Ok(DuplicateKey::Empty);
    };
    Ok(match slot.value {
        Value::Empty | Value::Array(_) => DuplicateKey::Empty,
        Value::Number(value) => {
            // Excel treats both signed representations of zero as the same value.
            DuplicateKey::Number(if value == 0.0 { 0 } else { value.to_bits() })
        }
        Value::Bool(value) => DuplicateKey::Bool(value),
        Value::Text(id) => DuplicateKey::Text(
            wb.intern()
                .strings
                .get(id)
                .unwrap_or_default()
                .to_lowercase(),
        ),
        Value::Error(value) => DuplicateKey::Error(value.as_str().to_string()),
    })
}

/// Remove duplicate rows, optionally preserving the first row as a header.
pub fn remove_duplicates_with_header(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    columns: &[u16],
    has_headers: bool,
) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    wb.ensure_range_not_array_formula_output(sheet, r0, c0, r1, c1)?;
    if wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
        .merges
        .iter()
        .any(|merge| overlaps(*merge, range))
    {
        return Err(CoreError::new(
            "edit.removeduplicates.merge",
            "remove duplicates does not accept merged cells in the selected range",
        ));
    }
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
        if has_headers && r == r0 {
            let row = (c0..=c1)
                .map(|col| wb.get(sheet, r, col).map(|slot| slot.copied()))
                .collect::<Result<Vec<_>, _>>()?;
            kept.push((r, row));
            continue;
        }
        let key: Vec<DuplicateKey> = cols
            .iter()
            .map(|&col| duplicate_key(wb, sheet, r, col))
            .collect::<Result<_, _>>()?;
        if seen.insert(key) {
            let row = (c0..=c1)
                .map(|col| wb.get(sheet, r, col).map(|slot| slot.copied()))
                .collect::<Result<Vec<_>, _>>()?;
            kept.push((r, row));
        }
    }
    let total = usize::try_from(r1 - r0 + 1).unwrap_or(usize::MAX);
    let removed = u32::try_from(total.saturating_sub(kept.len())).unwrap_or(u32::MAX);
    let row_map: std::collections::BTreeMap<u32, u32> = kept
        .iter()
        .enumerate()
        .map(|(offset, (source_row, _))| (*source_row, r0 + offset as u32))
        .collect();
    for (offset, (_, row)) in kept.iter().enumerate() {
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
    wb.mutate_sheet_edit(sheet, |sheet| {
        let notes: Vec<_> = sheet
            .notes
            .iter()
            .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
            .map(|(coord, value)| (*coord, value.clone()))
            .collect();
        let comments: Vec<_> = sheet
            .comments
            .iter()
            .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
            .map(|(coord, value)| (*coord, value.clone()))
            .collect();
        let hyperlinks: Vec<_> = sheet
            .hyperlinks
            .iter()
            .filter(|((row, col), _)| *row >= r0 && *row <= r1 && *col >= c0 && *col <= c1)
            .map(|(coord, value)| (*coord, value.clone()))
            .collect();
        sheet
            .notes
            .retain(|(row, col), _| !(*row >= r0 && *row <= r1 && *col >= c0 && *col <= c1));
        sheet
            .comments
            .retain(|(row, col), _| !(*row >= r0 && *row <= r1 && *col >= c0 && *col <= c1));
        sheet
            .hyperlinks
            .retain(|(row, col), _| !(*row >= r0 && *row <= r1 && *col >= c0 && *col <= c1));
        for ((row, col), value) in notes {
            if let Some(target_row) = row_map.get(&row) {
                sheet.notes.insert((*target_row, col), value);
            }
        }
        for ((row, col), value) in comments {
            if let Some(target_row) = row_map.get(&row) {
                sheet.comments.insert((*target_row, col), value);
            }
        }
        for ((row, col), value) in hyperlinks {
            if let Some(target_row) = row_map.get(&row) {
                sheet.hyperlinks.insert((*target_row, col), value);
            }
        }
        Ok(())
    })?;
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
    if h > MAX_ROWS - dest.row || u32::from(w) > u32::from(MAX_COLS - dest.col) {
        return Err(CoreError::addr_ref(
            "consolidated output exceeds the worksheet grid",
        ));
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

/// Auto-fit selected columns using a frontend-supplied text measurement callback.
pub fn autofit_columns(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    mut measure: impl FnMut(&str, &Style) -> u32,
) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    let cells: Vec<_> = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
        .store
        .iter_region(r0, c0, r1, c1)
        .collect();
    let mut widths = std::collections::BTreeMap::new();
    for (_, col, slot) in cells {
        let text = display_text(wb, slot);
        let style = wb
            .intern()
            .styles
            .get(slot.style)
            .cloned()
            .unwrap_or_default();
        let width = measure(&text, &style);
        widths
            .entry(col)
            .and_modify(|current: &mut u32| *current = (*current).max(width))
            .or_insert(width);
    }
    let mut changed = 0u32;
    for col in c0..=c1 {
        let width = widths.get(&col).copied().unwrap_or(24).max(24);
        wb.set_col_width(sheet, col, width)?;
        changed += 1;
    }
    Ok(changed)
}

/// Auto-fit selected rows using a frontend-supplied text measurement callback.
pub fn autofit_rows(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
    mut measure: impl FnMut(&str, &Style) -> u32,
) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    let cells: Vec<_> = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id("unknown sheet"))?
        .store
        .iter_region(r0, c0, r1, c1)
        .collect();
    let mut heights = std::collections::BTreeMap::new();
    for (row, _, slot) in cells {
        let text = display_text(wb, slot);
        let style = wb
            .intern()
            .styles
            .get(slot.style)
            .cloned()
            .unwrap_or_default();
        let height = measure(&text, &style);
        heights
            .entry(row)
            .and_modify(|current: &mut u32| *current = (*current).max(height))
            .or_insert(height);
    }
    let mut changed = 0u32;
    for row in r0..=r1 {
        let height = heights
            .get(&row)
            .copied()
            .unwrap_or(crate::geometry::DEFAULT_ROW_PX)
            .max(crate::geometry::DEFAULT_ROW_PX);
        wb.set_row_height(sheet, row, height)?;
        changed += 1;
    }
    Ok(changed)
}

fn display_text(wb: &Workbook, slot: CellSlot) -> String {
    match slot.value {
        Value::Empty => String::new(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => if value { "TRUE" } else { "FALSE" }.into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or_default().to_string(),
        Value::Error(error) => error.as_str().to_string(),
        Value::Array(_) => String::new(),
    }
}

/// Default column auto-fit width in pixels from display text.
#[must_use]
pub fn autofit_width(text: &str) -> u32 {
    (text.chars().count() as u32)
        .saturating_mul(8)
        .saturating_add(12)
        .max(24)
}

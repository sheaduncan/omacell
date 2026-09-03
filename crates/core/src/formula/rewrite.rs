//! Reference rewriting: copy/fill, cut/move, insert/delete, rename.

use crate::addr::{CellRef, RangeRef, SheetSpec, col_from_letters};
use crate::error::ErrorKind;
use crate::limits::{MAX_COLS, MAX_ROWS};

use super::Formula;
use super::ast::{Callee, Expr, ExprKind};
use super::error::ParseError;
use super::parser::parse;
use super::printer::print;

/// Rewrite ops used by the rewrite corpus and dependents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RewriteOp {
    /// Copy/fill: relative axes move by `(dcol, drow)`; absolute stay.
    Copy {
        /// Column delta (positive = right).
        dcol: i32,
        /// Row delta (positive = down).
        drow: i32,
    },
    /// Cut-paste: every ref fully inside `src` retargets to `dest` (abs too).
    Move {
        /// Cut range (A1 text, e.g. `A1:B2`).
        src: String,
        /// Destination origin (A1 cell).
        dest: String,
    },
    /// Insert `count` rows before 1-based `at`.
    InsertRows {
        /// 1-based insert row.
        at: u32,
        /// Rows to insert.
        count: u32,
    },
    /// Delete `count` rows starting at 1-based `at`.
    DeleteRows {
        /// 1-based first deleted row.
        at: u32,
        /// Rows to delete.
        count: u32,
    },
    /// Insert `count` columns before column `at` (letters).
    InsertCols {
        /// Column letters of the insert point.
        at: String,
        /// Columns to insert.
        count: u16,
    },
    /// Delete `count` columns starting at `at`.
    DeleteCols {
        /// Column letters of the first deleted column.
        at: String,
        /// Columns to delete.
        count: u16,
    },
    /// Rename a sheet qualifier.
    SheetRename {
        /// Old sheet name.
        old: String,
        /// New sheet name.
        new: String,
    },
    /// Rename a table in structured refs.
    TableRename {
        /// Old table name.
        old: String,
        /// New table name.
        new: String,
    },
}

/// Parse `src`, apply `op`, return canonical print.
pub fn rewrite_print(src: &str, op: &RewriteOp) -> Result<String, ParseError> {
    let f = parse(src)?;
    let ast = apply(&f.ast, op)?;
    Ok(print(&Formula {
        ast,
        style: f.style,
        base_row: f.base_row,
        base_col: f.base_col,
    }))
}

/// Apply a rewrite to an AST.
pub fn apply(expr: &Expr, op: &RewriteOp) -> Result<Expr, ParseError> {
    match op {
        RewriteOp::Copy { dcol, drow } => Ok(copy_delta(expr, *drow, *dcol)),
        RewriteOp::Move { src, dest } => {
            let src_r = parse_range_a1(src)?;
            let dest_c = parse_cell_a1(dest)?;
            Ok(move_range(expr, src_r, dest_c))
        }
        RewriteOp::InsertRows { at, count } => {
            let at0 = at
                .checked_sub(1)
                .ok_or_else(|| ParseError::parse("insert row must be 1-based", 0, Vec::new()))?;
            validate_row_rewrite(at0, *count)?;
            Ok(adjust_rows(expr, at0, *count, false))
        }
        RewriteOp::DeleteRows { at, count } => {
            let at0 = at
                .checked_sub(1)
                .ok_or_else(|| ParseError::parse("delete row must be 1-based", 0, Vec::new()))?;
            validate_row_rewrite(at0, *count)?;
            Ok(adjust_rows(expr, at0, *count, true))
        }
        RewriteOp::InsertCols { at, count } => {
            let col =
                col_from_letters(at).map_err(|e| ParseError::parse(e.message, 0, Vec::new()))?;
            validate_col_rewrite(col, *count)?;
            Ok(adjust_cols(expr, col, *count, false))
        }
        RewriteOp::DeleteCols { at, count } => {
            let col =
                col_from_letters(at).map_err(|e| ParseError::parse(e.message, 0, Vec::new()))?;
            validate_col_rewrite(col, *count)?;
            Ok(adjust_cols(expr, col, *count, true))
        }
        RewriteOp::SheetRename { old, new } => Ok(rename_sheet(expr, old, new)),
        RewriteOp::TableRename { old, new } => Ok(rename_table(expr, old, new)),
    }
}

fn validate_row_rewrite(at: u32, count: u32) -> Result<(), ParseError> {
    if at >= MAX_ROWS || count > MAX_ROWS - at {
        return Err(ParseError::parse(
            "row rewrite is outside the worksheet grid",
            0,
            vec!["valid row range".into()],
        ));
    }
    Ok(())
}

fn validate_col_rewrite(at: u16, count: u16) -> Result<(), ParseError> {
    if u32::from(count) > u32::from(MAX_COLS - at) {
        return Err(ParseError::parse(
            "column rewrite is outside the worksheet grid",
            0,
            vec!["valid column range".into()],
        ));
    }
    Ok(())
}

/// Copy/fill: relative row/col shift. Absolute axes stay. Out of grid → `#REF!`.
#[must_use]
pub fn copy_delta(expr: &Expr, drow: i32, dcol: i32) -> Expr {
    map_refs_including_external(
        expr,
        &mut |cell| shift_cell(*cell, drow, dcol, true),
        &mut |r| shift_range(*r, drow, dcol, true),
    )
}

/// Cut-paste: refs fully inside `src` retarget by dest − src.start (absolute too),
/// while refs fully inside the overwritten destination become `#REF!`.
#[must_use]
pub fn move_range(expr: &Expr, src: RangeRef, dest: CellRef) -> Expr {
    let drow = i32::try_from(dest.row)
        .unwrap_or(0)
        .saturating_sub(i32::try_from(src.start.row).unwrap_or(0));
    let dcol = i32::from(dest.col).saturating_sub(i32::from(src.start.col));
    let destination = shift_range(src, drow, dcol, false).ok();
    map_refs(
        expr,
        &mut |cell| {
            if cell_in_range(*cell, src) {
                shift_cell(*cell, drow, dcol, false)
            } else if destination.is_some_and(|range| cell_in_range(*cell, range)) {
                Err(ErrorKind::Ref)
            } else {
                Ok(*cell)
            }
        },
        &mut |r| {
            if range_fully_in(*r, src) {
                shift_range(*r, drow, dcol, false)
            } else if destination.is_some_and(|range| range_fully_in(*r, range)) {
                Err(ErrorKind::Ref)
            } else {
                Ok(*r)
            }
        },
    )
}

/// Insert/delete rows. `at` is 0-based; `delete` shrinks / `#REF!`s deleted rows.
#[must_use]
pub fn adjust_rows(expr: &Expr, at: u32, count: u32, delete: bool) -> Expr {
    map_refs(
        expr,
        &mut |cell| match adj_row(cell.row, at, count, delete) {
            None => Err(ErrorKind::Ref),
            Some(row) => Ok(CellRef { row, ..*cell }),
        },
        &mut |r| adjust_range_rows(*r, at, count, delete),
    )
}

/// Insert/delete columns. `at` is 0-based.
#[must_use]
pub fn adjust_cols(expr: &Expr, at: u16, count: u16, delete: bool) -> Expr {
    map_refs(
        expr,
        &mut |cell| match adj_col(cell.col, at, count, delete) {
            None => Err(ErrorKind::Ref),
            Some(col) => Ok(CellRef { col, ..*cell }),
        },
        &mut |r| adjust_range_cols(*r, at, count, delete),
    )
}

/// Rename sheet qualifiers (case-insensitive match, new spelling used).
#[must_use]
pub fn rename_sheet(expr: &Expr, old: &str, new: &str) -> Expr {
    let Expr { kind, span } = expr.clone();
    let kind = match kind {
        ExprKind::Array(rows) => ExprKind::Array(
            rows.into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|cell| rename_sheet(&cell, old, new))
                        .collect()
                })
                .collect(),
        ),
        ExprKind::Cell { sheet, cell } => ExprKind::Cell {
            sheet: sheet.map(|sheet| rename_spec(sheet, old, new)),
            cell,
        },
        ExprKind::Range { sheet, range } => ExprKind::Range {
            sheet: sheet.map(|sheet| rename_spec(sheet, old, new)),
            range,
        },
        ExprKind::ThreeD { sheets, inner } => ExprKind::ThreeD {
            sheets: rename_spec(sheets, old, new),
            inner: Box::new(rename_sheet(&inner, old, new)),
        },
        ExprKind::Name { sheet, name } => ExprKind::Name {
            sheet: sheet.map(|sheet| rename_spec(sheet, old, new)),
            name,
        },
        // A local workbook rename must not retarget another workbook's sheet.
        external @ ExprKind::External { .. } => external,
        ExprKind::Prefix { op, expr } => ExprKind::Prefix {
            op,
            expr: Box::new(rename_sheet(&expr, old, new)),
        },
        ExprKind::Postfix { expr, op } => ExprKind::Postfix {
            expr: Box::new(rename_sheet(&expr, old, new)),
            op,
        },
        ExprKind::Binary { op, left, right } => ExprKind::Binary {
            op,
            left: Box::new(rename_sheet(&left, old, new)),
            right: Box::new(rename_sheet(&right, old, new)),
        },
        ExprKind::Paren(inner) => ExprKind::Paren(Box::new(rename_sheet(&inner, old, new))),
        ExprKind::Call { callee, args } => {
            let callee = match callee {
                Callee::Name(name) => Callee::Name(name),
                Callee::Expr(expr) => Callee::Expr(Box::new(rename_sheet(&expr, old, new))),
            };
            let args = args
                .into_iter()
                .map(|arg| arg.map(|expr| rename_sheet(&expr, old, new)))
                .collect();
            ExprKind::Call { callee, args }
        }
        other => other,
    };
    Expr { kind, span }
}

/// Rename table names in structured refs (case-insensitive match).
#[must_use]
pub fn rename_table(expr: &Expr, old: &str, new: &str) -> Expr {
    expr.clone().map(&mut |e| {
        let kind = match e.kind {
            ExprKind::Structured(mut sr) => {
                if sr
                    .table
                    .as_ref()
                    .is_some_and(|t| t.eq_ignore_ascii_case(old))
                {
                    sr.table = Some(new.to_string());
                }
                ExprKind::Structured(sr)
            }
            other => other,
        };
        Expr { kind, span: e.span }
    })
}

fn rename_spec(mut spec: SheetSpec, old: &str, new: &str) -> SheetSpec {
    if spec.start.eq_ignore_ascii_case(old) {
        spec.start = new.to_string();
    }
    if spec
        .end
        .as_ref()
        .is_some_and(|e| e.eq_ignore_ascii_case(old))
    {
        spec.end = Some(new.to_string());
    }
    spec
}

fn map_refs<Fc, Fr>(expr: &Expr, fc: &mut Fc, fr: &mut Fr) -> Expr
where
    Fc: FnMut(&CellRef) -> Result<CellRef, ErrorKind>,
    Fr: FnMut(&RangeRef) -> Result<RangeRef, ErrorKind>,
{
    map_refs_with(expr, fc, fr, false)
}

fn map_refs_including_external<Fc, Fr>(expr: &Expr, fc: &mut Fc, fr: &mut Fr) -> Expr
where
    Fc: FnMut(&CellRef) -> Result<CellRef, ErrorKind>,
    Fr: FnMut(&RangeRef) -> Result<RangeRef, ErrorKind>,
{
    map_refs_with(expr, fc, fr, true)
}

fn map_refs_with<Fc, Fr>(expr: &Expr, fc: &mut Fc, fr: &mut Fr, traverse_external: bool) -> Expr
where
    Fc: FnMut(&CellRef) -> Result<CellRef, ErrorKind>,
    Fr: FnMut(&RangeRef) -> Result<RangeRef, ErrorKind>,
{
    let mut rewrite = |e: Expr| {
        let kind = match e.kind {
            ExprKind::Cell { sheet, cell } => match fc(&cell) {
                Ok(cell) => ExprKind::Cell { sheet, cell },
                Err(k) => ExprKind::Error(k),
            },
            ExprKind::Range { sheet, range } => match fr(&range) {
                Ok(range) => ExprKind::Range { sheet, range },
                Err(k) => ExprKind::Error(k),
            },
            other => other,
        };
        Expr { kind, span: e.span }
    };
    if traverse_external {
        expr.clone().map(&mut rewrite)
    } else {
        expr.clone().map_local(&mut rewrite)
    }
}

fn shift_cell(cell: CellRef, drow: i32, dcol: i32, rel_only: bool) -> Result<CellRef, ErrorKind> {
    let row = if !rel_only || !cell.row_abs {
        add_u32(cell.row, drow, MAX_ROWS)?
    } else {
        cell.row
    };
    let col = if !rel_only || !cell.col_abs {
        add_u16(cell.col, dcol, MAX_COLS)?
    } else {
        cell.col
    };
    Ok(CellRef { row, col, ..cell })
}

fn shift_range(
    range: RangeRef,
    drow: i32,
    dcol: i32,
    rel_only: bool,
) -> Result<RangeRef, ErrorKind> {
    if range.whole_col && !range.whole_row {
        let start = shift_cell(range.start, 0, dcol, rel_only)?;
        let end = shift_cell(range.end, 0, dcol, rel_only)?;
        return Ok(RangeRef {
            start,
            end,
            ..range
        });
    }
    if range.whole_row && !range.whole_col {
        let start = shift_cell(range.start, drow, 0, rel_only)?;
        let end = shift_cell(range.end, drow, 0, rel_only)?;
        return Ok(RangeRef {
            start,
            end,
            ..range
        });
    }
    Ok(RangeRef {
        start: shift_cell(range.start, drow, dcol, rel_only)?,
        end: shift_cell(range.end, drow, dcol, rel_only)?,
        ..range
    })
}

fn add_u32(v: u32, d: i32, max: u32) -> Result<u32, ErrorKind> {
    let n = i64::from(v) + i64::from(d);
    if n < 0 || n >= i64::from(max) {
        Err(ErrorKind::Ref)
    } else {
        Ok(n as u32)
    }
}

fn add_u16(v: u16, d: i32, max: u16) -> Result<u16, ErrorKind> {
    let n = i32::from(v) + d;
    if n < 0 || n >= i32::from(max) {
        Err(ErrorKind::Ref)
    } else {
        Ok(n as u16)
    }
}

fn norm_range(r: RangeRef) -> (u32, u32, u16, u16) {
    let r1 = r.start.row.min(r.end.row);
    let r2 = r.start.row.max(r.end.row);
    let c1 = r.start.col.min(r.end.col);
    let c2 = r.start.col.max(r.end.col);
    (r1, r2, c1, c2)
}

fn cell_in_range(cell: CellRef, src: RangeRef) -> bool {
    let (r1, r2, c1, c2) = norm_range(src);
    let row_ok = src.whole_col || (cell.row >= r1 && cell.row <= r2);
    let col_ok = src.whole_row || (cell.col >= c1 && cell.col <= c2);
    row_ok && col_ok
}

fn range_fully_in(inner: RangeRef, src: RangeRef) -> bool {
    let (ir1, ir2, ic1, ic2) = norm_range(inner);
    let (sr1, sr2, sc1, sc2) = norm_range(src);
    let rows = inner.whole_col || src.whole_col || (ir1 >= sr1 && ir2 <= sr2);
    let cols = inner.whole_row || src.whole_row || (ic1 >= sc1 && ic2 <= sc2);
    // A cell-range is fully inside a cell-range only if both axes are inside.
    if inner.whole_col && !src.whole_col {
        return false;
    }
    if inner.whole_row && !src.whole_row {
        return false;
    }
    rows && cols
}

fn adj_row(row: u32, at: u32, count: u32, delete: bool) -> Option<u32> {
    if delete {
        if row < at {
            Some(row)
        } else if row < at.saturating_add(count) {
            None
        } else {
            Some(row - count)
        }
    } else if row >= at {
        add_u32(row, count as i32, MAX_ROWS).ok()
    } else {
        Some(row)
    }
}

fn adj_col(col: u16, at: u16, count: u16, delete: bool) -> Option<u16> {
    if delete {
        if col < at {
            Some(col)
        } else if col < at.saturating_add(count) {
            None
        } else {
            Some(col - count)
        }
    } else if col >= at {
        add_u16(col, i32::from(count), MAX_COLS).ok()
    } else {
        Some(col)
    }
}

fn adjust_range_rows(
    range: RangeRef,
    at: u32,
    count: u32,
    delete: bool,
) -> Result<RangeRef, ErrorKind> {
    if range.whole_col && !range.whole_row {
        return Ok(range);
    }
    let s = adj_row(range.start.row, at, count, delete);
    let e = adj_row(range.end.row, at, count, delete);
    match (s, e, delete) {
        (None, None, _) => Err(ErrorKind::Ref),
        (Some(sr), Some(er), _) => Ok(RangeRef {
            start: CellRef {
                row: sr,
                ..range.start
            },
            end: CellRef {
                row: er,
                ..range.end
            },
            ..range
        }),
        (None, Some(er), true) => Ok(RangeRef {
            start: CellRef {
                row: at,
                ..range.start
            },
            end: CellRef {
                row: er,
                ..range.end
            },
            ..range
        }),
        (Some(sr), None, true) => {
            if at == 0 {
                return Err(ErrorKind::Ref);
            }
            Ok(RangeRef {
                start: CellRef {
                    row: sr,
                    ..range.start
                },
                end: CellRef {
                    row: at - 1,
                    ..range.end
                },
                ..range
            })
        }
        _ => Err(ErrorKind::Ref),
    }
}

fn adjust_range_cols(
    range: RangeRef,
    at: u16,
    count: u16,
    delete: bool,
) -> Result<RangeRef, ErrorKind> {
    if range.whole_row && !range.whole_col {
        return Ok(range);
    }
    let s = adj_col(range.start.col, at, count, delete);
    let e = adj_col(range.end.col, at, count, delete);
    match (s, e, delete) {
        (None, None, _) => Err(ErrorKind::Ref),
        (Some(sc), Some(ec), _) => Ok(RangeRef {
            start: CellRef {
                col: sc,
                ..range.start
            },
            end: CellRef {
                col: ec,
                ..range.end
            },
            ..range
        }),
        (None, Some(ec), true) => Ok(RangeRef {
            start: CellRef {
                col: at,
                ..range.start
            },
            end: CellRef {
                col: ec,
                ..range.end
            },
            ..range
        }),
        (Some(sc), None, true) => {
            if at == 0 {
                return Err(ErrorKind::Ref);
            }
            Ok(RangeRef {
                start: CellRef {
                    col: sc,
                    ..range.start
                },
                end: CellRef {
                    col: at - 1,
                    ..range.end
                },
                ..range
            })
        }
        _ => Err(ErrorKind::Ref),
    }
}

fn parse_cell_a1(s: &str) -> Result<CellRef, ParseError> {
    match crate::addr::parse_a1_cell(s) {
        Ok(c) => Ok(c),
        Err(e) => Err(ParseError::parse(e.message, 0, Vec::new())),
    }
}

fn parse_range_a1(s: &str) -> Result<RangeRef, ParseError> {
    match crate::addr::parse_a1(s) {
        Ok(p) if p.sheet.is_some() => Err(ParseError::parse(
            "move source must be sheet-local",
            0,
            vec!["unqualified range".into()],
        )),
        Ok(p) => match p.kind {
            crate::addr::RefKind::Cell(c) => Ok(RangeRef::from_corners(c, c)),
            crate::addr::RefKind::Range(r) => Ok(r),
        },
        Err(e) => Err(ParseError::parse(e.message, 0, Vec::new())),
    }
}

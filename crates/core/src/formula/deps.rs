//! Dependency extraction for the recalc graph (WP-04).

use crate::addr::{RangeRef, SheetSpec};
use crate::limits::{MAX_COLS, MAX_ROWS};

use super::ast::{Callee, Expr, ExprKind};

/// Names of functions that are volatile every recalc pass (F-3.6).
pub const VOLATILE_FUNCS: &[&str] = &[
    "NOW",
    "TODAY",
    "RAND",
    "RANDBETWEEN",
    "RANDARRAY",
    "OFFSET",
    "INDIRECT",
    "INFO",
    "CELL",
];

/// Functions whose effective referenced ranges are not known until eval (F-3.6).
pub const DYNAMIC_FUNCS: &[&str] = &["OFFSET", "INDIRECT"];

const RESIZED_RANGE_FUNCS: &[&str] = &["SUMIF", "AVERAGEIF"];

/// Precedents extracted from a formula AST.
///
/// ```
/// use omacell_core::formula::{parse, collect_deps};
/// let f = parse("=NOW()+A1").unwrap();
/// let d = collect_deps(&f.ast);
/// assert!(d.volatile);
/// assert_eq!(d.ranges.len(), 1);
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deps {
    /// Current-workbook cell/range/3-D bodies (cells as degenerate ranges).
    /// External-workbook references are not representable here and are omitted.
    pub ranges: Vec<(Option<SheetSpec>, RangeRef)>,
    /// Current-workbook defined names, with optional sheet qualifier.
    pub names: Vec<(Option<SheetSpec>, String)>,
    /// Structured-reference table names (unqualified omitted).
    pub tables: Vec<String>,
    /// True if a volatile function appears.
    pub volatile: bool,
    /// True if a function with evaluation-resolved precedents appears.
    pub dynamic: bool,
}

/// Walk `expr` and collect precedents, names, tables, and volatility flags.
#[must_use]
pub fn collect_deps(expr: &Expr) -> Deps {
    let mut deps = Deps::default();
    collect(expr, None, &mut deps);
    deps.tables.sort();
    deps
}

fn collect(expr: &Expr, inherited_sheet: Option<&SheetSpec>, deps: &mut Deps) {
    match &expr.kind {
        ExprKind::Cell { sheet, cell } => {
            deps.ranges.push((
                sheet.clone().or_else(|| inherited_sheet.cloned()),
                RangeRef::from_corners(*cell, *cell),
            ));
        }
        ExprKind::Range { sheet, range } => {
            deps.ranges
                .push((sheet.clone().or_else(|| inherited_sheet.cloned()), *range));
        }
        ExprKind::ThreeD { sheets, inner } => {
            collect(inner, Some(sheets), deps);
        }
        ExprKind::Name { sheet, name } => {
            deps.names.push((
                sheet.clone().or_else(|| inherited_sheet.cloned()),
                name.clone(),
            ));
        }
        ExprKind::Structured(sr) => {
            if let Some(t) = &sr.table
                && !deps.tables.iter().any(|x| x.eq_ignore_ascii_case(t))
            {
                deps.tables.push(t.clone());
            }
        }
        ExprKind::Call {
            callee: Callee::Name(n),
            args,
        } => {
            let u = n.to_ascii_uppercase();
            if VOLATILE_FUNCS.iter().any(|v| *v == u) {
                deps.volatile = true;
            }
            if DYNAMIC_FUNCS.iter().any(|v| *v == u) {
                deps.dynamic = true;
            }
            if RESIZED_RANGE_FUNCS.iter().any(|v| *v == u)
                && args.get(2).and_then(Option::as_ref).is_some()
            {
                if collect_static_resized_range(args, inherited_sheet, deps) {
                    return;
                }
                deps.dynamic = true;
            }
            for arg in args.iter().flatten() {
                collect(arg, inherited_sheet, deps);
            }
        }
        ExprKind::Call {
            callee: Callee::Expr(callee),
            args,
        } => {
            collect(callee, inherited_sheet, deps);
            for arg in args.iter().flatten() {
                collect(arg, inherited_sheet, deps);
            }
        }
        ExprKind::Array(rows) => {
            for cell in rows.iter().flatten() {
                collect(cell, inherited_sheet, deps);
            }
        }
        ExprKind::External { .. } => {}
        ExprKind::Prefix { expr: inner, .. }
        | ExprKind::Postfix { expr: inner, .. }
        | ExprKind::Paren(inner) => collect(inner, inherited_sheet, deps),
        ExprKind::Binary { left, right, .. } => {
            collect(left, inherited_sheet, deps);
            collect(right, inherited_sheet, deps);
        }
        ExprKind::Number(_) | ExprKind::String(_) | ExprKind::Bool(_) | ExprKind::Error(_) => {}
    }
}

fn collect_static_resized_range(
    args: &[Option<Expr>],
    inherited_sheet: Option<&SheetSpec>,
    deps: &mut Deps,
) -> bool {
    let Some(criteria) = args.first().and_then(Option::as_ref) else {
        return false;
    };
    let Some(values) = args.get(2).and_then(Option::as_ref) else {
        return false;
    };
    let Some((_, criteria_range)) = static_range(criteria, inherited_sheet) else {
        return false;
    };
    let Some((values_sheet, values_range)) = static_range(values, inherited_sheet) else {
        return false;
    };
    let Some(effective_range) = resize_from_top_left(criteria_range, values_range) else {
        return false;
    };

    for (index, arg) in args.iter().enumerate() {
        if index != 2
            && let Some(arg) = arg
        {
            collect(arg, inherited_sheet, deps);
        }
    }
    deps.ranges.push((values_sheet, effective_range));
    true
}

fn static_range(
    expr: &Expr,
    inherited_sheet: Option<&SheetSpec>,
) -> Option<(Option<SheetSpec>, RangeRef)> {
    match &expr.kind {
        ExprKind::Cell { sheet, cell } => Some((
            sheet.clone().or_else(|| inherited_sheet.cloned()),
            RangeRef::from_corners(*cell, *cell),
        )),
        ExprKind::Range { sheet, range } => {
            Some((sheet.clone().or_else(|| inherited_sheet.cloned()), *range))
        }
        ExprKind::Paren(inner) => static_range(inner, inherited_sheet),
        _ => None,
    }
}

fn resize_from_top_left(criteria: RangeRef, values: RangeRef) -> Option<RangeRef> {
    let rows = criteria.start.row.abs_diff(criteria.end.row) + 1;
    let cols = u32::from(criteria.start.col.abs_diff(criteria.end.col)) + 1;
    let start_row = values.start.row.min(values.end.row);
    let start_col = values.start.col.min(values.end.col);
    let end_row = start_row
        .checked_add(rows - 1)
        .filter(|row| *row < MAX_ROWS)?;
    let end_col = u32::from(start_col)
        .checked_add(cols - 1)
        .filter(|col| *col < u32::from(MAX_COLS))
        .and_then(|col| u16::try_from(col).ok())?;

    let mut effective = values;
    effective.start.row = start_row;
    effective.start.col = start_col;
    effective.end.row = end_row;
    effective.end.col = end_col;
    effective.whole_col = start_row == 0 && end_row == MAX_ROWS - 1;
    effective.whole_row = start_col == 0 && end_col == MAX_COLS - 1;
    Some(effective)
}

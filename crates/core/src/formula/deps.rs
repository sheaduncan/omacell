//! Dependency extraction for the recalc graph (WP-04).

use crate::addr::{RangeRef, SheetSpec};

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

/// Functions whose referenced ranges are not known until eval (F-3.6).
pub const DYNAMIC_FUNCS: &[&str] = &["OFFSET", "INDIRECT"];

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
    /// True if `INDIRECT` / `OFFSET` appears.
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

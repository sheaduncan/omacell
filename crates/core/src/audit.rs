//! Deterministic workbook audit, error explanations, and formula traces (F-3.8, A-4.5).

use std::collections::{BTreeMap, BTreeSet};

use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId, col_to_letters};
use crate::eval::{FnRegistry, eval_formula, format_runtime};
use crate::formula::{
    BinOp, Callee, ExprKind, PrintOptions, RefStyle, collect_deps, parse, print_expr, print_with,
};
use crate::graph::{CellCoord, DepGraph, Precedent};
use crate::recalc::RecalcEngine;
use crate::value::Value;
use crate::workbook::Workbook;

/// Finding severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Blocks correctness (cycles).
    Error,
    /// Likely defect.
    Warning,
    /// Informational.
    Info,
}

/// Optional one-command fix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FixCommand {
    /// Dotted command id.
    pub id: String,
    /// JSON arguments.
    pub args: serde_json::Value,
}

/// One deterministic finding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Stable check id (`audit.hardcoded_constant`).
    pub id: String,
    /// Severity.
    pub severity: Severity,
    /// Sheet name.
    pub sheet: String,
    /// A1 cell or range.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Human message.
    pub message: String,
    /// Safe fix, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<FixCommand>,
}

/// Versioned audit envelope (`docs/schemas/audit.schema.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditReport {
    /// Schema version (currently 1).
    pub schema: u32,
    /// Findings in deterministic order.
    pub findings: Vec<Finding>,
}

/// Evaluate-Formula step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalStep {
    /// Printed sub-expression.
    pub expr: String,
    /// Intermediate value.
    pub value: String,
}

/// Diagnostic bundle for `omacell agent diagnose` (A-5.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    /// Error explanations for cells currently in error.
    pub errors: Vec<ErrorExplanation>,
    /// Direct neighborhood of `origin`.
    pub neighborhood: Neighborhood,
    /// Recent undo unit ids, newest last.
    pub undo: Vec<String>,
}

/// Explained cell error.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorExplanation {
    /// Sheet name.
    pub sheet: String,
    /// A1.
    #[serde(rename = "ref")]
    pub cell_ref: String,
    /// Display token (`#NAME?`).
    pub kind: String,
    /// Explanation.
    pub message: String,
}

/// Precedents and dependents of one cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Neighborhood {
    /// Origin A1 with sheet.
    pub origin: String,
    /// Direct precedents.
    pub precedents: Vec<String>,
    /// Direct dependents.
    pub dependents: Vec<String>,
}

/// Identity redaction hook for WP-22.
pub fn redact_identity(report: AuditReport) -> AuditReport {
    report
}

/// Run every deterministic check. Findings are sorted by `(id, sheet, ref)`.
#[must_use]
pub fn audit_workbook(wb: &Workbook, engine: &RecalcEngine) -> AuditReport {
    let mut findings = Vec::new();
    hardcoded_constants(wb, &mut findings);
    inconsistent_formulas(wb, &mut findings);
    range_short(wb, &mut findings);
    unused_names(wb, &mut findings);
    circular(wb, engine.graph(), &mut findings);
    external_links(wb, &mut findings);
    volatiles(wb, engine.graph(), &mut findings);
    merges_in_tables(wb, &mut findings);
    findings.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.sheet.cmp(&b.sheet))
            .then_with(|| a.cell_ref.cmp(&b.cell_ref))
    });
    AuditReport {
        schema: 1,
        findings,
    }
}

/// Direct or transitive formula precedents as `Sheet!A1`.
#[must_use]
pub fn precedents_of(
    wb: &Workbook,
    engine: &RecalcEngine,
    cell: CellCoord,
    transitive: bool,
) -> Vec<String> {
    walk_prec(wb, engine.graph(), cell, transitive)
}

/// Direct or transitive dependents as `Sheet!A1`.
#[must_use]
pub fn dependents_of(
    wb: &Workbook,
    engine: &RecalcEngine,
    cell: CellCoord,
    transitive: bool,
) -> Vec<String> {
    let graph = engine.graph();
    let mut out = Vec::new();
    let mut seen = FxHashSet::default();
    let mut stack = vec![cell];
    seen.insert(cell);
    while let Some(cur) = stack.pop() {
        for dep in graph.dependents(cur) {
            if !seen.insert(dep) {
                continue;
            }
            out.push(coord_a1(wb, dep));
            if transitive {
                stack.push(dep);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Evaluate-Formula steps (children first).
#[must_use]
pub fn eval_steps(wb: &Workbook, engine: &RecalcEngine, cell: CellCoord) -> Vec<EvalStep> {
    let Some(src) = formula_src(wb, cell) else {
        return Vec::new();
    };
    let Ok(parsed) = parse(&src) else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    parsed.ast.walk(&mut |expr| {
        let printed = print_expr(
            expr,
            PrintOptions {
                style: RefStyle::A1,
                base_row: cell.row,
                base_col: cell.col,
            },
        );
        let (value, _) = eval_formula(wb, engine.registry(), engine.spill(), cell, expr, 0);
        steps.push(EvalStep {
            expr: printed,
            value: format_runtime(&value),
        });
    });
    steps
}

/// Explain a cell error, or `None` when the cell is not an error.
#[must_use]
pub fn explain_error(
    wb: &Workbook,
    engine: &RecalcEngine,
    cell: CellCoord,
) -> Option<ErrorExplanation> {
    let slot = wb.get(cell.sheet, cell.row, cell.col).ok().flatten()?;
    let Value::Error(kind) = slot.value else {
        return None;
    };
    let sheet = sheet_name(wb, cell.sheet);
    let cell_ref = cell_a1(cell.row, cell.col);
    let message = match kind {
        crate::error::ErrorKind::Name => explain_name(wb, engine.registry(), cell),
        crate::error::ErrorKind::Ref => explain_ref(wb, cell),
        crate::error::ErrorKind::Spill => explain_spill(wb, engine, cell),
        crate::error::ErrorKind::Div0 => explain_div0(wb, engine, cell),
        other => format!("{} in {sheet}!{cell_ref}", other.as_str()),
    };
    Some(ErrorExplanation {
        sheet,
        cell_ref,
        kind: kind.as_str().to_string(),
        message,
    })
}

/// Diagnostic bundle around `origin`.
#[must_use]
pub fn diagnose(wb: &Workbook, engine: &RecalcEngine, origin: CellCoord) -> DiagnosticBundle {
    let mut errors = Vec::new();
    for sheet in wb.sheets() {
        for (row, col, slot) in sheet.store.iter() {
            if matches!(slot.value, Value::Error(_)) {
                let coord = CellCoord::new(sheet.id, row, col);
                if let Some(exp) = explain_error(wb, engine, coord) {
                    errors.push(exp);
                }
            }
        }
    }
    errors.sort_by(|a, b| {
        a.sheet
            .cmp(&b.sheet)
            .then_with(|| a.cell_ref.cmp(&b.cell_ref))
    });
    let undo = wb
        .undo_log()
        .history()
        .map(|tx| tx.id.index().to_string())
        .collect();
    DiagnosticBundle {
        errors,
        neighborhood: Neighborhood {
            origin: coord_a1(wb, origin),
            precedents: precedents_of(wb, engine, origin, false),
            dependents: dependents_of(wb, engine, origin, false),
        },
        undo,
    }
}

fn hardcoded_constants(wb: &Workbook, out: &mut Vec<Finding>) {
    for sheet in wb.sheets() {
        for (row, col, slot) in sheet.store.iter() {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(src) = wb.intern().formulas.get(fid) else {
                continue;
            };
            let Ok(parsed) = parse(src) else {
                continue;
            };
            if matches!(parsed.ast.kind, ExprKind::Number(_)) {
                continue;
            }
            let mut hit = None;
            parsed.ast.walk(&mut |expr| {
                if let ExprKind::Number(n) = expr.kind {
                    hit = Some(n);
                }
            });
            if let Some(n) = hit {
                out.push(finding(
                    "audit.hardcoded_constant",
                    Severity::Warning,
                    &sheet.name,
                    &cell_a1(row, col),
                    format!("formula embeds constant {n}"),
                    None,
                ));
            }
        }
    }
}

fn inconsistent_formulas(wb: &Workbook, out: &mut Vec<Finding>) {
    for sheet in wb.sheets() {
        let mut by_col: BTreeMap<u16, Vec<(u32, String)>> = BTreeMap::new();
        for (row, col, slot) in sheet.store.iter() {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(src) = wb.intern().formulas.get(fid) else {
                continue;
            };
            let Ok(parsed) = parse(src) else {
                continue;
            };
            let r1c1 = print_with(
                &parsed,
                PrintOptions {
                    style: RefStyle::R1C1,
                    base_row: row,
                    base_col: col,
                },
            );
            by_col.entry(col).or_default().push((row, r1c1));
        }
        for (col, mut cells) in by_col {
            if cells.len() < 3 {
                continue;
            }
            cells.sort_by_key(|(r, _)| *r);
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for (_, p) in &cells {
                *counts.entry(p.as_str()).or_insert(0) += 1;
            }
            let Some((majority, count)) = counts.iter().max_by_key(|(_, n)| *n) else {
                continue;
            };
            if *count < 3 || *count == cells.len() {
                continue;
            }
            let majority = (*majority).to_string();
            for (row, pat) in &cells {
                if pat != &majority {
                    out.push(finding(
                        "audit.inconsistent_formula",
                        Severity::Warning,
                        &sheet.name,
                        &cell_a1(*row, col),
                        format!("R1C1 {pat} differs from majority {majority}"),
                        None,
                    ));
                }
            }
        }
    }
}

fn range_short(wb: &Workbook, out: &mut Vec<Finding>) {
    for sheet in wb.sheets() {
        let Some(used) = sheet.used_range() else {
            continue;
        };
        for (row, col, slot) in sheet.store.iter() {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(src) = wb.intern().formulas.get(fid) else {
                continue;
            };
            let Ok(parsed) = parse(src) else {
                continue;
            };
            let mut short = None;
            parsed.ast.walk(&mut |expr| {
                if let ExprKind::Range { range, .. } = expr.kind
                    && range.start.col == range.end.col
                {
                    let c = range.start.col;
                    let end = range.start.row.max(range.end.row);
                    if end < used.max_row && c >= used.min_col && c <= used.max_col {
                        short = Some((c, end, used.max_row));
                    }
                }
            });
            if let Some((c, end, data_end)) = short {
                out.push(finding(
                    "audit.range_short",
                    Severity::Warning,
                    &sheet.name,
                    &cell_a1(row, col),
                    format!(
                        "range ends at {} while data continues to {}",
                        cell_a1(end, c),
                        cell_a1(data_end, c)
                    ),
                    None,
                ));
            }
        }
    }
}

fn unused_names(wb: &Workbook, out: &mut Vec<Finding>) {
    let mut used = BTreeSet::new();
    for sheet in wb.sheets() {
        for (_, _, slot) in sheet.store.iter() {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(src) = wb.intern().formulas.get(fid) else {
                continue;
            };
            let Ok(parsed) = parse(src) else {
                continue;
            };
            for (_, name) in collect_deps(&parsed.ast).names {
                used.insert(name.to_lowercase());
            }
        }
    }
    let first = wb
        .sheets()
        .next()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "Sheet1".into());
    for name in wb.names().iter() {
        if used.contains(&name.name.to_lowercase()) {
            continue;
        }
        out.push(finding(
            "audit.unused_name",
            Severity::Info,
            &first,
            "A1",
            format!("defined name {} is never referenced", name.name),
            Some(FixCommand {
                id: "name.remove".into(),
                args: serde_json::json!({"name": name.name}),
            }),
        ));
    }
}

fn circular(wb: &Workbook, graph: &DepGraph, out: &mut Vec<Finding>) {
    let cells = graph.formula_cells();
    for coord in graph.circular_set(&cells) {
        out.push(finding(
            "audit.circular",
            Severity::Error,
            &sheet_name(wb, coord.sheet),
            &cell_a1(coord.row, coord.col),
            "cell is part of a circular reference".into(),
            None,
        ));
    }
}

fn external_links(wb: &Workbook, out: &mut Vec<Finding>) {
    for sheet in wb.sheets() {
        for (row, col, slot) in sheet.store.iter() {
            let Some(fid) = slot.formula else {
                continue;
            };
            let Some(src) = wb.intern().formulas.get(fid) else {
                continue;
            };
            let Ok(parsed) = parse(src) else {
                continue;
            };
            let mut book = None;
            parsed.ast.walk(&mut |expr| {
                if let ExprKind::External { workbook, .. } = &expr.kind {
                    book = Some(workbook.clone());
                }
            });
            if let Some(workbook) = book {
                out.push(finding(
                    "audit.external_link",
                    Severity::Info,
                    &sheet.name,
                    &cell_a1(row, col),
                    format!("formula references [{workbook}]"),
                    None,
                ));
            }
        }
        let mut hrefs: Vec<_> = sheet.hyperlinks.iter().collect();
        hrefs.sort_by_key(|(k, _)| *k);
        for ((row, col), link) in hrefs {
            if is_external_hyperlink(&link.target) {
                out.push(finding(
                    "audit.external_link",
                    Severity::Info,
                    &sheet.name,
                    &cell_a1(*row, *col),
                    format!("hyperlink {}", link.target),
                    None,
                ));
            }
        }
    }
}

fn volatiles(wb: &Workbook, graph: &DepGraph, out: &mut Vec<Finding>) {
    for coord in graph.volatiles() {
        out.push(finding(
            "audit.volatile",
            Severity::Info,
            &sheet_name(wb, coord.sheet),
            &cell_a1(coord.row, coord.col),
            "formula is volatile".into(),
            None,
        ));
    }
}

fn merges_in_tables(wb: &Workbook, out: &mut Vec<Finding>) {
    for table in wb.tables().iter() {
        let Some(sheet) = wb.sheet(table.sheet) else {
            continue;
        };
        let tr = RangeRef::from_corners(
            crate::addr::CellRef::new(table.start_row, table.start_col)
                .unwrap_or(unreachable_cell()),
            crate::addr::CellRef::new(table.end_row, table.end_col).unwrap_or(unreachable_cell()),
        );
        for merge in &sheet.merges {
            if ranges_overlap(*merge, tr) {
                out.push(finding(
                    "audit.merge_in_table",
                    Severity::Warning,
                    &sheet.name,
                    &merge.to_a1(),
                    format!("merged cells overlap table {}", table.name),
                    Some(FixCommand {
                        id: "range.unmerge".into(),
                        args: serde_json::json!({"range": merge.to_a1()}),
                    }),
                ));
            }
        }
    }
}

fn ranges_overlap(a: RangeRef, b: RangeRef) -> bool {
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

fn walk_prec(wb: &Workbook, graph: &DepGraph, cell: CellCoord, transitive: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = FxHashSet::default();
    let mut stack = vec![cell];
    seen.insert(cell);
    while let Some(cur) = stack.pop() {
        for prec in graph.precedents(cur) {
            let cells = expand_prec(prec);
            for p in cells {
                if !seen.insert(p) {
                    continue;
                }
                out.push(coord_a1(wb, p));
                if transitive {
                    stack.push(p);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn expand_prec(p: &Precedent) -> Vec<CellCoord> {
    match p {
        Precedent::Cell(c) => vec![*c],
        Precedent::Range { sheet, range, .. } => {
            let (r0, c0, r1, c1) = norm(*range);
            let mut v = Vec::new();
            let r1 = r1.min(r0.saturating_add(256));
            let c1 = c1.min(c0.saturating_add(32));
            for r in r0..=r1 {
                for c in c0..=c1 {
                    v.push(CellCoord::new(*sheet, r, c));
                }
            }
            v
        }
        Precedent::ThreeD { sheets, range } => {
            let mut v = Vec::new();
            for sheet in sheets {
                v.extend(expand_prec(&Precedent::Range {
                    sheet: *sheet,
                    range: *range,
                    whole_col: false,
                    whole_row: false,
                }));
            }
            v
        }
    }
}

fn explain_name(wb: &Workbook, registry: &FnRegistry, cell: CellCoord) -> String {
    let Some(src) = formula_src(wb, cell) else {
        return "#NAME? unknown token".into();
    };
    let Ok(parsed) = parse(&src) else {
        return "#NAME? unknown token".into();
    };
    let mut token = None;
    parsed.ast.walk(&mut |expr| match &expr.kind {
        ExprKind::Call {
            callee: Callee::Name(name),
            ..
        } if registry.lookup(name).is_none()
            && !matches!(
                name.to_ascii_uppercase().as_str(),
                "LET" | "LAMBDA" | "ISOMITTED"
            ) =>
        {
            token = Some(name.clone());
        }
        ExprKind::Name { name, .. } if wb.names().resolve(cell.sheet, name).is_none() => {
            token = Some(name.clone());
        }
        _ => {}
    });
    match token {
        Some(t) => format!("#NAME? unknown token {t}"),
        None => "#NAME? unknown token".into(),
    }
}

fn explain_ref(wb: &Workbook, cell: CellCoord) -> String {
    let Some(src) = formula_src(wb, cell) else {
        return "#REF! deleted range".into();
    };
    if src.contains("#REF!") {
        return format!("#REF! deleted range in {src}");
    }
    let Ok(parsed) = parse(&src) else {
        return "#REF! deleted range".into();
    };
    let mut hit = None;
    parsed.ast.walk(&mut |expr| {
        if let ExprKind::Error(crate::error::ErrorKind::Ref) = expr.kind {
            hit = Some("#REF!".to_string());
        }
    });
    match hit {
        Some(t) => format!("#REF! deleted range {t}"),
        None => "#REF! deleted range".into(),
    }
}

fn explain_spill(wb: &Workbook, engine: &RecalcEngine, cell: CellCoord) -> String {
    if let Some(region) = engine.spill().get(cell)
        && let Some(blocker) = region.blocked_by
    {
        return format!("#SPILL! blocked by {}", coord_a1(wb, blocker));
    }
    "#SPILL! blocked".into()
}

fn explain_div0(wb: &Workbook, _engine: &RecalcEngine, cell: CellCoord) -> String {
    let Some(src) = formula_src(wb, cell) else {
        return "#DIV/0! divisor is zero".into();
    };
    let Ok(parsed) = parse(&src) else {
        return "#DIV/0! divisor is zero".into();
    };
    let mut operand = None;
    parsed.ast.walk(&mut |expr| {
        if let ExprKind::Binary {
            op: BinOp::Div,
            right,
            ..
        } = &expr.kind
        {
            operand = Some(print_expr(
                right,
                PrintOptions {
                    style: RefStyle::A1,
                    base_row: cell.row,
                    base_col: cell.col,
                },
            ));
        }
    });
    match operand {
        Some(op) => format!("#DIV/0! divisor {op} is zero"),
        None => "#DIV/0! divisor is zero".into(),
    }
}

fn formula_src(wb: &Workbook, cell: CellCoord) -> Option<String> {
    let slot = wb.get(cell.sheet, cell.row, cell.col).ok().flatten()?;
    let fid = slot.formula?;
    wb.intern().formulas.get(fid).map(str::to_string)
}

fn finding(
    id: &str,
    severity: Severity,
    sheet: &str,
    cell_ref: &str,
    message: String,
    fix: Option<FixCommand>,
) -> Finding {
    Finding {
        id: id.into(),
        severity,
        sheet: sheet.into(),
        cell_ref: cell_ref.into(),
        message,
        fix,
    }
}

fn sheet_name(wb: &Workbook, id: SheetId) -> String {
    wb.sheet(id)
        .map(|s| s.name.clone())
        .unwrap_or_else(|| format!("Sheet{}", id.index() + 1))
}

fn cell_a1(row: u32, col: u16) -> String {
    format!(
        "{}{}",
        col_to_letters(col).unwrap_or_else(|_| "A".into()),
        row + 1
    )
}

fn coord_a1(wb: &Workbook, cell: CellCoord) -> String {
    format!(
        "{}!{}",
        sheet_name(wb, cell.sheet),
        cell_a1(cell.row, cell.col)
    )
}

fn is_external_hyperlink(target: &str) -> bool {
    let t = target.trim();
    t.contains("://") || t.starts_with("file:") || t.starts_with("\\\\")
}

fn unreachable_cell() -> crate::addr::CellRef {
    crate::addr::CellRef {
        sheet: None,
        row: 0,
        col: 0,
        row_abs: false,
        col_abs: false,
    }
}

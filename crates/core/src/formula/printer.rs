//! Canonical formula printer (A1 / R1C1).

use crate::addr::{CellRef, RangeRef, SheetSpec, quote_sheet_name};

use super::ast::{Callee, Expr, ExprKind, PostfixOp, PrefixOp, StructuredRef, TableColumns};
use super::{Formula, PrintOptions, RefStyle};

/// Print `formula` in the style it was parsed with.
///
/// ```
/// use omacell_core::formula::{parse, print};
/// assert_eq!(print(&parse("= a1 + 1 ").unwrap()), "=A1+1");
/// ```
#[must_use]
pub fn print(formula: &Formula) -> String {
    print_with(
        formula,
        PrintOptions {
            style: formula.style,
            base_row: formula.base_row,
            base_col: formula.base_col,
        },
    )
}

/// Print with an explicit style (A1 or R1C1).
#[must_use]
pub fn print_with(formula: &Formula, opts: PrintOptions) -> String {
    let mut s = String::from("=");
    write_expr(&mut s, &formula.ast, &opts);
    s
}

/// Print an expression (no leading `=`).
#[must_use]
pub fn print_expr(expr: &Expr, opts: PrintOptions) -> String {
    let mut s = String::new();
    write_expr(&mut s, expr, &opts);
    s
}

fn write_expr(s: &mut String, expr: &Expr, opts: &PrintOptions) {
    match &expr.kind {
        ExprKind::Number(n) => s.push_str(&format!("{n}")),
        ExprKind::String(v) => {
            s.push('"');
            s.push_str(&v.replace('"', "\"\""));
            s.push('"');
        }
        ExprKind::Bool(true) => s.push_str("TRUE"),
        ExprKind::Bool(false) => s.push_str("FALSE"),
        ExprKind::Error(e) => s.push_str(e.as_str()),
        ExprKind::Array(rows) => {
            s.push('{');
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    s.push(';');
                }
                for (j, c) in row.iter().enumerate() {
                    if j > 0 {
                        s.push(',');
                    }
                    write_expr(s, c, opts);
                }
            }
            s.push('}');
        }
        ExprKind::Cell { sheet, cell } => {
            write_sheet(s, sheet.as_ref());
            s.push_str(&print_cell(*cell, opts));
        }
        ExprKind::Range { sheet, range } => {
            write_sheet(s, sheet.as_ref());
            s.push_str(&print_range(*range, opts));
        }
        ExprKind::ThreeD { sheets, inner } => {
            s.push_str(&formula_sheet_prefix(sheets));
            write_expr(s, inner, opts);
        }
        ExprKind::Name { sheet, name } => {
            write_sheet(s, sheet.as_ref());
            s.push_str(name);
        }
        ExprKind::Structured(sr) => write_structured(s, sr),
        ExprKind::External {
            workbook,
            inner,
            quoted,
        } => {
            write_external(s, workbook, inner, *quoted, opts);
        }
        ExprKind::Prefix { op, expr } => {
            s.push_str(match op {
                PrefixOp::Plus => "+",
                PrefixOp::Minus => "-",
                PrefixOp::ImplicitIntersect => "@",
            });
            write_expr(s, expr, opts);
        }
        ExprKind::Postfix { expr, op } => {
            write_expr(s, expr, opts);
            s.push_str(match op {
                PostfixOp::Percent => "%",
                PostfixOp::Spill => "#",
            });
        }
        ExprKind::Binary { op, left, right } => {
            write_expr(s, left, opts);
            s.push_str(op.as_str());
            write_expr(s, right, opts);
        }
        ExprKind::Paren(inner) => {
            s.push('(');
            write_expr(s, inner, opts);
            s.push(')');
        }
        ExprKind::Call { callee, args } => {
            match callee {
                Callee::Name(n) => s.push_str(&n.to_ascii_uppercase()),
                Callee::Expr(e) => write_expr(s, e, opts),
            }
            s.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                if let Some(e) = a {
                    write_expr(s, e, opts);
                }
            }
            s.push(')');
        }
    }
}

fn write_sheet(s: &mut String, sheet: Option<&SheetSpec>) {
    if let Some(sheet) = sheet {
        s.push_str(&formula_sheet_prefix(sheet));
    }
}

fn formula_sheet_prefix(sheet: &SheetSpec) -> String {
    match &sheet.end {
        None => format!("{}!", formula_quote_sheet(&sheet.start)),
        Some(end) => {
            let a = formula_quote_sheet(&sheet.start);
            let b = formula_quote_sheet(end);
            if a.starts_with('\'') || b.starts_with('\'') {
                format!(
                    "'{}:{}'!",
                    sheet.start.replace('\'', "''"),
                    end.replace('\'', "''")
                )
            } else {
                format!("{}:{}!", sheet.start, end)
            }
        }
    }
}

fn formula_quote_sheet(name: &str) -> String {
    if name.eq_ignore_ascii_case("TRUE") || name.eq_ignore_ascii_case("FALSE") {
        format!("'{}'", name.replace('\'', "''"))
    } else {
        quote_sheet_name(name)
    }
}

fn print_cell(cell: CellRef, opts: &PrintOptions) -> String {
    match opts.style {
        RefStyle::A1 => cell.to_a1(),
        RefStyle::R1C1 => cell.to_r1c1(opts.base_row, opts.base_col),
    }
}

fn print_range(range: RangeRef, opts: &PrintOptions) -> String {
    match opts.style {
        RefStyle::A1 => range.to_a1(),
        RefStyle::R1C1 => range.to_r1c1(opts.base_row, opts.base_col),
    }
}

fn write_structured(s: &mut String, sr: &StructuredRef) {
    if let Some(t) = &sr.table {
        s.push_str(t);
    }
    if !sr.inner.is_empty() {
        s.push_str(&sr.inner);
        return;
    }
    s.push('[');
    if sr.this_row && sr.columns.is_some() && sr.item.is_none() {
        s.push('@');
        if let Some(TableColumns::One(c)) = &sr.columns {
            s.push_str(c);
        }
    } else {
        if let Some(item) = sr.item {
            s.push_str(item.as_str());
        }
        if let Some(cols) = &sr.columns {
            if sr.item.is_some() {
                s.push(',');
            }
            match cols {
                TableColumns::One(c) => s.push_str(c),
                TableColumns::Span { start, end } => {
                    s.push_str(start);
                    s.push(':');
                    s.push_str(end);
                }
            }
        } else if sr.this_row && sr.item.is_none() {
            s.push('@');
        }
    }
    s.push(']');
}

fn write_external(s: &mut String, book: &str, inner: &Expr, quoted: bool, opts: &PrintOptions) {
    match &inner.kind {
        ExprKind::Cell {
            sheet: Some(spec),
            cell,
        } => {
            write_ext_sheet(s, book, spec, quoted);
            s.push_str(&print_cell(*cell, opts));
        }
        ExprKind::Range {
            sheet: Some(spec),
            range,
        } => {
            write_ext_sheet(s, book, spec, quoted);
            s.push_str(&print_range(*range, opts));
        }
        ExprKind::Name {
            sheet: Some(spec),
            name,
        } => {
            write_ext_sheet(s, book, spec, quoted);
            s.push_str(name);
        }
        ExprKind::ThreeD { sheets, inner } => {
            write_ext_sheet(s, book, sheets, quoted);
            write_expr(s, inner, opts);
        }
        _ => {
            s.push('[');
            s.push_str(book);
            s.push(']');
            write_expr(s, inner, opts);
        }
    }
}

fn write_ext_sheet(s: &mut String, book: &str, spec: &SheetSpec, quoted: bool) {
    let combined = match &spec.end {
        Some(end) => format!("[{book}]{}:{}", spec.start, end),
        None => format!("[{book}]{}", spec.start),
    };
    let needs = quoted
        || quote_sheet_name(&spec.start).starts_with('\'')
        || spec
            .end
            .as_ref()
            .is_some_and(|e| quote_sheet_name(e).starts_with('\''));
    if needs {
        s.push('\'');
        s.push_str(&combined.replace('\'', "''"));
        s.push('\'');
        s.push('!');
    } else {
        s.push_str(&combined);
        s.push('!');
    }
}

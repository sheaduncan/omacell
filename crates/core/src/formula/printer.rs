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
    write_expr(&mut s, &formula.ast, &opts, &mut Vec::new());
    s
}

/// Print an expression (no leading `=`).
#[must_use]
pub fn print_expr(expr: &Expr, opts: PrintOptions) -> String {
    let mut s = String::new();
    write_expr(&mut s, expr, &opts, &mut Vec::new());
    s
}

type Bindings = Vec<(String, String)>;

fn write_expr(s: &mut String, expr: &Expr, opts: &PrintOptions, bindings: &mut Bindings) {
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
                    write_expr(s, c, opts, bindings);
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
            write_expr(s, inner, opts, bindings);
        }
        ExprKind::Name { sheet, name } => {
            write_sheet(s, sheet.as_ref());
            if sheet.is_none()
                && let Some(spelling) = binding_spelling(bindings, name)
            {
                s.push_str(spelling);
            } else {
                s.push_str(name);
            }
        }
        ExprKind::Structured(sr) => write_structured(s, sr),
        ExprKind::External {
            workbook,
            inner,
            quoted,
        } => {
            write_external(s, workbook, inner, *quoted, opts, bindings);
        }
        ExprKind::Prefix { op, expr } => {
            s.push_str(match op {
                PrefixOp::Plus => "+",
                PrefixOp::Minus => "-",
                PrefixOp::ImplicitIntersect => "@",
            });
            write_expr(s, expr, opts, bindings);
        }
        ExprKind::Postfix { expr, op } => {
            write_expr(s, expr, opts, bindings);
            s.push_str(match op {
                PostfixOp::Percent => "%",
                PostfixOp::Spill => "#",
            });
        }
        ExprKind::Binary { op, left, right } => {
            write_expr(s, left, opts, bindings);
            s.push_str(op.as_str());
            write_expr(s, right, opts, bindings);
        }
        ExprKind::Paren(inner) => {
            s.push('(');
            write_expr(s, inner, opts, bindings);
            s.push(')');
        }
        ExprKind::Call { callee, args } => {
            if let Callee::Name(name) = callee {
                if name.eq_ignore_ascii_case("LET")
                    && args.len() >= 3
                    && !args.len().is_multiple_of(2)
                {
                    write_let_call(s, args, opts, bindings);
                    return;
                }
                if name.eq_ignore_ascii_case("LAMBDA") && !args.is_empty() {
                    write_lambda_call(s, args, opts, bindings);
                    return;
                }
            }
            match callee {
                Callee::Name(n) => match binding_spelling(bindings, n) {
                    Some(spelling) => s.push_str(spelling),
                    None => s.push_str(&n.to_ascii_uppercase()),
                },
                Callee::Expr(e) => write_expr(s, e, opts, bindings),
            }
            write_args(s, args, opts, bindings);
        }
    }
}

fn binding_spelling<'a>(bindings: &'a Bindings, name: &str) -> Option<&'a str> {
    bindings
        .iter()
        .rev()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, spelling)| spelling.as_str())
}

fn binding_name(expr: Option<&Expr>) -> Option<&str> {
    match expr.map(|expr| &expr.kind) {
        Some(ExprKind::Name { sheet: None, name }) => Some(name),
        _ => None,
    }
}

fn push_binding(bindings: &mut Bindings, name: &str) {
    bindings.push((name.to_ascii_uppercase(), name.to_string()));
}

fn write_binding_expr(s: &mut String, expr: &Expr, opts: &PrintOptions, bindings: &mut Bindings) {
    if let ExprKind::Name { sheet: None, name } = &expr.kind {
        s.push_str(name);
    } else {
        write_expr(s, expr, opts, bindings);
    }
}

fn write_args(s: &mut String, args: &[Option<Expr>], opts: &PrintOptions, bindings: &mut Bindings) {
    s.push('(');
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        if let Some(expr) = arg {
            write_expr(s, expr, opts, bindings);
        }
    }
    s.push(')');
}

fn write_let_call(
    s: &mut String,
    args: &[Option<Expr>],
    opts: &PrintOptions,
    bindings: &mut Bindings,
) {
    s.push_str("LET(");
    let base = bindings.len();
    let last = args.len() - 1;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        if let Some(expr) = arg {
            if i < last && i.is_multiple_of(2) {
                write_binding_expr(s, expr, opts, bindings);
            } else {
                write_expr(s, expr, opts, bindings);
            }
        }
        if i < last
            && i % 2 == 1
            && let Some(name) = binding_name(args.get(i - 1).and_then(Option::as_ref))
        {
            push_binding(bindings, name);
        }
    }
    bindings.truncate(base);
    s.push(')');
}

fn write_lambda_call(
    s: &mut String,
    args: &[Option<Expr>],
    opts: &PrintOptions,
    bindings: &mut Bindings,
) {
    s.push_str("LAMBDA(");
    let base = bindings.len();
    let last = args.len() - 1;
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        if let Some(expr) = arg {
            if i < last {
                write_binding_expr(s, expr, opts, bindings);
                if let Some(name) = binding_name(Some(expr)) {
                    push_binding(bindings, name);
                }
            } else {
                write_expr(s, expr, opts, bindings);
            }
        }
    }
    bindings.truncate(base);
    s.push(')');
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

fn write_external(
    s: &mut String,
    book: &str,
    inner: &Expr,
    quoted: bool,
    opts: &PrintOptions,
    bindings: &mut Bindings,
) {
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
            write_expr(s, inner, opts, bindings);
        }
        _ => {
            s.push('[');
            s.push_str(book);
            s.push(']');
            write_expr(s, inner, opts, bindings);
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

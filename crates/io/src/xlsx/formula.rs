//! SpreadsheetML's reserved formula-name prefixes at the file boundary.

use omacell_core::formula::{Callee, Expr, ExprKind, Formula, PrefixOp, parse, print};

const XL_FUNCTION_PREFIX: &str = "_xlfn.";
const XL_WORKSHEET_PREFIX: &str = "_xlfn._xlws.";
const XL_PARAMETER_PREFIX: &str = "_xlpm.";

// Microsoft [MS-XLSX] 2.2.3, plus newer functions already present in the
// Omacell catalog and written with the same reserved prefix by current Excel.
const FUTURE_FUNCTIONS: &[&str] = &[
    "ACOT",
    "ACOTH",
    "AGGREGATE",
    "ARABIC",
    "ARRAYTOTEXT",
    "BASE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.DIST.RANGE",
    "BINOM.INV",
    "BITAND",
    "BITLSHIFT",
    "BITOR",
    "BITRSHIFT",
    "BITXOR",
    "BYCOL",
    "BYROW",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "COMBINA",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COT",
    "COTH",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "CSC",
    "CSCH",
    "DAYS",
    "DECIMAL",
    "DROP",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPAND",
    "EXPON.DIST",
    "F.DIST",
    "F.DIST.RT",
    "F.INV",
    "F.INV.RT",
    "F.TEST",
    "FIELDVALUE",
    "FILTERXML",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.ETS.CONFINT",
    "FORECAST.ETS.SEASONALITY",
    "FORECAST.ETS.STAT",
    "FORECAST.LINEAR",
    "FORMULATEXT",
    "GAMMA",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "GAUSS",
    "HSTACK",
    "HYPGEOM.DIST",
    "IFNA",
    "IFS",
    "IMCOSH",
    "IMCOT",
    "IMCSC",
    "IMCSCH",
    "IMSEC",
    "IMSECH",
    "IMSINH",
    "IMTAN",
    "ISFORMULA",
    "ISOMITTED",
    "ISOWEEKNUM",
    "LAMBDA",
    "LET",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAKEARRAY",
    "MAP",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "MUNIT",
    "NEGBINOM.DIST",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "NUMBERVALUE",
    "PDURATION",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "PERMUTATIONA",
    "PHI",
    "POISSON.DIST",
    "PQSOURCE",
    "PYTHON_STR",
    "PYTHON_TYPE",
    "PYTHON_TYPENAME",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "QUERYSTRING",
    "RANDARRAY",
    "RANK.AVG",
    "RANK.EQ",
    "REDUCE",
    "REGEXEXTRACT",
    "REGEXREPLACE",
    "REGEXTEST",
    "RRI",
    "SCAN",
    "SEC",
    "SECH",
    "SEQUENCE",
    "SHEET",
    "SHEETS",
    "SKEW.P",
    "SORTBY",
    "STDEV.P",
    "STDEV.S",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "UNICHAR",
    "UNICODE",
    "UNIQUE",
    "VALUETOTEXT",
    "VAR.P",
    "VAR.S",
    "VSTACK",
    "WEBSERVICE",
    "WEIBULL.DIST",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
    "XOR",
    "Z.TEST",
];

const WORKSHEET_ONLY_FUNCTIONS: &[&str] = &["FILTER", "PY", "SORT"];

pub(crate) fn from_xlsx(source: &str) -> String {
    let Ok(mut formula) = parse(source) else {
        return source.to_string();
    };
    let mut changed = false;
    formula.ast = formula.ast.map(&mut |mut expr| {
        let implicit_intersection = match &mut expr.kind {
            ExprKind::Call {
                callee: Callee::Name(name),
                args,
            } if name.eq_ignore_ascii_case("_xlfn.SINGLE") && args.len() == 1 => {
                args.first_mut().and_then(Option::take)
            }
            _ => None,
        };
        if let Some(inner) = implicit_intersection {
            expr.kind = ExprKind::Prefix {
                op: PrefixOp::ImplicitIntersect,
                expr: Box::new(inner),
            };
            changed = true;
            return expr;
        }
        match &mut expr.kind {
            ExprKind::Name {
                sheet: None, name, ..
            } => {
                if let Some(unprefixed) = strip_prefix(name, XL_PARAMETER_PREFIX) {
                    *name = unprefixed.to_string();
                    changed = true;
                }
            }
            ExprKind::Call {
                callee: Callee::Name(name),
                ..
            } => {
                if let Some(unprefixed) = unprefixed_function(name) {
                    *name = unprefixed.to_string();
                    changed = true;
                } else if let Some(unprefixed) = strip_prefix(name, XL_PARAMETER_PREFIX) {
                    *name = unprefixed.to_string();
                    changed = true;
                }
            }
            _ => {}
        }
        expr
    });
    if changed {
        print_like(source, &formula)
    } else {
        source.to_string()
    }
}

pub(crate) fn to_xlsx(source: &str) -> String {
    let normalized = from_xlsx(source);
    let Ok(mut formula) = parse(&normalized) else {
        return source.to_string();
    };
    let mut changed = normalized != source;
    add_xlsx_prefixes(&mut formula.ast, &mut Vec::new(), &mut changed);
    if changed {
        print_like(source, &formula)
    } else {
        source.to_string()
    }
}

fn add_xlsx_prefixes(expr: &mut Expr, bindings: &mut Vec<(String, String)>, changed: &mut bool) {
    if let ExprKind::Prefix {
        op: PrefixOp::ImplicitIntersect,
        expr: inner,
    } = &mut expr.kind
    {
        add_xlsx_prefixes(inner, bindings, changed);
        let inner = (**inner).clone();
        expr.kind = ExprKind::Call {
            callee: Callee::Name("_xlfn.SINGLE".into()),
            args: vec![Some(inner)],
        };
        *changed = true;
        return;
    }
    match &mut expr.kind {
        ExprKind::Array(rows) => {
            for cell in rows.iter_mut().flatten() {
                add_xlsx_prefixes(cell, bindings, changed);
            }
        }
        ExprKind::ThreeD { inner, .. }
        | ExprKind::External { inner, .. }
        | ExprKind::Prefix { expr: inner, .. }
        | ExprKind::Postfix { expr: inner, .. }
        | ExprKind::Paren(inner) => add_xlsx_prefixes(inner, bindings, changed),
        ExprKind::Binary { left, right, .. } => {
            add_xlsx_prefixes(left, bindings, changed);
            add_xlsx_prefixes(right, bindings, changed);
        }
        ExprKind::Name {
            sheet: None, name, ..
        } => {
            prefix_binding_use(name, bindings, changed);
        }
        ExprKind::Call { callee, args } => {
            let named = match callee {
                Callee::Name(name) => Some(name.as_str()),
                Callee::Expr(_) => None,
            };
            if named.is_some_and(|name| name.eq_ignore_ascii_case("LET"))
                && args.len() >= 3
                && !args.len().is_multiple_of(2)
            {
                prefix_let(args, bindings, changed);
            } else if named.is_some_and(|name| name.eq_ignore_ascii_case("LAMBDA"))
                && !args.is_empty()
            {
                prefix_lambda(args, bindings, changed);
            } else {
                if let Callee::Expr(callee) = callee {
                    add_xlsx_prefixes(callee, bindings, changed);
                }
                for arg in args.iter_mut().flatten() {
                    add_xlsx_prefixes(arg, bindings, changed);
                }
            }
            if let Callee::Name(name) = callee
                && !prefix_binding_use(name, bindings, changed)
                && let Some(prefixed) = prefixed_function(name)
            {
                *name = prefixed;
                *changed = true;
            }
        }
        ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Error(_)
        | ExprKind::Cell { .. }
        | ExprKind::Range { .. }
        | ExprKind::Name { .. }
        | ExprKind::Structured(_) => {}
    }
}

fn prefix_let(args: &mut [Option<Expr>], bindings: &mut Vec<(String, String)>, changed: &mut bool) {
    let base = bindings.len();
    let last = args.len() - 1;
    let mut pending = None;
    for (index, arg) in args.iter_mut().enumerate() {
        if index < last && index.is_multiple_of(2) {
            pending = arg.as_mut().and_then(|expr| prefix_binding(expr, changed));
        } else if let Some(expr) = arg {
            add_xlsx_prefixes(expr, bindings, changed);
        }
        if index < last
            && index % 2 == 1
            && let Some(binding) = pending.take()
        {
            bindings.push(binding);
        }
    }
    bindings.truncate(base);
}

fn prefix_lambda(
    args: &mut [Option<Expr>],
    bindings: &mut Vec<(String, String)>,
    changed: &mut bool,
) {
    let base = bindings.len();
    let last = args.len() - 1;
    for (index, arg) in args.iter_mut().enumerate() {
        if index < last {
            if let Some(binding) = arg.as_mut().and_then(|expr| prefix_binding(expr, changed)) {
                bindings.push(binding);
            }
        } else if let Some(expr) = arg {
            add_xlsx_prefixes(expr, bindings, changed);
        }
    }
    bindings.truncate(base);
}

fn prefix_binding(expr: &mut Expr, changed: &mut bool) -> Option<(String, String)> {
    let ExprKind::Name {
        sheet: None, name, ..
    } = &mut expr.kind
    else {
        return None;
    };
    let original = name.clone();
    let spelling = format!("{XL_PARAMETER_PREFIX}{original}");
    *name = spelling.clone();
    *changed = true;
    Some((original, spelling))
}

fn prefix_binding_use(
    name: &mut String,
    bindings: &[(String, String)],
    changed: &mut bool,
) -> bool {
    let Some((_, spelling)) = bindings
        .iter()
        .rev()
        .find(|(binding, _)| binding.eq_ignore_ascii_case(name))
    else {
        return false;
    };
    *name = spelling.clone();
    *changed = true;
    true
}

fn unprefixed_function(name: &str) -> Option<&str> {
    if let Some(unprefixed) = strip_prefix(name, XL_WORKSHEET_PREFIX)
        && contains_ascii_case(WORKSHEET_ONLY_FUNCTIONS, unprefixed)
    {
        return Some(unprefixed);
    }
    let unprefixed = strip_prefix(name, XL_FUNCTION_PREFIX)?;
    contains_ascii_case(FUTURE_FUNCTIONS, unprefixed).then_some(unprefixed)
}

fn prefixed_function(name: &str) -> Option<String> {
    if contains_ascii_case(WORKSHEET_ONLY_FUNCTIONS, name) {
        Some(format!("{XL_WORKSHEET_PREFIX}{name}"))
    } else if contains_ascii_case(FUTURE_FUNCTIONS, name) {
        Some(format!("{XL_FUNCTION_PREFIX}{name}"))
    } else {
        None
    }
}

fn contains_ascii_case(haystack: &[&str], needle: &str) -> bool {
    haystack
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(needle))
}

fn strip_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, tail) = value.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

fn print_like(source: &str, formula: &Formula) -> String {
    let printed = print(formula);
    if source.trim_start().starts_with('=') {
        printed
    } else {
        printed.strip_prefix('=').unwrap_or(&printed).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_function_round_trips_through_its_xlsx_spelling() {
        for name in FUTURE_FUNCTIONS {
            let canonical = format!("={name}(1)");
            let encoded = to_xlsx(&canonical);
            assert!(
                encoded.starts_with("=_XLFN."),
                "{name} encoded as {encoded}"
            );
            assert_eq!(from_xlsx(&encoded), canonical);
        }
        for name in WORKSHEET_ONLY_FUNCTIONS {
            let canonical = format!("={name}(1)");
            let encoded = to_xlsx(&canonical);
            assert!(
                encoded.starts_with("=_XLFN._XLWS."),
                "{name} encoded as {encoded}"
            );
            assert_eq!(from_xlsx(&encoded), canonical);
        }
    }

    #[test]
    fn lexical_binding_wins_over_a_same_named_future_function() {
        let canonical = "=LAMBDA(MAP,LET(x,MAP(1),LAMBDA(y,x+y)(2)))";
        let encoded = to_xlsx(canonical);
        assert_eq!(
            encoded,
            "=_XLFN.LAMBDA(_xlpm.MAP,_XLFN.LET(_xlpm.x,_XLPM.MAP(1),_XLFN.LAMBDA(_xlpm.y,_xlpm.x+_xlpm.y)(2)))"
        );
        assert_eq!(from_xlsx(&encoded), canonical);
    }

    #[test]
    fn unrelated_names_and_strings_are_not_rewritten() {
        for source in [
            "=_xlfn.NOT_A_FUNCTION(1)",
            "=\"_xlfn.XLOOKUP(\"&A1",
            "=Sheet1!MAP+MAP",
        ] {
            assert_eq!(from_xlsx(source), source);
            assert_eq!(to_xlsx(source), source);
        }
    }
}

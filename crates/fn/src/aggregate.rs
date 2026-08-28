//! Criteria aggregation, `AGGREGATE`, and `SUBTOTAL` (WP-05a).

use omacell_core::addr::SheetId;
use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, RuntimeValue};

use crate::common::{
    Criteria, arg_number, count_args, counta_args, criteria_match, flatten, is_nested_aggregate,
    median, parse_criteria, percentile_exc, percentile_inc, register_specs, rt_num, sorted, stdev,
    variance,
};
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Aggregate specs.
pub const SPECS: &[FunctionSpec] = &[
    AGGREGATE, AVERAGEIF, AVERAGEIFS, COUNTIF, COUNTIFS, MAXIFS, MINIFS, SUBTOTAL, SUMIF, SUMIFS,
];

/// Register criteria-aggregation functions.
pub fn register_aggregate(registry: &mut omacell_core::eval::FnRegistry) {
    register_specs(registry, SPECS);
}

macro_rules! afn {
    ($id:ident, $name:expr, $cat:expr, $args:expr, $min:expr, $max:expr, $sig:expr, $doc:expr, $body:expr) => {
        crate::define_fn! {
        const $id = {
            name: $name,
            aliases: &[],
            tier: 0,
            category: $cat,
            arg_kinds: $args,
            min_args: $min,
            max_args: $max,
            volatile: false,
            array: ArrayBehavior::None,
            async_node: false,
            signature: $sig,
            doc: $doc,
            body: $body,
        };
        }
    };
}

afn!(
    SUMIF,
    "SUMIF",
    "math",
    &[ArgKind::Range, ArgKind::Any, ArgKind::Range],
    2,
    3,
    "SUMIF(range, criteria, [sum_range])",
    "Sums cells that meet a criterion. Wildcards * ? ~ and comparison prefixes.",
    FnBody::Eager(sumif_impl)
);
afn!(
    SUMIFS,
    "SUMIFS",
    "math",
    &[ArgKind::Range, ArgKind::Range, ArgKind::Any],
    3,
    255,
    "SUMIFS(sum_range, criteria_range1, criteria1, ...)",
    "Sums cells that meet all criteria.",
    FnBody::Eager(sumifs_impl)
);
afn!(
    COUNTIF,
    "COUNTIF",
    "statistical",
    &[ArgKind::Range, ArgKind::Any],
    2,
    2,
    "COUNTIF(range, criteria)",
    "Counts cells that meet a criterion.",
    FnBody::Eager(countif_impl)
);
afn!(
    COUNTIFS,
    "COUNTIFS",
    "statistical",
    &[ArgKind::Range, ArgKind::Any],
    2,
    255,
    "COUNTIFS(criteria_range1, criteria1, ...)",
    "Counts rows that meet all criteria.",
    FnBody::Eager(countifs_impl)
);
afn!(
    AVERAGEIF,
    "AVERAGEIF",
    "statistical",
    &[ArgKind::Range, ArgKind::Any, ArgKind::Range],
    2,
    3,
    "AVERAGEIF(range, criteria, [average_range])",
    "Average of cells that meet a criterion.",
    FnBody::Eager(averageif_impl)
);
afn!(
    AVERAGEIFS,
    "AVERAGEIFS",
    "statistical",
    &[ArgKind::Range, ArgKind::Range, ArgKind::Any],
    3,
    255,
    "AVERAGEIFS(average_range, criteria_range1, criteria1, ...)",
    "Average of cells that meet all criteria.",
    FnBody::Eager(averageifs_impl)
);
afn!(
    MAXIFS,
    "MAXIFS",
    "statistical",
    &[ArgKind::Range, ArgKind::Range, ArgKind::Any],
    3,
    255,
    "MAXIFS(max_range, criteria_range1, criteria1, ...)",
    "Maximum of cells that meet all criteria.",
    FnBody::Eager(maxifs_impl)
);
afn!(
    MINIFS,
    "MINIFS",
    "statistical",
    &[ArgKind::Range, ArgKind::Range, ArgKind::Any],
    3,
    255,
    "MINIFS(min_range, criteria_range1, criteria1, ...)",
    "Minimum of cells that meet all criteria.",
    FnBody::Eager(minifs_impl)
);
afn!(
    SUBTOTAL,
    "SUBTOTAL",
    "math",
    &[ArgKind::Number, ArgKind::Range],
    2,
    255,
    "SUBTOTAL(function_num, ref1, [ref2], ...)",
    "Aggregate that ignores nested SUBTOTAL/AGGREGATE. 101–111 also skip hidden rows.",
    FnBody::Eager(subtotal_impl)
);
afn!(
    AGGREGATE,
    "AGGREGATE",
    "math",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Any],
    3,
    255,
    "AGGREGATE(function_num, options, ref1, [ref2], ...)",
    "19 functions with options to ignore hidden rows, errors, and nested SUBTOTAL/AGGREGATE.",
    FnBody::Eager(aggregate_impl)
);

fn crit_of(ctx: &EvalCtx<'_>, arg: &ArgVal) -> Result<Criteria, ErrorKind> {
    let s = match ctx.materialize(arg.value.clone()) {
        RuntimeValue::Scalar(s) => s,
        _ => return Err(ErrorKind::Value),
    };
    parse_criteria(&s)
}

fn if_fold(
    ctx: &EvalCtx<'_>,
    range: &ArgVal,
    criteria: &ArgVal,
    values: &ArgVal,
    mut acc: impl FnMut(f64),
) -> Result<u32, ErrorKind> {
    let crit = crit_of(ctx, criteria)?;
    let tests = flatten(ctx, &range.value)?;
    let nums = flatten(ctx, &values.value)?;
    if tests.len() != nums.len() {
        return Err(ErrorKind::Value);
    }
    let mut n = 0u32;
    for (t, v) in tests.iter().zip(nums.iter()) {
        if !criteria_match(t, &crit) {
            continue;
        }
        match v {
            Scalar::Error(e) => return Err(*e),
            Scalar::Number(x) if x.is_finite() => {
                acc(*x);
                n += 1;
            }
            Scalar::Number(_) => return Err(ErrorKind::Num),
            _ => {}
        }
    }
    Ok(n)
}

fn sumif_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(range), Some(criteria)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let values = args.get(2).unwrap_or(range);
    let mut sum = 0.0;
    match if_fold(ctx, range, criteria, values, |x| sum += x) {
        Ok(_) => rt_num(sum),
        Err(e) => RuntimeValue::error(e),
    }
}

fn countif_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(range), Some(criteria)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let crit = match crit_of(ctx, criteria) {
        Ok(c) => c,
        Err(e) => return RuntimeValue::error(e),
    };
    let tests = match flatten(ctx, &range.value) {
        Ok(v) => v,
        Err(e) => return RuntimeValue::error(e),
    };
    rt_num(tests.iter().filter(|t| criteria_match(t, &crit)).count() as f64)
}

fn averageif_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(range), Some(criteria)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let values = args.get(2).unwrap_or(range);
    let mut sum = 0.0;
    match if_fold(ctx, range, criteria, values, |x| sum += x) {
        Ok(0) => RuntimeValue::error(ErrorKind::Div0),
        Ok(n) => rt_num(sum / f64::from(n)),
        Err(e) => RuntimeValue::error(e),
    }
}

struct IfsPair {
    tests: Vec<Scalar>,
    crit: Criteria,
}

fn parse_ifs(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<(Vec<Scalar>, Vec<IfsPair>), ErrorKind> {
    if args.len() < 3 || (args.len() - 1) % 2 != 0 {
        return Err(ErrorKind::Value);
    }
    let first = args.first().ok_or(ErrorKind::Value)?;
    let values = flatten(ctx, &first.value)?;
    let mut pairs = Vec::new();
    for chunk in args.get(1..).unwrap_or(&[]).chunks_exact(2) {
        let tests = flatten(ctx, &chunk[0].value)?;
        if tests.len() != values.len() {
            return Err(ErrorKind::Value);
        }
        pairs.push(IfsPair {
            tests,
            crit: crit_of(ctx, &chunk[1])?,
        });
    }
    Ok((values, pairs))
}

fn countifs_pairs(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<(usize, Vec<IfsPair>), ErrorKind> {
    if args.len() < 2 || args.len() % 2 != 0 {
        return Err(ErrorKind::Value);
    }
    let mut pairs = Vec::new();
    let mut len = None;
    for chunk in args.chunks_exact(2) {
        let tests = flatten(ctx, &chunk[0].value)?;
        match len {
            None => len = Some(tests.len()),
            Some(l) if l != tests.len() => return Err(ErrorKind::Value),
            _ => {}
        }
        pairs.push(IfsPair {
            tests,
            crit: crit_of(ctx, &chunk[1])?,
        });
    }
    Ok((len.unwrap_or(0), pairs))
}

fn row_ok(i: usize, pairs: &[IfsPair]) -> bool {
    pairs.iter().all(|p| criteria_match(&p.tests[i], &p.crit))
}

fn sumifs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match parse_ifs(ctx, args) {
        Err(e) => RuntimeValue::error(e),
        Ok((values, pairs)) => {
            let mut sum = 0.0;
            for (i, value) in values.iter().enumerate() {
                if !row_ok(i, &pairs) {
                    continue;
                }
                match value {
                    Scalar::Error(e) => return RuntimeValue::error(*e),
                    Scalar::Number(n) if n.is_finite() => sum += n,
                    Scalar::Number(_) => return RuntimeValue::error(ErrorKind::Num),
                    _ => {}
                }
            }
            rt_num(sum)
        }
    }
}

fn countifs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match countifs_pairs(ctx, args) {
        Err(e) => RuntimeValue::error(e),
        Ok((len, pairs)) => rt_num((0..len).filter(|&i| row_ok(i, &pairs)).count() as f64),
    }
}

fn averageifs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match parse_ifs(ctx, args) {
        Err(e) => RuntimeValue::error(e),
        Ok((values, pairs)) => {
            let mut sum = 0.0;
            let mut n = 0.0;
            for (i, value) in values.iter().enumerate() {
                if !row_ok(i, &pairs) {
                    continue;
                }
                match value {
                    Scalar::Error(e) => return RuntimeValue::error(*e),
                    Scalar::Number(x) if x.is_finite() => {
                        sum += x;
                        n += 1.0;
                    }
                    Scalar::Number(_) => return RuntimeValue::error(ErrorKind::Num),
                    _ => {}
                }
            }
            if n == 0.0 {
                RuntimeValue::error(ErrorKind::Div0)
            } else {
                rt_num(sum / n)
            }
        }
    }
}

fn maxifs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    minmax_ifs(ctx, args, true)
}
fn minifs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    minmax_ifs(ctx, args, false)
}

fn minmax_ifs(ctx: &EvalCtx<'_>, args: &[ArgVal], max: bool) -> RuntimeValue {
    match parse_ifs(ctx, args) {
        Err(e) => RuntimeValue::error(e),
        Ok((values, pairs)) => {
            let mut best: Option<f64> = None;
            for (i, value) in values.iter().enumerate() {
                if !row_ok(i, &pairs) {
                    continue;
                }
                match value {
                    Scalar::Error(e) => return RuntimeValue::error(*e),
                    Scalar::Number(x) if x.is_finite() => {
                        best = Some(match best {
                            None => *x,
                            Some(b) if max => b.max(*x),
                            Some(b) => b.min(*x),
                        });
                    }
                    Scalar::Number(_) => return RuntimeValue::error(ErrorKind::Num),
                    _ => {}
                }
            }
            match best {
                Some(n) => rt_num(n),
                None => rt_num(0.0),
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AggOpts {
    ignore_hidden: bool,
    ignore_errors: bool,
    ignore_nested: bool,
}

fn collect_filtered(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    opts: AggOpts,
) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for arg in args {
        if arg.omitted {
            continue;
        }
        match &arg.value {
            RuntimeValue::Ref(r) => {
                let mut err = None;
                ctx.for_each_stored_cell(r, &mut |sheet, row, col, s| {
                    if err.is_some() {
                        return;
                    }
                    if let Err(e) = consider_cell(ctx, sheet, row, col, s, opts, &mut out) {
                        err = Some(e);
                    }
                });
                if let Some(e) = err {
                    return Err(e);
                }
            }
            other => {
                for s in flatten(ctx, other)? {
                    push_filtered(s, opts.ignore_errors, &mut out)?;
                }
            }
        }
    }
    Ok(out)
}

fn consider_cell(
    ctx: &EvalCtx<'_>,
    sheet: SheetId,
    row: u32,
    col: u16,
    s: Scalar,
    opts: AggOpts,
    out: &mut Vec<f64>,
) -> Result<(), ErrorKind> {
    if opts.ignore_hidden && ctx.is_row_hidden(sheet, row) {
        return Ok(());
    }
    if opts.ignore_nested
        && let Some(src) = ctx.formula_source(sheet, row, col)
        && is_nested_aggregate(src)
    {
        return Ok(());
    }
    push_filtered(s, opts.ignore_errors, out)
}

fn push_filtered(s: Scalar, ignore_errors: bool, out: &mut Vec<f64>) -> Result<(), ErrorKind> {
    match s {
        Scalar::Error(_) if ignore_errors => Ok(()),
        Scalar::Error(e) => Err(e),
        Scalar::Number(n) if n.is_finite() => {
            out.push(n);
            Ok(())
        }
        Scalar::Number(_) => Err(ErrorKind::Num),
        _ => Ok(()),
    }
}

fn apply_fn_num(num: i32, values: &[f64], k: Option<f64>) -> Result<f64, ErrorKind> {
    match num {
        1 => mean(values),
        2 | 3 => Ok(values.len() as f64),
        4 => values
            .iter()
            .copied()
            .reduce(f64::max)
            .ok_or(ErrorKind::Num),
        5 => values
            .iter()
            .copied()
            .reduce(f64::min)
            .ok_or(ErrorKind::Num),
        6 => Ok(if values.is_empty() {
            0.0
        } else {
            values.iter().fold(1.0, |a, b| a * b)
        }),
        7 => stdev(values, true),
        8 => stdev(values, false),
        9 => Ok(values.iter().sum()),
        10 => variance(values, true),
        11 => variance(values, false),
        12 => median(values),
        13 => mode_sngl(values),
        14 => large_small(values, k.ok_or(ErrorKind::Value)?, true),
        15 => large_small(values, k.ok_or(ErrorKind::Value)?, false),
        16 => percentile_inc(&sorted(values.to_vec()), k.ok_or(ErrorKind::Value)?),
        17 => quartile_inc(values, k.ok_or(ErrorKind::Value)?),
        18 => percentile_exc(&sorted(values.to_vec()), k.ok_or(ErrorKind::Value)?),
        19 => quartile_exc(values, k.ok_or(ErrorKind::Value)?),
        _ => Err(ErrorKind::Value),
    }
}

fn mean(values: &[f64]) -> Result<f64, ErrorKind> {
    if values.is_empty() {
        Err(ErrorKind::Div0)
    } else {
        Ok(values.iter().sum::<f64>() / values.len() as f64)
    }
}

fn mode_sngl(values: &[f64]) -> Result<f64, ErrorKind> {
    if values.is_empty() {
        return Err(ErrorKind::Na);
    }
    let mut best_n = 0usize;
    let mut best = values[0];
    for (i, v) in values.iter().enumerate() {
        let c = values.iter().filter(|x| *x == v).count();
        if c > best_n {
            best_n = c;
            best = *v;
        } else if c == best_n && c > 1 {
            let first_best = values.iter().position(|x| *x == best).unwrap_or(0);
            if i < first_best {
                best = *v;
            }
        }
    }
    if best_n < 2 {
        Err(ErrorKind::Na)
    } else {
        Ok(best)
    }
}

fn large_small(values: &[f64], k: f64, large: bool) -> Result<f64, ErrorKind> {
    if values.is_empty() {
        return Err(ErrorKind::Num);
    }
    let k = k.trunc() as i64;
    if k < 1 || k as usize > values.len() {
        return Err(ErrorKind::Num);
    }
    let mut s = sorted(values.to_vec());
    if large {
        s.reverse();
    }
    Ok(s[k as usize - 1])
}

fn quartile_inc(values: &[f64], q: f64) -> Result<f64, ErrorKind> {
    let k = match q.trunc() as i32 {
        0 => 0.0,
        1 => 0.25,
        2 => 0.5,
        3 => 0.75,
        4 => 1.0,
        _ => return Err(ErrorKind::Num),
    };
    percentile_inc(&sorted(values.to_vec()), k)
}

fn quartile_exc(values: &[f64], q: f64) -> Result<f64, ErrorKind> {
    match q.trunc() as i32 {
        1 => percentile_exc(&sorted(values.to_vec()), 0.25),
        2 => percentile_exc(&sorted(values.to_vec()), 0.5),
        3 => percentile_exc(&sorted(values.to_vec()), 0.75),
        _ => Err(ErrorKind::Num),
    }
}

fn subtotal_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let num = match arg_number(ctx, args, 0) {
        Ok(n) => n.trunc() as i32,
        Err(e) => return RuntimeValue::error(e),
    };
    let (fn_num, ignore_hidden) = if (1..=11).contains(&num) {
        (num, false)
    } else if (101..=111).contains(&num) {
        (num - 100, true)
    } else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let opts = AggOpts {
        ignore_hidden,
        ignore_errors: false,
        ignore_nested: true,
    };
    if fn_num == 2 || fn_num == 3 {
        return subtotal_count(ctx, args.get(1..).unwrap_or(&[]), opts, fn_num == 3);
    }
    match collect_filtered(ctx, args.get(1..).unwrap_or(&[]), opts)
        .and_then(|v| apply_fn_num(fn_num, &v, None))
    {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}

fn subtotal_count(ctx: &EvalCtx<'_>, args: &[ArgVal], opts: AggOpts, counta: bool) -> RuntimeValue {
    let mut n = 0.0;
    for arg in args {
        if arg.omitted {
            continue;
        }
        match &arg.value {
            RuntimeValue::Ref(r) => {
                ctx.for_each_stored_cell(r, &mut |sheet, row, col, s| {
                    if opts.ignore_hidden && ctx.is_row_hidden(sheet, row) {
                        return;
                    }
                    if opts.ignore_nested
                        && let Some(src) = ctx.formula_source(sheet, row, col)
                        && is_nested_aggregate(src)
                    {
                        return;
                    }
                    if counta {
                        if !matches!(s, Scalar::Empty) {
                            n += 1.0;
                        }
                    } else if matches!(s, Scalar::Number(v) if v.is_finite()) {
                        n += 1.0;
                    }
                });
            }
            _ => {
                let fake = [arg.clone()];
                let r = if counta {
                    counta_args(ctx, &fake)
                } else {
                    count_args(ctx, &fake)
                };
                match r {
                    Ok(c) => n += c,
                    Err(e) => return RuntimeValue::error(e),
                }
            }
        }
    }
    rt_num(n)
}

fn aggregate_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let fn_num = match arg_number(ctx, args, 0) {
        Ok(n) => n.trunc() as i32,
        Err(e) => return RuntimeValue::error(e),
    };
    let options = match arg_number(ctx, args, 1) {
        Ok(n) => n.trunc() as i32,
        Err(e) => return RuntimeValue::error(e),
    };
    if !(1..=19).contains(&fn_num) || !(0..=7).contains(&options) {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let opts = AggOpts {
        ignore_nested: matches!(options, 0 | 2 | 4 | 6),
        ignore_hidden: matches!(options, 4..=7),
        ignore_errors: matches!(options, 2 | 3 | 6 | 7),
    };
    let rest = args.get(2..).unwrap_or(&[]);
    let k = if matches!(fn_num, 14..=19) {
        rest.get(1).and_then(|a| {
            if a.omitted {
                None
            } else {
                arg_number(ctx, rest, 1).ok()
            }
        })
    } else {
        None
    };
    let data_args = if matches!(fn_num, 14..=19) {
        let Some(first) = rest.first() else {
            return RuntimeValue::error(ErrorKind::Value);
        };
        std::slice::from_ref(first)
    } else {
        rest
    };
    match collect_filtered(ctx, data_args, opts).and_then(|v| apply_fn_num(fn_num, &v, k)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}

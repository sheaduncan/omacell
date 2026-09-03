//! Logical functions (WP-05a). `IF`/`IFS`/`SWITCH`/`IFERROR`/`IFNA` are lazy.

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, RuntimeArray, RuntimeValue, eval_expr};
use omacell_core::formula::Expr;

use crate::common::{Origin, for_each_value, register_specs, rt_bool};
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Logical specs.
pub const SPECS: &[FunctionSpec] = &[
    AND, FALSE, IF, IFERROR, IFNA, IFS, NOT, OR, SWITCH, TRUE, XOR,
];

/// Register logical functions.
pub fn register_logical(registry: &mut omacell_core::eval::FnRegistry) {
    register_specs(registry, SPECS);
}

macro_rules! lfn {
    ($id:ident, $name:expr, $args:expr, $min:expr, $max:expr, $arr:expr, $sig:expr, $doc:expr, $body:expr) => {
        crate::define_fn! {
        const $id = {
            name: $name,
            aliases: &[],
            tier: 0,
            category: "logical",
            arg_kinds: $args,
            min_args: $min,
            max_args: $max,
            volatile: false,
            array: $arr,
            async_node: false,
            signature: $sig,
            doc: $doc,
            body: $body,
        };
        }
    };
}

lfn!(
    IF,
    "IF",
    &[ArgKind::Logical, ArgKind::Any, ArgKind::Any],
    2,
    3,
    ArrayBehavior::None,
    "IF(logical_test, value_if_true, [value_if_false])",
    "Returns one value if the test is TRUE and another if FALSE. Unselected branches are not evaluated.",
    FnBody::Lazy(if_impl)
);
lfn!(
    IFS,
    "IFS",
    &[ArgKind::Logical, ArgKind::Any],
    2,
    255,
    ArrayBehavior::None,
    "IFS(logical1, value1, [logical2, value2], ...)",
    "Returns the value for the first true test. Later pairs are not evaluated.",
    FnBody::Lazy(ifs_impl)
);
lfn!(
    SWITCH,
    "SWITCH",
    &[ArgKind::Any],
    3,
    255,
    ArrayBehavior::None,
    "SWITCH(expression, value1, result1, [value2, result2], ..., [default])",
    "Matches expression against values. Unselected results are not evaluated.",
    FnBody::Lazy(switch_impl)
);
lfn!(
    IFERROR,
    "IFERROR",
    &[ArgKind::Any, ArgKind::Any],
    2,
    2,
    ArrayBehavior::None,
    "IFERROR(value, value_if_error)",
    "Returns value unless it is an error; the fallback is not evaluated otherwise.",
    FnBody::Lazy(iferror_impl)
);
lfn!(
    IFNA,
    "IFNA",
    &[ArgKind::Any, ArgKind::Any],
    2,
    2,
    ArrayBehavior::None,
    "IFNA(value, value_if_na)",
    "Returns value unless it is #N/A; the fallback is not evaluated otherwise.",
    FnBody::Lazy(ifna_impl)
);
lfn!(
    AND,
    "AND",
    &[ArgKind::Logical],
    1,
    255,
    ArrayBehavior::None,
    "AND(logical1, [logical2], ...)",
    "TRUE if all arguments are TRUE. Does not short-circuit.",
    FnBody::Eager(and_impl)
);
lfn!(
    OR,
    "OR",
    &[ArgKind::Logical],
    1,
    255,
    ArrayBehavior::None,
    "OR(logical1, [logical2], ...)",
    "TRUE if any argument is TRUE. Does not short-circuit.",
    FnBody::Eager(or_impl)
);
lfn!(
    XOR,
    "XOR",
    &[ArgKind::Logical],
    1,
    255,
    ArrayBehavior::None,
    "XOR(logical1, [logical2], ...)",
    "TRUE if an odd number of arguments are TRUE.",
    FnBody::Eager(xor_impl)
);
lfn!(
    NOT,
    "NOT",
    &[ArgKind::Logical],
    1,
    1,
    ArrayBehavior::LiftAll,
    "NOT(logical)",
    "Inverts a logical value.",
    FnBody::Eager(not_impl)
);
lfn!(
    TRUE,
    "TRUE",
    &[],
    0,
    0,
    ArrayBehavior::None,
    "TRUE()",
    "The logical value TRUE.",
    FnBody::Eager(true_impl)
);
lfn!(
    FALSE,
    "FALSE",
    &[],
    0,
    0,
    ArrayBehavior::None,
    "FALSE()",
    "The logical value FALSE.",
    FnBody::Eager(false_impl)
);

fn eval_one(ctx: &mut EvalCtx<'_>, expr: &Option<Expr>) -> RuntimeValue {
    match expr {
        Some(e) => eval_expr(ctx, e),
        None => RuntimeValue::Scalar(Scalar::Empty),
    }
}

fn as_bool_scalar(ctx: &mut EvalCtx<'_>, v: RuntimeValue) -> Result<bool, ErrorKind> {
    match ctx.materialize(v) {
        RuntimeValue::Scalar(s) => coerce::to_bool(&s),
        _ => Err(ErrorKind::Value),
    }
}

fn if_impl(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    let Some(Some(test_expr)) = args.first() else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let test = eval_expr(ctx, test_expr);
    let test = ctx.materialize(test);
    let RuntimeValue::Scalar(test) = test else {
        return array_if(ctx, args, test);
    };
    let cond = match coerce::to_bool(&test) {
        Ok(value) => value,
        Err(error) => return RuntimeValue::error(error),
    };
    let branch = if cond { 1 } else { 2 };
    match args.get(branch) {
        Some(Some(expr)) => eval_expr(ctx, expr),
        Some(None) | None if !cond => RuntimeValue::Scalar(Scalar::Bool(false)),
        _ => RuntimeValue::Scalar(Scalar::Empty),
    }
}

fn array_if(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>], test: RuntimeValue) -> RuntimeValue {
    let RuntimeValue::Array(test_array) = &test else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    if let Err(error) = test_array.validate() {
        return RuntimeValue::error(error);
    }

    let mut needs_true = false;
    let mut needs_false = false;
    for condition in test_array.values.iter() {
        match coerce::to_bool(condition) {
            Ok(true) => needs_true = true,
            Ok(false) => needs_false = true,
            Err(_) => {}
        }
    }

    let true_value = needs_true.then(|| eval_array_if_branch(ctx, args, 1, true));
    let false_value = needs_false.then(|| eval_array_if_branch(ctx, args, 2, false));
    let mut rows = test_array.rows;
    let mut cols = test_array.cols;
    for value in [&true_value, &false_value].into_iter().flatten() {
        let (value_rows, value_cols) = match value_shape(value) {
            Ok(shape) => shape,
            Err(error) => return RuntimeValue::error(error),
        };
        rows = rows.max(value_rows);
        cols = cols.max(value_cols);
    }
    let Ok(len) = RuntimeArray::checked_len(rows, cols) else {
        return RuntimeValue::error(ErrorKind::Num);
    };

    let mut values = Vec::with_capacity(len);
    for row in 0..rows {
        for col in 0..cols {
            let condition = value_at(&test, row, col);
            values.push(match coerce::to_bool(&condition) {
                Ok(true) => selected_value_at(true_value.as_ref(), row, col),
                Ok(false) => selected_value_at(false_value.as_ref(), row, col),
                Err(error) => Scalar::Error(error),
            });
        }
    }
    RuntimeValue::array(rows, cols, values)
}

fn eval_array_if_branch(
    ctx: &mut EvalCtx<'_>,
    args: &[Option<Expr>],
    index: usize,
    true_branch: bool,
) -> RuntimeValue {
    match args.get(index) {
        Some(Some(expr)) => {
            let value = eval_expr(ctx, expr);
            ctx.materialize(value)
        }
        Some(None) | None if !true_branch => RuntimeValue::Scalar(Scalar::Bool(false)),
        _ => RuntimeValue::Scalar(Scalar::Empty),
    }
}

fn value_shape(value: &RuntimeValue) -> Result<(u32, u32), ErrorKind> {
    match value {
        RuntimeValue::Array(array) => {
            array.validate()?;
            Ok((array.rows, array.cols))
        }
        RuntimeValue::Scalar(_) | RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Ok((1, 1)),
    }
}

fn selected_value_at(value: Option<&RuntimeValue>, row: u32, col: u32) -> Scalar {
    value
        .map(|value| value_at(value, row, col))
        .unwrap_or(Scalar::Error(ErrorKind::Value))
}

fn value_at(value: &RuntimeValue, row: u32, col: u32) -> Scalar {
    match value {
        RuntimeValue::Scalar(scalar) => scalar.clone(),
        RuntimeValue::Array(array) => {
            let row = if array.rows == 1 {
                0
            } else if row < array.rows {
                row
            } else {
                return Scalar::Error(ErrorKind::Na);
            };
            let col = if array.cols == 1 {
                0
            } else if col < array.cols {
                col
            } else {
                return Scalar::Error(ErrorKind::Na);
            };
            let index = (row as usize)
                .saturating_mul(array.cols as usize)
                .saturating_add(col as usize);
            array
                .values
                .get(index)
                .cloned()
                .unwrap_or(Scalar::Error(ErrorKind::Value))
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Scalar::Error(ErrorKind::Value),
    }
}

fn ifs_impl(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    if args.len() < 2 || !args.len().is_multiple_of(2) {
        return RuntimeValue::error(ErrorKind::Value);
    }
    for pair in args.as_chunks::<2>().0 {
        let test = eval_one(ctx, &pair[0]);
        match as_bool_scalar(ctx, test) {
            Ok(true) => return eval_one(ctx, &pair[1]),
            Ok(false) => {}
            Err(e) => return RuntimeValue::error(e),
        }
    }
    RuntimeValue::error(ErrorKind::Na)
}

fn switch_impl(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    if args.len() < 3 {
        return RuntimeValue::error(ErrorKind::Value);
    }
    let expr = eval_one(ctx, &args[0]);
    let expr = ctx.materialize(expr);
    let RuntimeValue::Scalar(target) = expr else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    if let Some(e) = target.error() {
        return RuntimeValue::error(e);
    }
    let rest = &args[1..];
    let (pairs, default) = if rest.len() % 2 == 1 {
        let (p, d) = rest.split_at(rest.len() - 1);
        (p, d.first())
    } else {
        (rest, None)
    };
    for chunk in pairs.as_chunks::<2>().0 {
        let cand = eval_one(ctx, &chunk[0]);
        let cand = ctx.materialize(cand);
        let RuntimeValue::Scalar(s) = cand else {
            return RuntimeValue::error(ErrorKind::Value);
        };
        if let Some(e) = s.error() {
            return RuntimeValue::error(e);
        }
        match coerce::compare(&target, &s) {
            Ok(omacell_core::coerce::Cmp::Eq) => return eval_one(ctx, &chunk[1]),
            Ok(_) => {}
            Err(e) => return RuntimeValue::error(e),
        }
    }
    match default {
        Some(d) => eval_one(ctx, d),
        None => RuntimeValue::error(ErrorKind::Na),
    }
}

fn iferror_impl(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    let value = eval_one(ctx, args.first().unwrap_or(&None));
    if value.error_kind().is_some() {
        eval_one(ctx, args.get(1).unwrap_or(&None))
    } else {
        value
    }
}

fn ifna_impl(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    let value = eval_one(ctx, args.first().unwrap_or(&None));
    if value.error_kind() == Some(ErrorKind::Na) {
        eval_one(ctx, args.get(1).unwrap_or(&None))
    } else {
        value
    }
}

fn fold_logicals(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    mut combine: impl FnMut(bool, bool) -> bool,
    init: Option<bool>,
) -> Result<bool, ErrorKind> {
    let mut acc = init;
    let mut seen = false;
    for_each_value(ctx, args, &mut |s, origin| {
        if matches!(s, Scalar::Empty)
            || matches!((&s, origin), (Scalar::Text(_), Origin::Aggregate))
        {
            return Ok(());
        }
        let b = coerce::to_bool(&s)?;
        acc = Some(match acc {
            None => b,
            Some(a) => combine(a, b),
        });
        seen = true;
        Ok(())
    })?;
    if !seen {
        Err(ErrorKind::Value)
    } else {
        Ok(acc.unwrap_or(false))
    }
}

fn and_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match fold_logicals(ctx, args, |a, b| a && b, Some(true)) {
        Ok(b) => rt_bool(b),
        Err(e) => RuntimeValue::error(e),
    }
}
fn or_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match fold_logicals(ctx, args, |a, b| a || b, Some(false)) {
        Ok(b) => rt_bool(b),
        Err(e) => RuntimeValue::error(e),
    }
}
fn xor_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match fold_logicals(ctx, args, |a, b| a ^ b, Some(false)) {
        Ok(b) => rt_bool(b),
        Err(e) => RuntimeValue::error(e),
    }
}
fn not_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match crate::common::arg_scalar(ctx, args, 0).and_then(|s| coerce::to_bool(&s)) {
        Ok(b) => rt_bool(!b),
        Err(e) => RuntimeValue::error(e),
    }
}
fn true_impl(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    rt_bool(true)
}
fn false_impl(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    rt_bool(false)
}

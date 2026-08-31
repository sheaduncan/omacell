//! Logical functions (WP-05a). `IF`/`IFS`/`SWITCH`/`IFERROR`/`IFNA` are lazy.

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, RuntimeValue, eval_expr};
use omacell_core::formula::Expr;

use crate::common::{for_each_value, register_specs, rt_bool};
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
    let cond = match as_bool_scalar(ctx, test) {
        Ok(b) => b,
        Err(e) => return RuntimeValue::error(e),
    };
    let branch = if cond { 1 } else { 2 };
    match args.get(branch) {
        Some(Some(expr)) => eval_expr(ctx, expr),
        Some(None) | None if !cond => RuntimeValue::Scalar(Scalar::Bool(false)),
        _ => RuntimeValue::Scalar(Scalar::Empty),
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
    for_each_value(ctx, args, &mut |s, _origin| {
        if matches!(s, Scalar::Empty) {
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

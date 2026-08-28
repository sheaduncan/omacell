//! Representative probe functions. WP-05a/b/c replace these registrations.

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};
use omacell_core::formula::Expr;

use crate::metadata::{ArgKind, ArrayBehavior, FnStrategy, FunctionSpec};

/// Probe specs in declaration order (JSON output is re-sorted by name).
pub const PROBE_SPECS: &[FunctionSpec] = &[ABS, SUM, IF, NOW, RAND, SEQUENCE];

/// Register probe functions (and aliases) onto `registry`.
pub fn register_probes(registry: &mut FnRegistry) {
    for spec in PROBE_SPECS {
        registry.register(spec.to_fn_def());
        for alias in spec.aliases {
            let mut def = spec.to_fn_def();
            def.name = alias;
            registry.register(def);
        }
    }
}

const ABS: FunctionSpec = FunctionSpec {
    name: "ABS",
    aliases: &[],
    tier: 0,
    category: "math",
    arg_kinds: &[ArgKind::Number],
    min_args: 1,
    max_args: 1,
    strategy: FnStrategy::Eager,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "ABS(number)",
    doc: "Absolute value of a number.",
    body: FnBody::Eager(abs_impl),
};

const SUM: FunctionSpec = FunctionSpec {
    name: "SUM",
    aliases: &[],
    tier: 0,
    category: "math",
    arg_kinds: &[ArgKind::Any],
    min_args: 1,
    max_args: 255,
    strategy: FnStrategy::Eager,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "SUM(number1, [number2], ...)",
    doc: "Adds all numbers in the arguments, walking ranges.",
    body: FnBody::Eager(sum_impl),
};

const IF: FunctionSpec = FunctionSpec {
    name: "IF",
    aliases: &[],
    tier: 0,
    category: "logical",
    arg_kinds: &[ArgKind::Logical, ArgKind::Any, ArgKind::Any],
    min_args: 2,
    max_args: 3,
    strategy: FnStrategy::Lazy,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "IF(logical_test, value_if_true, [value_if_false])",
    doc: "Returns one value if the test is TRUE and another if FALSE. Unselected branches are not evaluated.",
    body: FnBody::Lazy(if_impl),
};

const NOW: FunctionSpec = FunctionSpec {
    name: "NOW",
    aliases: &[],
    tier: 0,
    category: "date",
    arg_kinds: &[],
    min_args: 0,
    max_args: 0,
    strategy: FnStrategy::Eager,
    volatile: true,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "NOW()",
    doc: "Current date and time as a serial, sampled once per recalc pass.",
    body: FnBody::Eager(now_impl),
};

const RAND: FunctionSpec = FunctionSpec {
    name: "RAND",
    aliases: &[],
    tier: 0,
    category: "math",
    arg_kinds: &[],
    min_args: 0,
    max_args: 0,
    strategy: FnStrategy::Eager,
    volatile: true,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "RAND()",
    doc: "Uniform random in [0, 1), derived from the pass nonce and cell.",
    body: FnBody::Eager(rand_impl),
};

const SEQUENCE: FunctionSpec = FunctionSpec {
    name: "SEQUENCE",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    strategy: FnStrategy::Eager,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "SEQUENCE(rows, [columns])",
    doc: "Returns a sequence array. Invalid or out-of-grid shapes are `#NUM!`.",
    body: FnBody::Eager(sequence_impl),
};

fn abs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let value = args
        .first()
        .map(|arg| ctx.materialize(arg.value.clone()))
        .unwrap_or(RuntimeValue::error(ErrorKind::Value));
    match value {
        RuntimeValue::Scalar(scalar) => match coerce::to_number(&scalar) {
            Ok(n) => RuntimeValue::Scalar(Scalar::Number(n.abs())),
            Err(e) => RuntimeValue::error(e),
        },
        _ => RuntimeValue::error(ErrorKind::Value),
    }
}

fn sum_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let mut acc = 0.0;
    let mut err = None;
    let mut add = |s: Scalar| {
        if err.is_some() {
            return;
        }
        if let Some(e) = s.error() {
            err = Some(e);
            return;
        }
        if matches!(s, Scalar::Text(_) | Scalar::Empty) {
            return;
        }
        match coerce::to_number(&s) {
            Ok(n) => acc += n,
            Err(e) => err = Some(e),
        }
    };
    for a in args {
        if a.omitted {
            continue;
        }
        match &a.value {
            RuntimeValue::Ref(r) => ctx.for_each_cell(r, &mut add),
            RuntimeValue::Scalar(s) => add(s.clone()),
            RuntimeValue::Array(ar) => {
                for s in ar.values.iter() {
                    add(s.clone());
                }
            }
            RuntimeValue::Lambda(_) => return RuntimeValue::error(ErrorKind::Value),
        }
    }
    match err {
        Some(e) => RuntimeValue::error(e),
        None => RuntimeValue::Scalar(Scalar::Number(acc)),
    }
}

fn if_impl(ctx: &mut EvalCtx<'_>, args: &[Option<Expr>]) -> RuntimeValue {
    let Some(Some(test_expr)) = args.first() else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let test = omacell_core::eval::eval_expr(ctx, test_expr);
    let test = ctx.materialize(test);
    let RuntimeValue::Scalar(scalar) = test else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let cond = match coerce::to_bool(&scalar) {
        Ok(b) => b,
        Err(e) => return RuntimeValue::error(e),
    };
    let branch = if cond { 1 } else { 2 };
    match args.get(branch) {
        Some(Some(expr)) => omacell_core::eval::eval_expr(ctx, expr),
        _ => RuntimeValue::Scalar(Scalar::Bool(false)),
    }
}

fn now_impl(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Number(ctx.clock()))
}

fn rand_impl(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Number(ctx.random_unit("RAND", 0)))
}

fn sequence_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let rows = match args.first() {
        Some(a) if !a.omitted => match ctx.materialize(a.value.clone()) {
            RuntimeValue::Scalar(s) => match coerce::to_number(&s) {
                Ok(n) if n.is_finite() => n.round() as i64,
                Ok(_) => return RuntimeValue::error(ErrorKind::Num),
                Err(e) => return RuntimeValue::error(e),
            },
            _ => return RuntimeValue::error(ErrorKind::Value),
        },
        _ => return RuntimeValue::error(ErrorKind::Value),
    };
    let cols = match args.get(1) {
        Some(a) if !a.omitted => match ctx.materialize(a.value.clone()) {
            RuntimeValue::Scalar(s) => match coerce::to_number(&s) {
                Ok(n) if n.is_finite() => n.round() as i64,
                Ok(_) => return RuntimeValue::error(ErrorKind::Num),
                Err(e) => return RuntimeValue::error(e),
            },
            _ => return RuntimeValue::error(ErrorKind::Value),
        },
        _ => 1,
    };
    if rows < 1 || cols < 1 {
        return RuntimeValue::error(ErrorKind::Num);
    }
    let rows_u = u32::try_from(rows).unwrap_or(u32::MAX);
    let cols_u = u32::try_from(cols).unwrap_or(u32::MAX);
    let Some(len) = rows_u.checked_mul(cols_u) else {
        return RuntimeValue::error(ErrorKind::Num);
    };
    if rows_u > omacell_core::limits::MAX_ROWS || cols_u > u32::from(omacell_core::limits::MAX_COLS)
    {
        return RuntimeValue::error(ErrorKind::Num);
    }
    let mut values = Vec::with_capacity(len as usize);
    for i in 0..len {
        values.push(Scalar::Number(f64::from(i + 1)));
    }
    RuntimeValue::array(rows_u, cols_u, values)
}

//! Lambda helpers (`MAP`, `REDUCE`, `SCAN`, `BYROW`, `BYCOL`, `MAKEARRAY`)
//! plus catalog-only `LET` / `LAMBDA` / `ISOMITTED` (evaluator-owned).

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};
use omacell_core::formula::Expr;

use crate::args;
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Helper + catalog-only language-construct specs.
pub const SPECS: &[FunctionSpec] = &[
    MAP, REDUCE, SCAN, BYROW, BYCOL, MAKEARRAY, LET, LAMBDA, ISOMITTED,
];

/// Register lambda helpers. Skips evaluator-owned `LET`/`LAMBDA`/`ISOMITTED`.
pub fn register_lambda(registry: &mut FnRegistry) {
    for spec in SPECS {
        if omacell_core::lambda::is_language_fn(spec.name) {
            continue;
        }
        args::register_spec(registry, spec);
    }
}

crate::define_fn! {
const MAP = {
    name: "MAP",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Array, ArgKind::Any],
    min_args: 2,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "MAP(array1, [array2], ..., lambda)",
    doc: "Applies a LAMBDA to each coordinate of one or more arrays, padding missing coordinates with `#N/A`.",
    body: FnBody::Eager(map_impl),
};
}

crate::define_fn! {
const REDUCE = {
    name: "REDUCE",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Any],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "REDUCE(initial_value, array, lambda)",
    doc: "Folds an array with a LAMBDA(accumulator, value).",
    body: FnBody::Eager(reduce_impl),
};
}

crate::define_fn! {
const SCAN = {
    name: "SCAN",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Any, ArgKind::Array, ArgKind::Any],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "SCAN(initial_value, array, lambda)",
    doc: "Returns intermediate REDUCE accumulators, one per array element.",
    body: FnBody::Eager(scan_impl),
};
}

crate::define_fn! {
const BYROW = {
    name: "BYROW",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Array, ArgKind::Any],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "BYROW(array, lambda)",
    doc: "Applies a scalar-returning LAMBDA to each row of an array.",
    body: FnBody::Eager(byrow_impl),
};
}

crate::define_fn! {
const BYCOL = {
    name: "BYCOL",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Array, ArgKind::Any],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "BYCOL(array, lambda)",
    doc: "Applies a scalar-returning LAMBDA to each column of an array.",
    body: FnBody::Eager(bycol_impl),
};
}

crate::define_fn! {
const MAKEARRAY = {
    name: "MAKEARRAY",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Any],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "MAKEARRAY(rows, cols, lambda)",
    doc: "Builds an array by calling LAMBDA(row, col) for each cell. Invalid shapes are `#NUM!` before allocation.",
    body: FnBody::Eager(makearray_impl),
};
}

fn language_stub(_ctx: &mut EvalCtx<'_>, _args: &[Option<Expr>]) -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Name)
}

crate::define_fn! {
const LET = {
    name: "LET",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Any],
    min_args: 3,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "LET(name1, value1, ..., calc)",
    doc: "Names calculations inside a formula. Evaluator language construct; not registered.",
    body: FnBody::Lazy(language_stub),
};
}

crate::define_fn! {
const LAMBDA = {
    name: "LAMBDA",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Any],
    min_args: 1,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "LAMBDA(parameter, ..., calc)",
    doc: "Creates a reusable function value. Evaluator language construct; not registered.",
    body: FnBody::Lazy(language_stub),
};
}

crate::define_fn! {
const ISOMITTED = {
    name: "ISOMITTED",
    aliases: &[],
    tier: 0,
    category: "lambda",
    arg_kinds: &[ArgKind::Any],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "ISOMITTED(argument)",
    doc: "TRUE when a LAMBDA optional argument was omitted. Evaluator language construct; not registered.",
    body: FnBody::Lazy(language_stub),
};
}

fn err(e: ErrorKind) -> RuntimeValue {
    RuntimeValue::error(e)
}

fn scalarize(v: RuntimeValue) -> Result<Scalar, ErrorKind> {
    match v {
        RuntimeValue::Scalar(s) => Ok(s),
        RuntimeValue::Array(a) => {
            a.validate()?;
            if a.rows == 1 && a.cols == 1 {
                Ok(a.values.first().cloned().unwrap_or(Scalar::Empty))
            } else {
                Err(ErrorKind::Value)
            }
        }
        RuntimeValue::Lambda(_) => Err(ErrorKind::Value),
        RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

fn map_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    if args.len() < 2 {
        return err(ErrorKind::Value);
    }
    let Some((lam_arg, arrays_args)) = args.split_last() else {
        return err(ErrorKind::Value);
    };
    let lam = match args::lambda_of(lam_arg) {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    let mut arrays = Vec::new();
    for a in arrays_args {
        if a.omitted {
            continue;
        }
        if let Some(e) = a.value.error_kind() {
            return err(e);
        }
        match args::arg_array(ctx, a) {
            Ok(arr) => arrays.push(arr),
            Err(e) => return err(e),
        }
    }
    if arrays.is_empty() {
        return err(ErrorKind::Value);
    }
    let rows = arrays.iter().map(|a| a.rows).max().unwrap_or(1);
    let cols = arrays.iter().map(|a| a.cols).max().unwrap_or(1);
    let Ok(len) = args::check_shape(rows, cols) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for r in 0..rows {
        for c in 0..cols {
            let mut argv = Vec::with_capacity(arrays.len());
            for a in &arrays {
                let value = if r < a.rows && c < a.cols {
                    args::at(a, r, c)
                } else {
                    Scalar::Error(ErrorKind::Na)
                };
                argv.push(RuntimeValue::Scalar(value));
            }
            let out = args::apply_lambda(ctx, &lam, argv);
            match scalarize(out) {
                Ok(s) => values.push(s),
                Err(e) => values.push(Scalar::Error(e)),
            }
        }
    }
    args::array_result(rows, cols, values)
}

fn reduce_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(init_arg) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = init_arg.value.error_kind() {
        return err(e);
    }
    let array = match args.get(1).ok_or(ErrorKind::Value).and_then(|a| {
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        args::arg_array(ctx, a)
    }) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let lam = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(args::lambda_of)
    {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    let mut acc = init_arg.value.clone();
    for s in array.values.iter() {
        acc = args::apply_lambda(ctx, &lam, vec![acc, RuntimeValue::Scalar(s.clone())]);
        if let Some(e) = acc.error_kind() {
            return err(e);
        }
    }
    acc
}

fn scan_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(init_arg) = args.first() else {
        return err(ErrorKind::Value);
    };
    if let Some(e) = init_arg.value.error_kind() {
        return err(e);
    }
    let array = match args.get(1).ok_or(ErrorKind::Value).and_then(|a| {
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        args::arg_array(ctx, a)
    }) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let lam = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(args::lambda_of)
    {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    let Ok(len) = args::check_shape(array.rows, array.cols) else {
        return err(ErrorKind::Num);
    };
    let mut acc = init_arg.value.clone();
    let mut values = Vec::with_capacity(len);
    for s in array.values.iter() {
        acc = args::apply_lambda(ctx, &lam, vec![acc, RuntimeValue::Scalar(s.clone())]);
        match scalarize(acc.clone()) {
            Ok(s) => values.push(s),
            Err(e) => values.push(Scalar::Error(e)),
        }
        if let Some(e) = acc.error_kind() {
            // Keep filling with the same error so the origin shows it.
            while values.len() < len {
                values.push(Scalar::Error(e));
            }
            break;
        }
    }
    args::array_result(array.rows, array.cols, values)
}

fn byrow_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    by_axis(ctx, args, true)
}

fn bycol_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    by_axis(ctx, args, false)
}

fn by_axis(ctx: &mut EvalCtx<'_>, args: &[ArgVal], by_row: bool) -> RuntimeValue {
    let array = match args.first().ok_or(ErrorKind::Value).and_then(|a| {
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        args::arg_array(ctx, a)
    }) {
        Ok(a) => a,
        Err(e) => return err(e),
    };
    let lam = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(args::lambda_of)
    {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    let n = if by_row { array.rows } else { array.cols };
    let mut values = Vec::with_capacity(n as usize);
    for i in 0..n {
        let slice = if by_row {
            let mut v = Vec::with_capacity(array.cols as usize);
            for c in 0..array.cols {
                v.push(args::at(&array, i, c));
            }
            match RuntimeValue::try_array(1, array.cols, v) {
                Ok(x) => x,
                Err(e) => return err(e),
            }
        } else {
            let mut v = Vec::with_capacity(array.rows as usize);
            for r in 0..array.rows {
                v.push(args::at(&array, r, i));
            }
            match RuntimeValue::try_array(array.rows, 1, v) {
                Ok(x) => x,
                Err(e) => return err(e),
            }
        };
        let out = args::apply_lambda(ctx, &lam, vec![slice]);
        match out {
            RuntimeValue::Scalar(s) => values.push(s),
            RuntimeValue::Array(a) => {
                if let Err(e) = a.validate() {
                    return err(e);
                }
                if a.rows != 1 || a.cols != 1 {
                    return err(ErrorKind::Calc);
                }
                let Some(value) = a.values.first().cloned() else {
                    return err(ErrorKind::Calc);
                };
                values.push(value);
            }
            RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => return err(ErrorKind::Value),
        }
    }
    if by_row {
        args::array_result(n, 1, values)
    } else {
        args::array_result(1, n, values)
    }
}

fn makearray_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let rows_n = match args.first() {
        Some(a) => match args::number(ctx, a) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let cols_n = match args.get(1) {
        Some(a) => match args::number(ctx, a) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => return err(ErrorKind::Value),
    };
    let rows = match args::pos_u32(rows_n) {
        Ok(n) => n,
        Err(_) => return err(ErrorKind::Num),
    };
    let cols = match args::pos_u32(cols_n) {
        Ok(n) => n,
        Err(_) => return err(ErrorKind::Num),
    };
    let lam = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(args::lambda_of)
    {
        Ok(l) => l,
        Err(e) => return err(e),
    };
    let Ok(len) = args::check_shape(rows, cols) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for r in 0..rows {
        for c in 0..cols {
            let out = args::apply_lambda(
                ctx,
                &lam,
                vec![
                    RuntimeValue::Scalar(Scalar::Number(f64::from(r + 1))),
                    RuntimeValue::Scalar(Scalar::Number(f64::from(c + 1))),
                ],
            );
            match scalarize(out) {
                Ok(s) => values.push(s),
                Err(e) => values.push(Scalar::Error(e)),
            }
        }
    }
    args::array_result(rows, cols, values)
}

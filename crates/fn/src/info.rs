//! Information functions (WP-05a). `ISOMITTED` is catalog-only.

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, RuntimeValue};

use crate::common::{
    abs_a1, arg_scalar, as_reference, ref_origin, register_specs, rt_bool, rt_num, rt_text,
};
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Information specs that are registered (excludes `ISOMITTED`).
pub const SPECS: &[FunctionSpec] = &[
    CELL, ERROR_TYPE, ISBLANK, ISERR, ISERROR, ISEVEN, ISFORMULA, ISLOGICAL, ISNA, ISNONTEXT,
    ISNUMBER, ISODD, ISREF, ISTEXT, N, NA, TYPE,
];

/// Register information functions. Does not register `ISOMITTED`.
pub fn register_info(registry: &mut omacell_core::eval::FnRegistry) {
    register_specs(registry, SPECS);
}

crate::define_fn! {
const ISOMITTED = {
    name: "ISOMITTED",
    aliases: &[],
    tier: 0,
    category: "logical",
    arg_kinds: &[ArgKind::Any],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "ISOMITTED(argument)",
    doc: "TRUE when a LAMBDA argument was omitted. Language construct (WP-04); catalog metadata only.",
    body: FnBody::Lazy(isomitted_stub),
};
}

/// Catalog spec for the WP-04 `ISOMITTED` language construct.
pub const ISOMITTED_SPEC: FunctionSpec = ISOMITTED;

fn isomitted_stub(
    _ctx: &mut EvalCtx<'_>,
    _args: &[Option<omacell_core::formula::Expr>],
) -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Name)
}

macro_rules! ifn {
    ($id:ident, $name:expr, $args:expr, $min:expr, $max:expr, $arr:expr, $vol:expr, $sig:expr, $doc:expr, $body:expr) => {
        crate::define_fn! {
        const $id = {
            name: $name,
            aliases: &[],
            tier: 0,
            category: "information",
            arg_kinds: $args,
            min_args: $min,
            max_args: $max,
            volatile: $vol,
            array: $arr,
            async_node: false,
            signature: $sig,
            doc: $doc,
            body: $body,
        };
        }
    };
}

ifn!(
    NA,
    "NA",
    &[],
    0,
    0,
    ArrayBehavior::None,
    false,
    "NA()",
    "The `#N/A` error value.",
    FnBody::Eager(na_impl)
);
ifn!(
    N,
    "N",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "N(value)",
    "Converts a value to a number (text → 0).",
    FnBody::Eager(n_impl)
);
ifn!(
    TYPE,
    "TYPE",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::None,
    false,
    "TYPE(value)",
    "1 number, 2 text, 4 logical, 16 error, 64 array.",
    FnBody::Eager(type_impl)
);
ifn!(
    ERROR_TYPE,
    "ERROR.TYPE",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ERROR.TYPE(error_val)",
    "Excel ERROR.TYPE code, or `#N/A` when the value is not an error with a code.",
    FnBody::Eager(error_type_impl)
);
ifn!(
    CELL,
    "CELL",
    &[ArgKind::Text, ArgKind::Range],
    1,
    2,
    ArrayBehavior::None,
    true,
    "CELL(info_type, [reference])",
    "Subset: address, col, row, contents, type, format. Omitted reference is the formula cell.",
    FnBody::Eager(cell_impl)
);
ifn!(
    ISBLANK,
    "ISBLANK",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISBLANK(value)",
    "TRUE if the value is empty.",
    FnBody::Eager(isblank_impl)
);
ifn!(
    ISERR,
    "ISERR",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISERR(value)",
    "TRUE if the value is any error except `#N/A`.",
    FnBody::Eager(iserr_impl)
);
ifn!(
    ISERROR,
    "ISERROR",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISERROR(value)",
    "TRUE if the value is any error.",
    FnBody::Eager(iserror_impl)
);
ifn!(
    ISNA,
    "ISNA",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISNA(value)",
    "TRUE if the value is `#N/A`.",
    FnBody::Eager(isna_impl)
);
ifn!(
    ISLOGICAL,
    "ISLOGICAL",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISLOGICAL(value)",
    "TRUE if the value is a logical.",
    FnBody::Eager(islogical_impl)
);
ifn!(
    ISNONTEXT,
    "ISNONTEXT",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISNONTEXT(value)",
    "TRUE if the value is not text.",
    FnBody::Eager(isnontext_impl)
);
ifn!(
    ISNUMBER,
    "ISNUMBER",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISNUMBER(value)",
    "TRUE if the value is a number.",
    FnBody::Eager(isnumber_impl)
);
ifn!(
    ISTEXT,
    "ISTEXT",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISTEXT(value)",
    "TRUE if the value is text.",
    FnBody::Eager(istext_impl)
);
ifn!(
    ISEVEN,
    "ISEVEN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISEVEN(number)",
    "TRUE if the truncated number is even.",
    FnBody::Eager(iseven_impl)
);
ifn!(
    ISODD,
    "ISODD",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISODD(number)",
    "TRUE if the truncated number is odd.",
    FnBody::Eager(isodd_impl)
);
ifn!(
    ISREF,
    "ISREF",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::None,
    false,
    "ISREF(value)",
    "TRUE if the argument is a reference (not materialized).",
    FnBody::Eager(isref_impl)
);
ifn!(
    ISFORMULA,
    "ISFORMULA",
    &[ArgKind::Range],
    1,
    1,
    ArrayBehavior::None,
    false,
    "ISFORMULA(reference)",
    "TRUE if the referenced cell contains a formula.",
    FnBody::Eager(isformula_impl)
);

fn na_impl(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::error(ErrorKind::Na)
}

fn n_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match arg_scalar(ctx, args, 0) {
        Ok(Scalar::Error(e)) => RuntimeValue::error(e),
        Ok(Scalar::Number(n)) => rt_num(n),
        Ok(Scalar::Bool(b)) => rt_num(if b { 1.0 } else { 0.0 }),
        Ok(Scalar::Empty | Scalar::Text(_)) => rt_num(0.0),
        Err(e) => RuntimeValue::error(e),
    }
}

fn type_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(arg) = args.first() else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    match &arg.value {
        RuntimeValue::Array(_) => rt_num(64.0),
        RuntimeValue::Lambda(_) => rt_num(16.0),
        RuntimeValue::Ref(_) => type_of(&ctx.materialize(arg.value.clone())),
        RuntimeValue::Scalar(_) => type_of(&arg.value),
    }
}

fn type_of(v: &RuntimeValue) -> RuntimeValue {
    match v {
        RuntimeValue::Array(_) => rt_num(64.0),
        RuntimeValue::Lambda(_) => rt_num(16.0),
        RuntimeValue::Ref(_) => rt_num(0.0),
        RuntimeValue::Scalar(s) => match s {
            Scalar::Number(_) | Scalar::Empty => rt_num(1.0),
            Scalar::Text(_) => rt_num(2.0),
            Scalar::Bool(_) => rt_num(4.0),
            Scalar::Error(_) => rt_num(16.0),
        },
    }
}

fn error_type_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match arg_scalar(ctx, args, 0) {
        Ok(Scalar::Error(e)) => match e.error_type() {
            Some(c) => rt_num(f64::from(c)),
            None => RuntimeValue::error(ErrorKind::Na),
        },
        Ok(_) => RuntimeValue::error(ErrorKind::Na),
        Err(e) => match e.error_type() {
            Some(c) => rt_num(f64::from(c)),
            None => RuntimeValue::error(ErrorKind::Na),
        },
    }
}

fn isblank_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(matches!(arg_scalar(ctx, args, 0), Ok(Scalar::Empty)))
}
fn iserr_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(match arg_scalar(ctx, args, 0) {
        Ok(Scalar::Error(e)) => e != ErrorKind::Na,
        Err(e) => e != ErrorKind::Na,
        _ => false,
    })
}
fn iserror_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(matches!(
        arg_scalar(ctx, args, 0),
        Ok(Scalar::Error(_)) | Err(_)
    ))
}
fn isna_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(matches!(
        arg_scalar(ctx, args, 0),
        Ok(Scalar::Error(ErrorKind::Na)) | Err(ErrorKind::Na)
    ))
}
fn islogical_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(matches!(arg_scalar(ctx, args, 0), Ok(Scalar::Bool(_))))
}
fn isnumber_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(matches!(arg_scalar(ctx, args, 0), Ok(Scalar::Number(_))))
}
fn istext_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(matches!(arg_scalar(ctx, args, 0), Ok(Scalar::Text(_))))
}
fn isnontext_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(!matches!(arg_scalar(ctx, args, 0), Ok(Scalar::Text(_))))
}
fn iseven_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    parity(ctx, args, true)
}
fn isodd_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    parity(ctx, args, false)
}
fn parity(ctx: &EvalCtx<'_>, args: &[ArgVal], even: bool) -> RuntimeValue {
    match crate::common::arg_number(ctx, args, 0) {
        Ok(n) => {
            let t = n.trunc() as i64;
            rt_bool(if even { t % 2 == 0 } else { t % 2 != 0 })
        }
        Err(e) => RuntimeValue::error(e),
    }
}
fn isref_impl(_ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rt_bool(
        args.first()
            .is_some_and(|a| as_reference(&a.value).is_some()),
    )
}
fn isformula_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let Some(arg) = args.first() else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let Some(r) = as_reference(&arg.value) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let Some((sheet, row, col)) = ref_origin(r) else {
        return rt_bool(false);
    };
    rt_bool(ctx.formula_source(sheet, row, col).is_some())
}

fn cell_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let info = match arg_scalar(ctx, args, 0).and_then(|s| coerce::to_text(&s)) {
        Ok(t) => t.to_ascii_lowercase(),
        Err(e) => return RuntimeValue::error(e),
    };
    let (sheet, row, col) = match args.get(1) {
        Some(a) if !a.omitted => match as_reference(&a.value).and_then(ref_origin) {
            Some(t) => t,
            None => return RuntimeValue::error(ErrorKind::Value),
        },
        _ => (ctx.cell.sheet, ctx.cell.row, ctx.cell.col),
    };
    match info.as_str() {
        "address" => {
            let addr = abs_a1(row, col);
            if let Some(name) = ctx.sheet_name(sheet)
                && sheet != ctx.cell.sheet
            {
                return rt_text(format!("'{name}'!{addr}"));
            }
            rt_text(addr)
        }
        "col" => rt_num(f64::from(col) + 1.0),
        "row" => rt_num(f64::from(row) + 1.0),
        "contents" => match ctx.read_cell(sheet, row, col) {
            Scalar::Error(e) => RuntimeValue::error(e),
            other => RuntimeValue::Scalar(other),
        },
        "type" => {
            let s = ctx.read_cell(sheet, row, col);
            rt_text(match s {
                Scalar::Empty => "b",
                Scalar::Text(_) => "l",
                _ => "v",
            })
        }
        "format" => rt_text(cell_format_code(ctx.cell_num_fmt(sheet, row, col).index())),
        _ => RuntimeValue::error(ErrorKind::Value),
    }
}

fn cell_format_code(id: u32) -> &'static str {
    match id {
        0 => "G",
        1 => "F0",
        2 => "F2",
        3 => ",0",
        4 => ",2",
        9 => "P0",
        10 => "P2",
        11 => "S2",
        14 => "D4",
        15 => "D1",
        16 => "D2",
        17 | 22 => "D3",
        18 => "D6",
        19 => "D7",
        20 => "D8",
        21 => "D9",
        _ => "G",
    }
}

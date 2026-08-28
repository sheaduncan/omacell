//! Remaining probe functions owned by WP-05b (`NOW`) and WP-05c (`SEQUENCE`).

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};

use crate::common::register_specs;
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Probe specs still shipped until WP-05b/05c replace them.
pub const PROBE_SPECS: &[FunctionSpec] = &[NOW, SEQUENCE];

/// Register remaining probe functions (and aliases) onto `registry`.
pub fn register_probes(registry: &mut FnRegistry) {
    register_specs(registry, PROBE_SPECS);
}

crate::define_fn! {
const NOW = {
    name: "NOW",
    aliases: &[],
    tier: 0,
    category: "date",
    arg_kinds: &[],
    min_args: 0,
    max_args: 0,
    volatile: true,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "NOW()",
    doc: "Current date and time as a serial, sampled once per recalc pass.",
    body: FnBody::Eager(now_impl),
};
}

crate::define_fn! {
const SEQUENCE = {
    name: "SEQUENCE",
    aliases: &[],
    tier: 0,
    category: "array",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::ReturnsArray,
    async_node: false,
    signature: "SEQUENCE(rows, [columns])",
    doc: "Returns a sequence array. Invalid or out-of-grid shapes are `#NUM!`.",
    body: FnBody::Eager(sequence_impl),
};
}

fn now_impl(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Number(ctx.clock()))
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
    let Ok(len) = omacell_core::eval::RuntimeArray::checked_len(rows_u, cols_u) else {
        return RuntimeValue::error(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for i in 0..len {
        values.push(Scalar::Number((i + 1) as f64));
    }
    RuntimeValue::array(rows_u, cols_u, values)
}

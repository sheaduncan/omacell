//! Shared argument helpers for text and date functions.

use std::sync::Arc;

use omacell_core::coerce::{self, Scalar};
use omacell_core::dates::DateSystem;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnRegistry, RuntimeValue};

use crate::FunctionSpec;

/// Excel cell-text cap (result of `REPT` / `CONCAT` / regex-replace).
pub(crate) const MAX_EXCEL_TEXT: usize = 32_767;

/// Register `specs` and their aliases.
pub(crate) fn register_specs(registry: &mut FnRegistry, specs: &[FunctionSpec]) {
    for spec in specs {
        registry.register(spec.to_fn_def());
        for alias in spec.aliases {
            let mut def = spec.to_fn_def();
            def.name = alias;
            registry.register(def);
        }
    }
}

pub(crate) fn date_system(ctx: &EvalCtx<'_>) -> DateSystem {
    ctx.workbook().settings().date_system
}

pub(crate) fn number(n: f64) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Number(n))
}

pub(crate) fn text(s: impl Into<Arc<str>>) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Text(s.into()))
}

pub(crate) fn boolean(b: bool) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Bool(b))
}

pub(crate) fn err(e: ErrorKind) -> RuntimeValue {
    RuntimeValue::error(e)
}

pub(crate) fn scalar(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<Scalar, ErrorKind> {
    match ctx.materialize(arg.value.clone()) {
        RuntimeValue::Scalar(s) => match s.error() {
            Some(e) => Err(e),
            None => Ok(s),
        },
        RuntimeValue::Array(_) | RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => {
            Err(ErrorKind::Value)
        }
    }
}

pub(crate) fn to_text(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<Arc<str>, ErrorKind> {
    coerce::to_text(&scalar(ctx, arg)?)
}

pub(crate) fn to_number(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<f64, ErrorKind> {
    coerce::to_number(&scalar(ctx, arg)?)
}

pub(crate) fn to_bool(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<bool, ErrorKind> {
    coerce::to_bool(&scalar(ctx, arg)?)
}

pub(crate) fn optional(args: &[ArgVal], index: usize) -> Option<&ArgVal> {
    args.get(index).filter(|arg| !arg.omitted)
}

/// Truncate toward zero. Non-finite → `#NUM!`.
pub(crate) fn trunc_i64(n: f64) -> Result<i64, ErrorKind> {
    if !n.is_finite() {
        return Err(ErrorKind::Num);
    }
    let t = n.trunc();
    if t > i64::MAX as f64 || t < i64::MIN as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(t as i64)
}

pub(crate) fn trunc_u32_len(n: f64) -> Result<i64, ErrorKind> {
    let v = trunc_i64(n)?;
    if v < 0 { Err(ErrorKind::Value) } else { Ok(v) }
}

pub(crate) fn too_long(len: usize) -> Result<(), ErrorKind> {
    if len > MAX_EXCEL_TEXT {
        Err(ErrorKind::Value)
    } else {
        Ok(())
    }
}

/// One-to-one uppercase (Excel: `ß` stays `ß`).
pub(crate) fn excel_upper_char(c: char) -> char {
    let mut up = c.to_uppercase();
    match (up.next(), up.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

/// One-to-one lowercase.
pub(crate) fn excel_lower_char(c: char) -> char {
    let mut low = c.to_lowercase();
    match (low.next(), low.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

pub(crate) fn excel_upper(s: &str) -> String {
    s.chars().map(excel_upper_char).collect()
}

pub(crate) fn excel_lower(s: &str) -> String {
    s.chars().map(excel_lower_char).collect()
}

pub(crate) fn chars_of(s: &str) -> Vec<char> {
    s.chars().collect()
}

pub(crate) fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub(crate) fn skip_take_chars(s: &str, skip: usize, take: usize) -> String {
    s.chars().skip(skip).take(take).collect()
}

pub(crate) fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    let skip = count.saturating_sub(n);
    s.chars().skip(skip).collect()
}

/// Walk every scalar in an argument (range, array, or scalar).
pub(crate) fn walk_arg(
    ctx: &mut EvalCtx<'_>,
    arg: &ArgVal,
    visit: &mut dyn FnMut(Scalar) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    match &arg.value {
        RuntimeValue::Ref(reference) => {
            let mut first_err = None;
            ctx.for_each_cell(reference, &mut |s| {
                if first_err.is_some() {
                    return;
                }
                if let Err(e) = visit(s) {
                    first_err = Some(e);
                }
            });
            match first_err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
        RuntimeValue::Array(array) => {
            array.validate()?;
            for s in array.values.iter() {
                visit(s.clone())?;
            }
            Ok(())
        }
        RuntimeValue::Scalar(s) => visit(s.clone()),
        RuntimeValue::Lambda(_) => Err(ErrorKind::Value),
    }
}

pub(crate) fn collect_scalars(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    start: usize,
) -> Result<Vec<Scalar>, ErrorKind> {
    let mut out = Vec::new();
    for arg in args.iter().skip(start) {
        if arg.omitted {
            continue;
        }
        walk_arg(ctx, arg, &mut |s| {
            out.push(s);
            Ok(())
        })?;
    }
    Ok(out)
}

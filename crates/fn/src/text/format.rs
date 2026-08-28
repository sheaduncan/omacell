//! TEXT, FIXED, DOLLAR, ARRAYTOTEXT, VALUETOTEXT.

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, RuntimeValue};
use omacell_core::locale::LocaleId;
use omacell_core::numfmt::{self, FormatOptions, FormatValue};

use crate::util::{
    date_system, err, optional, scalar, text, to_bool, to_number, to_text, trunc_i64,
};

pub(crate) fn text_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let value = match scalar(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let fmt = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let fv = match &value {
        Scalar::Empty => FormatValue::Empty,
        Scalar::Number(n) => FormatValue::Number(*n),
        Scalar::Bool(b) => FormatValue::Bool(*b),
        Scalar::Text(t) => FormatValue::Text(t),
        Scalar::Error(e) => return err(*e),
    };
    let opts = FormatOptions {
        locale: ctx.locale(),
        date_system: date_system(ctx),
        width: None,
    };
    text(numfmt::format_with(fv, &fmt, &opts).text)
}

pub(crate) fn fixed_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let n = match to_number(ctx, &args[0]) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let decimals = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(d) => d,
            Err(e) => return err(e),
        },
        None => 2,
    };
    let no_commas = match optional(args, 2) {
        Some(a) => match to_bool(ctx, a) {
            Ok(b) => b,
            Err(e) => return err(e),
        },
        None => false,
    };
    text(format_fixed(n, decimals, no_commas, ctx.locale()))
}

pub(crate) fn dollar_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let n = match to_number(ctx, &args[0]) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let decimals = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(d) => d,
            Err(e) => return err(e),
        },
        None => 2,
    };
    let currency = ctx.locale().info().currency;
    let body = format_fixed(n.abs(), decimals, false, ctx.locale());
    if n < 0.0 {
        text(format!("({currency}{body})"))
    } else {
        text(format!("{currency}{body}"))
    }
}

pub(crate) fn arraytotext_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let strict = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(0) => false,
            Ok(1) => true,
            Ok(_) => return err(ErrorKind::Value),
            Err(e) => return err(e),
        },
        None => false,
    };
    let value = ctx.materialize(args[0].value.clone());
    match array_to_text(&value, strict) {
        Ok(s) => text(s),
        Err(e) => err(e),
    }
}

pub(crate) fn valuetotext_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    arraytotext_impl(ctx, args)
}

fn array_to_text(value: &RuntimeValue, strict: bool) -> Result<String, ErrorKind> {
    match value {
        RuntimeValue::Scalar(s) => scalar_to_text(s, strict),
        RuntimeValue::Array(a) => {
            a.validate()?;
            let cols = a.cols as usize;
            let mut rows = Vec::new();
            for r in 0..a.rows as usize {
                let mut cells = Vec::new();
                for c in 0..cols {
                    let idx = r.saturating_mul(cols).saturating_add(c);
                    cells.push(scalar_to_text(
                        a.values.get(idx).unwrap_or(&Scalar::Empty),
                        strict,
                    )?);
                }
                rows.push(cells);
            }
            if strict {
                if a.rows == 1 && a.cols == 1 {
                    Ok(rows[0][0].clone())
                } else {
                    let body = rows
                        .iter()
                        .map(|r| r.join(","))
                        .collect::<Vec<_>>()
                        .join(";");
                    Ok(format!("{{{body}}}"))
                }
            } else {
                Ok(rows
                    .iter()
                    .map(|r| r.join(", "))
                    .collect::<Vec<_>>()
                    .join("; "))
            }
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

fn scalar_to_text(s: &Scalar, strict: bool) -> Result<String, ErrorKind> {
    match s {
        Scalar::Error(e) => Ok(e.as_str().to_string()),
        Scalar::Empty => Ok(if strict {
            "\"\"".to_string()
        } else {
            String::new()
        }),
        Scalar::Number(n) => {
            if !n.is_finite() {
                return Err(ErrorKind::Num);
            }
            Ok(omacell_core::eval::format_runtime(&RuntimeValue::Scalar(
                Scalar::Number(*n),
            )))
        }
        Scalar::Bool(true) => Ok("TRUE".into()),
        Scalar::Bool(false) => Ok("FALSE".into()),
        Scalar::Text(t) => Ok(if strict {
            format!("\"{}\"", t.replace('"', "\"\""))
        } else {
            t.to_string()
        }),
    }
}

fn format_fixed(n: f64, decimals: i64, no_commas: bool, locale: LocaleId) -> String {
    let rounded = if decimals >= 0 {
        round_half_away(n, decimals as i32)
    } else {
        let scale = 10f64.powi((-decimals) as i32);
        round_half_away(n / scale, 0) * scale
    };
    let dec = if decimals > 0 { decimals as usize } else { 0 };
    let mut code = if no_commas {
        "0".to_string()
    } else {
        "#,##0".to_string()
    };
    if dec > 0 {
        code.push('.');
        code.push_str(&"0".repeat(dec));
    }
    numfmt::format(FormatValue::Number(rounded), &code, locale).text
}

fn round_half_away(n: f64, places: i32) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let scale = 10f64.powi(places);
    let x = n * scale;
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let ax = x.abs();
    let floor = ax.floor();
    let rounded = if ax - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    };
    sign * rounded / scale
}

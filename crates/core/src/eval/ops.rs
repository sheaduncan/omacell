//! Operators with dynamic-array broadcasting (F-3.3, F-3.5).

use std::sync::Arc;

use crate::coerce::{self, CmpOp, Scalar, finite_or_num, first_error, to_number, to_text};
use crate::error::ErrorKind;
use crate::formula::BinOp;
use crate::locale::LocaleId;

use super::{RuntimeArray, RuntimeValue};

#[derive(Clone, Copy)]
pub(super) enum OperandOrigin {
    Expression,
    Reference,
}

/// Broadcast two values and apply a scalar operator.
pub(super) fn binary(
    op: BinOp,
    left: RuntimeValue,
    right: RuntimeValue,
    locale: LocaleId,
    left_origin: OperandOrigin,
    right_origin: OperandOrigin,
) -> RuntimeValue {
    match op {
        BinOp::Range | BinOp::Isect | BinOp::Union => {
            // Range operators are handled in the walker (need refs).
            RuntimeValue::error(ErrorKind::Value)
        }
        BinOp::Concat => lift2(left, right, concat_scalar),
        BinOp::Eq => lift2(left, right, |a, b| cmp_scalar(CmpOp::Eq, a, b)),
        BinOp::Ne => lift2(left, right, |a, b| cmp_scalar(CmpOp::Ne, a, b)),
        BinOp::Lt => lift2(left, right, |a, b| cmp_scalar(CmpOp::Lt, a, b)),
        BinOp::Le => lift2(left, right, |a, b| cmp_scalar(CmpOp::Le, a, b)),
        BinOp::Gt => lift2(left, right, |a, b| cmp_scalar(CmpOp::Gt, a, b)),
        BinOp::Ge => lift2(left, right, |a, b| cmp_scalar(CmpOp::Ge, a, b)),
        BinOp::Add => lift2(left, right, |a, b| {
            arith(a, b, locale, left_origin, right_origin, |x, y| x + y)
        }),
        BinOp::Sub => lift2(left, right, |a, b| {
            arith(a, b, locale, left_origin, right_origin, |x, y| x - y)
        }),
        BinOp::Mul => lift2(left, right, |a, b| {
            arith(a, b, locale, left_origin, right_origin, |x, y| x * y)
        }),
        BinOp::Div => lift2(left, right, |a, b| {
            div_scalar(a, b, locale, left_origin, right_origin)
        }),
        BinOp::Pow => lift2(left, right, |a, b| {
            pow_scalar(a, b, locale, left_origin, right_origin)
        }),
    }
}

/// Unary minus / plus, broadcasting over arrays.
pub(super) fn unary_minus(
    v: RuntimeValue,
    locale: LocaleId,
    origin: OperandOrigin,
) -> RuntimeValue {
    lift1(v, |s| match arithmetic_number(s, locale, origin) {
        Ok(n) => finite_or_num(-n),
        Err(e) => Scalar::Error(e),
    })
}

/// Unary plus, with the same arithmetic coercion as unary minus.
pub(super) fn unary_plus(v: RuntimeValue, locale: LocaleId, origin: OperandOrigin) -> RuntimeValue {
    lift1(v, |s| match arithmetic_number(s, locale, origin) {
        Ok(n) => finite_or_num(n),
        Err(e) => Scalar::Error(e),
    })
}

/// Percent postfix.
pub(super) fn percent(v: RuntimeValue, locale: LocaleId, origin: OperandOrigin) -> RuntimeValue {
    lift1(v, |s| match arithmetic_number(s, locale, origin) {
        Ok(n) => finite_or_num(n / 100.0),
        Err(e) => Scalar::Error(e),
    })
}

fn arith(
    a: &Scalar,
    b: &Scalar,
    locale: LocaleId,
    left_origin: OperandOrigin,
    right_origin: OperandOrigin,
    f: impl Fn(f64, f64) -> f64,
) -> Scalar {
    if let Some(e) = first_error(a, b) {
        return Scalar::Error(e);
    }
    match (
        arithmetic_number(a, locale, left_origin),
        arithmetic_number(b, locale, right_origin),
    ) {
        (Ok(x), Ok(y)) => finite_or_num(f(x, y)),
        (Err(e), _) | (_, Err(e)) => Scalar::Error(e),
    }
}

fn div_scalar(
    a: &Scalar,
    b: &Scalar,
    locale: LocaleId,
    left_origin: OperandOrigin,
    right_origin: OperandOrigin,
) -> Scalar {
    if let Some(e) = first_error(a, b) {
        return Scalar::Error(e);
    }
    match (
        arithmetic_number(a, locale, left_origin),
        arithmetic_number(b, locale, right_origin),
    ) {
        (Ok(_), Ok(0.0)) => Scalar::Error(ErrorKind::Div0),
        (Ok(x), Ok(y)) => finite_or_num(x / y),
        (Err(e), _) | (_, Err(e)) => Scalar::Error(e),
    }
}

fn pow_scalar(
    a: &Scalar,
    b: &Scalar,
    locale: LocaleId,
    left_origin: OperandOrigin,
    right_origin: OperandOrigin,
) -> Scalar {
    if let Some(e) = first_error(a, b) {
        return Scalar::Error(e);
    }
    match (
        arithmetic_number(a, locale, left_origin),
        arithmetic_number(b, locale, right_origin),
    ) {
        (Ok(x), Ok(y)) => pow_excel(x, y),
        (Err(e), _) | (_, Err(e)) => Scalar::Error(e),
    }
}

fn arithmetic_number(
    scalar: &Scalar,
    locale: LocaleId,
    origin: OperandOrigin,
) -> Result<f64, ErrorKind> {
    match scalar {
        Scalar::Text(_) if matches!(origin, OperandOrigin::Reference) => Err(ErrorKind::Value),
        Scalar::Text(text) => {
            coerce::parse_numeric_text_with_locale(text, locale).ok_or(ErrorKind::Value)
        }
        other => to_number(other),
    }
}

fn pow_excel(x: f64, y: f64) -> Scalar {
    if x == 0.0 && y == 0.0 {
        return Scalar::Error(ErrorKind::Num);
    }
    if x == 0.0 && y < 0.0 {
        return Scalar::Error(ErrorKind::Div0);
    }
    if x < 0.0 {
        // Integer exponents are allowed; fractional → #NUM!.
        if y.fract() != 0.0 {
            return Scalar::Error(ErrorKind::Num);
        }
    }
    finite_or_num(x.powf(y))
}

fn concat_scalar(a: &Scalar, b: &Scalar) -> Scalar {
    if let Some(e) = first_error(a, b) {
        return Scalar::Error(e);
    }
    match (to_text(a), to_text(b)) {
        (Ok(l), Ok(r)) => {
            if l.is_empty() && r.is_empty() {
                Scalar::Text(Arc::from(""))
            } else if l.is_empty() {
                Scalar::Text(r)
            } else if r.is_empty() {
                Scalar::Text(l)
            } else {
                let mut s = String::with_capacity(l.len() + r.len());
                s.push_str(&l);
                s.push_str(&r);
                Scalar::Text(Arc::from(s))
            }
        }
        (Err(e), _) | (_, Err(e)) => Scalar::Error(e),
    }
}

fn cmp_scalar(op: CmpOp, a: &Scalar, b: &Scalar) -> Scalar {
    match coerce::compare_op(op, a, b) {
        Ok(v) => Scalar::Bool(v),
        Err(e) => Scalar::Error(e),
    }
}

fn lift1(v: RuntimeValue, f: impl Fn(&Scalar) -> Scalar) -> RuntimeValue {
    match v {
        RuntimeValue::Lambda(_) => RuntimeValue::error(ErrorKind::Value),
        RuntimeValue::Ref(_) => RuntimeValue::error(ErrorKind::Value),
        RuntimeValue::Scalar(s) => RuntimeValue::Scalar(f(&s)),
        RuntimeValue::Array(a) => {
            if let Err(error) = a.validate() {
                return RuntimeValue::error(error);
            }
            let values: Vec<Scalar> = a.values.iter().map(f).collect();
            RuntimeValue::array(a.rows, a.cols, values)
        }
    }
}

fn lift2(
    left: RuntimeValue,
    right: RuntimeValue,
    f: impl Fn(&Scalar, &Scalar) -> Scalar,
) -> RuntimeValue {
    let l = as_grid(left);
    let r = as_grid(right);
    let (Ok(l), Ok(r)) = (l, r) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let rows = l.rows.max(r.rows);
    let cols = l.cols.max(r.cols);
    if rows == 1 && cols == 1 {
        return RuntimeValue::Scalar(f(l.at(0, 0), r.at(0, 0)));
    }
    let Ok(len) = RuntimeArray::checked_len(rows, cols) else {
        return RuntimeValue::error(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(len);
    for i in 0..rows {
        for j in 0..cols {
            let lv = pick(&l, i, j);
            let rv = pick(&r, i, j);
            values.push(match (lv, rv) {
                (Some(a), Some(b)) => f(a, b),
                _ => Scalar::Error(ErrorKind::Na),
            });
        }
    }
    RuntimeValue::array(rows, cols, values)
}

struct Grid {
    rows: u32,
    cols: u32,
    values: Arc<[Scalar]>,
}

impl Grid {
    fn at(&self, row: u32, col: u32) -> &Scalar {
        let i = (row as usize) * (self.cols as usize) + (col as usize);
        self.values.get(i).unwrap_or(&Scalar::Empty)
    }
}

fn as_grid(v: RuntimeValue) -> Result<Grid, ErrorKind> {
    match v {
        RuntimeValue::Scalar(s) => Ok(Grid {
            rows: 1,
            cols: 1,
            values: Arc::from([s]),
        }),
        RuntimeValue::Array(a) => {
            a.validate()?;
            Ok(Grid {
                rows: a.rows,
                cols: a.cols,
                values: Arc::clone(&a.values),
            })
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

fn pick(g: &Grid, row: u32, col: u32) -> Option<&Scalar> {
    let rr = if g.rows == 1 {
        0
    } else if row < g.rows {
        row
    } else {
        return None;
    };
    let cc = if g.cols == 1 {
        0
    } else if col < g.cols {
        col
    } else {
        return None;
    };
    Some(g.at(rr, cc))
}

//! Excel F-3.5 coercion and comparison.
//!
//! Empty → 0 or `""` by context; `TRUE`/`FALSE` → 1/0 in arithmetic; numeric
//! text coerces in arithmetic but not in comparison; text comparison is
//! case-insensitive; errors propagate in evaluation order.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::error::ErrorKind;

/// A scalar runtime value (arrays and lambdas live in [`crate::eval::RuntimeValue`]).
#[derive(Clone, Debug)]
pub enum Scalar {
    /// Empty cell / omitted-as-empty.
    Empty,
    /// IEEE 754 number (non-finite values become [`ErrorKind::Num`] at the operator).
    Number(f64),
    /// Boolean.
    Bool(bool),
    /// Text payload (not interned; commit interned this).
    Text(Arc<str>),
    /// Excel error.
    Error(ErrorKind),
}

impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Empty, Self::Empty) => true,
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a.as_ref() == b.as_ref(),
            (Self::Error(a), Self::Error(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Scalar {}

impl Scalar {
    /// Excel error, if any.
    #[must_use]
    pub fn error(&self) -> Option<ErrorKind> {
        match self {
            Self::Error(e) => Some(*e),
            _ => None,
        }
    }

    /// Whether this is an empty cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Text payload for this scalar (numbers via General, bools as TRUE/FALSE).
    pub fn as_text(&self) -> Result<Arc<str>, ErrorKind> {
        to_text(self)
    }
}

impl From<ErrorKind> for Scalar {
    fn from(e: ErrorKind) -> Self {
        Self::Error(e)
    }
}

/// First error in left-to-right evaluation order.
#[must_use]
pub fn first_error(left: &Scalar, right: &Scalar) -> Option<ErrorKind> {
    left.error().or_else(|| right.error())
}

/// Parse Excel-ish numeric text. Leading/trailing space is ignored; empty
/// (after trim) is 0. Thousands separators are not accepted.
#[must_use]
pub fn parse_numeric_text(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    if t.eq_ignore_ascii_case("TRUE") {
        return Some(1.0);
    }
    if t.eq_ignore_ascii_case("FALSE") {
        return Some(0.0);
    }
    // Reject commas / currency — those are locale-formatted, not raw numeric text.
    if t.bytes().any(|b| b == b',' || b == b'$' || b == b'%') {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Coerce to a number for arithmetic (empty → 0, bool → 0/1, numeric text).
pub fn to_number(s: &Scalar) -> Result<f64, ErrorKind> {
    match s {
        Scalar::Empty => Ok(0.0),
        Scalar::Number(n) if n.is_finite() => Ok(*n),
        Scalar::Number(_) => Err(ErrorKind::Num),
        Scalar::Bool(true) => Ok(1.0),
        Scalar::Bool(false) => Ok(0.0),
        Scalar::Text(t) => parse_numeric_text(t).ok_or(ErrorKind::Value),
        Scalar::Error(e) => Err(*e),
    }
}

/// Coerce to text for concatenation (empty → `""`).
pub fn to_text(s: &Scalar) -> Result<Arc<str>, ErrorKind> {
    match s {
        Scalar::Empty => Ok(Arc::from("")),
        Scalar::Number(n) => {
            if !n.is_finite() {
                return Err(ErrorKind::Num);
            }
            Ok(Arc::from(crate::numfmt::general(*n)))
        }
        Scalar::Bool(true) => Ok(Arc::from("TRUE")),
        Scalar::Bool(false) => Ok(Arc::from("FALSE")),
        Scalar::Text(t) => Ok(Arc::clone(t)),
        Scalar::Error(e) => Err(*e),
    }
}

/// Coerce to bool for logical context (IF tests): empty/0/FALSE/"" → false.
pub fn to_bool(s: &Scalar) -> Result<bool, ErrorKind> {
    match s {
        Scalar::Empty => Ok(false),
        Scalar::Number(n) if n.is_finite() => Ok(*n != 0.0),
        Scalar::Number(_) => Err(ErrorKind::Num),
        Scalar::Bool(b) => Ok(*b),
        Scalar::Text(t) => {
            if t.eq_ignore_ascii_case("TRUE") {
                Ok(true)
            } else if t.eq_ignore_ascii_case("FALSE") {
                Ok(false)
            } else {
                Err(ErrorKind::Value)
            }
        }
        Scalar::Error(e) => Err(*e),
    }
}

/// Comparison result. Errors have already been stripped by the caller or are
/// returned here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    /// Left < right.
    Lt,
    /// Left = right.
    Eq,
    /// Left > right.
    Gt,
}

impl Cmp {
    /// Convert to [`Ordering`].
    #[must_use]
    pub fn ordering(self) -> Ordering {
        match self {
            Self::Lt => Ordering::Less,
            Self::Eq => Ordering::Equal,
            Self::Gt => Ordering::Greater,
        }
    }

    /// Apply an Excel comparison operator token.
    #[must_use]
    pub fn matches(self, op: CmpOp) -> bool {
        match op {
            CmpOp::Eq => self == Self::Eq,
            CmpOp::Ne => self != Self::Eq,
            CmpOp::Lt => self == Self::Lt,
            CmpOp::Le => self != Self::Gt,
            CmpOp::Gt => self == Self::Gt,
            CmpOp::Ge => self != Self::Lt,
        }
    }
}

/// Comparison operators (F-3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
}

fn text_key(s: &str) -> String {
    s.to_lowercase()
}

fn cmp_text(a: &str, b: &str) -> Cmp {
    match text_key(a).cmp(&text_key(b)) {
        Ordering::Less => Cmp::Lt,
        Ordering::Equal => Cmp::Eq,
        Ordering::Greater => Cmp::Gt,
    }
}

fn empty_equal_to(other: &Scalar) -> bool {
    match other {
        Scalar::Empty => true,
        Scalar::Number(n) if *n == 0.0 => true,
        Scalar::Bool(false) => true,
        Scalar::Text(t) if t.is_empty() => true,
        _ => false,
    }
}

/// Excel comparison. Does **not** coerce numeric text to numbers. Empty equals
/// 0, `""`, and FALSE. Type rank otherwise: number < text < bool.
pub fn compare(left: &Scalar, right: &Scalar) -> Result<Cmp, ErrorKind> {
    if let Some(e) = first_error(left, right) {
        return Err(e);
    }
    if left.is_empty() || right.is_empty() {
        if empty_equal_to(left) && empty_equal_to(right) {
            return Ok(Cmp::Eq);
        }
        // Treat remaining empties as 0 so `empty < 1` and `empty < TRUE`.
        let l = if left.is_empty() {
            &Scalar::Number(0.0)
        } else {
            left
        };
        let r = if right.is_empty() {
            &Scalar::Number(0.0)
        } else {
            right
        };
        return compare_ranked(l, r);
    }
    compare_ranked(left, right)
}

fn rank(s: &Scalar) -> u8 {
    match s {
        Scalar::Number(_) | Scalar::Empty => 0,
        Scalar::Text(_) => 1,
        Scalar::Bool(_) => 2,
        Scalar::Error(_) => 3,
    }
}

fn compare_ranked(left: &Scalar, right: &Scalar) -> Result<Cmp, ErrorKind> {
    match (left, right) {
        (Scalar::Number(a), Scalar::Number(b)) => Ok(if a < b {
            Cmp::Lt
        } else if a > b {
            Cmp::Gt
        } else {
            Cmp::Eq
        }),
        (Scalar::Bool(a), Scalar::Bool(b)) => Ok(match a.cmp(b) {
            Ordering::Less => Cmp::Lt,
            Ordering::Equal => Cmp::Eq,
            Ordering::Greater => Cmp::Gt,
        }),
        (Scalar::Text(a), Scalar::Text(b)) => Ok(cmp_text(a, b)),
        (Scalar::Empty, Scalar::Empty) => Ok(Cmp::Eq),
        (l, r) => {
            let rl = rank(l);
            let rr = rank(r);
            if rl < rr {
                Ok(Cmp::Lt)
            } else if rl > rr {
                Ok(Cmp::Gt)
            } else {
                Ok(Cmp::Eq)
            }
        }
    }
}

/// Apply a comparison operator with F-3.5 rules.
pub fn compare_op(op: CmpOp, left: &Scalar, right: &Scalar) -> Result<bool, ErrorKind> {
    Ok(compare(left, right)?.matches(op))
}

/// Finite or `#NUM!`.
#[must_use]
pub fn finite_or_num(n: f64) -> Scalar {
    if n.is_finite() {
        Scalar::Number(n)
    } else {
        Scalar::Error(ErrorKind::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn t(s: &str) -> Scalar {
        Scalar::Text(Arc::from(s))
    }

    #[test]
    fn empty_arithmetic_is_zero() {
        assert_eq!(to_number(&Scalar::Empty).unwrap(), 0.0);
    }

    #[test]
    fn bool_arithmetic() {
        assert_eq!(to_number(&Scalar::Bool(true)).unwrap(), 1.0);
        assert_eq!(to_number(&Scalar::Bool(false)).unwrap(), 0.0);
    }

    #[test]
    fn numeric_text_arithmetic_not_comparison() {
        assert_eq!(to_number(&t("1")).unwrap(), 1.0);
        assert!(!compare_op(CmpOp::Eq, &t("1"), &Scalar::Number(1.0)).unwrap());
    }

    #[test]
    fn text_compare_case_insensitive() {
        assert!(compare_op(CmpOp::Eq, &t("A"), &t("a")).unwrap());
        assert!(compare_op(CmpOp::Lt, &t("A"), &t("b")).unwrap());
    }

    #[test]
    fn errors_left_to_right() {
        let l = Scalar::Error(ErrorKind::Div0);
        let r = Scalar::Error(ErrorKind::Name);
        assert_eq!(first_error(&l, &r), Some(ErrorKind::Div0));
        assert_eq!(first_error(&Scalar::Number(1.0), &r), Some(ErrorKind::Name));
    }

    #[test]
    fn empty_equals_zero_and_blank() {
        assert!(compare_op(CmpOp::Eq, &Scalar::Empty, &Scalar::Number(0.0)).unwrap());
        assert!(compare_op(CmpOp::Eq, &Scalar::Empty, &t("")).unwrap());
        assert!(compare_op(CmpOp::Eq, &Scalar::Empty, &Scalar::Bool(false)).unwrap());
    }

    proptest! {
        #[test]
        fn number_compare_matches_f64(a in -1e6f64..1e6, b in -1e6f64..1e6) {
            prop_assume!(a.is_finite() && b.is_finite());
            let got = compare(&Scalar::Number(a), &Scalar::Number(b)).unwrap();
            if a < b {
                prop_assert_eq!(got, Cmp::Lt);
            } else if a > b {
                prop_assert_eq!(got, Cmp::Gt);
            } else {
                prop_assert_eq!(got, Cmp::Eq);
            }
        }

        #[test]
        fn numeric_text_always_coerces_in_to_number(n in -1e4f64..1e4) {
            prop_assume!(n.is_finite());
            let s = format!("{n}");
            if let Some(p) = parse_numeric_text(&s) {
                prop_assert_eq!(to_number(&t(&s)).unwrap(), p);
            }
        }
    }
}

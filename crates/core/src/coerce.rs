//! Excel F-3.5 coercion and comparison.
//!
//! Empty → 0 or `""` by context; `TRUE`/`FALSE` → 1/0 in arithmetic; numeric
//! text coerces in arithmetic but not in comparison; text comparison is
//! case-insensitive; errors propagate in evaluation order.

use std::cmp::Ordering;
use std::sync::Arc;

use crate::error::ErrorKind;
use crate::locale::LocaleId;

const MAX_NUMERIC_TEXT_LEN: usize = 32_767;

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

/// Parse en-US Excel numeric text used by formula coercion.
///
/// Leading/trailing space is ignored. Empty and boolean text are not numeric;
/// en-US grouping, currency, percentage, decimal, and exponent syntax are
/// accepted. Input is bounded to 32,767 bytes.
#[must_use]
pub fn parse_numeric_text(s: &str) -> Option<f64> {
    parse_numeric_text_with_locale(s, LocaleId::EN_US)
}

pub(crate) fn parse_numeric_text_with_locale(s: &str, locale: LocaleId) -> Option<f64> {
    if s.len() > MAX_NUMERIC_TEXT_LEN {
        return None;
    }
    let mut text = s.trim();
    if text.is_empty() {
        return None;
    }

    let mut negative = false;
    let mut signed = false;
    if text.starts_with('(') && text.ends_with(')') {
        negative = true;
        text = text.get(1..text.len().saturating_sub(1))?.trim();
    }

    let mut percent = false;
    if let Some(rest) = text.strip_suffix('%') {
        percent = true;
        text = rest.trim_end();
    }
    consume_sign(&mut text, &mut negative, &mut signed)?;

    let currency = locale.info().currency;
    if let Some(rest) = text.strip_prefix(currency) {
        text = rest.trim_start();
    } else if let Some(rest) = text.strip_suffix(currency) {
        text = rest.trim_end();
    }
    consume_sign(&mut text, &mut negative, &mut signed)?;
    if text.is_empty() {
        return None;
    }

    let (mantissa, exponent) = split_exponent(text)?;
    let separators = locale.separators();
    let (integer, fraction) = match mantissa.split_once(separators.decimal) {
        Some((integer, fraction)) if !fraction.contains(separators.decimal) => {
            (integer, Some(fraction))
        }
        Some(_) => return None,
        None => (mantissa, None),
    };
    let integer = normalize_integer(integer, separators.thousands)?;
    let fraction = fraction.unwrap_or("");
    if !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || integer.is_empty() && fraction.is_empty()
    {
        return None;
    }

    let mut normalized = String::with_capacity(text.len().saturating_add(2));
    if negative {
        normalized.push('-');
    }
    if integer.is_empty() {
        normalized.push('0');
    } else {
        normalized.push_str(&integer);
    }
    if fraction.is_empty() {
        if mantissa.contains(separators.decimal) {
            normalized.push('.');
        }
    } else {
        normalized.push('.');
        normalized.push_str(fraction);
    }
    if let Some(exponent) = exponent {
        normalized.push('e');
        normalized.push_str(exponent);
    }

    let mut number = normalized.parse::<f64>().ok()?;
    if percent {
        number /= 100.0;
    }
    number.is_finite().then_some(number)
}

fn consume_sign(text: &mut &str, negative: &mut bool, signed: &mut bool) -> Option<()> {
    if let Some(rest) = text.strip_prefix('-') {
        if *signed || *negative {
            return None;
        }
        *negative = true;
        *signed = true;
        *text = rest.trim_start();
    } else if let Some(rest) = text.strip_prefix('+') {
        if *signed || *negative {
            return None;
        }
        *signed = true;
        *text = rest.trim_start();
    }
    Some(())
}

fn split_exponent(text: &str) -> Option<(&str, Option<&str>)> {
    let mut split = text.split(['e', 'E']);
    let mantissa = split.next()?;
    let exponent = split.next();
    if split.next().is_some() {
        return None;
    }
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    Some((mantissa, exponent))
}

fn normalize_integer(integer: &str, group: char) -> Option<String> {
    if !integer.contains(group) {
        return integer
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| integer.to_string());
    }
    let mut groups = integer.split(group);
    let first = groups.next()?;
    if first.is_empty() || first.len() > 3 || !first.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut normalized = first.to_string();
    for digits in groups {
        if digits.len() != 3 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        normalized.push_str(digits);
    }
    Some(normalized)
}

/// Coerce a scalar function argument to a number.
///
/// Empty and empty text become 0, booleans become 0/1, and numeric text uses
/// en-US syntax. Formula operators apply stricter, origin-aware text rules.
pub fn to_number(s: &Scalar) -> Result<f64, ErrorKind> {
    match s {
        Scalar::Empty => Ok(0.0),
        Scalar::Number(n) if n.is_finite() => Ok(*n),
        Scalar::Number(_) => Err(ErrorKind::Num),
        Scalar::Bool(true) => Ok(1.0),
        Scalar::Bool(false) => Ok(0.0),
        Scalar::Text(t) if t.trim().is_empty() => Ok(0.0),
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
            Ok(Arc::from(crate::numfmt::general_for_width(*n, 24)))
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
        (Scalar::Number(a), Scalar::Number(b)) => {
            if !a.is_finite() || !b.is_finite() {
                return Err(ErrorKind::Num);
            }
            Ok(if a < b {
                Cmp::Lt
            } else if a > b {
                Cmp::Gt
            } else {
                Cmp::Eq
            })
        }
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
    fn numeric_text_rejects_empty_and_boolean_words() {
        assert_eq!(parse_numeric_text(""), None);
        assert_eq!(parse_numeric_text("  "), None);
        assert_eq!(to_number(&t("")), Ok(0.0));
        assert_eq!(to_number(&t("TRUE")), Err(ErrorKind::Value));
        assert_eq!(to_number(&t("false")), Err(ErrorKind::Value));
    }

    #[test]
    fn numeric_text_accepts_en_us_group_currency_and_percent() {
        assert_eq!(to_number(&t("1,234.5")).unwrap(), 1234.5);
        assert_eq!(to_number(&t("$5")).unwrap(), 5.0);
        assert_eq!(to_number(&t("5%")).unwrap(), 0.05);
        assert_eq!(parse_numeric_text("12,34"), None);
    }

    #[test]
    fn numeric_text_input_is_bounded() {
        let oversized = "0".repeat(MAX_NUMERIC_TEXT_LEN + 1);
        assert_eq!(parse_numeric_text(&oversized), None);
    }

    #[test]
    fn number_to_text_keeps_formula_precision() {
        assert_eq!(
            to_text(&Scalar::Number(1.0 / 3.0)).unwrap().as_ref(),
            "0.333333333333333"
        );
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

//! Shared walk, coerce, rounding, criteria, and statistics helpers.

use std::collections::HashMap;
use std::sync::Arc;

use omacell_core::addr::{SheetId, col_to_letters};
use omacell_core::coerce::{self, CmpOp, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, Reference, RuntimeValue};

/// `#NUM!` for non-finite results.
#[must_use]
pub fn rt_num(n: f64) -> RuntimeValue {
    if n.is_finite() {
        RuntimeValue::Scalar(Scalar::Number(n))
    } else {
        RuntimeValue::error(ErrorKind::Num)
    }
}

/// Boolean result.
#[must_use]
pub fn rt_bool(b: bool) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Bool(b))
}

/// Text result.
#[must_use]
pub fn rt_text(s: impl Into<Arc<str>>) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Text(s.into()))
}

/// Origin of a visited value (literal vs range/array).
#[derive(Clone, Copy, Debug)]
pub enum Origin {
    /// Function argument written as a scalar.
    Literal,
    /// Range or array element.
    Aggregate,
}

/// Materialize a single argument to a scalar (1×1 arrays unwrap).
pub fn arg_scalar(ctx: &EvalCtx<'_>, args: &[ArgVal], i: usize) -> Result<Scalar, ErrorKind> {
    let Some(arg) = args.get(i) else {
        return Err(ErrorKind::Value);
    };
    if arg.omitted {
        return Ok(Scalar::Empty);
    }
    match ctx.materialize(arg.value.clone()) {
        RuntimeValue::Scalar(s) => Ok(s),
        RuntimeValue::Array(a) => {
            if a.rows == 1 && a.cols == 1 {
                Ok(a.values.first().cloned().unwrap_or(Scalar::Empty))
            } else {
                Err(ErrorKind::Value)
            }
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

/// Numeric argument, coercing literals.
pub fn arg_number(ctx: &EvalCtx<'_>, args: &[ArgVal], i: usize) -> Result<f64, ErrorKind> {
    coerce::to_number(&arg_scalar(ctx, args, i)?)
}

/// Optional numeric argument with default when omitted.
pub fn arg_number_or(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    i: usize,
    default: f64,
) -> Result<f64, ErrorKind> {
    match args.get(i) {
        None | Some(ArgVal { omitted: true, .. }) => Ok(default),
        Some(_) => arg_number(ctx, args, i),
    }
}

/// Unary numeric function.
pub fn unary(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    f: impl FnOnce(f64) -> Result<f64, ErrorKind>,
) -> RuntimeValue {
    match arg_number(ctx, args, 0).and_then(f) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}

/// Binary numeric function.
pub fn binary(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    f: impl FnOnce(f64, f64) -> Result<f64, ErrorKind>,
) -> RuntimeValue {
    let out = (|| {
        let a = arg_number(ctx, args, 0)?;
        let b = arg_number(ctx, args, 1)?;
        f(a, b)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}

/// Excel `INT`: floor toward −∞.
#[must_use]
pub fn excel_int(n: f64) -> f64 {
    n.floor()
}

/// Truncate toward zero.
#[must_use]
pub fn trunc_toward_zero(n: f64) -> f64 {
    n.trunc()
}

/// Half-away-from-zero rounding to `digits` decimal places (Excel `ROUND`).
#[must_use]
pub fn round_half_away(n: f64, digits: i32) -> f64 {
    decimal_round(n, digits, DecimalRound::Nearest)
}

/// `ROUNDUP`: away from zero.
#[must_use]
pub fn round_up(n: f64, digits: i32) -> f64 {
    decimal_round(n, digits, DecimalRound::Away)
}

/// `ROUNDDOWN`: toward zero.
#[must_use]
pub fn round_down(n: f64, digits: i32) -> f64 {
    decimal_round(n, digits, DecimalRound::Toward)
}

#[derive(Clone, Copy)]
enum DecimalRound {
    Nearest,
    Away,
    Toward,
}

fn decimal_round(n: f64, digits: i32, mode: DecimalRound) -> f64 {
    if !n.is_finite() {
        return n;
    }
    if digits > 20 {
        return n;
    }
    if digits < -20 {
        return 0.0;
    }
    if n == 0.0 {
        return n;
    }

    let sign = if n < 0.0 { -1.0 } else { 1.0 };
    let magnitude = n.abs();
    let rough_exponent = magnitude.log10().floor() as i32;
    let rough_discarded = 14_i32.saturating_sub(rough_exponent).saturating_sub(digits);

    // The supported digit range makes this path relevant only for values far
    // below the requested quantum. Avoid 10^-324 underflow while retaining
    // ROUNDUP's away-from-zero behavior.
    if rough_discarded > 16 {
        return match mode {
            DecimalRound::Away => sign * 10f64.powi(-digits),
            DecimalRound::Nearest | DecimalRound::Toward => sign * 0.0,
        };
    }

    let Some((coefficient, exponent)) = decimal_coefficient(magnitude) else {
        return n;
    };
    let discarded = 14_i32.saturating_sub(exponent).saturating_sub(digits);
    if discarded <= 0 {
        let normalized = coefficient as f64 * 10f64.powi(exponent - 14);
        return if normalized.is_finite() {
            sign * normalized
        } else {
            n
        };
    }

    let discarded = discarded as u32;
    let (kept, remainder, divisor) = if discarded > 15 {
        (0, coefficient, None)
    } else {
        let divisor = 10_u64.pow(discarded);
        (coefficient / divisor, coefficient % divisor, Some(divisor))
    };
    let increment = match mode {
        DecimalRound::Nearest => divisor.is_some_and(|divisor| remainder * 2 >= divisor),
        DecimalRound::Away => remainder != 0,
        DecimalRound::Toward => false,
    };
    let rounded = kept + u64::from(increment);
    let out = sign * rounded as f64 * 10f64.powi(-digits);
    if out.is_finite() { out } else { n }
}

fn decimal_coefficient(magnitude: f64) -> Option<(u64, i32)> {
    let mut exponent = magnitude.log10().floor() as i32;
    let power = 10f64.powi(exponent);
    if !power.is_finite() || power == 0.0 {
        return None;
    }
    let scaled = magnitude / power * 1e14;
    if !scaled.is_finite() {
        return None;
    }
    let mut coefficient = scaled.round();
    if coefficient >= 1e15 {
        coefficient /= 10.0;
        exponent += 1;
    } else if coefficient < 1e14 {
        coefficient *= 10.0;
        exponent -= 1;
    }
    Some((coefficient as u64, exponent))
}

/// Excel `MOD`: remainder with divisor sign (`n - d * INT(n/d)`).
pub fn excel_mod(n: f64, d: f64) -> Result<f64, ErrorKind> {
    if d == 0.0 {
        return Err(ErrorKind::Div0);
    }
    if !n.is_finite() || !d.is_finite() {
        return Err(ErrorKind::Num);
    }
    let q = excel_int(n / d);
    let r = n - d * q;
    if r.is_finite() {
        Ok(r)
    } else {
        Err(ErrorKind::Num)
    }
}

/// Visit every scalar in `args`.
pub fn for_each_value(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    visit: &mut impl FnMut(Scalar, Origin) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    for arg in args {
        if arg.omitted {
            continue;
        }
        for_each_one(ctx, &arg.value, visit)?;
    }
    Ok(())
}

fn for_each_one(
    ctx: &EvalCtx<'_>,
    value: &RuntimeValue,
    visit: &mut impl FnMut(Scalar, Origin) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    match value {
        RuntimeValue::Scalar(s) => visit(s.clone(), Origin::Literal),
        RuntimeValue::Array(a) => {
            a.validate()?;
            for s in a.values.iter() {
                visit(s.clone(), Origin::Aggregate)?;
            }
            Ok(())
        }
        RuntimeValue::Ref(r) => {
            let mut err = None;
            ctx.for_each_stored_cell(r, &mut |_sh, _row, _col, s| {
                if err.is_some() {
                    return;
                }
                if let Err(e) = visit(s, Origin::Aggregate) {
                    err = Some(e);
                }
            });
            match err {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }
        RuntimeValue::Lambda(_) => Err(ErrorKind::Value),
    }
}

/// Collect numbers with Excel SUM/AVERAGE range-vs-literal rules.
pub fn collect_numbers(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for_each_value(ctx, args, &mut |s, origin| {
        push_number(&mut out, s, origin, false)
    })?;
    Ok(out)
}

/// AVERAGEA / STDEVA / VARA: text and FALSE → 0, TRUE → 1 in aggregates.
pub fn collect_numbers_a(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for_each_value(ctx, args, &mut |s, origin| {
        push_number(&mut out, s, origin, true)
    })?;
    Ok(out)
}

fn push_number(
    out: &mut Vec<f64>,
    s: Scalar,
    origin: Origin,
    alpha: bool,
) -> Result<(), ErrorKind> {
    if let Some(e) = s.error() {
        return Err(e);
    }
    match origin {
        Origin::Literal => {
            out.push(coerce::to_number(&s)?);
            Ok(())
        }
        Origin::Aggregate => match s {
            Scalar::Number(n) if n.is_finite() => {
                out.push(n);
                Ok(())
            }
            Scalar::Number(_) => Err(ErrorKind::Num),
            Scalar::Bool(b) if alpha => {
                out.push(if b { 1.0 } else { 0.0 });
                Ok(())
            }
            Scalar::Text(_) if alpha => {
                out.push(0.0);
                Ok(())
            }
            Scalar::Empty | Scalar::Bool(_) | Scalar::Text(_) => Ok(()),
            Scalar::Error(e) => Err(e),
        },
    }
}

/// SUM of numbers (range skips text/bool; literals coerce).
pub fn sum_args(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<f64, ErrorKind> {
    Ok(collect_numbers(ctx, args)?.iter().sum())
}

/// PRODUCT; empty number list → 0.
pub fn product_args(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<f64, ErrorKind> {
    let nums = collect_numbers(ctx, args)?;
    if nums.is_empty() {
        return Ok(0.0);
    }
    Ok(nums.iter().fold(1.0, |a, b| a * b))
}

/// COUNT: numbers in ranges; literals coerce bool/numeric-text.
pub fn count_args(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<f64, ErrorKind> {
    let mut n = 0.0;
    for_each_value(ctx, args, &mut |s, origin| {
        if s.error().is_some() {
            return Ok(());
        }
        let keep = match origin {
            Origin::Literal => coerce::to_number(&s).is_ok(),
            Origin::Aggregate => matches!(s, Scalar::Number(v) if v.is_finite()),
        };
        if keep {
            n += 1.0;
        }
        Ok(())
    })?;
    Ok(n)
}

/// COUNTA: non-empty values (errors count).
pub fn counta_args(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<f64, ErrorKind> {
    let mut n = 0.0;
    for_each_value(ctx, args, &mut |s, _origin| {
        if !matches!(s, Scalar::Empty) {
            n += 1.0;
        }
        Ok(())
    })?;
    Ok(n)
}

/// COUNTBLANK of refs/arrays (literals: blank if empty/`""`).
pub fn countblank_args(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<f64, ErrorKind> {
    let mut n = 0.0;
    for arg in args {
        if arg.omitted {
            n += 1.0;
            continue;
        }
        match &arg.value {
            RuntimeValue::Ref(r) => {
                let total = ctx.reference_cell_count(r);
                let mut occupied_nonblank = 0u64;
                ctx.for_each_stored_cell(r, &mut |_, _, _, s| {
                    if !is_blank(&s) {
                        occupied_nonblank += 1;
                    }
                });
                n += (total.saturating_sub(occupied_nonblank)) as f64;
            }
            RuntimeValue::Array(a) => {
                a.validate()?;
                n += a.values.iter().filter(|s| is_blank(s)).count() as f64;
            }
            RuntimeValue::Scalar(s) => {
                if is_blank(s) {
                    n += 1.0;
                }
            }
            RuntimeValue::Lambda(_) => return Err(ErrorKind::Value),
        }
    }
    Ok(n)
}

fn is_blank(s: &Scalar) -> bool {
    match s {
        Scalar::Empty => true,
        Scalar::Text(t) if t.is_empty() => true,
        _ => false,
    }
}

/// Sample variance (`n-1`) or population (`n`).
pub fn variance(values: &[f64], sample: bool) -> Result<f64, ErrorKind> {
    let n = values.len();
    let denom = if sample { n.saturating_sub(1) } else { n };
    if n == 0 || denom == 0 {
        return Err(ErrorKind::Div0);
    }
    let mean = values.iter().sum::<f64>() / n as f64;
    let ss: f64 = values
        .iter()
        .map(|x| {
            let d = x - mean;
            d * d
        })
        .sum();
    Ok(ss / denom as f64)
}

/// Sample or population standard deviation.
pub fn stdev(values: &[f64], sample: bool) -> Result<f64, ErrorKind> {
    Ok(variance(values, sample)?.sqrt())
}

/// Sorted copy of numbers.
#[must_use]
pub fn sorted(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values
}

/// Inclusive percentile (`PERCENTILE.INC`). `k` in [0, 1].
pub fn percentile_inc(sorted_vals: &[f64], k: f64) -> Result<f64, ErrorKind> {
    if sorted_vals.is_empty() {
        return Err(ErrorKind::Num);
    }
    if !(0.0..=1.0).contains(&k) || !k.is_finite() {
        return Err(ErrorKind::Num);
    }
    let n = sorted_vals.len();
    if n == 1 {
        return Ok(sorted_vals[0]);
    }
    let pos = k * (n as f64 - 1.0);
    interpolate(sorted_vals, pos)
}

/// Exclusive percentile (`PERCENTILE.EXC`).
pub fn percentile_exc(sorted_vals: &[f64], k: f64) -> Result<f64, ErrorKind> {
    let n = sorted_vals.len();
    if n < 2 {
        return Err(ErrorKind::Num);
    }
    let lo = 1.0 / (n as f64 + 1.0);
    let hi = n as f64 / (n as f64 + 1.0);
    if !k.is_finite() || k <= lo || k >= hi {
        return Err(ErrorKind::Num);
    }
    let pos = k * (n as f64 + 1.0) - 1.0;
    interpolate(sorted_vals, pos)
}

fn interpolate(sorted_vals: &[f64], pos: f64) -> Result<f64, ErrorKind> {
    if pos <= 0.0 {
        return Ok(sorted_vals[0]);
    }
    let last = sorted_vals.len() - 1;
    if pos >= last as f64 {
        return Ok(sorted_vals[last]);
    }
    let lo = pos.floor() as usize;
    let frac = pos - lo as f64;
    Ok(sorted_vals[lo] + frac * (sorted_vals[lo + 1] - sorted_vals[lo]))
}

/// Median of an unsorted list.
pub fn median(values: &[f64]) -> Result<f64, ErrorKind> {
    if values.is_empty() {
        return Err(ErrorKind::Num);
    }
    let s = sorted(values.to_vec());
    let n = s.len();
    if n % 2 == 1 {
        Ok(s[n / 2])
    } else {
        Ok((s[n / 2 - 1] + s[n / 2]) / 2.0)
    }
}

/// Integer GCD (non-negative).
#[must_use]
pub fn gcd_i(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Truncate toward zero and reject non-integers beyond 2^53.
pub fn as_nonneg_int(n: f64) -> Result<u64, ErrorKind> {
    if !n.is_finite() {
        return Err(ErrorKind::Num);
    }
    let t = n.trunc().abs();
    if t >= (1u64 << 53) as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(t as u64)
}

/// Value, occurrence count, and first position for each distinct finite number.
///
/// The returned vector remains in first-seen order; the hash map is lookup-only,
/// so process-randomized hash seeds cannot affect formula results.
#[must_use]
pub fn frequencies(values: &[f64]) -> Vec<(f64, usize, usize)> {
    let mut items: Vec<(f64, usize, usize)> = Vec::new();
    let mut positions = HashMap::<u64, usize>::new();
    for (first, value) in values.iter().copied().enumerate() {
        let key = if value == 0.0 { 0 } else { value.to_bits() };
        if let Some(index) = positions.get(&key).copied() {
            items[index].1 += 1;
        } else {
            positions.insert(key, items.len());
            items.push((value, 1, first));
        }
    }
    items
}

/// nCk as f64 with overflow → `#NUM!`.
pub fn combin(n: u64, k: u64) -> Result<f64, ErrorKind> {
    if k > n {
        return Err(ErrorKind::Num);
    }
    let k = k.min(n - k);
    let mut acc = 1.0;
    for i in 0..k {
        acc *= (n - i) as f64;
        acc /= (i + 1) as f64;
        if !acc.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    Ok(acc.round())
}

/// nPk.
pub fn permut(n: u64, k: u64) -> Result<f64, ErrorKind> {
    if k > n {
        return Err(ErrorKind::Num);
    }
    let mut acc = 1.0;
    for i in 0..k {
        acc *= (n - i) as f64;
        if !acc.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    Ok(acc)
}

/// Factorial; Excel truncates toward zero and caps at 170.
pub fn factorial(n: f64) -> Result<f64, ErrorKind> {
    if n < 0.0 || !n.is_finite() {
        return Err(ErrorKind::Num);
    }
    let k = n.trunc() as u64;
    if k > 170 {
        return Err(ErrorKind::Num);
    }
    let mut acc = 1.0;
    for i in 2..=k {
        acc *= i as f64;
    }
    Ok(acc)
}

/// Double factorial.
pub fn factdouble(n: f64) -> Result<f64, ErrorKind> {
    if n < 0.0 {
        return Err(ErrorKind::Num);
    }
    let k = n.trunc() as i64;
    if k > 300 {
        return Err(ErrorKind::Num);
    }
    let mut acc = 1.0;
    let mut i = k;
    while i > 1 {
        acc *= i as f64;
        if !acc.is_finite() {
            return Err(ErrorKind::Num);
        }
        i -= 2;
    }
    Ok(acc)
}

/// Pairwise walk of two arrays/ranges of equal length.
pub fn for_each_pair(
    ctx: &EvalCtx<'_>,
    x: &RuntimeValue,
    y: &RuntimeValue,
    visit: &mut impl FnMut(Scalar, Scalar) -> Result<(), ErrorKind>,
) -> Result<(), ErrorKind> {
    let xs = flatten(ctx, x)?;
    let ys = flatten(ctx, y)?;
    if xs.len() != ys.len() {
        return Err(ErrorKind::Na);
    }
    for (a, b) in xs.into_iter().zip(ys) {
        visit(a, b)?;
    }
    Ok(())
}

/// Flatten a ref/array/scalar into row-major scalars (full rectangle for refs).
pub fn flatten(ctx: &EvalCtx<'_>, value: &RuntimeValue) -> Result<Vec<Scalar>, ErrorKind> {
    let mut out = Vec::new();
    match value {
        RuntimeValue::Scalar(s) => out.push(s.clone()),
        RuntimeValue::Array(a) => {
            a.validate()?;
            out.extend(a.values.iter().cloned());
        }
        RuntimeValue::Ref(r) => {
            ctx.for_each_cell(r, &mut |s| out.push(s));
        }
        RuntimeValue::Lambda(_) => return Err(ErrorKind::Value),
    }
    Ok(out)
}

/// Criteria parsed from a SUMIF/COUNTIF criterion.
#[derive(Clone, Debug)]
pub enum Criteria {
    /// Compare using an operator.
    Cmp {
        /// Operator.
        op: CmpOp,
        /// Right-hand scalar.
        rhs: Scalar,
        /// Wildcard pattern when `op` is Eq/Ne and rhs is text.
        pattern: Option<Wildcard>,
    },
}

/// Excel wildcard (`* ? ~`).
#[derive(Clone, Debug)]
pub struct Wildcard {
    tokens: Vec<WildTok>,
}

#[derive(Clone, Debug)]
enum WildTok {
    Any,
    One,
    Lit(char),
}

impl Wildcard {
    /// Parse a pattern. `~` escapes the next `*`, `?`, or `~`.
    pub fn parse(pat: &str) -> Self {
        let mut tokens = Vec::new();
        let mut chars = pat.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '~' {
                if let Some(n) = chars.next() {
                    tokens.push(WildTok::Lit(n.to_ascii_lowercase()));
                } else {
                    tokens.push(WildTok::Lit('~'));
                }
                continue;
            }
            if c == '*' || c == '?' {
                tokens.push(if c == '*' { WildTok::Any } else { WildTok::One });
            } else {
                tokens.push(WildTok::Lit(c.to_ascii_lowercase()));
            }
        }
        Self { tokens }
    }

    /// Case-insensitive match.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        match_tokens(&self.tokens, text)
    }
}

fn match_tokens(tokens: &[WildTok], hay: &str) -> bool {
    let hay = hay
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut token = 0usize;
    let mut character = 0usize;
    let mut star = None;
    let mut star_character = 0usize;
    while character < hay.len() {
        match tokens.get(token) {
            Some(WildTok::One) => {
                token += 1;
                character += 1;
            }
            Some(WildTok::Lit(expected)) if *expected == hay[character] => {
                token += 1;
                character += 1;
            }
            Some(WildTok::Any) => {
                star = Some(token);
                token += 1;
                star_character = character;
            }
            _ => {
                let Some(star_token) = star else {
                    return false;
                };
                star_character += 1;
                character = star_character;
                token = star_token + 1;
            }
        }
    }
    tokens[token..]
        .iter()
        .all(|token| matches!(token, WildTok::Any))
}

/// Parse a criterion scalar.
pub fn parse_criteria(s: &Scalar) -> Result<Criteria, ErrorKind> {
    match s {
        Scalar::Number(_) | Scalar::Bool(_) | Scalar::Empty => Ok(Criteria::Cmp {
            op: CmpOp::Eq,
            rhs: s.clone(),
            pattern: None,
        }),
        Scalar::Error(e) => Ok(Criteria::Cmp {
            op: CmpOp::Eq,
            rhs: Scalar::Error(*e),
            pattern: None,
        }),
        Scalar::Text(t) => parse_criteria_text(t),
    }
}

fn parse_criteria_text(t: &str) -> Result<Criteria, ErrorKind> {
    let (op, rest) = if let Some(r) = t.strip_prefix("<>") {
        (CmpOp::Ne, r)
    } else if let Some(r) = t.strip_prefix(">=") {
        (CmpOp::Ge, r)
    } else if let Some(r) = t.strip_prefix("<=") {
        (CmpOp::Le, r)
    } else if let Some(r) = t.strip_prefix('>') {
        (CmpOp::Gt, r)
    } else if let Some(r) = t.strip_prefix('<') {
        (CmpOp::Lt, r)
    } else if let Some(r) = t.strip_prefix('=') {
        (CmpOp::Eq, r)
    } else {
        (CmpOp::Eq, t)
    };
    let rhs = if rest.is_empty() {
        Scalar::Empty
    } else if rest.eq_ignore_ascii_case("TRUE") {
        Scalar::Bool(true)
    } else if rest.eq_ignore_ascii_case("FALSE") {
        Scalar::Bool(false)
    } else if let Some(n) = coerce::parse_numeric_text(rest) {
        Scalar::Number(n)
    } else {
        Scalar::Text(Arc::from(rest))
    };
    let pattern = match &rhs {
        Scalar::Text(p)
            if matches!(op, CmpOp::Eq | CmpOp::Ne)
                && p.chars().any(|c| matches!(c, '*' | '?' | '~')) =>
        {
            Some(Wildcard::parse(p))
        }
        _ => None,
    };
    Ok(Criteria::Cmp { op, rhs, pattern })
}

/// Whether `value` satisfies `criteria`.
pub fn criteria_match(value: &Scalar, crit: &Criteria) -> bool {
    if let Scalar::Error(_) = value {
        return match crit {
            Criteria::Cmp {
                op: CmpOp::Eq,
                rhs: Scalar::Error(e),
                ..
            } => value.error() == Some(*e),
            Criteria::Cmp {
                op: CmpOp::Ne,
                rhs: Scalar::Error(e),
                ..
            } => value.error() != Some(*e),
            _ => false,
        };
    }
    let Criteria::Cmp { op, rhs, pattern } = crit;
    if let Some(pat) = pattern {
        let Scalar::Text(text) = value else {
            return matches!(op, CmpOp::Ne) && !matches!(value, Scalar::Empty);
        };
        let hit = pat.matches(text);
        return match op {
            CmpOp::Eq => hit,
            CmpOp::Ne => !hit,
            _ => false,
        };
    }

    match rhs {
        Scalar::Empty => {
            let blank = matches!(value, Scalar::Empty)
                || matches!(value, Scalar::Text(text) if text.is_empty());
            match op {
                CmpOp::Eq => blank,
                CmpOp::Ne => !blank,
                _ => false,
            }
        }
        Scalar::Number(_) => {
            let numeric_text;
            let comparable = match value {
                Scalar::Number(_) => value,
                Scalar::Text(text) => {
                    let trimmed = text.trim();
                    if trimmed.is_empty()
                        || trimmed.eq_ignore_ascii_case("TRUE")
                        || trimmed.eq_ignore_ascii_case("FALSE")
                    {
                        return matches!(op, CmpOp::Ne);
                    }
                    let Some(number) = coerce::parse_numeric_text(trimmed) else {
                        return matches!(op, CmpOp::Ne);
                    };
                    numeric_text = Scalar::Number(number);
                    &numeric_text
                }
                Scalar::Empty | Scalar::Bool(_) => return matches!(op, CmpOp::Ne),
                Scalar::Error(_) => return false,
            };
            coerce::compare_op(*op, comparable, rhs).unwrap_or(false)
        }
        Scalar::Bool(_) => match value {
            Scalar::Bool(_) => coerce::compare_op(*op, value, rhs).unwrap_or(false),
            Scalar::Empty | Scalar::Number(_) | Scalar::Text(_) => matches!(op, CmpOp::Ne),
            Scalar::Error(_) => false,
        },
        Scalar::Text(_) => match value {
            Scalar::Text(_) => coerce::compare_op(*op, value, rhs).unwrap_or(false),
            Scalar::Empty | Scalar::Number(_) | Scalar::Bool(_) => matches!(op, CmpOp::Ne),
            Scalar::Error(_) => false,
        },
        Scalar::Error(error) => match value {
            Scalar::Error(value_error) => match op {
                CmpOp::Eq => value_error == error,
                CmpOp::Ne => value_error != error,
                _ => false,
            },
            _ => matches!(op, CmpOp::Ne),
        },
    }
}

/// First reference in an argument (for CELL / ISREF / ISFORMULA).
#[must_use]
pub fn as_reference(value: &RuntimeValue) -> Option<&Reference> {
    match value {
        RuntimeValue::Ref(r) => Some(r),
        _ => None,
    }
}

/// Top-left cell of a reference.
#[must_use]
pub fn ref_origin(r: &Reference) -> Option<(SheetId, u32, u16)> {
    match r {
        Reference::Range {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        } => Some((
            *sheet,
            (*start_row).min(*end_row),
            (*start_col).min(*end_col),
        )),
        Reference::ThreeD {
            sheets,
            start_row,
            start_col,
            end_row,
            end_col,
        } => Some((
            *sheets.first()?,
            (*start_row).min(*end_row),
            (*start_col).min(*end_col),
        )),
        Reference::Union(v) => v.first().and_then(ref_origin),
    }
}

/// A1 address with `$` (CELL("address")).
#[must_use]
pub fn abs_a1(row: u32, col: u16) -> String {
    match col_to_letters(col) {
        Ok(letters) => format!("${}${}", letters, row + 1),
        Err(_) => "#REF!".to_string(),
    }
}

/// Whether a formula source is a `SUBTOTAL`/`AGGREGATE` call (nested skip).
#[must_use]
pub fn is_nested_aggregate(source: &str) -> bool {
    let t = source
        .trim_start_matches('=')
        .trim_start()
        .to_ascii_uppercase();
    t.starts_with("SUBTOTAL(") || t.starts_with("AGGREGATE(")
}

/// Collect paired numbers from two lists, skipping text/bool/empty pairs.
pub fn paired_numbers(
    ctx: &EvalCtx<'_>,
    x: &RuntimeValue,
    y: &RuntimeValue,
) -> Result<Vec<(f64, f64)>, ErrorKind> {
    let mut out = Vec::new();
    for_each_pair(ctx, x, y, &mut |a, b| {
        if let Some(e) = a.error().or(b.error()) {
            return Err(e);
        }
        match (&a, &b) {
            (Scalar::Number(xn), Scalar::Number(yn)) if xn.is_finite() && yn.is_finite() => {
                out.push((*xn, *yn));
            }
            (Scalar::Number(_), Scalar::Number(_)) => return Err(ErrorKind::Num),
            _ => {}
        }
        Ok(())
    })?;
    Ok(out)
}

/// Linear regression slope/intercept on paired numbers.
pub fn slope_intercept(pairs: &[(f64, f64)]) -> Result<(f64, f64), ErrorKind> {
    let n = pairs.len() as f64;
    if n < 2.0 {
        return Err(ErrorKind::Div0);
    }
    let sx: f64 = pairs.iter().map(|(x, _)| *x).sum();
    let sy: f64 = pairs.iter().map(|(_, y)| *y).sum();
    let sxx: f64 = pairs.iter().map(|(x, _)| x * x).sum();
    let sxy: f64 = pairs.iter().map(|(x, y)| x * y).sum();
    let denom = n * sxx - sx * sx;
    if denom == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let slope = (n * sxy - sx * sy) / denom;
    let intercept = (sy - slope * sx) / n;
    if slope.is_finite() && intercept.is_finite() {
        Ok((slope, intercept))
    } else {
        Err(ErrorKind::Num)
    }
}

/// Pearson r.
pub fn correl(pairs: &[(f64, f64)]) -> Result<f64, ErrorKind> {
    let n = pairs.len() as f64;
    if n < 2.0 {
        return Err(ErrorKind::Div0);
    }
    let sx: f64 = pairs.iter().map(|(x, _)| *x).sum();
    let sy: f64 = pairs.iter().map(|(_, y)| *y).sum();
    let sxx: f64 = pairs.iter().map(|(x, _)| x * x).sum();
    let syy: f64 = pairs.iter().map(|(_, y)| y * y).sum();
    let sxy: f64 = pairs.iter().map(|(x, y)| x * y).sum();
    let num = n * sxy - sx * sy;
    let den = ((n * sxx - sx * sx) * (n * syy - sy * sy)).sqrt();
    if den == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let r = num / den;
    if r.is_finite() {
        Ok(r)
    } else {
        Err(ErrorKind::Num)
    }
}

/// Population or sample covariance.
pub fn covariance(pairs: &[(f64, f64)], sample: bool) -> Result<f64, ErrorKind> {
    let n = pairs.len();
    let denom = if sample { n.saturating_sub(1) } else { n };
    if n == 0 || denom == 0 {
        return Err(ErrorKind::Div0);
    }
    let mx = pairs.iter().map(|(x, _)| *x).sum::<f64>() / n as f64;
    let my = pairs.iter().map(|(_, y)| *y).sum::<f64>() / n as f64;
    let acc: f64 = pairs.iter().map(|(x, y)| (x - mx) * (y - my)).sum();
    Ok(acc / denom as f64)
}

/// Register a spec and its aliases.
pub fn register_spec(registry: &mut omacell_core::eval::FnRegistry, spec: &crate::FunctionSpec) {
    registry.register(spec.to_fn_def());
    for alias in spec.aliases {
        let mut def = spec.to_fn_def();
        def.name = alias;
        registry.register(def);
    }
}

/// Register every spec in a slice.
pub fn register_specs(
    registry: &mut omacell_core::eval::FnRegistry,
    specs: &[crate::FunctionSpec],
) {
    for spec in specs {
        register_spec(registry, spec);
    }
}

#[cfg(test)]
mod tests {
    use super::{Wildcard, frequencies};

    #[test]
    fn frequency_table_handles_large_inputs_in_one_pass() {
        let values = (0..100_000)
            .map(|index| f64::from(index % 100))
            .collect::<Vec<_>>();
        let records = frequencies(&values);
        assert_eq!(records.len(), 100);
        assert!(records.iter().all(|(_, count, _)| *count == 1_000));
    }

    #[test]
    fn wildcard_matching_is_non_recursive_and_unicode_scalar_aware() {
        assert!(Wildcard::parse("?").matches("é"));
        let pattern = format!("{}b", "*".repeat(10_000));
        assert!(!Wildcard::parse(&pattern).matches(&"a".repeat(10_000)));
    }
}

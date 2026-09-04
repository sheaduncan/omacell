//! Financial core (WP-05c).
//!
//! Solver policy: Newton–Raphson; `RATE`/`IRR` at most 20 iterations;
//! `XIRR` at most 100; success when the normalized residual is below `1e-8`
//! or successive rate estimates differ by at most `1e-7`; otherwise `#NUM!`.
//! Default guess is `0.1`.

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};

use crate::args;
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Maximum Newton iterations for `RATE` and `IRR` (Excel).
pub const RATE_IRR_MAX_ITERS: u32 = 20;
/// Maximum Newton iterations for `XIRR`.
pub const XIRR_MAX_ITERS: u32 = 100;
/// Normalized residual that counts as convergence.
pub const SOLVER_TOL: f64 = 1e-8;
/// Maximum difference between successive rate estimates.
pub const SOLVER_RATE_TOL: f64 = 1e-7;
/// Default guess for iterative rate solvers.
pub const DEFAULT_GUESS: f64 = 0.1;

/// Financial specs in declaration order.
pub const SPECS: &[FunctionSpec] = &[
    PMT, IPMT, PPMT, NPV, XNPV, IRR, XIRR, MIRR, FV, PV, RATE, NPER, SLN, DB, DDB, SYD, EFFECT,
    NOMINAL, CUMIPMT, CUMPRINC,
];

/// Register financial functions.
pub fn register_financial(registry: &mut FnRegistry) {
    for spec in SPECS {
        args::register_spec(registry, spec);
    }
}

crate::define_fn! {
const PMT = {
    name: "PMT",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "PMT(rate, nper, pv, [fv], [type])",
    doc: "Payment for a loan with constant payments and a constant interest rate.",
    body: FnBody::Eager(pmt_impl),
};
}

crate::define_fn! {
const IPMT = {
    name: "IPMT",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 4,
    max_args: 6,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "IPMT(rate, per, nper, pv, [fv], [type])",
    doc: "Interest portion of a loan payment for a given period.",
    body: FnBody::Eager(ipmt_impl),
};
}

crate::define_fn! {
const PPMT = {
    name: "PPMT",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 4,
    max_args: 6,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "PPMT(rate, per, nper, pv, [fv], [type])",
    doc: "Principal portion of a loan payment for a given period.",
    body: FnBody::Eager(ppmt_impl),
};
}

crate::define_fn! {
const NPV = {
    name: "NPV",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Any],
    min_args: 2,
    max_args: 255,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "NPV(rate, value1, [value2], ...)",
    doc: "Net present value of cash flows at the end of periods 1..n.",
    body: FnBody::Eager(npv_impl),
};
}

crate::define_fn! {
const XNPV = {
    name: "XNPV",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Array, ArgKind::Array],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "XNPV(rate, values, dates)",
    doc: "Net present value of cash flows at irregular dates (365-day year).",
    body: FnBody::Eager(xnpv_impl),
};
}

crate::define_fn! {
const IRR = {
    name: "IRR",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Array, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "IRR(values, [guess])",
    doc: "Internal rate of return. Newton, 20 iterations, default guess 0.1, tol 1e-8.",
    body: FnBody::Eager(irr_impl),
};
}

crate::define_fn! {
const XIRR = {
    name: "XIRR",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Array, ArgKind::Array, ArgKind::Number],
    min_args: 2,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "XIRR(values, dates, [guess])",
    doc: "Internal rate of return for irregular dates. Newton, 100 iterations, tol 1e-8.",
    body: FnBody::Eager(xirr_impl),
};
}

crate::define_fn! {
const MIRR = {
    name: "MIRR",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Array, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "MIRR(values, finance_rate, reinvest_rate)",
    doc: "Modified internal rate of return.",
    body: FnBody::Eager(mirr_impl),
};
}

crate::define_fn! {
const FV = {
    name: "FV",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "FV(rate, nper, pmt, [pv], [type])",
    doc: "Future value of an investment with constant payments.",
    body: FnBody::Eager(fv_impl),
};
}

crate::define_fn! {
const PV = {
    name: "PV",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "PV(rate, nper, pmt, [fv], [type])",
    doc: "Present value of an investment with constant payments.",
    body: FnBody::Eager(pv_impl),
};
}

crate::define_fn! {
const RATE = {
    name: "RATE",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 6,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "RATE(nper, pmt, pv, [fv], [type], [guess])",
    doc: "Interest rate per period. Newton, 20 iterations, default guess 0.1, tol 1e-8.",
    body: FnBody::Eager(rate_impl),
};
}

crate::define_fn! {
const NPER = {
    name: "NPER",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "NPER(rate, pmt, pv, [fv], [type])",
    doc: "Number of periods for an investment with constant payments.",
    body: FnBody::Eager(nper_impl),
};
}

crate::define_fn! {
const SLN = {
    name: "SLN",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "SLN(cost, salvage, life)",
    doc: "Straight-line depreciation for one period.",
    body: FnBody::Eager(sln_impl),
};
}

crate::define_fn! {
const DB = {
    name: "DB",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 4,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "DB(cost, salvage, life, period, [month])",
    doc: "Fixed-declining balance depreciation.",
    body: FnBody::Eager(db_impl),
};
}

crate::define_fn! {
const DDB = {
    name: "DDB",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 4,
    max_args: 5,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "DDB(cost, salvage, life, period, [factor])",
    doc: "Double-declining balance depreciation, clamped at salvage.",
    body: FnBody::Eager(ddb_impl),
};
}

crate::define_fn! {
const SYD = {
    name: "SYD",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 4,
    max_args: 4,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "SYD(cost, salvage, life, per)",
    doc: "Sum-of-years-digits depreciation.",
    body: FnBody::Eager(syd_impl),
};
}

crate::define_fn! {
const EFFECT = {
    name: "EFFECT",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "EFFECT(nominal_rate, npery)",
    doc: "Effective annual interest rate.",
    body: FnBody::Eager(effect_impl),
};
}

crate::define_fn! {
const NOMINAL = {
    name: "NOMINAL",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "NOMINAL(effect_rate, npery)",
    doc: "Nominal annual interest rate.",
    body: FnBody::Eager(nominal_impl),
};
}

crate::define_fn! {
const CUMIPMT = {
    name: "CUMIPMT",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 6,
    max_args: 6,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "CUMIPMT(rate, nper, pv, start_period, end_period, type)",
    doc: "Cumulative interest between two periods. `pv` is a positive loan amount; result is negative.",
    body: FnBody::Eager(cumipmt_impl),
};
}

crate::define_fn! {
const CUMPRINC = {
    name: "CUMPRINC",
    aliases: &[],
    tier: 0,
    category: "financial",
    arg_kinds: &[ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number, ArgKind::Number],
    min_args: 6,
    max_args: 6,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "CUMPRINC(rate, nper, pv, start_period, end_period, type)",
    doc: "Cumulative principal between two periods. `pv` is a positive loan amount; result is negative.",
    body: FnBody::Eager(cumprinc_impl),
};
}

fn err(e: ErrorKind) -> RuntimeValue {
    RuntimeValue::error(e)
}

fn num(n: f64) -> RuntimeValue {
    if n.is_finite() {
        RuntimeValue::Scalar(Scalar::Number(n))
    } else {
        err(ErrorKind::Num)
    }
}

fn type_flag(n: f64) -> Result<f64, ErrorKind> {
    let t = args::trunc_i64(n)?;
    if t == 0 {
        Ok(0.0)
    } else if t == 1 {
        Ok(1.0)
    } else {
        Err(ErrorKind::Num)
    }
}

fn pmt_value(rate: f64, nper: f64, pv: f64, fv: f64, typ: f64) -> Result<f64, ErrorKind> {
    if nper == 0.0 {
        return Err(ErrorKind::Num);
    }
    if rate == 0.0 {
        return Ok(-(pv + fv) / nper);
    }
    let pow = (1.0 + rate).powf(nper);
    if !pow.is_finite() {
        return Err(ErrorKind::Num);
    }
    let denom = (1.0 + rate * typ) * (pow - 1.0);
    if denom == 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok(-(rate * (pv * pow + fv)) / denom)
}

fn fv_value(rate: f64, nper: f64, pmt: f64, pv: f64, typ: f64) -> Result<f64, ErrorKind> {
    if rate == 0.0 {
        return Ok(-(pv + pmt * nper));
    }
    let pow = (1.0 + rate).powf(nper);
    if !pow.is_finite() {
        return Err(ErrorKind::Num);
    }
    Ok(-pv * pow - pmt * (1.0 + rate * typ) * (pow - 1.0) / rate)
}

fn pv_value(rate: f64, nper: f64, pmt: f64, fv: f64, typ: f64) -> Result<f64, ErrorKind> {
    if rate == 0.0 {
        return Ok(-(fv + pmt * nper));
    }
    let pow = (1.0 + rate).powf(nper);
    if !pow.is_finite() || pow == 0.0 {
        return Err(ErrorKind::Num);
    }
    Ok((-(fv) - pmt * (1.0 + rate * typ) * (pow - 1.0) / rate) / pow)
}

fn ipmt_value(
    rate: f64,
    per: f64,
    nper: f64,
    pv: f64,
    fv: f64,
    typ: f64,
) -> Result<f64, ErrorKind> {
    if per < 1.0 || per > nper {
        return Err(ErrorKind::Num);
    }
    if typ == 1.0 && per == 1.0 {
        return Ok(0.0);
    }
    let pmt = pmt_value(rate, nper, pv, fv, typ)?;
    let prior_periods = if typ == 1.0 { per - 2.0 } else { per - 1.0 };
    let remaining = fv_value(rate, prior_periods, pmt, pv, typ)?;
    let post_payment = if typ == 1.0 {
        remaining - pmt
    } else {
        remaining
    };
    Ok(post_payment * rate)
}

fn pmt_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    tvm(ctx, args, Tvm::Pmt)
}

fn fv_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    tvm(ctx, args, Tvm::Fv)
}

fn pv_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    tvm(ctx, args, Tvm::Pv)
}

enum Tvm {
    Pmt,
    Fv,
    Pv,
}

fn tvm(ctx: &mut EvalCtx<'_>, args: &[ArgVal], kind: Tvm) -> RuntimeValue {
    let rate = match args
        .first()
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let nper = match args
        .get(1)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let third = match args
        .get(2)
        .ok_or(ErrorKind::Value)
        .and_then(|a| args::number(ctx, a))
    {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let fourth = match args::opt_number(ctx, args, 3, 0.0) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let typ = match args::opt_number(ctx, args, 4, 0.0).and_then(type_flag) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let r = match kind {
        Tvm::Pmt => pmt_value(rate, nper, third, fourth, typ),
        Tvm::Fv => fv_value(rate, nper, third, fourth, typ),
        Tvm::Pv => pv_value(rate, nper, third, fourth, typ),
    };
    match r {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn ipmt_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let rate = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let per = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let nper = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let pv = args::number(ctx, args.get(3).ok_or(ErrorKind::Value)?)?;
        let fv = args::opt_number(ctx, args, 4, 0.0)?;
        let typ = type_flag(args::opt_number(ctx, args, 5, 0.0)?)?;
        ipmt_value(rate, per, nper, pv, fv, typ)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn ppmt_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let rate = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let per = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let nper = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let pv = args::number(ctx, args.get(3).ok_or(ErrorKind::Value)?)?;
        let fv = args::opt_number(ctx, args, 4, 0.0)?;
        let typ = type_flag(args::opt_number(ctx, args, 5, 0.0)?)?;
        let pmt = pmt_value(rate, nper, pv, fv, typ)?;
        let ipmt = ipmt_value(rate, per, nper, pv, fv, typ)?;
        Ok(pmt - ipmt)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn collect_flows(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    skip: usize,
) -> Result<Vec<f64>, ErrorKind> {
    let mut out = Vec::new();
    for a in args.iter().skip(skip) {
        if a.omitted {
            continue;
        }
        if let Some(e) = a.value.error_kind() {
            return Err(e);
        }
        let array = args::arg_array(ctx, a)?;
        for s in array.values.iter() {
            if let Some(e) = s.error() {
                return Err(e);
            }
            if matches!(s, Scalar::Empty | Scalar::Text(_)) {
                continue;
            }
            out.push(coerce::to_number(s)?);
        }
    }
    Ok(out)
}

fn npv_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let rate = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        if rate == -1.0 {
            return Err(ErrorKind::Num);
        }
        let flows = collect_flows(ctx, args, 1)?;
        let mut acc = 0.0;
        for (i, v) in flows.iter().enumerate() {
            acc += v / (1.0 + rate).powf((i + 1) as f64);
        }
        Ok(acc)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn collect_pair(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    vi: usize,
    di: usize,
) -> Result<(Vec<f64>, Vec<f64>), ErrorKind> {
    let values = match args.get(vi) {
        Some(a) => collect_flows(ctx, std::slice::from_ref(a), 0)?,
        None => return Err(ErrorKind::Value),
    };
    let dates = match args.get(di) {
        Some(a) => collect_flows(ctx, std::slice::from_ref(a), 0)?,
        None => return Err(ErrorKind::Value),
    };
    if values.len() != dates.len() || values.is_empty() {
        return Err(ErrorKind::Num);
    }
    Ok((values, dates))
}

fn xnpv_of(rate: f64, values: &[f64], dates: &[f64]) -> Result<f64, ErrorKind> {
    if rate <= -1.0 {
        return Err(ErrorKind::Num);
    }
    let d0 = dates[0];
    let mut acc = 0.0;
    for (v, d) in values.iter().zip(dates.iter()) {
        if *d < d0 {
            return Err(ErrorKind::Num);
        }
        acc += v / (1.0 + rate).powf((*d - d0) / 365.0);
    }
    Ok(acc)
}

fn xnpv_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let rate = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let (values, dates) = collect_pair(ctx, args, 1, 2)?;
        let n = xnpv_of(rate, &values, &dates)?;
        Ok(if n.abs() < 1e-12 { 0.0 } else { n })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn npv_poly(rate: f64, values: &[f64]) -> f64 {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| v / (1.0 + rate).powf(i as f64))
        .sum()
}

fn npv_deriv(rate: f64, values: &[f64]) -> f64 {
    values
        .iter()
        .enumerate()
        .skip(1)
        .map(|(i, v)| -f64::from(i as u32) * v / (1.0 + rate).powf(i as f64 + 1.0))
        .sum()
}

fn newton_rate<F, G>(
    guess: f64,
    max_iters: u32,
    scale: f64,
    mut f: F,
    mut df: G,
) -> Result<f64, ErrorKind>
where
    F: FnMut(f64) -> f64,
    G: FnMut(f64) -> f64,
{
    let residual_tolerance = SOLVER_TOL * scale.max(1.0);
    let mut x = guess;
    for _ in 0..max_iters {
        let fx = f(x);
        if !fx.is_finite() {
            return Err(ErrorKind::Num);
        }
        if fx == 0.0 {
            return Ok(x);
        }
        let d = df(x);
        if !d.is_finite() || d == 0.0 {
            return Err(ErrorKind::Num);
        }
        let next = x - fx / d;
        if !next.is_finite() {
            return Err(ErrorKind::Num);
        }
        if fx.abs() <= residual_tolerance || (next - x).abs() <= SOLVER_RATE_TOL {
            return Ok(next);
        }
        x = next;
    }
    let fx = f(x);
    if fx.is_finite() && fx.abs() <= residual_tolerance {
        Ok(x)
    } else {
        Err(ErrorKind::Num)
    }
}

fn cashflow_scale(values: &[f64]) -> f64 {
    values.iter().map(|value| value.abs()).fold(1.0, f64::max)
}

fn irr_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let values = collect_flows(ctx, std::slice::from_ref(first), 0)?;
        let guess = args::opt_number(ctx, args, 1, DEFAULT_GUESS)?;
        let r = newton_rate(
            guess,
            RATE_IRR_MAX_ITERS,
            cashflow_scale(&values),
            |r| npv_poly(r, &values),
            |r| npv_deriv(r, &values),
        )?;
        Ok(if r.abs() < 1e-12 { 0.0 } else { r })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn xirr_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let (values, dates) = collect_pair(ctx, args, 0, 1)?;
        let guess = args::opt_number(ctx, args, 2, DEFAULT_GUESS)?;
        let d0 = dates[0];
        if dates.windows(2).all(|w| w[0] == w[1]) {
            let sum: f64 = values.iter().sum();
            return if sum.abs() < SOLVER_TOL {
                Ok(0.0)
            } else {
                Err(ErrorKind::Num)
            };
        }
        let r = newton_rate(
            guess,
            XIRR_MAX_ITERS,
            cashflow_scale(&values),
            |r| xnpv_of(r, &values, &dates).unwrap_or(f64::NAN),
            |r| {
                if r <= -1.0 {
                    return f64::NAN;
                }
                values
                    .iter()
                    .zip(dates.iter())
                    .map(|(v, d)| {
                        let t = (*d - d0) / 365.0;
                        -t * v / (1.0 + r).powf(t + 1.0)
                    })
                    .sum()
            },
        )?;
        Ok(if r.abs() < 1e-12 { 0.0 } else { r })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn mirr_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let values = collect_flows(ctx, std::slice::from_ref(first), 0)?;
        let finance = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let reinvest = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        if finance == -1.0 || reinvest == -1.0 {
            return Err(ErrorKind::Num);
        }
        let n = values.len();
        if n < 2 {
            return Err(ErrorKind::Num);
        }
        let mut neg_pv = 0.0;
        let mut pos_fv = 0.0;
        for (i, v) in values.iter().enumerate() {
            if *v < 0.0 {
                neg_pv += v / (1.0 + finance).powf(i as f64);
            } else if *v > 0.0 {
                pos_fv += v * (1.0 + reinvest).powf((n - 1 - i) as f64);
            }
        }
        if neg_pv == 0.0 || pos_fv == 0.0 {
            return Err(ErrorKind::Div0);
        }
        let ratio = pos_fv / -neg_pv;
        if ratio <= 0.0 {
            return Err(ErrorKind::Num);
        }
        Ok(ratio.powf(1.0 / (n as f64 - 1.0)) - 1.0)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn annuity_eq(rate: f64, nper: f64, pmt: f64, pv: f64, fv: f64, typ: f64) -> f64 {
    if rate == 0.0 {
        return pv + pmt * nper + fv;
    }
    let pow = (1.0 + rate).powf(nper);
    pv * pow + pmt * (1.0 + rate * typ) * (pow - 1.0) / rate + fv
}

fn annuity_d_rate(rate: f64, nper: f64, pmt: f64, pv: f64, fv: f64, typ: f64) -> f64 {
    // numerical derivative fallback is used when rate is near 0
    let h = 1e-8;
    (annuity_eq(rate + h, nper, pmt, pv, fv, typ) - annuity_eq(rate, nper, pmt, pv, fv, typ)) / h
}

fn rate_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let nper = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let pmt = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let pv = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let fv = args::opt_number(ctx, args, 3, 0.0)?;
        let typ = type_flag(args::opt_number(ctx, args, 4, 0.0)?)?;
        let guess = args::opt_number(ctx, args, 5, DEFAULT_GUESS)?;
        if nper == 0.0 {
            return Err(ErrorKind::Num);
        }
        if pv + pmt * nper + fv == 0.0 {
            return Ok(0.0);
        }
        let scale = pmt.abs().max(pv.abs()).max(fv.abs());
        let r = newton_rate(
            guess,
            RATE_IRR_MAX_ITERS,
            scale,
            |r| annuity_eq(r, nper, pmt, pv, fv, typ),
            |r| annuity_d_rate(r, nper, pmt, pv, fv, typ),
        )?;
        Ok(if r.abs() < 1e-12 { 0.0 } else { r })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn nper_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let rate = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let pmt = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let pv = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let fv = args::opt_number(ctx, args, 3, 0.0)?;
        let typ = type_flag(args::opt_number(ctx, args, 4, 0.0)?)?;
        if rate == -1.0 {
            return Err(ErrorKind::Num);
        }
        if rate == 0.0 {
            if pmt == 0.0 {
                return Err(ErrorKind::Num);
            }
            return Ok(-(pv + fv) / pmt);
        }
        let a = pmt * (1.0 + rate * typ) - fv * rate;
        let b = pv * rate + pmt * (1.0 + rate * typ);
        if a == 0.0 || b == 0.0 || a.signum() == b.signum() && a.abs() / b.abs() <= 0.0 {
            // fall through
        }
        let inner = a / b;
        if inner <= 0.0 {
            return Err(ErrorKind::Num);
        }
        Ok((inner.ln()) / (1.0 + rate).ln())
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn sln_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let cost = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let salvage = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let life = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        if life == 0.0 {
            return Err(ErrorKind::Div0);
        }
        Ok((cost - salvage) / life)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn excel_round3(n: f64) -> f64 {
    (n * 1000.0).round() / 1000.0
}

fn db_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let cost = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let salvage = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let life = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let period = args::number(ctx, args.get(3).ok_or(ErrorKind::Value)?)?;
        let month = args::opt_number(ctx, args, 4, 12.0)?;
        let month_i = args::trunc_i64(month)?;
        if !(1..=12).contains(&month_i) || life <= 0.0 || period < 1.0 {
            return Err(ErrorKind::Num);
        }
        let rate = if cost == 0.0 {
            0.0
        } else {
            excel_round3(1.0 - (salvage / cost).powf(1.0 / life))
        };
        let last = if month_i == 12 { life } else { life + 1.0 };
        if period > last {
            return Err(ErrorKind::Num);
        }
        let periods = args::trunc_i64(period)?;
        let mut first_dep = cost * rate * (month_i as f64) / 12.0;
        if first_dep > cost - salvage && salvage < cost {
            first_dep = (cost - salvage).max(0.0);
        }
        if periods == 1 {
            return Ok(first_dep);
        }
        let first_book = cost - first_dep;
        let mut book = first_book * (1.0 - rate).powf((periods - 2) as f64);
        if salvage < first_book {
            book = book.max(salvage);
        }
        let mut dep = if periods as f64 <= life {
            book * rate
        } else {
            (book * rate * (12.0 - month_i as f64) / 12.0).max(0.0)
        };
        if dep > book - salvage && salvage < book {
            dep = (book - salvage).max(0.0);
        }
        Ok(dep)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn ddb_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let cost = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let salvage = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let life = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let period = args::number(ctx, args.get(3).ok_or(ErrorKind::Value)?)?;
        let factor = args::opt_number(ctx, args, 4, 2.0)?;
        if life <= 0.0 || period < 1.0 || period > life || factor <= 0.0 {
            return Err(ErrorKind::Num);
        }
        let n = args::trunc_i64(period)?;
        let rate = factor / life;
        let first_dep = (cost * rate).min((cost - salvage).max(0.0));
        let first_book = cost - first_dep;
        let book = if n == 1 {
            cost
        } else if first_book <= salvage || 1.0 - rate <= 0.0 {
            first_book
        } else {
            (first_book * (1.0 - rate).powf((n - 2) as f64)).max(salvage)
        };
        let dep = (book * rate).min((book - salvage).max(0.0));
        Ok(dep)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn syd_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let cost = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let salvage = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let life = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let per = args::number(ctx, args.get(3).ok_or(ErrorKind::Value)?)?;
        if life <= 0.0 || per < 1.0 || per > life {
            return Err(ErrorKind::Num);
        }
        let denom = life * (life + 1.0) / 2.0;
        if denom == 0.0 {
            return Err(ErrorKind::Num);
        }
        Ok((cost - salvage) * (life - per + 1.0) / denom)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn effect_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let nominal = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let npery = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let n = args::trunc_i64(npery)?;
        if nominal <= 0.0 || n < 1 {
            return Err(ErrorKind::Num);
        }
        Ok((n as f64 * (nominal / n as f64).ln_1p()).exp_m1())
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn nominal_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let effective = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let npery = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let n = args::trunc_i64(npery)?;
        if effective <= 0.0 || n < 1 {
            return Err(ErrorKind::Num);
        }
        Ok(n as f64 * ((1.0 + effective).powf(1.0 / n as f64) - 1.0))
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn cumipmt_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    cum(ctx, args, true)
}

fn cumprinc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    cum(ctx, args, false)
}

fn signed_balance_after_payments(
    rate: f64,
    payments: i64,
    pmt: f64,
    pv: f64,
    typ: f64,
) -> Result<f64, ErrorKind> {
    if payments == 0 {
        return Ok(-pv);
    }
    let elapsed = if typ == 1.0 {
        (payments - 1) as f64
    } else {
        payments as f64
    };
    let balance = fv_value(rate, elapsed, pmt, pv, typ)?;
    Ok(if typ == 1.0 { balance - pmt } else { balance })
}

fn cum(ctx: &mut EvalCtx<'_>, args: &[ArgVal], interest: bool) -> RuntimeValue {
    match (|| {
        let rate = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let nper = args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?;
        let pv = args::number(ctx, args.get(2).ok_or(ErrorKind::Value)?)?;
        let start = args::number(ctx, args.get(3).ok_or(ErrorKind::Value)?)?;
        let end = args::number(ctx, args.get(4).ok_or(ErrorKind::Value)?)?;
        let typ = type_flag(args::number(ctx, args.get(5).ok_or(ErrorKind::Value)?)?)?;
        if rate <= 0.0 || nper <= 0.0 || pv <= 0.0 || start < 1.0 || end > nper || start > end {
            return Err(ErrorKind::Num);
        }
        let s = args::trunc_i64(start)?;
        let e = args::trunc_i64(end)?;
        let pmt = pmt_value(rate, nper, pv, 0.0, typ)?;
        let before = signed_balance_after_payments(rate, s - 1, pmt, pv, typ)?;
        let after = signed_balance_after_payments(rate, e, pmt, pv, typ)?;
        let principal = before - after;
        let result = if interest {
            pmt * (e - s + 1) as f64 - principal
        } else {
            principal
        };
        Ok(if result.abs() < 1e-12 { 0.0 } else { result })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

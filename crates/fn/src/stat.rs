//! Descriptive statistics (WP-05a).

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, RuntimeValue};

use crate::common::{
    arg_number, collect_numbers, collect_numbers_a, correl, count_args, counta_args,
    countblank_args, covariance, median, paired_numbers, percentile_exc, percentile_inc,
    register_specs, rt_num, slope_intercept, sorted, stdev, variance,
};
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Statistical specs (compatibility names are aliases).
pub const SPECS: &[FunctionSpec] = &[
    AVEDEV,
    AVERAGE,
    AVERAGEA,
    CORREL,
    COUNT,
    COUNTA,
    COUNTBLANK,
    COVARIANCE_P,
    COVARIANCE_S,
    DEVSQ,
    FORECAST_LINEAR,
    FREQUENCY,
    GEOMEAN,
    HARMEAN,
    INTERCEPT,
    KURT,
    LARGE,
    MAX,
    MAXA,
    MEDIAN,
    MIN,
    MINA,
    MODE_MULT,
    MODE_SNGL,
    PEARSON,
    PERCENTILE_EXC,
    PERCENTILE_INC,
    PERCENTRANK_EXC,
    PERCENTRANK_INC,
    QUARTILE_EXC,
    QUARTILE_INC,
    RANK_AVG,
    RANK_EQ,
    RSQ,
    SKEW,
    SKEW_P,
    SLOPE,
    SMALL,
    STANDARDIZE,
    STDEV_P,
    STDEV_S,
    STDEVA,
    STDEVPA,
    TRIMMEAN,
    VAR_P,
    VAR_S,
    VARA,
    VARPA,
];

/// Register statistical functions.
pub fn register_stat(registry: &mut omacell_core::eval::FnRegistry) {
    register_specs(registry, SPECS);
}

macro_rules! st {
    ($id:ident, $name:expr, $args:expr, $min:expr, $max:expr, $arr:expr, $sig:expr, $doc:expr, $body:expr $(, $alias:expr)*) => {
        crate::define_fn! {
        const $id = {
            name: $name,
            aliases: &[$($alias),*],
            tier: 0,
            category: "statistical",
            arg_kinds: $args,
            min_args: $min,
            max_args: $max,
            volatile: false,
            array: $arr,
            async_node: false,
            signature: $sig,
            doc: $doc,
            body: $body,
        };
        }
    };
}

st!(
    AVERAGE,
    "AVERAGE",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "AVERAGE(number1, [number2], ...)",
    "Arithmetic mean of numbers.",
    FnBody::Eager(average_impl)
);
st!(
    AVERAGEA,
    "AVERAGEA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "AVERAGEA(value1, [value2], ...)",
    "Mean including text as 0 and logicals as 0/1.",
    FnBody::Eager(averagea_impl)
);
st!(
    MEDIAN,
    "MEDIAN",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "MEDIAN(number1, [number2], ...)",
    "Median of numbers.",
    FnBody::Eager(median_impl)
);
st!(
    MODE_SNGL,
    "MODE.SNGL",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "MODE.SNGL(number1, [number2], ...)",
    "First most frequent number.",
    FnBody::Eager(mode_sngl_impl),
    "MODE"
);
st!(
    MODE_MULT,
    "MODE.MULT",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::ReturnsArray,
    "MODE.MULT(number1, [number2], ...)",
    "Vertical array of all modes.",
    FnBody::Eager(mode_mult_impl)
);
st!(
    MIN,
    "MIN",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "MIN(number1, [number2], ...)",
    "Smallest number.",
    FnBody::Eager(min_impl)
);
st!(
    MINA,
    "MINA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "MINA(value1, [value2], ...)",
    "Smallest value including logicals.",
    FnBody::Eager(mina_impl)
);
st!(
    MAX,
    "MAX",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "MAX(number1, [number2], ...)",
    "Largest number.",
    FnBody::Eager(max_impl)
);
st!(
    MAXA,
    "MAXA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "MAXA(value1, [value2], ...)",
    "Largest value including logicals.",
    FnBody::Eager(maxa_impl)
);
st!(
    LARGE,
    "LARGE",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "LARGE(array, k)",
    "k-th largest number.",
    FnBody::Eager(large_impl)
);
st!(
    SMALL,
    "SMALL",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "SMALL(array, k)",
    "k-th smallest number.",
    FnBody::Eager(small_impl)
);
st!(
    COUNT,
    "COUNT",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "COUNT(value1, [value2], ...)",
    "Count of numbers.",
    FnBody::Eager(count_impl)
);
st!(
    COUNTA,
    "COUNTA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "COUNTA(value1, [value2], ...)",
    "Count of non-empty values.",
    FnBody::Eager(counta_impl)
);
st!(
    COUNTBLANK,
    "COUNTBLANK",
    &[ArgKind::Range],
    1,
    255,
    ArrayBehavior::None,
    "COUNTBLANK(range)",
    "Count of blank cells.",
    FnBody::Eager(countblank_impl)
);
st!(
    STDEV_S,
    "STDEV.S",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "STDEV.S(number1, [number2], ...)",
    "Sample standard deviation.",
    FnBody::Eager(stdev_s_impl),
    "STDEV"
);
st!(
    STDEV_P,
    "STDEV.P",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "STDEV.P(number1, [number2], ...)",
    "Population standard deviation.",
    FnBody::Eager(stdev_p_impl),
    "STDEVP"
);
st!(
    STDEVA,
    "STDEVA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "STDEVA(value1, [value2], ...)",
    "Sample standard deviation including text/logicals.",
    FnBody::Eager(stdeva_impl)
);
st!(
    STDEVPA,
    "STDEVPA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "STDEVPA(value1, [value2], ...)",
    "Population standard deviation including text/logicals.",
    FnBody::Eager(stdevpa_impl)
);
st!(
    VAR_S,
    "VAR.S",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "VAR.S(number1, [number2], ...)",
    "Sample variance.",
    FnBody::Eager(var_s_impl),
    "VAR"
);
st!(
    VAR_P,
    "VAR.P",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "VAR.P(number1, [number2], ...)",
    "Population variance.",
    FnBody::Eager(var_p_impl),
    "VARP"
);
st!(
    VARA,
    "VARA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "VARA(value1, [value2], ...)",
    "Sample variance including text/logicals.",
    FnBody::Eager(vara_impl)
);
st!(
    VARPA,
    "VARPA",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "VARPA(value1, [value2], ...)",
    "Population variance including text/logicals.",
    FnBody::Eager(varpa_impl)
);
st!(
    RANK_EQ,
    "RANK.EQ",
    &[ArgKind::Number, ArgKind::Range, ArgKind::Number],
    2,
    3,
    ArrayBehavior::None,
    "RANK.EQ(number, ref, [order])",
    "Rank with ties getting the same rank.",
    FnBody::Eager(rank_eq_impl),
    "RANK"
);
st!(
    RANK_AVG,
    "RANK.AVG",
    &[ArgKind::Number, ArgKind::Range, ArgKind::Number],
    2,
    3,
    ArrayBehavior::None,
    "RANK.AVG(number, ref, [order])",
    "Rank with ties getting the average rank.",
    FnBody::Eager(rank_avg_impl)
);
st!(
    PERCENTILE_INC,
    "PERCENTILE.INC",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "PERCENTILE.INC(array, k)",
    "Inclusive percentile.",
    FnBody::Eager(percentile_inc_impl),
    "PERCENTILE"
);
st!(
    PERCENTILE_EXC,
    "PERCENTILE.EXC",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "PERCENTILE.EXC(array, k)",
    "Exclusive percentile.",
    FnBody::Eager(percentile_exc_impl)
);
st!(
    PERCENTRANK_INC,
    "PERCENTRANK.INC",
    &[ArgKind::Array, ArgKind::Number, ArgKind::Number],
    2,
    3,
    ArrayBehavior::None,
    "PERCENTRANK.INC(array, x, [significance])",
    "Inclusive percent rank.",
    FnBody::Eager(percentrank_inc_impl),
    "PERCENTRANK"
);
st!(
    PERCENTRANK_EXC,
    "PERCENTRANK.EXC",
    &[ArgKind::Array, ArgKind::Number, ArgKind::Number],
    2,
    3,
    ArrayBehavior::None,
    "PERCENTRANK.EXC(array, x, [significance])",
    "Exclusive percent rank.",
    FnBody::Eager(percentrank_exc_impl)
);
st!(
    QUARTILE_INC,
    "QUARTILE.INC",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "QUARTILE.INC(array, quart)",
    "Inclusive quartile (0–4).",
    FnBody::Eager(quartile_inc_impl),
    "QUARTILE"
);
st!(
    QUARTILE_EXC,
    "QUARTILE.EXC",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "QUARTILE.EXC(array, quart)",
    "Exclusive quartile (1–3).",
    FnBody::Eager(quartile_exc_impl)
);
st!(
    CORREL,
    "CORREL",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "CORREL(array1, array2)",
    "Pearson correlation coefficient.",
    FnBody::Eager(correl_impl)
);
st!(
    PEARSON,
    "PEARSON",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "PEARSON(array1, array2)",
    "Alias of CORREL.",
    FnBody::Eager(correl_impl)
);
st!(
    COVARIANCE_P,
    "COVARIANCE.P",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "COVARIANCE.P(array1, array2)",
    "Population covariance.",
    FnBody::Eager(covar_p_impl),
    "COVAR"
);
st!(
    COVARIANCE_S,
    "COVARIANCE.S",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "COVARIANCE.S(array1, array2)",
    "Sample covariance.",
    FnBody::Eager(covar_s_impl)
);
st!(
    SLOPE,
    "SLOPE",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "SLOPE(known_y's, known_x's)",
    "Slope of linear regression.",
    FnBody::Eager(slope_impl)
);
st!(
    INTERCEPT,
    "INTERCEPT",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "INTERCEPT(known_y's, known_x's)",
    "Intercept of linear regression.",
    FnBody::Eager(intercept_impl)
);
st!(
    RSQ,
    "RSQ",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "RSQ(known_y's, known_x's)",
    "Square of Pearson r.",
    FnBody::Eager(rsq_impl)
);
st!(
    FORECAST_LINEAR,
    "FORECAST.LINEAR",
    &[ArgKind::Number, ArgKind::Array, ArgKind::Array],
    3,
    3,
    ArrayBehavior::None,
    "FORECAST.LINEAR(x, known_y's, known_x's)",
    "Linear forecast at x.",
    FnBody::Eager(forecast_impl),
    "FORECAST"
);
st!(
    GEOMEAN,
    "GEOMEAN",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "GEOMEAN(number1, [number2], ...)",
    "Geometric mean.",
    FnBody::Eager(geomean_impl)
);
st!(
    HARMEAN,
    "HARMEAN",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "HARMEAN(number1, [number2], ...)",
    "Harmonic mean.",
    FnBody::Eager(harmean_impl)
);
st!(
    TRIMMEAN,
    "TRIMMEAN",
    &[ArgKind::Array, ArgKind::Number],
    2,
    2,
    ArrayBehavior::None,
    "TRIMMEAN(array, percent)",
    "Mean excluding a fraction of tails.",
    FnBody::Eager(trimmean_impl)
);
st!(
    DEVSQ,
    "DEVSQ",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "DEVSQ(number1, [number2], ...)",
    "Sum of squared deviations from the mean.",
    FnBody::Eager(devsq_impl)
);
st!(
    AVEDEV,
    "AVEDEV",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "AVEDEV(number1, [number2], ...)",
    "Average of absolute deviations from the mean.",
    FnBody::Eager(avedev_impl)
);
st!(
    SKEW,
    "SKEW",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "SKEW(number1, [number2], ...)",
    "Sample skewness.",
    FnBody::Eager(skew_impl)
);
st!(
    SKEW_P,
    "SKEW.P",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "SKEW.P(number1, [number2], ...)",
    "Population skewness.",
    FnBody::Eager(skewp_impl)
);
st!(
    KURT,
    "KURT",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "KURT(number1, [number2], ...)",
    "Sample excess kurtosis.",
    FnBody::Eager(kurt_impl)
);
st!(
    FREQUENCY,
    "FREQUENCY",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::ReturnsArray,
    "FREQUENCY(data_array, bins_array)",
    "Vertical frequency array (bins+1).",
    FnBody::Eager(frequency_impl)
);
st!(
    STANDARDIZE,
    "STANDARDIZE",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    3,
    3,
    ArrayBehavior::LiftAll,
    "STANDARDIZE(x, mean, standard_dev)",
    "(x − mean) / standard_dev.",
    FnBody::Eager(standardize_impl)
);

fn nums(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<Vec<f64>, ErrorKind> {
    collect_numbers(ctx, args)
}
fn numsa(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<Vec<f64>, ErrorKind> {
    collect_numbers_a(ctx, args)
}
fn map_nums(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    f: impl FnOnce(&[f64]) -> Result<f64, ErrorKind>,
) -> RuntimeValue {
    match nums(ctx, args).and_then(|v| f(&v)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn map_numsa(
    ctx: &EvalCtx<'_>,
    args: &[ArgVal],
    f: impl FnOnce(&[f64]) -> Result<f64, ErrorKind>,
) -> RuntimeValue {
    match numsa(ctx, args).and_then(|v| f(&v)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn mean(v: &[f64]) -> Result<f64, ErrorKind> {
    if v.is_empty() {
        Err(ErrorKind::Div0)
    } else {
        Ok(v.iter().sum::<f64>() / v.len() as f64)
    }
}
fn average_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, mean)
}
fn averagea_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, mean)
}
fn median_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, median)
}
fn min_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| Ok(min_or_zero(v)))
}
fn max_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| Ok(max_or_zero(v)))
}
fn mina_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, |v| Ok(min_or_zero(v)))
}
fn maxa_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, |v| Ok(max_or_zero(v)))
}
fn min_or_zero(values: &[f64]) -> f64 {
    values.iter().copied().reduce(f64::min).unwrap_or(0.0)
}
fn max_or_zero(values: &[f64]) -> f64 {
    values.iter().copied().reduce(f64::max).unwrap_or(0.0)
}
fn count_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match count_args(ctx, args) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn counta_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match counta_args(ctx, args) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn countblank_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match countblank_args(ctx, args) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn stdev_s_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| stdev(v, true))
}
fn stdev_p_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| stdev(v, false))
}
fn stdeva_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, |v| stdev(v, true))
}
fn stdevpa_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, |v| stdev(v, false))
}
fn var_s_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| variance(v, true))
}
fn var_p_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| variance(v, false))
}
fn vara_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, |v| variance(v, true))
}
fn varpa_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_numsa(ctx, args, |v| variance(v, false))
}

fn kth(ctx: &EvalCtx<'_>, args: &[ArgVal], large: bool) -> RuntimeValue {
    let out = (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let v = collect_numbers(ctx, std::slice::from_ref(first))?;
        let k = arg_number(ctx, args, 1)?.trunc() as i64;
        if v.is_empty() || k < 1 || k as usize > v.len() {
            return Err(ErrorKind::Num);
        }
        let mut s = sorted(v);
        if large {
            s.reverse();
        }
        Ok(s[k as usize - 1])
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn large_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    kth(ctx, args, true)
}
fn small_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    kth(ctx, args, false)
}

fn mode_sngl_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| {
        let items = crate::common::frequencies(v);
        let maxc = items.iter().map(|(_, c, _)| *c).max().unwrap_or(0);
        if maxc < 2 {
            return Err(ErrorKind::Na);
        }
        items
            .into_iter()
            .filter(|(_, c, _)| *c == maxc)
            .min_by_key(|(_, _, i)| *i)
            .map(|(v, _, _)| v)
            .ok_or(ErrorKind::Na)
    })
}
fn mode_mult_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match nums(ctx, args) {
        Err(e) => RuntimeValue::error(e),
        Ok(v) => {
            let items = crate::common::frequencies(&v);
            let maxc = items.iter().map(|(_, c, _)| *c).max().unwrap_or(0);
            if maxc < 2 {
                return RuntimeValue::error(ErrorKind::Na);
            }
            let mut modes: Vec<(usize, f64)> = items
                .into_iter()
                .filter(|(_, c, _)| *c == maxc)
                .map(|(val, _, i)| (i, val))
                .collect();
            modes.sort_by_key(|(i, _)| *i);
            let values: Vec<Scalar> = modes.into_iter().map(|(_, v)| Scalar::Number(v)).collect();
            let rows = values.len() as u32;
            RuntimeValue::array(rows, 1, values)
        }
    }
}
fn percentile_inc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let v = sorted(collect_numbers(ctx, std::slice::from_ref(first))?);
        percentile_inc(&v, arg_number(ctx, args, 1)?)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn percentile_exc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let v = sorted(collect_numbers(ctx, std::slice::from_ref(first))?);
        percentile_exc(&v, arg_number(ctx, args, 1)?)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn quartile(ctx: &EvalCtx<'_>, args: &[ArgVal], exc: bool) -> RuntimeValue {
    let out = (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let v = sorted(collect_numbers(ctx, std::slice::from_ref(first))?);
        let q = arg_number(ctx, args, 1)?.trunc() as i32;
        if exc {
            match q {
                1 => percentile_exc(&v, 0.25),
                2 => percentile_exc(&v, 0.5),
                3 => percentile_exc(&v, 0.75),
                _ => Err(ErrorKind::Num),
            }
        } else {
            let k = match q {
                0 => 0.0,
                1 => 0.25,
                2 => 0.5,
                3 => 0.75,
                4 => 1.0,
                _ => return Err(ErrorKind::Num),
            };
            percentile_inc(&v, k)
        }
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn quartile_inc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    quartile(ctx, args, false)
}
fn quartile_exc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    quartile(ctx, args, true)
}

fn rank(ctx: &EvalCtx<'_>, args: &[ArgVal], avg: bool) -> RuntimeValue {
    let out = (|| {
        let x = arg_number(ctx, args, 0)?;
        let list = args.get(1).ok_or(ErrorKind::Value)?;
        let v = collect_numbers(ctx, std::slice::from_ref(list))?;
        if v.is_empty() {
            return Err(ErrorKind::Na);
        }
        let order = crate::common::arg_number_or(ctx, args, 2, 0.0)?;
        let desc = order == 0.0;
        let mut s = v.clone();
        if desc {
            s.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            s = sorted(s);
        }
        let positions: Vec<usize> = s
            .iter()
            .enumerate()
            .filter(|(_, y)| **y == x)
            .map(|(i, _)| i + 1)
            .collect();
        if positions.is_empty() {
            return Err(ErrorKind::Na);
        }
        if avg {
            Ok(positions.iter().map(|p| *p as f64).sum::<f64>() / positions.len() as f64)
        } else {
            Ok(positions[0] as f64)
        }
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn rank_eq_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rank(ctx, args, false)
}
fn rank_avg_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    rank(ctx, args, true)
}

fn percentrank(ctx: &EvalCtx<'_>, args: &[ArgVal], inc: bool) -> RuntimeValue {
    let out = (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let mut v = collect_numbers(ctx, std::slice::from_ref(first))?;
        let x = arg_number(ctx, args, 1)?;
        let sig = crate::common::arg_number_or(ctx, args, 2, 3.0)?.trunc();
        if sig < 1.0 {
            return Err(ErrorKind::Num);
        }
        let sig = sig as i32;
        if v.is_empty() {
            return Err(ErrorKind::Na);
        }
        v = sorted(v);
        let n = v.len();
        if x < v[0] || x > v[n - 1] {
            return Err(ErrorKind::Na);
        }
        let r = if inc {
            if n == 1 {
                1.0
            } else {
                let mut lo = 0usize;
                while lo < n && v[lo] < x {
                    lo += 1;
                }
                if lo < n && v[lo] == x {
                    lo as f64 / (n as f64 - 1.0)
                } else {
                    let i = lo.saturating_sub(1);
                    let frac = (x - v[i]) / (v[i + 1] - v[i]);
                    (i as f64 + frac) / (n as f64 - 1.0)
                }
            }
        } else {
            let mut lo = 0usize;
            while lo < n && v[lo] < x {
                lo += 1;
            }
            if lo < n && v[lo] == x {
                (lo as f64 + 1.0) / (n as f64 + 1.0)
            } else {
                let i = lo.saturating_sub(1);
                let frac = (x - v[i]) / (v[i + 1] - v[i]);
                (i as f64 + 1.0 + frac) / (n as f64 + 1.0)
            }
        };
        Ok(crate::common::round_down(r, sig))
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn percentrank_inc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    percentrank(ctx, args, true)
}
fn percentrank_exc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    percentrank(ctx, args, false)
}

/// Excel `SLOPE`/`INTERCEPT`/`RSQ` take (known_y's, known_x's).
fn yx_pairs(
    ctx: &EvalCtx<'_>,
    y: &RuntimeValue,
    x: &RuntimeValue,
) -> Result<Vec<(f64, f64)>, ErrorKind> {
    paired_numbers(ctx, x, y)
}
fn pairs(ctx: &EvalCtx<'_>, args: &[ArgVal]) -> Result<Vec<(f64, f64)>, ErrorKind> {
    let a = args.first().ok_or(ErrorKind::Value)?;
    let b = args.get(1).ok_or(ErrorKind::Value)?;
    paired_numbers(ctx, &a.value, &b.value)
}
fn correl_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match pairs(ctx, args).and_then(|p| correl(&p)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn covar_p_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match pairs(ctx, args).and_then(|p| covariance(&p, false)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn covar_s_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match pairs(ctx, args).and_then(|p| covariance(&p, true)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn slope_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(y), Some(x)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    match yx_pairs(ctx, &y.value, &x.value).and_then(|p| slope_intercept(&p).map(|(s, _)| s)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn intercept_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(y), Some(x)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    match yx_pairs(ctx, &y.value, &x.value).and_then(|p| slope_intercept(&p).map(|(_, i)| i)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn rsq_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(y), Some(x)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    match yx_pairs(ctx, &y.value, &x.value).and_then(|p| correl(&p).map(|r| r * r)) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn forecast_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let x = arg_number(ctx, args, 0)?;
        let y = args.get(1).ok_or(ErrorKind::Value)?;
        let xs = args.get(2).ok_or(ErrorKind::Value)?;
        let p = yx_pairs(ctx, &y.value, &xs.value)?;
        let (slope, intercept) = slope_intercept(&p)?;
        Ok(intercept + slope * x)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn geomean_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| {
        if v.is_empty() || v.iter().any(|x| *x <= 0.0) {
            return Err(ErrorKind::Num);
        }
        Ok((v.iter().map(|x| x.ln()).sum::<f64>() / v.len() as f64).exp())
    })
}
fn harmean_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| {
        if v.is_empty() || v.iter().any(|x| *x <= 0.0) {
            return Err(ErrorKind::Num);
        }
        Ok(v.len() as f64 / v.iter().map(|x| 1.0 / x).sum::<f64>())
    })
}
fn trimmean_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let first = args.first().ok_or(ErrorKind::Value)?;
        let mut v = sorted(collect_numbers(ctx, std::slice::from_ref(first))?);
        let p = arg_number(ctx, args, 1)?;
        if !(0.0..1.0).contains(&p) {
            return Err(ErrorKind::Num);
        }
        let n = v.len();
        let drop = ((n as f64) * p / 2.0).floor() as usize;
        if drop * 2 >= n {
            return Err(ErrorKind::Num);
        }
        v.drain(n - drop..);
        v.drain(0..drop);
        mean(&v)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn devsq_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| {
        let m = mean(v)?;
        Ok(v.iter()
            .map(|x| {
                let d = x - m;
                d * d
            })
            .sum())
    })
}
fn avedev_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| {
        let m = mean(v)?;
        Ok(v.iter().map(|x| (x - m).abs()).sum::<f64>() / v.len() as f64)
    })
}
fn skew_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| skewness(v, true))
}
fn skewp_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| skewness(v, false))
}
fn skewness(v: &[f64], sample: bool) -> Result<f64, ErrorKind> {
    let n = v.len() as f64;
    if (sample && n < 3.0) || n < 2.0 {
        return Err(ErrorKind::Div0);
    }
    let m = mean(v)?;
    let s = stdev(v, sample)?;
    if s == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let m3: f64 = v
        .iter()
        .map(|x| {
            let z = (x - m) / s;
            z * z * z
        })
        .sum();
    if sample {
        Ok(n / ((n - 1.0) * (n - 2.0)) * m3)
    } else {
        Ok(m3 / n)
    }
}
fn kurt_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    map_nums(ctx, args, |v| {
        let n = v.len() as f64;
        if n < 4.0 {
            return Err(ErrorKind::Div0);
        }
        let m = mean(v)?;
        let s = stdev(v, true)?;
        if s == 0.0 {
            return Err(ErrorKind::Div0);
        }
        let m4: f64 = v.iter().map(|x| ((x - m) / s).powi(4)).sum();
        let a = n * (n + 1.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
        let b = 3.0 * (n - 1.0) * (n - 1.0) / ((n - 2.0) * (n - 3.0));
        Ok(a * m4 - b)
    })
}
fn frequency_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let (Some(data_arg), Some(bins_arg)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let data = match collect_numbers(ctx, std::slice::from_ref(data_arg)) {
        Ok(v) => v,
        Err(e) => return RuntimeValue::error(e),
    };
    let bins = match collect_numbers(ctx, std::slice::from_ref(bins_arg)) {
        Ok(v) => sorted(v),
        Err(e) => return RuntimeValue::error(e),
    };
    if bins.is_empty() {
        return RuntimeValue::array(1, 1, vec![Scalar::Number(data.len() as f64)]);
    }
    let Some(count_len) = bins.len().checked_add(1) else {
        return RuntimeValue::error(ErrorKind::Num);
    };
    let Ok(rows) = u32::try_from(count_len) else {
        return RuntimeValue::error(ErrorKind::Num);
    };
    if omacell_core::eval::RuntimeArray::checked_len(rows, 1).is_err() {
        return RuntimeValue::error(ErrorKind::Num);
    }
    let mut counts = vec![0.0; count_len];
    for x in data {
        let bin = bins.partition_point(|bound| *bound < x);
        if let Some(count) = counts.get_mut(bin) {
            *count += 1.0;
        }
    }
    let values: Vec<Scalar> = counts.into_iter().map(Scalar::Number).collect();
    RuntimeValue::array(rows, 1, values)
}
fn standardize_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let x = arg_number(ctx, args, 0)?;
        let m = arg_number(ctx, args, 1)?;
        let s = arg_number(ctx, args, 2)?;
        if s == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            Ok((x - m) / s)
        }
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}

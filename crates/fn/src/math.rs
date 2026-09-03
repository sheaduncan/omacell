//! Math and trig functions (WP-05a).

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, RuntimeValue};

use crate::common::{
    self, Origin, arg_number, arg_number_or, as_nonneg_int, binary, combin, excel_int, excel_mod,
    factdouble, factorial, for_each_pair, for_each_value, gcd_i, permut, product_args,
    register_specs, round_down, round_half_away, round_up, rt_num, sum_args, trunc_toward_zero,
    unary,
};
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

macro_rules! spec {
    ($id:ident, $name:expr, $args:expr, $min:expr, $max:expr, $arr:expr, $sig:expr, $doc:expr, $body:expr) => {
        crate::define_fn! {
        const $id = {
            name: $name,
            aliases: &[],
            tier: 0,
            category: "math",
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

/// Math specs in declaration order.
pub const SPECS: &[FunctionSpec] = &[
    ABS,
    ACOS,
    ACOSH,
    ACOT,
    ACOTH,
    ASIN,
    ASINH,
    ATAN,
    ATANH,
    COS,
    COSH,
    COT,
    COTH,
    CSC,
    CSCH,
    DEGREES,
    EVEN,
    EXP,
    FACT,
    FACTDOUBLE,
    INT,
    LN,
    LOG10,
    ODD,
    RADIANS,
    SEC,
    SECH,
    SIGN,
    SIN,
    SINH,
    SQRT,
    SQRTPI,
    TAN,
    TANH,
    ATAN2,
    CEILING,
    CEILING_MATH,
    CEILING_PRECISE,
    COMBIN,
    COMBINA,
    FLOOR,
    FLOOR_MATH,
    FLOOR_PRECISE,
    GCD,
    ISO_CEILING,
    LCM,
    LOG,
    MOD,
    MROUND,
    PERMUT,
    PERMUTATIONA,
    PI,
    POWER,
    PRODUCT,
    QUOTIENT,
    ROUND,
    ROUNDDOWN,
    ROUNDUP,
    SUM,
    SUMSQ,
    SUMPRODUCT,
    SUMX2MY2,
    SUMX2PY2,
    SUMXMY2,
    TRUNC,
    RAND,
    RANDBETWEEN,
];

/// Register math functions (and aliases).
pub fn register_math(registry: &mut omacell_core::eval::FnRegistry) {
    register_specs(registry, SPECS);
}

spec!(
    ABS,
    "ABS",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ABS(number)",
    "Absolute value of a number.",
    FnBody::Eager(abs_impl)
);
spec!(
    ACOS,
    "ACOS",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ACOS(number)",
    "Arccosine in radians.",
    FnBody::Eager(acos_impl)
);
spec!(
    ACOSH,
    "ACOSH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ACOSH(number)",
    "Inverse hyperbolic cosine.",
    FnBody::Eager(acosh_impl)
);
spec!(
    ACOT,
    "ACOT",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ACOT(number)",
    "Arccotangent in radians.",
    FnBody::Eager(acot_impl)
);
spec!(
    ACOTH,
    "ACOTH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ACOTH(number)",
    "Inverse hyperbolic cotangent.",
    FnBody::Eager(acoth_impl)
);
spec!(
    ASIN,
    "ASIN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ASIN(number)",
    "Arcsine in radians.",
    FnBody::Eager(asin_impl)
);
spec!(
    ASINH,
    "ASINH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ASINH(number)",
    "Inverse hyperbolic sine.",
    FnBody::Eager(asinh_impl)
);
spec!(
    ATAN,
    "ATAN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ATAN(number)",
    "Arctangent in radians.",
    FnBody::Eager(atan_impl)
);
spec!(
    ATANH,
    "ATANH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ATANH(number)",
    "Inverse hyperbolic tangent.",
    FnBody::Eager(atanh_impl)
);
spec!(
    COS,
    "COS",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "COS(number)",
    "Cosine of an angle in radians.",
    FnBody::Eager(cos_impl)
);
spec!(
    COSH,
    "COSH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "COSH(number)",
    "Hyperbolic cosine.",
    FnBody::Eager(cosh_impl)
);
spec!(
    COT,
    "COT",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "COT(number)",
    "Cotangent of an angle in radians.",
    FnBody::Eager(cot_impl)
);
spec!(
    COTH,
    "COTH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "COTH(number)",
    "Hyperbolic cotangent.",
    FnBody::Eager(coth_impl)
);
spec!(
    CSC,
    "CSC",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "CSC(number)",
    "Cosecant of an angle in radians.",
    FnBody::Eager(csc_impl)
);
spec!(
    CSCH,
    "CSCH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "CSCH(number)",
    "Hyperbolic cosecant.",
    FnBody::Eager(csch_impl)
);
spec!(
    DEGREES,
    "DEGREES",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "DEGREES(angle)",
    "Converts radians to degrees.",
    FnBody::Eager(degrees_impl)
);
spec!(
    EVEN,
    "EVEN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "EVEN(number)",
    "Rounds away from zero to the next even integer.",
    FnBody::Eager(even_impl)
);
spec!(
    EXP,
    "EXP",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "EXP(number)",
    "e raised to a power.",
    FnBody::Eager(exp_impl)
);
spec!(
    FACT,
    "FACT",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "FACT(number)",
    "Factorial. Truncates toward zero; n>170 is #NUM!.",
    FnBody::Eager(fact_impl)
);
spec!(
    FACTDOUBLE,
    "FACTDOUBLE",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "FACTDOUBLE(number)",
    "Double factorial.",
    FnBody::Eager(factdouble_impl)
);
spec!(
    INT,
    "INT",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "INT(number)",
    "Rounds down toward −∞ to an integer.",
    FnBody::Eager(int_impl)
);
spec!(
    LN,
    "LN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "LN(number)",
    "Natural logarithm.",
    FnBody::Eager(ln_impl)
);
spec!(
    LOG10,
    "LOG10",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "LOG10(number)",
    "Base-10 logarithm.",
    FnBody::Eager(log10_impl)
);
spec!(
    ODD,
    "ODD",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "ODD(number)",
    "Rounds away from zero to the next odd integer.",
    FnBody::Eager(odd_impl)
);
spec!(
    RADIANS,
    "RADIANS",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "RADIANS(angle)",
    "Converts degrees to radians.",
    FnBody::Eager(radians_impl)
);
spec!(
    SEC,
    "SEC",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SEC(number)",
    "Secant of an angle in radians.",
    FnBody::Eager(sec_impl)
);
spec!(
    SECH,
    "SECH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SECH(number)",
    "Hyperbolic secant.",
    FnBody::Eager(sech_impl)
);
spec!(
    SIGN,
    "SIGN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SIGN(number)",
    "Sign of a number: −1, 0, or 1.",
    FnBody::Eager(sign_impl)
);
spec!(
    SIN,
    "SIN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SIN(number)",
    "Sine of an angle in radians.",
    FnBody::Eager(sin_impl)
);
spec!(
    SINH,
    "SINH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SINH(number)",
    "Hyperbolic sine.",
    FnBody::Eager(sinh_impl)
);
spec!(
    SQRT,
    "SQRT",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SQRT(number)",
    "Square root.",
    FnBody::Eager(sqrt_impl)
);
spec!(
    SQRTPI,
    "SQRTPI",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "SQRTPI(number)",
    "Square root of (number * π).",
    FnBody::Eager(sqrtpi_impl)
);
spec!(
    TAN,
    "TAN",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "TAN(number)",
    "Tangent of an angle in radians.",
    FnBody::Eager(tan_impl)
);
spec!(
    TANH,
    "TANH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "TANH(number)",
    "Hyperbolic tangent.",
    FnBody::Eager(tanh_impl)
);
spec!(
    ATAN2,
    "ATAN2",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "ATAN2(x_num, y_num)",
    "Arctangent of the specified x and y coordinates.",
    FnBody::Eager(atan2_impl)
);
spec!(
    CEILING,
    "CEILING",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "CEILING(number, significance)",
    "Rounds away from zero to a multiple of significance (legacy sign rules).",
    FnBody::Eager(ceiling_impl)
);
spec!(
    CEILING_MATH,
    "CEILING.MATH",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    1,
    3,
    ArrayBehavior::LiftAll,
    "CEILING.MATH(number, [significance], [mode])",
    "Rounds a number up to the nearest integer or multiple.",
    FnBody::Eager(ceiling_math_impl)
);
spec!(
    CEILING_PRECISE,
    "CEILING.PRECISE",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "CEILING.PRECISE(number, [significance])",
    "Rounds up (toward +∞) to a multiple of significance.",
    FnBody::Eager(ceiling_precise_impl)
);
spec!(
    COMBIN,
    "COMBIN",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "COMBIN(n, k)",
    "Combinations nCk.",
    FnBody::Eager(combin_impl)
);
spec!(
    COMBINA,
    "COMBINA",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "COMBINA(n, k)",
    "Combinations with repetition.",
    FnBody::Eager(combina_impl)
);
spec!(
    FLOOR,
    "FLOOR",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "FLOOR(number, significance)",
    "Rounds toward zero to a multiple of significance (legacy sign rules).",
    FnBody::Eager(floor_impl)
);
spec!(
    FLOOR_MATH,
    "FLOOR.MATH",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    1,
    3,
    ArrayBehavior::LiftAll,
    "FLOOR.MATH(number, [significance], [mode])",
    "Rounds a number down to the nearest integer or multiple.",
    FnBody::Eager(floor_math_impl)
);
spec!(
    FLOOR_PRECISE,
    "FLOOR.PRECISE",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "FLOOR.PRECISE(number, [significance])",
    "Rounds down (toward −∞) to a multiple of significance.",
    FnBody::Eager(floor_precise_impl)
);
spec!(
    GCD,
    "GCD",
    &[ArgKind::Number],
    1,
    255,
    ArrayBehavior::None,
    "GCD(number1, [number2], ...)",
    "Greatest common divisor. Truncates toward zero.",
    FnBody::Eager(gcd_impl)
);
spec!(
    ISO_CEILING,
    "ISO.CEILING",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "ISO.CEILING(number, [significance])",
    "Alias of CEILING.PRECISE.",
    FnBody::Eager(ceiling_precise_impl)
);
spec!(
    LCM,
    "LCM",
    &[ArgKind::Number],
    1,
    255,
    ArrayBehavior::None,
    "LCM(number1, [number2], ...)",
    "Least common multiple. Truncates toward zero.",
    FnBody::Eager(lcm_impl)
);
spec!(
    LOG,
    "LOG",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "LOG(number, [base])",
    "Logarithm with optional base (default 10).",
    FnBody::Eager(log_impl)
);
spec!(
    MOD,
    "MOD",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "MOD(number, divisor)",
    "Remainder after division; sign follows the divisor.",
    FnBody::Eager(mod_impl)
);
spec!(
    MROUND,
    "MROUND",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "MROUND(number, multiple)",
    "Rounds to the nearest multiple; half away from zero.",
    FnBody::Eager(mround_impl)
);
spec!(
    PERMUT,
    "PERMUT",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "PERMUT(n, k)",
    "Permutations nPk.",
    FnBody::Eager(permut_impl)
);
spec!(
    PERMUTATIONA,
    "PERMUTATIONA",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "PERMUTATIONA(n, k)",
    "Permutations with repetition n^k.",
    FnBody::Eager(permutationa_impl)
);
spec!(
    PI,
    "PI",
    &[],
    0,
    0,
    ArrayBehavior::None,
    "PI()",
    "The constant π.",
    FnBody::Eager(pi_impl)
);
spec!(
    POWER,
    "POWER",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "POWER(number, power)",
    "Number raised to a power.",
    FnBody::Eager(power_impl)
);
spec!(
    PRODUCT,
    "PRODUCT",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "PRODUCT(number1, [number2], ...)",
    "Multiplies numbers; empty list is 0.",
    FnBody::Eager(product_impl)
);
spec!(
    QUOTIENT,
    "QUOTIENT",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "QUOTIENT(numerator, denominator)",
    "Integer portion of a division (toward zero).",
    FnBody::Eager(quotient_impl)
);
spec!(
    ROUND,
    "ROUND",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "ROUND(number, num_digits)",
    "Rounds half away from zero.",
    FnBody::Eager(round_impl)
);
spec!(
    ROUNDDOWN,
    "ROUNDDOWN",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "ROUNDDOWN(number, num_digits)",
    "Rounds toward zero.",
    FnBody::Eager(rounddown_impl)
);
spec!(
    ROUNDUP,
    "ROUNDUP",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "ROUNDUP(number, num_digits)",
    "Rounds away from zero.",
    FnBody::Eager(roundup_impl)
);
spec!(
    SUM,
    "SUM",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "SUM(number1, [number2], ...)",
    "Adds numbers; ranges skip text and logicals.",
    FnBody::Eager(sum_impl)
);
spec!(
    SUMSQ,
    "SUMSQ",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "SUMSQ(number1, [number2], ...)",
    "Sum of squares.",
    FnBody::Eager(sumsq_impl)
);
spec!(
    SUMPRODUCT,
    "SUMPRODUCT",
    &[ArgKind::Array],
    1,
    255,
    ArrayBehavior::None,
    "SUMPRODUCT(array1, [array2], ...)",
    "Sum of products of corresponding components.",
    FnBody::Eager(sumproduct_impl)
);
spec!(
    SUMX2MY2,
    "SUMX2MY2",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "SUMX2MY2(array_x, array_y)",
    "Sum of difference of squares.",
    FnBody::Eager(sumx2my2_impl)
);
spec!(
    SUMX2PY2,
    "SUMX2PY2",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "SUMX2PY2(array_x, array_y)",
    "Sum of sum of squares.",
    FnBody::Eager(sumx2py2_impl)
);
spec!(
    SUMXMY2,
    "SUMXMY2",
    &[ArgKind::Array, ArgKind::Array],
    2,
    2,
    ArrayBehavior::None,
    "SUMXMY2(array_x, array_y)",
    "Sum of squared differences.",
    FnBody::Eager(sumxmy2_impl)
);
spec!(
    TRUNC,
    "TRUNC",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "TRUNC(number, [num_digits])",
    "Truncates toward zero.",
    FnBody::Eager(trunc_impl)
);
crate::define_fn! {
const RAND = {
    name: "RAND",
    aliases: &[],
    tier: 0,
    category: "math",
    arg_kinds: &[],
    min_args: 0,
    max_args: 0,
    volatile: true,
    array: ArrayBehavior::None,
    async_node: false,
    signature: "RAND()",
    doc: "Uniform random in [0, 1), derived from the pass nonce and cell.",
    body: FnBody::Eager(rand_impl),
};
}

crate::define_fn! {
const RANDBETWEEN = {
    name: "RANDBETWEEN",
    aliases: &[],
    tier: 0,
    category: "math",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: true,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "RANDBETWEEN(bottom, top)",
    doc: "Random integer in [bottom, top] from the pass nonce and cell.",
    body: FnBody::Eager(randbetween_impl),
};
}

fn finite_trig(n: f64) -> Result<f64, ErrorKind> {
    if n.is_finite() {
        Ok(n)
    } else {
        Err(ErrorKind::Num)
    }
}
fn abs_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| Ok(n.abs()))
}
fn acos_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if !(-1.0..=1.0).contains(&n) {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.acos())
        }
    })
}
fn acosh_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n < 1.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.acosh())
        }
    })
}
fn acot_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        let a = (1.0 / n).atan();
        if n < 0.0 {
            finite_trig(a + std::f64::consts::PI)
        } else {
            finite_trig(a)
        }
    })
}
fn acoth_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n.abs() <= 1.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig(((n + 1.0) / (n - 1.0)).ln() / 2.0)
        }
    })
}
fn asin_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if !(-1.0..=1.0).contains(&n) {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.asin())
        }
    })
}
fn asinh_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.asinh()))
}
fn atan_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.atan()))
}
fn atan2_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |x, y| {
        if x == 0.0 && y == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            finite_trig(y.atan2(x))
        }
    })
}
fn atanh_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n.abs() >= 1.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.atanh())
        }
    })
}
fn cos_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.cos()))
}
fn cosh_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.cosh()))
}
fn cot_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        let t = n.tan();
        if t == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            finite_trig(1.0 / t)
        }
    })
}
fn coth_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            finite_trig(n.cosh() / n.sinh())
        }
    })
}
fn csc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        let s = n.sin();
        if s == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            finite_trig(1.0 / s)
        }
    })
}
fn csch_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            finite_trig(1.0 / n.sinh())
        }
    })
}
fn sec_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        let c = n.cos();
        if c == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            finite_trig(1.0 / c)
        }
    })
}
fn sech_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(1.0 / n.cosh()))
}
fn sin_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.sin()))
}
fn sinh_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.sinh()))
}
fn tan_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.tan()))
}
fn tanh_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.tanh()))
}
fn exp_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| finite_trig(n.exp()))
}
fn ln_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n <= 0.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.ln())
        }
    })
}
fn log10_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n <= 0.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.log10())
        }
    })
}
fn log_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let n = arg_number(ctx, args, 0)?;
        let base = arg_number_or(ctx, args, 1, 10.0)?;
        if n <= 0.0 || base <= 0.0 || base == 1.0 {
            return Err(ErrorKind::Num);
        }
        finite_trig(n.log(base))
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn sqrt_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n < 0.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.sqrt())
        }
    })
}
fn sqrtpi_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        if n < 0.0 {
            Err(ErrorKind::Num)
        } else {
            finite_trig((n * std::f64::consts::PI).sqrt())
        }
    })
}
fn degrees_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| Ok(n.to_degrees()))
}
fn radians_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| Ok(n.to_radians()))
}
fn pi_impl(_ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    rt_num(std::f64::consts::PI)
}
fn sign_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| {
        Ok(if n > 0.0 {
            1.0
        } else if n < 0.0 {
            -1.0
        } else {
            0.0
        })
    })
}
fn int_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| Ok(excel_int(n)))
}
fn trunc_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let n = arg_number(ctx, args, 0)?;
        let d = arg_number_or(ctx, args, 1, 0.0)?;
        Ok(round_down(n, d.trunc() as i32))
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn round_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, d| Ok(round_half_away(n, d.trunc() as i32)))
}
fn roundup_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, d| Ok(round_up(n, d.trunc() as i32)))
}
fn rounddown_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, d| Ok(round_down(n, d.trunc() as i32)))
}
fn mod_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, excel_mod)
}
fn quotient_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, d| {
        if d == 0.0 {
            Err(ErrorKind::Div0)
        } else {
            Ok(trunc_toward_zero(n / d))
        }
    })
}
fn power_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, p| {
        if (n == 0.0 && p == 0.0) || (n < 0.0 && p.fract() != 0.0) {
            Err(ErrorKind::Num)
        } else {
            finite_trig(n.powf(p))
        }
    })
}
fn fact_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, factorial)
}
fn factdouble_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, factdouble)
}
fn combin_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, k| {
        if n < 0.0 || k < 0.0 {
            Err(ErrorKind::Num)
        } else {
            combin(as_nonneg_int(n)?, as_nonneg_int(k)?)
        }
    })
}
fn combina_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, k| {
        if n < 0.0 || k < 0.0 {
            return Err(ErrorKind::Num);
        }
        let n = as_nonneg_int(n)?;
        let k = as_nonneg_int(k)?;
        if n == 0 && k > 0 {
            return Err(ErrorKind::Num);
        }
        combin(n.checked_add(k).ok_or(ErrorKind::Num)?.saturating_sub(1), k)
    })
}
fn permut_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, k| {
        if n < 0.0 || k < 0.0 {
            Err(ErrorKind::Num)
        } else {
            permut(as_nonneg_int(n)?, as_nonneg_int(k)?)
        }
    })
}
fn permutationa_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, k| {
        if n < 0.0 || k < 0.0 {
            return Err(ErrorKind::Num);
        }
        let n = as_nonneg_int(n)?;
        let k = as_nonneg_int(k)?;
        if n == 0 && k == 0 {
            return Ok(1.0);
        }
        let p = (n as f64).powf(k as f64);
        if p.is_finite() {
            Ok(p)
        } else {
            Err(ErrorKind::Num)
        }
    })
}
fn even_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| Ok(to_even(n)))
}
fn odd_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    unary(ctx, args, |n| Ok(to_odd(n)))
}
fn to_even(n: f64) -> f64 {
    if n == 0.0 {
        return 0.0;
    }
    let away = if n > 0.0 { n.ceil() } else { n.floor() };
    if away % 2.0 == 0.0 {
        away
    } else if n > 0.0 {
        away + 1.0
    } else {
        away - 1.0
    }
}
fn to_odd(n: f64) -> f64 {
    if n == 0.0 {
        return 1.0;
    }
    let away = if n > 0.0 { n.ceil() } else { n.floor() };
    if away % 2.0 != 0.0 {
        away
    } else if n > 0.0 {
        away + 1.0
    } else {
        away - 1.0
    }
}
fn ceiling_math_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let n = arg_number(ctx, args, 0)?;
        let sig = arg_number_or(ctx, args, 1, 1.0)?.abs();
        let mode = arg_number_or(ctx, args, 2, 0.0)?;
        multiple_round(n, sig, true, mode != 0.0)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn floor_math_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let n = arg_number(ctx, args, 0)?;
        let sig = arg_number_or(ctx, args, 1, 1.0)?.abs();
        let mode = arg_number_or(ctx, args, 2, 0.0)?;
        multiple_round(n, sig, false, mode != 0.0)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn ceiling_precise_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let n = arg_number(ctx, args, 0)?;
        let sig = arg_number_or(ctx, args, 1, 1.0)?.abs();
        multiple_round(n, sig, true, false)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn floor_precise_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let n = arg_number(ctx, args, 0)?;
        let sig = arg_number_or(ctx, args, 1, 1.0)?.abs();
        multiple_round(n, sig, false, false)
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn ceiling_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, sig| {
        if sig == 0.0 {
            return Ok(0.0);
        }
        if n > 0.0 && sig < 0.0 {
            return Err(ErrorKind::Num);
        }
        Ok((n / sig).ceil() * sig)
    })
}
fn floor_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, sig| {
        if sig == 0.0 {
            return if n == 0.0 {
                Ok(0.0)
            } else {
                Err(ErrorKind::Div0)
            };
        }
        if n > 0.0 && sig < 0.0 {
            return Err(ErrorKind::Num);
        }
        Ok((n / sig).floor() * sig)
    })
}
fn multiple_round(n: f64, sig: f64, ceil: bool, neg_away: bool) -> Result<f64, ErrorKind> {
    if sig == 0.0 {
        return if n == 0.0 {
            Ok(0.0)
        } else {
            Err(ErrorKind::Div0)
        };
    }
    if n == 0.0 {
        return Ok(0.0);
    }
    let q = n / sig;
    let r = if n > 0.0 {
        if ceil { q.ceil() } else { q.floor() }
    } else if neg_away {
        if ceil { q.floor() } else { q.ceil() }
    } else if ceil {
        q.ceil()
    } else {
        q.floor()
    };
    let out = r * sig;
    if out.is_finite() {
        Ok(out)
    } else {
        Err(ErrorKind::Num)
    }
}
fn mround_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    binary(ctx, args, |n, m| {
        if m == 0.0 {
            return Ok(0.0);
        }
        if n * m < 0.0 {
            return Err(ErrorKind::Num);
        }
        Ok(round_half_away(n / m, 0) * m)
    })
}
fn gcd_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match gcd_lcm(ctx, args, true) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn lcm_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match gcd_lcm(ctx, args, false) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn gcd_lcm(ctx: &EvalCtx<'_>, args: &[ArgVal], gcd: bool) -> Result<f64, ErrorKind> {
    let mut acc: Option<u64> = None;
    for_each_value(ctx, args, &mut |s, origin| {
        if let Some(e) = s.error() {
            return Err(e);
        }
        let n = match origin {
            Origin::Literal => coerce::to_number(&s)?,
            Origin::Aggregate => match s {
                Scalar::Number(n) => n,
                Scalar::Empty => return Ok(()),
                _ => return Err(ErrorKind::Value),
            },
        };
        if n < 0.0 {
            return Err(ErrorKind::Num);
        }
        let v = as_nonneg_int(n)?;
        let next = match acc {
            None => v,
            Some(a) if gcd => gcd_i(a, v),
            Some(a) => {
                if a == 0 || v == 0 {
                    0
                } else {
                    let g = gcd_i(a, v);
                    a.checked_mul(v / g).ok_or(ErrorKind::Num)?
                }
            }
        };
        if next >= (1u64 << 53) {
            return Err(ErrorKind::Num);
        }
        acc = Some(next);
        Ok(())
    })?;
    Ok(acc.unwrap_or(0) as f64)
}
fn sum_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match sum_args(ctx, args) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn product_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match product_args(ctx, args) {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}
fn sumsq_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match common::collect_numbers(ctx, args) {
        Ok(v) => rt_num(v.iter().map(|x| x * x).sum()),
        Err(e) => RuntimeValue::error(e),
    }
}

struct SumproductValues {
    rows: u32,
    cols: u32,
    values: Vec<Scalar>,
}

fn sumproduct_values(
    ctx: &EvalCtx<'_>,
    value: &RuntimeValue,
) -> Result<SumproductValues, ErrorKind> {
    match ctx.materialize(value.clone()) {
        RuntimeValue::Scalar(value) => Ok(SumproductValues {
            rows: 1,
            cols: 1,
            values: vec![value],
        }),
        RuntimeValue::Array(array) => {
            array.validate()?;
            Ok(SumproductValues {
                rows: array.rows,
                cols: array.cols,
                values: array.values.to_vec(),
            })
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

fn sumproduct_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let arrays: Result<Vec<SumproductValues>, _> = args
        .iter()
        .filter(|a| !a.omitted)
        .map(|a| sumproduct_values(ctx, &a.value))
        .collect();
    match arrays {
        Err(e) => RuntimeValue::error(e),
        Ok(list) if list.is_empty() => RuntimeValue::error(ErrorKind::Value),
        Ok(list) => {
            let rows = list[0].rows;
            let cols = list[0].cols;
            if list
                .iter()
                .any(|values| values.rows != rows || values.cols != cols)
            {
                return RuntimeValue::error(ErrorKind::Value);
            }
            let len = list[0].values.len();
            let mut acc = 0.0;
            for i in 0..len {
                let mut prod = 1.0;
                for arr in &list {
                    let Some(value) = arr.values.get(i) else {
                        return RuntimeValue::error(ErrorKind::Value);
                    };
                    match value {
                        Scalar::Error(e) => return RuntimeValue::error(*e),
                        Scalar::Number(n) if n.is_finite() => prod *= n,
                        Scalar::Number(_) => return RuntimeValue::error(ErrorKind::Num),
                        _ => prod = 0.0,
                    }
                }
                acc += prod;
            }
            rt_num(acc)
        }
    }
}
fn pair_sum(ctx: &EvalCtx<'_>, args: &[ArgVal], f: impl Fn(f64, f64) -> f64) -> RuntimeValue {
    let (Some(x), Some(y)) = (args.first(), args.get(1)) else {
        return RuntimeValue::error(ErrorKind::Value);
    };
    let mut acc = 0.0;
    match for_each_pair(ctx, &x.value, &y.value, &mut |a, b| {
        if let Some(e) = a.error().or(b.error()) {
            return Err(e);
        }
        if let (Scalar::Number(xn), Scalar::Number(yn)) = (&a, &b)
            && xn.is_finite()
            && yn.is_finite()
        {
            acc += f(*xn, *yn);
        }
        Ok(())
    }) {
        Err(e) => RuntimeValue::error(e),
        Ok(()) => rt_num(acc),
    }
}
fn sumx2my2_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    pair_sum(ctx, args, |x, y| x * x - y * y)
}
fn sumx2py2_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    pair_sum(ctx, args, |x, y| x * x + y * y)
}
fn sumxmy2_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    pair_sum(ctx, args, |x, y| {
        let d = x - y;
        d * d
    })
}
fn rand_impl(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    rt_num(ctx.random_unit("RAND", 0))
}
fn randbetween_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let out = (|| {
        let bottom = arg_number(ctx, args, 0)?.ceil();
        let top = arg_number(ctx, args, 1)?.floor();
        if bottom > top {
            return Err(ErrorKind::Num);
        }
        let span = top - bottom + 1.0;
        let u = ctx.random_unit("RANDBETWEEN", 0);
        Ok((bottom + (u * span).floor()).min(top))
    })();
    match out {
        Ok(n) => rt_num(n),
        Err(e) => RuntimeValue::error(e),
    }
}

//! Engineering basics (WP-05c): `CONVERT`, base conversions, bit ops, `DELTA`, `GESTEP`.
//!
//! Kept as Tier 0 per Appendix D / this package, even though §6.4 prose groups
//! some of these under Tier 1.

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};

use crate::args;
use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};

/// Engineering specs in declaration order.
pub const SPECS: &[FunctionSpec] = &[
    CONVERT, DEC2BIN, DEC2OCT, DEC2HEX, BIN2DEC, OCT2DEC, HEX2DEC, BITAND, BITOR, BITXOR,
    BITLSHIFT, BITRSHIFT, DELTA, GESTEP,
];

/// Register engineering functions.
pub fn register_engineering(registry: &mut FnRegistry) {
    for spec in SPECS {
        args::register_spec(registry, spec);
    }
}

crate::define_fn! {
const CONVERT = {
    name: "CONVERT",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Text, ArgKind::Text],
    min_args: 3,
    max_args: 3,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "CONVERT(number, from_unit, to_unit)",
    doc: "Converts a number from one measurement system to another (Excel unit table).",
    body: FnBody::Eager(convert_impl),
};
}

crate::define_fn! {
const DEC2BIN = {
    name: "DEC2BIN",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "DEC2BIN(number, [places])",
    doc: "Converts decimal to binary (10-bit two's complement).",
    body: FnBody::Eager(dec2bin_impl),
};
}

crate::define_fn! {
const DEC2OCT = {
    name: "DEC2OCT",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "DEC2OCT(number, [places])",
    doc: "Converts decimal to octal (30-bit two's complement).",
    body: FnBody::Eager(dec2oct_impl),
};
}

crate::define_fn! {
const DEC2HEX = {
    name: "DEC2HEX",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "DEC2HEX(number, [places])",
    doc: "Converts decimal to hexadecimal (40-bit two's complement).",
    body: FnBody::Eager(dec2hex_impl),
};
}

crate::define_fn! {
const BIN2DEC = {
    name: "BIN2DEC",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Text],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "BIN2DEC(number)",
    doc: "Converts a 10-bit binary string to decimal.",
    body: FnBody::Eager(bin2dec_impl),
};
}

crate::define_fn! {
const OCT2DEC = {
    name: "OCT2DEC",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Text],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "OCT2DEC(number)",
    doc: "Converts a 10-digit octal string to decimal.",
    body: FnBody::Eager(oct2dec_impl),
};
}

crate::define_fn! {
const HEX2DEC = {
    name: "HEX2DEC",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Text],
    min_args: 1,
    max_args: 1,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "HEX2DEC(number)",
    doc: "Converts a 10-digit hexadecimal string to decimal.",
    body: FnBody::Eager(hex2dec_impl),
};
}

crate::define_fn! {
const BITAND = {
    name: "BITAND",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "BITAND(number1, number2)",
    doc: "Bitwise AND of two integers in 0..2^48-1.",
    body: FnBody::Eager(bitand_impl),
};
}

crate::define_fn! {
const BITOR = {
    name: "BITOR",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "BITOR(number1, number2)",
    doc: "Bitwise OR of two integers in 0..2^48-1.",
    body: FnBody::Eager(bitor_impl),
};
}

crate::define_fn! {
const BITXOR = {
    name: "BITXOR",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "BITXOR(number1, number2)",
    doc: "Bitwise XOR of two integers in 0..2^48-1.",
    body: FnBody::Eager(bitxor_impl),
};
}

crate::define_fn! {
const BITLSHIFT = {
    name: "BITLSHIFT",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "BITLSHIFT(number, shift_amount)",
    doc: "Shifts a 48-bit integer left (negative amount shifts right).",
    body: FnBody::Eager(bitlshift_impl),
};
}

crate::define_fn! {
const BITRSHIFT = {
    name: "BITRSHIFT",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 2,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "BITRSHIFT(number, shift_amount)",
    doc: "Shifts a 48-bit integer right (negative amount shifts left).",
    body: FnBody::Eager(bitrshift_impl),
};
}

crate::define_fn! {
const DELTA = {
    name: "DELTA",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "DELTA(number1, [number2])",
    doc: "Kronecker delta: 1 if the numbers are equal, otherwise 0.",
    body: FnBody::Eager(delta_impl),
};
}

crate::define_fn! {
const GESTEP = {
    name: "GESTEP",
    aliases: &[],
    tier: 0,
    category: "engineering",
    arg_kinds: &[ArgKind::Number, ArgKind::Number],
    min_args: 1,
    max_args: 2,
    volatile: false,
    array: ArrayBehavior::LiftAll,
    async_node: false,
    signature: "GESTEP(number, [step])",
    doc: "1 if number ≥ step (default 0), otherwise 0.",
    body: FnBody::Eager(gestep_impl),
};
}

fn err(e: ErrorKind) -> RuntimeValue {
    RuntimeValue::error(e)
}

fn num(n: f64) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Number(n))
}

fn text(s: String) -> RuntimeValue {
    RuntimeValue::Scalar(Scalar::Text(s.into()))
}

// ----- CONVERT -----

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dim {
    Mass,
    Dist,
    Time,
    Press,
    Force,
    Energy,
    Power,
    Mag,
    Temp,
    Vol,
    Area,
    Info,
    Speed,
}

struct Unit {
    name: &'static str,
    dim: Dim,
    /// Multiply by this to reach SI (or Kelvin offset handled separately).
    to_si: f64,
}

const UNITS: &[Unit] = &[
    // mass — gram is SI here
    Unit {
        name: "g",
        dim: Dim::Mass,
        to_si: 1.0,
    },
    Unit {
        name: "sg",
        dim: Dim::Mass,
        to_si: 14_593.902_937_206_4,
    },
    Unit {
        name: "lbm",
        dim: Dim::Mass,
        to_si: 453.592_37,
    },
    Unit {
        name: "u",
        dim: Dim::Mass,
        to_si: 1.660_538_86e-24,
    },
    Unit {
        name: "ozm",
        dim: Dim::Mass,
        to_si: 28.349_523_125,
    },
    Unit {
        name: "grain",
        dim: Dim::Mass,
        to_si: 0.064_798_91,
    },
    Unit {
        name: "cwt",
        dim: Dim::Mass,
        to_si: 45_359.237,
    },
    Unit {
        name: "uk_cwt",
        dim: Dim::Mass,
        to_si: 50_802.345_44,
    },
    Unit {
        name: "stone",
        dim: Dim::Mass,
        to_si: 6_350.293_18,
    },
    Unit {
        name: "ton",
        dim: Dim::Mass,
        to_si: 907_184.74,
    },
    Unit {
        name: "uk_ton",
        dim: Dim::Mass,
        to_si: 1_016_046.908_8,
    },
    // distance — metre
    Unit {
        name: "m",
        dim: Dim::Dist,
        to_si: 1.0,
    },
    Unit {
        name: "mi",
        dim: Dim::Dist,
        to_si: 1_609.344,
    },
    Unit {
        name: "Nmi",
        dim: Dim::Dist,
        to_si: 1_852.0,
    },
    Unit {
        name: "in",
        dim: Dim::Dist,
        to_si: 0.025_4,
    },
    Unit {
        name: "ft",
        dim: Dim::Dist,
        to_si: 0.304_8,
    },
    Unit {
        name: "yd",
        dim: Dim::Dist,
        to_si: 0.914_4,
    },
    Unit {
        name: "ang",
        dim: Dim::Dist,
        to_si: 1e-10,
    },
    Unit {
        name: "Pica",
        dim: Dim::Dist,
        to_si: 0.004_233_333_333_333_333,
    },
    Unit {
        name: "pica",
        dim: Dim::Dist,
        to_si: 0.004_233_333_333_333_333,
    },
    Unit {
        name: "ell",
        dim: Dim::Dist,
        to_si: 1.143,
    },
    Unit {
        name: "ly",
        dim: Dim::Dist,
        to_si: 9.460_730_472_580_8e15,
    },
    Unit {
        name: "parsec",
        dim: Dim::Dist,
        to_si: 3.085_677_581_281_56e16,
    },
    Unit {
        name: "survey_mi",
        dim: Dim::Dist,
        to_si: 1_609.347_218_694_437,
    },
    // time — second
    Unit {
        name: "sec",
        dim: Dim::Time,
        to_si: 1.0,
    },
    Unit {
        name: "mn",
        dim: Dim::Time,
        to_si: 60.0,
    },
    Unit {
        name: "hr",
        dim: Dim::Time,
        to_si: 3_600.0,
    },
    Unit {
        name: "day",
        dim: Dim::Time,
        to_si: 86_400.0,
    },
    Unit {
        name: "yr",
        dim: Dim::Time,
        to_si: 31_557_600.0,
    },
    // pressure — Pascal
    Unit {
        name: "Pa",
        dim: Dim::Press,
        to_si: 1.0,
    },
    Unit {
        name: "p",
        dim: Dim::Press,
        to_si: 1.0,
    },
    Unit {
        name: "atm",
        dim: Dim::Press,
        to_si: 101_325.0,
    },
    Unit {
        name: "mmHg",
        dim: Dim::Press,
        to_si: 133.322,
    },
    Unit {
        name: "Torr",
        dim: Dim::Press,
        to_si: 133.322,
    },
    Unit {
        name: "psi",
        dim: Dim::Press,
        to_si: 6_894.757_293_168_361,
    },
    // force — Newton
    Unit {
        name: "N",
        dim: Dim::Force,
        to_si: 1.0,
    },
    Unit {
        name: "dyn",
        dim: Dim::Force,
        to_si: 1e-5,
    },
    Unit {
        name: "pond",
        dim: Dim::Force,
        to_si: 0.009_806_65,
    },
    // energy — Joule
    Unit {
        name: "J",
        dim: Dim::Energy,
        to_si: 1.0,
    },
    Unit {
        name: "e",
        dim: Dim::Energy,
        to_si: 1e-7,
    },
    Unit {
        name: "c",
        dim: Dim::Energy,
        to_si: 4.184,
    },
    Unit {
        name: "cal",
        dim: Dim::Energy,
        to_si: 4.184,
    },
    Unit {
        name: "eV",
        dim: Dim::Energy,
        to_si: 1.602_176_46e-19,
    },
    Unit {
        name: "HPh",
        dim: Dim::Energy,
        to_si: 2_684_519.537_696_172_7,
    },
    Unit {
        name: "Wh",
        dim: Dim::Energy,
        to_si: 3_600.0,
    },
    Unit {
        name: "flb",
        dim: Dim::Energy,
        to_si: 1.355_817_948_331_400_4,
    },
    Unit {
        name: "BTU",
        dim: Dim::Energy,
        to_si: 1_055.055_852_62,
    },
    // power — Watt
    Unit {
        name: "W",
        dim: Dim::Power,
        to_si: 1.0,
    },
    Unit {
        name: "HP",
        dim: Dim::Power,
        to_si: 745.699_871_582_270_2,
    },
    Unit {
        name: "PS",
        dim: Dim::Power,
        to_si: 735.498_75,
    },
    // magnetism — Tesla
    Unit {
        name: "T",
        dim: Dim::Mag,
        to_si: 1.0,
    },
    Unit {
        name: "ga",
        dim: Dim::Mag,
        to_si: 1e-4,
    },
    // temperature (to_si unused; special-cased)
    Unit {
        name: "C",
        dim: Dim::Temp,
        to_si: 1.0,
    },
    Unit {
        name: "F",
        dim: Dim::Temp,
        to_si: 1.0,
    },
    Unit {
        name: "K",
        dim: Dim::Temp,
        to_si: 1.0,
    },
    Unit {
        name: "Rank",
        dim: Dim::Temp,
        to_si: 1.0,
    },
    Unit {
        name: "Reau",
        dim: Dim::Temp,
        to_si: 1.0,
    },
    // volume — litre
    Unit {
        name: "l",
        dim: Dim::Vol,
        to_si: 1.0,
    },
    Unit {
        name: "tsp",
        dim: Dim::Vol,
        to_si: 0.004_928_921_593_75,
    },
    Unit {
        name: "tspm",
        dim: Dim::Vol,
        to_si: 0.005,
    },
    Unit {
        name: "tbs",
        dim: Dim::Vol,
        to_si: 0.014_786_764_781_25,
    },
    Unit {
        name: "oz",
        dim: Dim::Vol,
        to_si: 0.029_573_529_562_5,
    },
    Unit {
        name: "cup",
        dim: Dim::Vol,
        to_si: 0.236_588_236_5,
    },
    Unit {
        name: "pt",
        dim: Dim::Vol,
        to_si: 0.473_176_473,
    },
    Unit {
        name: "uk_pt",
        dim: Dim::Vol,
        to_si: 0.568_261_25,
    },
    Unit {
        name: "qt",
        dim: Dim::Vol,
        to_si: 0.946_352_946,
    },
    Unit {
        name: "uk_qt",
        dim: Dim::Vol,
        to_si: 1.136_522_5,
    },
    Unit {
        name: "gal",
        dim: Dim::Vol,
        to_si: 3.785_411_784,
    },
    Unit {
        name: "uk_gal",
        dim: Dim::Vol,
        to_si: 4.546_09,
    },
    Unit {
        name: "ang3",
        dim: Dim::Vol,
        to_si: 1e-27,
    },
    Unit {
        name: "barrel",
        dim: Dim::Vol,
        to_si: 158.987_294_928,
    },
    Unit {
        name: "bushel",
        dim: Dim::Vol,
        to_si: 35.239_070_166_88,
    },
    Unit {
        name: "ft3",
        dim: Dim::Vol,
        to_si: 28.316_846_592,
    },
    Unit {
        name: "in3",
        dim: Dim::Vol,
        to_si: 0.016_387_064,
    },
    Unit {
        name: "ly3",
        dim: Dim::Vol,
        to_si: 8.467_866_646_237_15e47,
    },
    Unit {
        name: "m3",
        dim: Dim::Vol,
        to_si: 1_000.0,
    },
    Unit {
        name: "mi3",
        dim: Dim::Vol,
        to_si: 4.168_181_825_440_58e12,
    },
    Unit {
        name: "yd3",
        dim: Dim::Vol,
        to_si: 764.554_857_984,
    },
    Unit {
        name: "GRT",
        dim: Dim::Vol,
        to_si: 2_831.684_659_2,
    },
    Unit {
        name: "MTON",
        dim: Dim::Vol,
        to_si: 1.132_673_863_68,
    },
    // area — m^2
    Unit {
        name: "uk_acre",
        dim: Dim::Area,
        to_si: 4_046.856_422_4,
    },
    Unit {
        name: "us_acre",
        dim: Dim::Area,
        to_si: 4_046.872_609_874_252,
    },
    Unit {
        name: "ang2",
        dim: Dim::Area,
        to_si: 1e-20,
    },
    Unit {
        name: "ar",
        dim: Dim::Area,
        to_si: 100.0,
    },
    Unit {
        name: "ft2",
        dim: Dim::Area,
        to_si: 0.092_903_04,
    },
    Unit {
        name: "ha",
        dim: Dim::Area,
        to_si: 10_000.0,
    },
    Unit {
        name: "in2",
        dim: Dim::Area,
        to_si: 0.000_645_16,
    },
    Unit {
        name: "ly2",
        dim: Dim::Area,
        to_si: 8.950_541_494_433_86e31,
    },
    Unit {
        name: "m2",
        dim: Dim::Area,
        to_si: 1.0,
    },
    Unit {
        name: "Morgen",
        dim: Dim::Area,
        to_si: 2_504.0,
    },
    Unit {
        name: "mi2",
        dim: Dim::Area,
        to_si: 2_589_988.110_336,
    },
    Unit {
        name: "Nmi2",
        dim: Dim::Area,
        to_si: 3_429_904.0,
    },
    Unit {
        name: "Pica2",
        dim: Dim::Area,
        to_si: 1.792_111_111_111_111e-5,
    },
    Unit {
        name: "yd2",
        dim: Dim::Area,
        to_si: 0.836_127_36,
    },
    // information — bit
    Unit {
        name: "bit",
        dim: Dim::Info,
        to_si: 1.0,
    },
    Unit {
        name: "byte",
        dim: Dim::Info,
        to_si: 8.0,
    },
    // speed — m/s
    Unit {
        name: "admkn",
        dim: Dim::Speed,
        to_si: 0.514_773_333_333_333_3,
    },
    Unit {
        name: "kn",
        dim: Dim::Speed,
        to_si: 0.514_444_444_444_444_5,
    },
    Unit {
        name: "m/h",
        dim: Dim::Speed,
        to_si: 1.0 / 3_600.0,
    },
    Unit {
        name: "m/s",
        dim: Dim::Speed,
        to_si: 1.0,
    },
    Unit {
        name: "mph",
        dim: Dim::Speed,
        to_si: 0.447_04,
    },
];

const PREFIXES: &[(&str, f64)] = &[
    ("Y", 1e24),
    ("Z", 1e21),
    ("E", 1e18),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("da", 1e1),
    ("e", 1e1),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
    ("y", 1e-24),
];

fn lookup_unit(name: &str) -> Option<(Dim, f64)> {
    if let Some(u) = UNITS.iter().find(|u| u.name == name) {
        return Some((u.dim, u.to_si));
    }
    // prefix + unit (not for temperature)
    for (p, f) in PREFIXES {
        if let Some(rest) = name.strip_prefix(p)
            && !rest.is_empty()
            && let Some(u) = UNITS.iter().find(|u| u.name == rest)
            && u.dim != Dim::Temp
        {
            return Some((u.dim, u.to_si * f));
        }
    }
    None
}

fn to_kelvin(v: f64, unit: &str) -> Option<f64> {
    match unit {
        "C" => Some(v + 273.15),
        "F" => Some((v + 459.67) * 5.0 / 9.0),
        "K" => Some(v),
        "Rank" => Some(v * 5.0 / 9.0),
        "Reau" => Some(v * 1.25 + 273.15),
        _ => None,
    }
}

fn from_kelvin(k: f64, unit: &str) -> Option<f64> {
    match unit {
        "C" => Some(k - 273.15),
        "F" => Some(k * 9.0 / 5.0 - 459.67),
        "K" => Some(k),
        "Rank" => Some(k * 9.0 / 5.0),
        "Reau" => Some((k - 273.15) * 0.8),
        _ => None,
    }
}

fn convert_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let n = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let from = args::as_text(&args::scalar(ctx, args.get(1).ok_or(ErrorKind::Value)?)?)?;
        let to = args::as_text(&args::scalar(ctx, args.get(2).ok_or(ErrorKind::Value)?)?)?;
        if to_kelvin(n, &from).is_some() {
            let k = to_kelvin(n, &from).ok_or(ErrorKind::Na)?;
            return from_kelvin(k, &to).ok_or(ErrorKind::Na);
        }
        let (d1, s1) = lookup_unit(&from).ok_or(ErrorKind::Na)?;
        let (d2, s2) = lookup_unit(&to).ok_or(ErrorKind::Na)?;
        if d1 != d2 || s2 == 0.0 {
            return Err(ErrorKind::Na);
        }
        if d1 == Dim::Temp {
            return Err(ErrorKind::Na);
        }
        Ok(n * s1 / s2)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

// ----- bases -----

fn dec_to_base(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    bits: u32,
    radix: u32,
    digits: &'static [u8],
) -> RuntimeValue {
    match (|| {
        let n = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let v = args::trunc_i64(n)?;
        let half = 1i64 << (bits - 1);
        let min = -half;
        let max = half - 1;
        if v < min || v > max {
            return Err(ErrorKind::Num);
        }
        let unsigned = if v < 0 {
            (v + (1i64 << bits)) as u64
        } else {
            v as u64
        };
        let mut s = String::new();
        let mut x = unsigned;
        if x == 0 {
            s.push('0');
        } else {
            while x > 0 {
                let d = (x % u64::from(radix)) as usize;
                s.push(char::from(digits[d]));
                x /= u64::from(radix);
            }
            let rev: String = s.chars().rev().collect();
            s = rev;
        }
        if let Some(p) = args::opt_scalar(ctx, args, 1)? {
            if v < 0 {
                // places ignored for negatives
            } else {
                let places = args::trunc_i64(coerce::to_number(&p)?)?;
                if places < 1 || places as usize > 10 {
                    return Err(ErrorKind::Num);
                }
                if s.len() > places as usize {
                    return Err(ErrorKind::Num);
                }
                while s.len() < places as usize {
                    s.insert(0, '0');
                }
            }
        }
        Ok(s)
    })() {
        Ok(s) => text(s),
        Err(e) => err(e),
    }
}

fn base_to_dec(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    bits: u32,
    radix: u32,
    max_digits: usize,
) -> RuntimeValue {
    match (|| {
        let s = args::as_text(&args::scalar(ctx, args.first().ok_or(ErrorKind::Value)?)?)?;
        if s.is_empty() || s.len() > max_digits {
            return Err(ErrorKind::Num);
        }
        let mut acc: i128 = 0;
        for ch in s.chars() {
            let d = match ch {
                '0'..='9' => u32::from(ch as u8 - b'0'),
                'a'..='f' => u32::from(ch as u8 - b'a') + 10,
                'A'..='F' => u32::from(ch as u8 - b'A') + 10,
                _ => return Err(ErrorKind::Num),
            };
            if d >= radix {
                return Err(ErrorKind::Num);
            }
            acc = acc * i128::from(radix) + i128::from(d);
        }
        let width = 1i128 << bits;
        let half = 1i128 << (bits - 1);
        if acc >= half {
            acc -= width;
        }
        Ok(acc as f64)
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn dec2bin_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    dec_to_base(ctx, args, 10, 2, b"01")
}

fn dec2oct_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    dec_to_base(ctx, args, 30, 8, b"01234567")
}

fn dec2hex_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    dec_to_base(ctx, args, 40, 16, b"0123456789ABCDEF")
}

fn bin2dec_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    base_to_dec(ctx, args, 10, 2, 10)
}

fn oct2dec_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    base_to_dec(ctx, args, 30, 8, 10)
}

fn hex2dec_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    base_to_dec(ctx, args, 40, 16, 10)
}

// ----- bits -----

const BIT_MAX: u64 = (1u64 << 48) - 1;

fn bit_int(n: f64) -> Result<u64, ErrorKind> {
    let v = args::trunc_i64(n)?;
    if v < 0 {
        return Err(ErrorKind::Num);
    }
    let u = v as u64;
    if u > BIT_MAX {
        Err(ErrorKind::Num)
    } else {
        Ok(u)
    }
}

fn two_bits(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> Result<(u64, u64), ErrorKind> {
    let a = bit_int(args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?)?;
    let b = bit_int(args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?)?;
    Ok((a, b))
}

fn bitand_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match two_bits(ctx, args) {
        Ok((a, b)) => num((a & b) as f64),
        Err(e) => err(e),
    }
}

fn bitor_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match two_bits(ctx, args) {
        Ok((a, b)) => num((a | b) as f64),
        Err(e) => err(e),
    }
}

fn bitxor_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match two_bits(ctx, args) {
        Ok((a, b)) => num((a ^ b) as f64),
        Err(e) => err(e),
    }
}

fn shift(number: u64, amount: i64, left: bool) -> Result<u64, ErrorKind> {
    if amount.unsigned_abs() > 48 {
        return Err(ErrorKind::Num);
    }
    let left = if amount < 0 { !left } else { left };
    let k = amount.unsigned_abs();
    let out = if left {
        number.checked_shl(k as u32).unwrap_or(u64::MAX)
    } else {
        number.checked_shr(k as u32).unwrap_or(0)
    };
    if out > BIT_MAX {
        Err(ErrorKind::Num)
    } else {
        Ok(out)
    }
}

fn bitlshift_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let n = bit_int(args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?)?;
        let sh = args::trunc_i64(args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?)?;
        shift(n, sh, true)
    })() {
        Ok(n) => num(n as f64),
        Err(e) => err(e),
    }
}

fn bitrshift_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let n = bit_int(args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?)?;
        let sh = args::trunc_i64(args::number(ctx, args.get(1).ok_or(ErrorKind::Value)?)?)?;
        shift(n, sh, false)
    })() {
        Ok(n) => num(n as f64),
        Err(e) => err(e),
    }
}

fn delta_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let a = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let b = args::opt_number(ctx, args, 1, 0.0)?;
        Ok(if a == b { 1.0 } else { 0.0 })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

fn gestep_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match (|| {
        let a = args::number(ctx, args.first().ok_or(ErrorKind::Value)?)?;
        let step = args::opt_number(ctx, args, 1, 0.0)?;
        Ok(if a >= step { 1.0 } else { 0.0 })
    })() {
        Ok(n) => num(n),
        Err(e) => err(e),
    }
}

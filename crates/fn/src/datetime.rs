//! Tier-0 date and time functions (spec F-2.1, §6.4).

use std::collections::HashSet;

use omacell_core::coerce::{self, Scalar};
use omacell_core::dates::{self, CivilDate, DateSystem, MAX_SERIAL_1900, MAX_SERIAL_1904};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeArray, RuntimeValue};

use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};
use crate::text::{civil_serial, current_year, parse_date_string, parse_time_string};
use crate::util::{
    self, date_system, err, number, optional, scalar, to_number, to_text, trunc_i64, walk_arg,
};

/// Date/time specs in declaration order (JSON output is re-sorted).
pub const DATETIME_SPECS: &[FunctionSpec] = &[
    DATE,
    TIME,
    DATEVALUE,
    TIMEVALUE,
    YEAR,
    MONTH,
    DAY,
    HOUR,
    MINUTE,
    SECOND,
    WEEKDAY,
    WEEKNUM,
    ISOWEEKNUM,
    TODAY,
    NOW,
    EDATE,
    EOMONTH,
    DAYS,
    DAYS360,
    DATEDIF,
    YEARFRAC,
    NETWORKDAYS,
    NETWORKDAYS_INTL,
    WORKDAY,
    WORKDAY_INTL,
];

/// Register date/time functions onto `registry`.
pub fn register_datetime(registry: &mut FnRegistry) {
    util::register_specs(registry, DATETIME_SPECS);
}

macro_rules! dt_fn {
    ($id:ident, $name:literal, $args:expr, $min:expr, $max:expr, $array:expr, $vol:expr, $sig:literal, $doc:literal, $body:expr) => {
        crate::define_fn! {
            const $id = {
                name: $name,
                aliases: &[],
                tier: 0,
                category: "date",
                arg_kinds: $args,
                min_args: $min,
                max_args: $max,
                volatile: $vol,
                array: $array,
                async_node: false,
                signature: $sig,
                doc: $doc,
                body: FnBody::Eager($body),
            };
        }
    };
}

dt_fn!(
    DATE,
    "DATE",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    3,
    3,
    ArrayBehavior::LiftAll,
    false,
    "DATE(year, month, day)",
    "Serial for a civil date. Years 0–1899 add 1900; months and days overflow.",
    date_impl
);
dt_fn!(
    TIME,
    "TIME",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    3,
    3,
    ArrayBehavior::LiftAll,
    false,
    "TIME(hour, minute, second)",
    "Time as a fraction of a day. Wraps modulo 24 hours; negatives are `#NUM!`.",
    time_impl
);
dt_fn!(
    DATEVALUE,
    "DATEVALUE",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "DATEVALUE(date_text)",
    "Parse a date string using the pass locale.",
    datevalue_impl
);
dt_fn!(
    TIMEVALUE,
    "TIMEVALUE",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "TIMEVALUE(time_text)",
    "Parse a time string using the pass locale AM/PM markers.",
    timevalue_impl
);
dt_fn!(
    YEAR,
    "YEAR",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "YEAR(serial_number)",
    "Year of a date serial (Lotus day is 1900).",
    year_impl
);
dt_fn!(
    MONTH,
    "MONTH",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "MONTH(serial_number)",
    "Month of a date serial (1–12).",
    month_impl
);
dt_fn!(
    DAY,
    "DAY",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "DAY(serial_number)",
    "Day of month of a date serial (0 for Excel January 0, 1900).",
    day_impl
);
dt_fn!(
    HOUR,
    "HOUR",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "HOUR(serial_number)",
    "Hour 0–23 of a date/time serial.",
    hour_impl
);
dt_fn!(
    MINUTE,
    "MINUTE",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "MINUTE(serial_number)",
    "Minute 0–59 of a date/time serial.",
    minute_impl
);
dt_fn!(
    SECOND,
    "SECOND",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "SECOND(serial_number)",
    "Second 0–59 of a date/time serial.",
    second_impl
);
dt_fn!(
    WEEKDAY,
    "WEEKDAY",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    false,
    "WEEKDAY(serial_number, [return_type])",
    "Weekday number. Type 1 (default) is Sunday = 1.",
    weekday_impl
);
dt_fn!(
    WEEKNUM,
    "WEEKNUM",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    false,
    "WEEKNUM(serial_number, [return_type])",
    "Week number. Type 21 is ISO; otherwise the week containing 1 Jan is week 1.",
    weeknum_impl
);
dt_fn!(
    ISOWEEKNUM,
    "ISOWEEKNUM",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    false,
    "ISOWEEKNUM(date)",
    "ISO-8601 week number (Monday start; week 1 contains 4 January).",
    isoweeknum_impl
);
dt_fn!(
    TODAY,
    "TODAY",
    &[],
    0,
    0,
    ArrayBehavior::None,
    true,
    "TODAY()",
    "Integer date serial for this recalc pass (`EvalCtx::today()`).",
    today_impl
);
dt_fn!(
    NOW,
    "NOW",
    &[],
    0,
    0,
    ArrayBehavior::None,
    true,
    "NOW()",
    "Date-and-time serial for this recalc pass (`EvalCtx::clock()`).",
    now_impl
);
dt_fn!(
    EDATE,
    "EDATE",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    false,
    "EDATE(start_date, months)",
    "Add months, clipping the day to the last day of the target month.",
    edate_impl
);
dt_fn!(
    EOMONTH,
    "EOMONTH",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    false,
    "EOMONTH(start_date, months)",
    "Last day of the month `months` after `start_date`.",
    eomonth_impl
);
dt_fn!(
    DAYS,
    "DAYS",
    &[ArgKind::Number, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    false,
    "DAYS(end_date, start_date)",
    "Integer serial difference `end - start`.",
    days_impl
);
dt_fn!(
    DAYS360,
    "DAYS360",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Logical],
    2,
    3,
    ArrayBehavior::LiftAll,
    false,
    "DAYS360(start_date, end_date, [method])",
    "Days on a 360-day year. FALSE/omitted = US NASD; TRUE = European.",
    days360_impl
);
dt_fn!(
    DATEDIF,
    "DATEDIF",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Text],
    3,
    3,
    ArrayBehavior::LiftAll,
    false,
    "DATEDIF(start_date, end_date, unit)",
    "Difference in `Y` `M` `D` `YM` `YD` or `MD`. Start after end is `#NUM!`.",
    datedif_impl
);
dt_fn!(
    YEARFRAC,
    "YEARFRAC",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Number],
    2,
    3,
    ArrayBehavior::LiftAll,
    false,
    "YEARFRAC(start_date, end_date, [basis])",
    "Fraction of a year. Basis 0 US 30/360, 1 actual/actual, 2 actual/360, 3 actual/365, 4 EU 30/360.",
    yearfrac_impl
);
dt_fn!(
    NETWORKDAYS,
    "NETWORKDAYS",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Any],
    2,
    3,
    ArrayBehavior::None,
    false,
    "NETWORKDAYS(start_date, end_date, [holidays])",
    "Inclusive working days between two dates (weekend Saturday+Sunday).",
    networkdays_impl
);
dt_fn!(
    NETWORKDAYS_INTL,
    "NETWORKDAYS.INTL",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Any, ArgKind::Any],
    2,
    4,
    ArrayBehavior::None,
    false,
    "NETWORKDAYS.INTL(start_date, end_date, [weekend], [holidays])",
    "NETWORKDAYS with a weekend code or 7-character Monday-first mask.",
    networkdays_intl_impl
);
dt_fn!(
    WORKDAY,
    "WORKDAY",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Any],
    2,
    3,
    ArrayBehavior::None,
    false,
    "WORKDAY(start_date, days, [holidays])",
    "Date `days` working days after `start_date` (weekend Saturday+Sunday).",
    workday_impl
);
dt_fn!(
    WORKDAY_INTL,
    "WORKDAY.INTL",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Any, ArgKind::Any],
    2,
    4,
    ArrayBehavior::None,
    false,
    "WORKDAY.INTL(start_date, days, [weekend], [holidays])",
    "WORKDAY with a weekend code or 7-character Monday-first mask.",
    workday_intl_impl
);

fn date_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let year = match to_number(ctx, &args[0]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let month = match to_number(ctx, &args[1]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let day = match to_number(ctx, &args[2]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    match excel_date(year, month, day, date_system(ctx)) {
        Ok(n) => number(n as f64),
        Err(e) => err(e),
    }
}

fn time_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let h = match to_number(ctx, &args[0]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let m = match to_number(ctx, &args[1]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let s = match to_number(ctx, &args[2]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let total = match h
        .checked_mul(3600)
        .and_then(|v| m.checked_mul(60).and_then(|mm| v.checked_add(mm)))
        .and_then(|v| v.checked_add(s))
    {
        Some(t) => t,
        None => return err(ErrorKind::Num),
    };
    if total < 0 {
        return err(ErrorKind::Num);
    }
    let rem = total.rem_euclid(86_400);
    number(rem as f64 / 86_400.0)
}

fn datevalue_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let year = match current_year(ctx) {
        Ok(year) => year,
        Err(e) => return err(e),
    };
    match parse_date_string(&s, ctx.locale(), date_system(ctx), year) {
        Some(n) => number(n as f64),
        None => err(ErrorKind::Value),
    }
}

fn timevalue_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    match parse_time_string(&s, ctx.locale()) {
        Some(n) => number(n),
        None => err(ErrorKind::Value),
    }
}

fn year_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match civil_of(ctx, &args[0]) {
        Ok(d) => number(f64::from(d.year)),
        Err(e) => err(e),
    }
}

fn month_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match civil_of(ctx, &args[0]) {
        Ok(d) => number(f64::from(d.month)),
        Err(e) => err(e),
    }
}

fn day_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match civil_of(ctx, &args[0]) {
        Ok(d) => number(f64::from(d.day)),
        Err(e) => err(e),
    }
}

fn hour_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match hms_of(ctx, &args[0]) {
        Ok((h, _, _)) => number(f64::from(h)),
        Err(e) => err(e),
    }
}

fn minute_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match hms_of(ctx, &args[0]) {
        Ok((_, m, _)) => number(f64::from(m)),
        Err(e) => err(e),
    }
}

fn second_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match hms_of(ctx, &args[0]) {
        Ok((_, _, s)) => number(f64::from(s)),
        Err(e) => err(e),
    }
}

fn weekday_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let serial = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let rtype = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 1,
    };
    let sun0 = match dates::weekday_sun0(serial, date_system(ctx)) {
        Some(w) => w,
        None => return err(ErrorKind::Num),
    };
    match weekday_number(sun0, rtype) {
        Ok(n) => number(f64::from(n)),
        Err(e) => err(e),
    }
}

fn weeknum_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let serial = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let rtype = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 1,
    };
    if rtype == 21 {
        return match iso_week(serial, date_system(ctx)) {
            Ok(n) => number(f64::from(n)),
            Err(e) => err(e),
        };
    }
    match weeknum(serial, rtype, date_system(ctx)) {
        Ok(n) => number(f64::from(n)),
        Err(e) => err(e),
    }
}

fn isoweeknum_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let serial = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    match iso_week(serial, date_system(ctx)) {
        Ok(n) => number(f64::from(n)),
        Err(e) => err(e),
    }
}

fn today_impl(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    number(ctx.today())
}

fn now_impl(ctx: &mut EvalCtx<'_>, _args: &[ArgVal]) -> RuntimeValue {
    number(ctx.clock())
}

fn edate_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    shift_months(ctx, args, false)
}

fn eomonth_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    shift_months(ctx, args, true)
}

fn shift_months(ctx: &mut EvalCtx<'_>, args: &[ArgVal], eom: bool) -> RuntimeValue {
    let serial = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let months = match to_number(ctx, &args[1]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    match add_months(serial, months, eom, date_system(ctx)) {
        Ok(n) => number(n as f64),
        Err(e) => err(e),
    }
}

fn days_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let end = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let start = match day_serial(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    number((end - start) as f64)
}

fn days360_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let start = match civil_of(ctx, &args[0]) {
        Ok(d) => d,
        Err(e) => return err(e),
    };
    let end = match civil_of(ctx, &args[1]) {
        Ok(d) => d,
        Err(e) => return err(e),
    };
    let eu = match optional(args, 2) {
        Some(a) => match coerce::to_bool(&match scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        }) {
            Ok(b) => b,
            Err(e) => return err(e),
        },
        None => false,
    };
    let n = if eu {
        days360_eu(start, end)
    } else {
        days360_us(start, end, date_system(ctx))
    };
    number(n as f64)
}

fn datedif_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let start_s = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let end_s = match day_serial(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    if start_s > end_s {
        return err(ErrorKind::Num);
    }
    let unit = match to_text(ctx, &args[2]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let start = match dates::serial_to_date(start_s, date_system(ctx)) {
        Some(d) => d,
        None => return err(ErrorKind::Num),
    };
    let end = match dates::serial_to_date(end_s, date_system(ctx)) {
        Some(d) => d,
        None => return err(ErrorKind::Num),
    };
    match datedif(start, end, start_s, end_s, &unit, date_system(ctx)) {
        Ok(n) => number(n as f64),
        Err(e) => err(e),
    }
}

fn yearfrac_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let start_s = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let end_s = match day_serial(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let basis = match optional(args, 2) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 0,
    };
    match yearfrac(start_s, end_s, basis, date_system(ctx)) {
        Ok(n) => number(n),
        Err(e) => err(e),
    }
}

fn networkdays_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    networkdays_common(ctx, args, None, 2)
}

fn networkdays_intl_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let weekend = match optional(args, 2) {
        Some(a) => match weekend_mask(ctx, a) {
            Ok(m) => m,
            Err(e) => return err(e),
        },
        None => weekend_code(1).unwrap_or(0b0110_0000),
    };
    networkdays_common(ctx, args, Some(weekend), 3)
}

fn networkdays_common(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    weekend: Option<u8>,
    holiday_idx: usize,
) -> RuntimeValue {
    let start = match day_serial(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let end = match day_serial(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let mask = weekend.unwrap_or_else(|| weekend_code(1).unwrap_or(0b0110_0000));
    let holidays = match optional(args, holiday_idx) {
        Some(a) => match holiday_set(ctx, a) {
            Ok(h) => h,
            Err(e) => return err(e),
        },
        None => HashSet::new(),
    };
    match count_workdays(start, end, mask, &holidays, date_system(ctx)) {
        Ok(n) => number(n as f64),
        Err(e) => err(e),
    }
}

fn workday_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    workday_common(ctx, args, None, 2)
}

fn workday_intl_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let weekend = match optional(args, 2) {
        Some(a) => match weekend_mask(ctx, a) {
            Ok(m) => m,
            Err(e) => return err(e),
        },
        None => weekend_code(1).unwrap_or(0b0110_0000),
    };
    workday_common(ctx, args, Some(weekend), 3)
}

fn workday_common(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    weekend: Option<u8>,
    holiday_idx: usize,
) -> RuntimeValue {
    let mask = weekend.unwrap_or_else(|| weekend_code(1).unwrap_or(0b0110_0000));
    // Unlike NETWORKDAYS.INTL, WORKDAY.INTL rejects an all-weekend calendar.
    // https://support.microsoft.com/en-us/excel/functions/workday-intl-function
    if mask == 0b0111_1111 {
        return err(ErrorKind::Value);
    }
    let holidays = match optional(args, holiday_idx) {
        Some(a) => match holiday_set(ctx, a) {
            Ok(h) => h,
            Err(e) => return err(e),
        },
        None => HashSet::new(),
    };
    let system = date_system(ctx);
    let start_v = ctx.materialize(args[0].value.clone());
    let days_v = ctx.materialize(args[1].value.clone());
    workday_lifted(start_v, days_v, mask, &holidays, system)
}

fn workday_one(
    start: &Scalar,
    days: &Scalar,
    mask: u8,
    holidays: &HashSet<i64>,
    system: DateSystem,
) -> RuntimeValue {
    if let Some(e) = start.error().or_else(|| days.error()) {
        return err(e);
    }
    let start_n = match coerce::to_number(start) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let days_n = match coerce::to_number(days) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    let start_s = match dates::split_serial(start_n)
        .and_then(|(d, _)| dates::serial_to_date(d, system).map(|_| d))
    {
        Some(d) => d,
        None => return err(ErrorKind::Num),
    };
    let days_i = match trunc_i64(days_n) {
        Ok(n) => n,
        Err(e) => return err(e),
    };
    match add_workdays(start_s, days_i, mask, holidays, system) {
        Ok(n) => number(n as f64),
        Err(e) => err(e),
    }
}

fn workday_lifted(
    start: RuntimeValue,
    days: RuntimeValue,
    mask: u8,
    holidays: &HashSet<i64>,
    system: DateSystem,
) -> RuntimeValue {
    match (&start, &days) {
        (RuntimeValue::Scalar(s), RuntimeValue::Scalar(d)) => {
            workday_one(s, d, mask, holidays, system)
        }
        _ => {
            let start_a = match as_grid(&start) {
                Ok(g) => g,
                Err(e) => return err(e),
            };
            let days_a = match as_grid(&days) {
                Ok(g) => g,
                Err(e) => return err(e),
            };
            let rows = start_a.0.max(days_a.0);
            let cols = start_a.1.max(days_a.1);
            let value_count = match RuntimeArray::checked_len(rows, cols) {
                Ok(len) => len,
                Err(e) => return err(e),
            };
            let mut values = Vec::with_capacity(value_count);
            for r in 0..rows {
                for c in 0..cols {
                    let s = grid_at(&start_a, r, c);
                    let d = grid_at(&days_a, r, c);
                    match workday_one(&s, &d, mask, holidays, system) {
                        RuntimeValue::Scalar(sc) => values.push(sc),
                        other => {
                            if let Some(e) = other.error_kind() {
                                values.push(Scalar::Error(e));
                            } else {
                                values.push(Scalar::Error(ErrorKind::Value));
                            }
                        }
                    }
                }
            }
            RuntimeValue::array(rows, cols, values)
        }
    }
}

fn as_grid(v: &RuntimeValue) -> Result<(u32, u32, Vec<Scalar>), ErrorKind> {
    match v {
        RuntimeValue::Scalar(s) => Ok((1, 1, vec![s.clone()])),
        RuntimeValue::Array(a) => {
            a.validate()?;
            Ok((a.rows, a.cols, a.values.iter().cloned().collect()))
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Err(ErrorKind::Value),
    }
}

fn grid_at(g: &(u32, u32, Vec<Scalar>), row: u32, col: u32) -> Scalar {
    let (rows, cols, values) = g;
    let r = if *rows == 1 {
        0
    } else if row < *rows {
        row
    } else {
        return Scalar::Error(ErrorKind::Na);
    };
    let c = if *cols == 1 {
        0
    } else if col < *cols {
        col
    } else {
        return Scalar::Error(ErrorKind::Na);
    };
    values
        .get((r as usize) * (*cols as usize) + c as usize)
        .cloned()
        .unwrap_or(Scalar::Empty)
}

fn excel_date(year: i64, month: i64, day: i64, system: DateSystem) -> Result<i64, ErrorKind> {
    if !(0..=9999).contains(&year) {
        return Err(ErrorKind::Num);
    }
    let year = if year <= 1899 { year + 1900 } else { year };
    let month0 = month.checked_sub(1).ok_or(ErrorKind::Num)?;
    let idx = year
        .checked_mul(12)
        .and_then(|v| v.checked_add(month0))
        .ok_or(ErrorKind::Num)?;
    let y = idx.div_euclid(12);
    let m = idx.rem_euclid(12) + 1;
    let y32 = i32::try_from(y).map_err(|_| ErrorKind::Num)?;
    if !(0..=9999).contains(&y32) {
        return Err(ErrorKind::Num);
    }
    let first = dates::date_to_serial(
        CivilDate {
            year: y32,
            month: m as u8,
            day: 1,
            lotus_leap: false,
        },
        system,
    )
    .ok_or(ErrorKind::Num)?;
    let serial = first
        .checked_add(day.checked_sub(1).ok_or(ErrorKind::Num)?)
        .ok_or(ErrorKind::Num)?;
    check_serial(serial, system)?;
    Ok(serial)
}

fn check_serial(serial: i64, system: DateSystem) -> Result<(), ErrorKind> {
    match system {
        DateSystem::Excel1900 => {
            if (0..=MAX_SERIAL_1900).contains(&serial) {
                Ok(())
            } else {
                Err(ErrorKind::Num)
            }
        }
        DateSystem::Excel1904 => {
            if serial <= MAX_SERIAL_1904 {
                Ok(())
            } else {
                Err(ErrorKind::Num)
            }
        }
    }
}

fn day_serial(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<i64, ErrorKind> {
    let value = scalar(ctx, arg)?;
    let n = match &value {
        Scalar::Text(text) => {
            let year = current_year(ctx)?;
            parse_date_string(text, ctx.locale(), date_system(ctx), year)
                .map(|serial| serial as f64)
                .or_else(|| coerce::to_number(&value).ok())
                .ok_or(ErrorKind::Value)?
        }
        _ => coerce::to_number(&value)?,
    };
    let (day, _) = dates::split_serial(n).ok_or(ErrorKind::Num)?;
    dates::serial_to_date(day, date_system(ctx)).ok_or(ErrorKind::Num)?;
    Ok(day)
}

fn civil_of(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<CivilDate, ErrorKind> {
    let day = day_serial(ctx, arg)?;
    dates::serial_to_date(day, date_system(ctx)).ok_or(ErrorKind::Num)
}

fn hms_of(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<(u8, u8, u8), ErrorKind> {
    let value = scalar(ctx, arg)?;
    let n = match &value {
        Scalar::Text(text) => parse_time_string(text, ctx.locale())
            .or_else(|| coerce::to_number(&value).ok())
            .ok_or(ErrorKind::Value)?,
        _ => coerce::to_number(&value)?,
    };
    let (day, frac) = dates::split_serial(n).ok_or(ErrorKind::Num)?;
    dates::serial_to_date(day, date_system(ctx)).ok_or(ErrorKind::Num)?;
    let t = dates::time_from_fraction(frac, 0);
    Ok((t.hour, t.minute, t.second))
}

fn weekday_number(sun0: u8, rtype: i64) -> Result<u8, ErrorKind> {
    let n = match rtype {
        1 | 17 => sun0 + 1,
        2 | 11 => {
            if sun0 == 0 {
                7
            } else {
                sun0
            }
        }
        3 => {
            if sun0 == 0 {
                6
            } else {
                sun0 - 1
            }
        }
        12 => (sun0 + 5) % 7 + 1,
        13 => (sun0 + 4) % 7 + 1,
        14 => (sun0 + 3) % 7 + 1,
        15 => (sun0 + 2) % 7 + 1,
        16 => (sun0 + 1) % 7 + 1,
        _ => return Err(ErrorKind::Num),
    };
    Ok(n)
}

fn week_start_sun0(return_type: i64) -> Result<u8, ErrorKind> {
    match return_type {
        1 => Ok(0),
        2 | 11 => Ok(1),
        12 => Ok(2),
        13 => Ok(3),
        14 => Ok(4),
        15 => Ok(5),
        16 => Ok(6),
        17 => Ok(0),
        _ => Err(ErrorKind::Num),
    }
}

fn weeknum(serial: i64, rtype: i64, system: DateSystem) -> Result<u32, ErrorKind> {
    let start_wd = week_start_sun0(rtype)?;
    let d = dates::serial_to_date(serial, system).ok_or(ErrorKind::Num)?;
    let jan1 = dates::date_to_serial(
        CivilDate {
            year: d.year,
            month: 1,
            day: 1,
            lotus_leap: false,
        },
        system,
    )
    .ok_or(ErrorKind::Num)?;
    let jan1_sun0 = dates::weekday_sun0(jan1, system).ok_or(ErrorKind::Num)?;
    let offset = (i64::from(jan1_sun0) + 7 - i64::from(start_wd)) % 7;
    let week1_start = jan1 - offset;
    let week = ((serial - week1_start) / 7) + 1;
    u32::try_from(week).map_err(|_| ErrorKind::Num)
}

fn iso_week(serial: i64, system: DateSystem) -> Result<u32, ErrorKind> {
    let sun0 = dates::weekday_sun0(serial, system).ok_or(ErrorKind::Num)?;
    let iso_wd = if sun0 == 0 { 6i64 } else { i64::from(sun0) - 1 };
    let thursday = serial + (3 - iso_wd);
    let thu = dates::serial_to_date(thursday, system).ok_or(ErrorKind::Num)?;
    let jan4 = dates::date_to_serial(
        CivilDate {
            year: thu.year,
            month: 1,
            day: 4,
            lotus_leap: false,
        },
        system,
    )
    .ok_or(ErrorKind::Num)?;
    let jan4_sun0 = dates::weekday_sun0(jan4, system).ok_or(ErrorKind::Num)?;
    let jan4_iso = if jan4_sun0 == 0 {
        6i64
    } else {
        i64::from(jan4_sun0) - 1
    };
    let week1_monday = jan4 - jan4_iso;
    let week = ((thursday - 3 - week1_monday) / 7) + 1;
    u32::try_from(week).map_err(|_| ErrorKind::Num)
}

fn last_day_of_month(year: i32, month: u8, system: DateSystem) -> u8 {
    if system == DateSystem::Excel1900 && year == 1900 && month == 2 {
        return 29;
    }
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn add_months(serial: i64, months: i64, eom: bool, system: DateSystem) -> Result<i64, ErrorKind> {
    let d = dates::serial_to_date(serial, system).ok_or(ErrorKind::Num)?;
    let idx = i64::from(d.year)
        .checked_mul(12)
        .and_then(|v| v.checked_add(i64::from(d.month) - 1))
        .and_then(|v| v.checked_add(months))
        .ok_or(ErrorKind::Num)?;
    let y = idx.div_euclid(12);
    let m = (idx.rem_euclid(12) + 1) as u8;
    let y32 = i32::try_from(y).map_err(|_| ErrorKind::Num)?;
    if !(0..=9999).contains(&y32) {
        return Err(ErrorKind::Num);
    }
    let last = last_day_of_month(y32, m, system);
    let day = if eom { last } else { d.day.min(last) };
    civil_serial(i64::from(y32), i64::from(m), i64::from(day), system).ok_or(ErrorKind::Num)
}

fn days360_eu(start: CivilDate, end: CivilDate) -> i64 {
    let d1 = if start.day == 31 {
        30
    } else {
        i64::from(start.day)
    };
    let d2 = if end.day == 31 {
        30
    } else {
        i64::from(end.day)
    };
    (i64::from(end.year) - i64::from(start.year)) * 360
        + (i64::from(end.month) - i64::from(start.month)) * 30
        + (d2 - d1)
}

fn days360_us(start: CivilDate, end: CivilDate, system: DateSystem) -> i64 {
    let mut d1 = i64::from(start.day);
    let mut d2 = i64::from(end.day);
    let mut m2 = i64::from(end.month);
    let mut y2 = i64::from(end.year);
    let last1 = start.day == last_day_of_month(start.year, start.month, system);
    let last2 = end.day == last_day_of_month(end.year, end.month, system);
    if last1 {
        d1 = 30;
    }
    if last2 {
        if d1 < 30 {
            d2 = 1;
            m2 += 1;
            if m2 > 12 {
                m2 = 1;
                y2 += 1;
            }
        } else {
            d2 = 30;
        }
    }
    (y2 - i64::from(start.year)) * 360 + (m2 - i64::from(start.month)) * 30 + (d2 - d1)
}

fn yearfrac_days360_us(start: CivilDate, end: CivilDate) -> i64 {
    // YEARFRAC basis 0 is not identical to DAYS360's February-EOM handling.
    // Microsoft calls out that distinction as a known YEARFRAC compatibility
    // edge: https://support.microsoft.com/en-us/office/yearfrac-function-3844141e-c76d-4143-82b6-208454ddc6a8
    let d1 = if start.day == 31 {
        30
    } else {
        i64::from(start.day)
    };
    let mut d2 = i64::from(end.day);
    let mut m2 = i64::from(end.month);
    let mut y2 = i64::from(end.year);
    if d2 == 31 {
        if d1 < 30 {
            d2 = 1;
            m2 += 1;
            if m2 > 12 {
                m2 = 1;
                y2 += 1;
            }
        } else {
            d2 = 30;
        }
    }
    (y2 - i64::from(start.year)) * 360 + (m2 - i64::from(start.month)) * 30 + (d2 - d1)
}

fn datedif(
    start: CivilDate,
    end: CivilDate,
    start_s: i64,
    end_s: i64,
    unit: &str,
    system: DateSystem,
) -> Result<i64, ErrorKind> {
    match unit.to_ascii_uppercase().as_str() {
        "Y" => {
            let mut y = i64::from(end.year) - i64::from(start.year);
            if (end.month, end.day) < (start.month, start.day) {
                y -= 1;
            }
            Ok(y)
        }
        "M" => {
            let mut m = (i64::from(end.year) - i64::from(start.year)) * 12
                + (i64::from(end.month) - i64::from(start.month));
            if end.day < start.day {
                m -= 1;
            }
            Ok(m)
        }
        "D" => Ok(end_s - start_s),
        "YM" => {
            let mut m = i64::from(end.month) - i64::from(start.month);
            if end.day < start.day {
                m -= 1;
            }
            if m < 0 {
                m += 12;
            }
            Ok(m)
        }
        "MD" => {
            if end.day >= start.day {
                Ok(i64::from(end.day) - i64::from(start.day))
            } else {
                let mut ym = i64::from(end.month) - 1;
                let mut yy = end.year;
                if ym < 1 {
                    ym = 12;
                    yy -= 1;
                }
                let last = last_day_of_month(yy, ym as u8, system);
                Ok(i64::from(last) - i64::from(start.day) + i64::from(end.day))
            }
        }
        "YD" => {
            let mut s2 = civil_serial(
                i64::from(end.year),
                i64::from(start.month),
                i64::from(start.day),
                system,
            )
            .ok_or(ErrorKind::Num)?;
            if s2 > end_s {
                s2 = civil_serial(
                    i64::from(end.year) - 1,
                    i64::from(start.month),
                    i64::from(start.day),
                    system,
                )
                .ok_or(ErrorKind::Num)?;
            }
            Ok(end_s - s2)
        }
        _ => Err(ErrorKind::Num),
    }
}

fn yearfrac(start: i64, end: i64, basis: i64, system: DateSystem) -> Result<f64, ErrorKind> {
    let (s, e) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let ds = dates::serial_to_date(s, system).ok_or(ErrorKind::Num)?;
    let de = dates::serial_to_date(e, system).ok_or(ErrorKind::Num)?;
    let frac = match basis {
        0 => yearfrac_days360_us(ds, de) as f64 / 360.0,
        1 => actual_actual(s, e, ds, de, system)?,
        2 => (e - s) as f64 / 360.0,
        3 => (e - s) as f64 / 365.0,
        4 => days360_eu(ds, de) as f64 / 360.0,
        _ => return Err(ErrorKind::Num),
    };
    Ok(frac)
}

fn actual_actual(
    start: i64,
    end: i64,
    ds: CivilDate,
    de: CivilDate,
    system: DateSystem,
) -> Result<f64, ErrorKind> {
    if start == end {
        return Ok(0.0);
    }
    if ds.year == de.year {
        return Ok((end - start) as f64 / f64::from(days_in_year(ds.year, system)));
    }
    let within_one_year =
        de.year == ds.year.saturating_add(1) && (de.month, de.day) <= (ds.month, ds.day);
    let denominator = if within_one_year {
        if includes_feb_29(start, end, ds.year, de.year, system) {
            366.0
        } else {
            365.0
        }
    } else {
        let covered_years = f64::from(de.year - ds.year + 1);
        let covered_days: u32 = (ds.year..=de.year)
            .map(|year| u32::from(days_in_year(year, system)))
            .sum();
        f64::from(covered_days) / covered_years
    };
    Ok((end - start) as f64 / denominator)
}

fn includes_feb_29(
    start: i64,
    end: i64,
    first_year: i32,
    last_year: i32,
    system: DateSystem,
) -> bool {
    (first_year..=last_year).any(|year| {
        if days_in_year(year, system) != 366 {
            return false;
        }
        let lotus_leap = system == DateSystem::Excel1900 && year == 1900;
        dates::date_to_serial(
            CivilDate {
                year,
                month: 2,
                day: 29,
                lotus_leap,
            },
            system,
        )
        .is_some_and(|serial| (start..=end).contains(&serial))
    })
}

fn days_in_year(year: i32, system: DateSystem) -> u16 {
    if last_day_of_month(year, 2, system) == 29 {
        366
    } else {
        365
    }
}

/// Bit 0 = Monday … bit 6 = Sunday.
fn weekend_code(code: i64) -> Option<u8> {
    Some(match code {
        1 => 0b0110_0000,  // Sat Sun
        2 => 0b0100_0001,  // Sun Mon
        3 => 0b0000_0011,  // Mon Tue
        4 => 0b0000_0110,  // Tue Wed
        5 => 0b0000_1100,  // Wed Thu
        6 => 0b0001_1000,  // Thu Fri
        7 => 0b0011_0000,  // Fri Sat
        11 => 0b0100_0000, // Sun
        12 => 0b0000_0001, // Mon
        13 => 0b0000_0010,
        14 => 0b0000_0100,
        15 => 0b0000_1000,
        16 => 0b0001_0000,
        17 => 0b0010_0000, // Sat
        _ => return None,
    })
}

fn weekend_mask(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<u8, ErrorKind> {
    let s = scalar(ctx, arg)?;
    if let Scalar::Text(t) = &s {
        if t.chars().count() != 7 || t.chars().any(|c| c != '0' && c != '1') {
            return Err(ErrorKind::Value);
        }
        let mut mask = 0u8;
        for (i, c) in t.chars().enumerate() {
            if c == '1' {
                mask |= 1 << i;
            }
        }
        return Ok(mask);
    }
    let n = trunc_i64(coerce::to_number(&s)?)?;
    weekend_code(n).ok_or(ErrorKind::Num)
}

fn is_weekend(serial: i64, mask: u8, system: DateSystem) -> Result<bool, ErrorKind> {
    let sun0 = dates::weekday_sun0(serial, system).ok_or(ErrorKind::Num)?;
    let iso = if sun0 == 0 { 6 } else { sun0 - 1 };
    Ok(mask & (1 << iso) != 0)
}

fn holiday_set(ctx: &mut EvalCtx<'_>, arg: &ArgVal) -> Result<HashSet<i64>, ErrorKind> {
    let mut set = HashSet::new();
    walk_arg(ctx, arg, &mut |s| {
        if s.is_empty() {
            return Ok(());
        }
        if let Some(e) = s.error() {
            return Err(e);
        }
        let n = coerce::to_number(&s)?;
        let (day, _) = dates::split_serial(n).ok_or(ErrorKind::Num)?;
        set.insert(day);
        Ok(())
    })?;
    Ok(set)
}

fn count_workdays(
    start: i64,
    end: i64,
    mask: u8,
    holidays: &HashSet<i64>,
    system: DateSystem,
) -> Result<i64, ErrorKind> {
    let sign = if end < start { -1 } else { 1 };
    let (a, b) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let mut n = 0i64;
    for s in a..=b {
        if !is_weekend(s, mask, system)? && !holidays.contains(&s) {
            n += 1;
        }
    }
    Ok(sign * n)
}

fn add_workdays(
    start: i64,
    days: i64,
    mask: u8,
    holidays: &HashSet<i64>,
    system: DateSystem,
) -> Result<i64, ErrorKind> {
    if days == 0 {
        return Ok(start);
    }
    let step = if days > 0 { 1 } else { -1 };
    let mut remaining = days.unsigned_abs();
    if remaining > 1_000_000 {
        return Err(ErrorKind::Num);
    }
    let mut cur = start;
    let mut guard = 0u64;
    while remaining > 0 {
        cur = cur.checked_add(step).ok_or(ErrorKind::Num)?;
        check_serial(cur, system).or_else(|_| {
            if system == DateSystem::Excel1904 && cur < 0 {
                Ok(())
            } else {
                Err(ErrorKind::Num)
            }
        })?;
        if !is_weekend(cur, mask, system)? && !holidays.contains(&cur) {
            remaining -= 1;
        }
        guard += 1;
        if guard > 1_000_000 {
            return Err(ErrorKind::Num);
        }
    }
    check_serial(cur, system)?;
    Ok(cur)
}

//! VALUE / NUMBERVALUE / date and time text parsers.

use omacell_core::dates::{self, CivilDate, DateSystem};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, RuntimeValue};
use omacell_core::locale::{DateOrder, LocaleId};

use crate::util::{date_system, err, excel_lower, excel_upper, number, optional, to_text};

pub(crate) fn value_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let year = match current_year(ctx) {
        Ok(year) => year,
        Err(e) => return err(e),
    };
    match parse_value(&s, ctx.locale(), date_system(ctx), year) {
        Ok(n) => number(n),
        Err(e) => err(e),
    }
}

pub(crate) fn numbervalue_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let seps = ctx.locale().separators();
    let decimal = match optional(args, 1) {
        Some(a) => match to_text(ctx, a) {
            Ok(t) => t.chars().next().unwrap_or(seps.decimal),
            Err(e) => return err(e),
        },
        None => seps.decimal,
    };
    let group = match optional(args, 2) {
        Some(a) => match to_text(ctx, a) {
            Ok(t) => t.chars().next().unwrap_or(seps.thousands),
            Err(e) => return err(e),
        },
        None => seps.thousands,
    };
    if decimal == group {
        return err(ErrorKind::Value);
    }
    match parse_number_seps(&s, decimal, group, ctx.locale(), true) {
        Ok(n) => number(n),
        Err(e) => err(e),
    }
}

pub(crate) fn current_year(ctx: &EvalCtx<'_>) -> Result<i64, ErrorKind> {
    let (day, _) = dates::split_serial(ctx.today()).ok_or(ErrorKind::Num)?;
    dates::serial_to_date(day, date_system(ctx))
        .map(|date| i64::from(date.year))
        .ok_or(ErrorKind::Num)
}

pub(crate) fn parse_value(
    s: &str,
    locale: LocaleId,
    system: DateSystem,
    current_year: i64,
) -> Result<f64, ErrorKind> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(0.0);
    }
    if let Some((date, _)) = split_date_time(t) {
        let day = parse_date_string(date, locale, system, current_year).ok_or(ErrorKind::Value)?;
        let time = parse_time_string(t, locale).ok_or(ErrorKind::Value)?;
        return Ok(day as f64 + time);
    }
    if looks_like_date(t)
        && let Some(n) = parse_date_string(t, locale, system, current_year)
    {
        return Ok(n as f64);
    }
    if let Some(n) = parse_time_string(t, locale) {
        return Ok(n);
    }
    let seps = locale.separators();
    parse_number_seps(t, seps.decimal, seps.thousands, locale, true)
}

fn looks_like_date(s: &str) -> bool {
    let slashes = s.bytes().filter(|b| *b == b'/').count();
    let dashes = s.bytes().filter(|b| *b == b'-').count();
    let dots = s.bytes().filter(|b| *b == b'.').count();
    slashes >= 1 || dashes >= 2 || dots >= 2 || s.chars().any(|c| c.is_alphabetic())
}

pub(crate) fn parse_date_string(
    s: &str,
    locale: LocaleId,
    system: DateSystem,
    current_year: i64,
) -> Option<i64> {
    let input = s.trim();
    let t = split_date_time(input).map_or(input, |(date, _)| date);
    let info = locale.info();
    if let Some(n) = parse_iso_date(t, system) {
        return Some(n);
    }
    if let Some(n) = parse_named_month(t, info.months_full, info.months_abbr, current_year, system)
    {
        return Some(n);
    }
    let seps: &[char] = &['/', '-', '.', ' '];
    let parts: Vec<&str> = t
        .split(|c: char| seps.contains(&c))
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let a = parts[0].parse::<i64>().ok()?;
    let b = parts[1].parse::<i64>().ok()?;
    let c = parts[2].parse::<i64>().ok()?;
    let (y, m, d) = if parts[0].len() >= 4 {
        (a, b, c)
    } else {
        match info.date_order {
            DateOrder::Mdy => (c, a, b),
            DateOrder::Dmy => {
                if a > 31 {
                    (a, b, c)
                } else {
                    (c, b, a)
                }
            }
            DateOrder::Ymd => (a, b, c),
        }
    };
    let y = if (0..=99).contains(&y) {
        if y >= 30 { 1900 + y } else { 2000 + y }
    } else {
        y
    };
    civil_serial(y, m, d, system)
}

fn parse_iso_date(s: &str, system: DateSystem) -> Option<i64> {
    let mut parts = s.split('-');
    let y = parts.next()?.parse::<i64>().ok()?;
    let m = parts.next()?.parse::<i64>().ok()?;
    let d = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || s.len() < 8 {
        return None;
    }
    civil_serial(y, m, d, system)
}

fn parse_named_month(
    s: &str,
    full: [&str; 12],
    abbr: [&str; 12],
    current_year: i64,
    system: DateSystem,
) -> Option<i64> {
    let lower = excel_lower(s);
    let mut month = None;
    for (i, (f, a)) in full.iter().zip(abbr.iter()).enumerate() {
        if lower.contains(&excel_lower(f)) || lower.contains(&excel_lower(a)) {
            month = Some((i + 1) as i64);
            break;
        }
    }
    let m = month?;
    let nums: Vec<i64> = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();
    let (d, y) = match nums.as_slice() {
        [] => (1, current_year),
        [only] if *only > 31 => (1, *only),
        [only] => (*only, current_year),
        [first, second, ..] if *first > 31 => (*second, *first),
        [first, second, ..] => (*first, *second),
    };
    let y = if (0..=99).contains(&y) {
        if y >= 30 { 1900 + y } else { 2000 + y }
    } else {
        y
    };
    civil_serial(y, m, d, system)
}

pub(crate) fn civil_serial(year: i64, month: i64, day: i64, system: DateSystem) -> Option<i64> {
    let y = i32::try_from(year).ok()?;
    let m = u8::try_from(month).ok()?;
    let d = u8::try_from(day).ok()?;
    let lotus = y == 1900 && m == 2 && d == 29 && system == DateSystem::Excel1900;
    dates::date_to_serial(
        CivilDate {
            year: y,
            month: m,
            day: d,
            lotus_leap: lotus,
        },
        system,
    )
}

pub(crate) fn parse_time_string(s: &str, locale: LocaleId) -> Option<f64> {
    let info = locale.info();
    let input = s.trim();
    let mut t = split_date_time(input)
        .map_or(input, |(_, time)| time)
        .to_string();
    let mut pm = false;
    let mut am = false;
    let upper = excel_upper(&t);
    let am_u = excel_upper(info.am);
    let pm_u = excel_upper(info.pm);
    if !am_u.is_empty() && upper.ends_with(&am_u) {
        am = true;
        let keep = t.len().saturating_sub(info.am.len());
        t.truncate(keep);
    } else if !pm_u.is_empty() && upper.ends_with(&pm_u) {
        pm = true;
        let keep = t.len().saturating_sub(info.pm.len());
        t.truncate(keep);
    }
    t = t.trim().to_string();
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut h = parts[0].trim().parse::<f64>().ok()?;
    let m = parts[1].trim().parse::<f64>().ok()?;
    let sec = if parts.len() == 3 {
        parts[2].trim().parse::<f64>().ok()?
    } else {
        0.0
    };
    if am || pm {
        if !(1.0..=12.0).contains(&h) {
            return None;
        }
        if pm && h < 12.0 {
            h += 12.0;
        }
        if am && (h - 12.0).abs() < f64::EPSILON {
            h = 0.0;
        }
    }
    if !h.is_finite() || h < 0.0 || !(0.0..60.0).contains(&m) || !(0.0..60.0).contains(&sec) {
        return None;
    }
    Some((h * 3600.0 + m * 60.0 + sec).rem_euclid(86_400.0) / 86_400.0)
}

fn split_date_time(input: &str) -> Option<(&str, &str)> {
    let colon = input.find(':')?;
    let (split, whitespace) = input
        .get(..colon)?
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())?;
    let date = input.get(..split)?.trim_end();
    let time = input.get(split + whitespace.len_utf8()..)?.trim_start();
    if date.is_empty() || time.is_empty() {
        None
    } else {
        Some((date, time))
    }
}

fn parse_number_seps(
    s: &str,
    decimal: char,
    group: char,
    locale: LocaleId,
    allow_currency: bool,
) -> Result<f64, ErrorKind> {
    let mut t = s.trim().to_string();
    if t.is_empty() {
        return Ok(0.0);
    }
    let mut neg = false;
    if t.starts_with('(') && t.ends_with(')') && t.len() >= 2 {
        neg = true;
        t = t[1..t.len() - 1].to_string();
    }
    if let Some(rest) = t.strip_prefix('-') {
        neg = true;
        t = rest.to_string();
    } else if let Some(rest) = t.strip_prefix('+') {
        t = rest.to_string();
    }
    let mut pct = false;
    if let Some(rest) = t.strip_suffix('%') {
        pct = true;
        t = rest.to_string();
    }
    t = t.trim().to_string();
    if allow_currency {
        let cur = locale.info().currency;
        if let Some(rest) = t.strip_prefix(cur) {
            t = rest.trim().to_string();
        } else if let Some(rest) = t.strip_suffix(cur) {
            t = rest.trim().to_string();
        }
    }
    let mut out = String::new();
    let mut seen_dec = false;
    let mut seen_exp = false;
    let mut mantissa_digits = 0usize;
    let mut exponent_digits = 0usize;
    for c in t.chars() {
        if c == group && !seen_exp {
            continue;
        }
        if c == decimal && !seen_exp {
            if seen_dec {
                return Err(ErrorKind::Value);
            }
            seen_dec = true;
            out.push('.');
            continue;
        }
        if c.is_ascii_digit() {
            out.push(c);
            if seen_exp {
                exponent_digits += 1;
            } else {
                mantissa_digits += 1;
            }
            continue;
        }
        if matches!(c, 'e' | 'E') && !seen_exp && mantissa_digits > 0 {
            seen_exp = true;
            out.push('e');
            continue;
        }
        if matches!(c, '+' | '-') && seen_exp && out.ends_with('e') {
            out.push(c);
            continue;
        }
        if c == ' ' || c == '\u{00A0}' {
            continue;
        }
        return Err(ErrorKind::Value);
    }
    if mantissa_digits == 0 || seen_exp && exponent_digits == 0 {
        return Err(ErrorKind::Value);
    }
    let mut n: f64 = out.parse().map_err(|_| ErrorKind::Value)?;
    if pct {
        n /= 100.0;
    }
    if neg {
        n = -n;
    }
    if !n.is_finite() {
        return Err(ErrorKind::Num);
    }
    Ok(n)
}

//! Apply a parsed format to a value.

use crate::dates::{
    CivilDate, DateSystem, MAX_SERIAL_1900, MAX_SERIAL_1904, elapsed, serial_to_date, split_serial,
    time_from_fraction, weekday_sun0,
};
use crate::error::ErrorKind;
use crate::locale::LocaleInfo;
use crate::numfmt::fraction::render_fraction;
use crate::numfmt::general::{general, general_for_width};
use crate::numfmt::number::{excel_precision_15, group_int, split_fixed};
use crate::numfmt::parse::parse;
use crate::numfmt::token::{AmPmStyle, DigitKind, LayoutHints, ParsedFormat, Section, Token};
use crate::numfmt::{FormatOptions, FormatValue, Formatted};

const OVERFLOW: &str = "##########";

pub(crate) fn format_value(value: FormatValue<'_>, fmt: &str, opts: &FormatOptions) -> Formatted {
    match parse(fmt) {
        Ok(parsed) => format_parsed(value, &parsed, opts),
        Err(_) => fallback(value, opts),
    }
}

fn fallback(value: FormatValue<'_>, opts: &FormatOptions) -> Formatted {
    let text = match value {
        FormatValue::Empty => String::new(),
        FormatValue::Number(n) => general_maybe_width(n, opts.width),
        FormatValue::Bool(true) => "TRUE".into(),
        FormatValue::Bool(false) => "FALSE".into(),
        FormatValue::Text(s) => s.to_string(),
        FormatValue::Error(e) => e.as_str().to_string(),
    };
    Formatted::text(text)
}

pub(crate) fn format_parsed(
    value: FormatValue<'_>,
    parsed: &ParsedFormat,
    opts: &FormatOptions,
) -> Formatted {
    match value {
        FormatValue::Empty => Formatted::text(String::new()),
        FormatValue::Error(e) => Formatted::text(e.as_str().to_string()),
        FormatValue::Text(s) => render_text(s, parsed, opts),
        FormatValue::Bool(b) => render_bool(b),
        FormatValue::Number(n) => render_number(n, parsed, opts),
    }
}

fn render_bool(b: bool) -> Formatted {
    Formatted::text(if b { "TRUE" } else { "FALSE" }.into())
}

fn render_text(s: &str, parsed: &ParsedFormat, opts: &FormatOptions) -> Formatted {
    if let Some(sec) = parsed.text_section() {
        return apply_section(Payload::Text(s), sec, opts, false);
    }
    if parsed.sections.len() == 1 && parsed.sections[0].has_at() {
        return apply_section(Payload::Text(s), &parsed.sections[0], opts, false);
    }
    Formatted::text(s.to_string())
}

fn render_number(n: f64, parsed: &ParsedFormat, opts: &FormatOptions) -> Formatted {
    if !n.is_finite() {
        return Formatted::text(ErrorKind::Num.as_str().to_string());
    }
    let n = excel_precision_15(n);
    let sections = &parsed.sections;
    let has_cond = sections.iter().take(3).any(|s| s.condition.is_some());
    if has_cond {
        for sec in sections.iter().take(3) {
            if let Some(c) = sec.condition {
                if c.matches(n) {
                    return apply_section(Payload::Number(n), sec, opts, false);
                }
            } else {
                return apply_section(Payload::Number(n.abs()), sec, opts, n < 0.0);
            }
        }
        return Formatted::text(OVERFLOW.into());
    }
    match sections.len() {
        0 => Formatted::text(general_maybe_width(n, opts.width)),
        1 => {
            let date_like = sections[0].is_date() || sections[0].is_time();
            if date_like {
                apply_section(Payload::Number(n), &sections[0], opts, false)
            } else {
                apply_section(Payload::Number(n.abs()), &sections[0], opts, n < 0.0)
            }
        }
        2 => {
            if n < 0.0 {
                apply_section(Payload::Number(n.abs()), &sections[1], opts, false)
            } else {
                apply_section(Payload::Number(n.abs()), &sections[0], opts, false)
            }
        }
        _ => {
            if n < 0.0 {
                apply_section(Payload::Number(n.abs()), &sections[1], opts, false)
            } else if n == 0.0 {
                apply_section(Payload::Number(0.0), &sections[2], opts, false)
            } else {
                apply_section(Payload::Number(n.abs()), &sections[0], opts, false)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Payload<'a> {
    Number(f64),
    Text(&'a str),
}

fn apply_section(
    payload: Payload<'_>,
    section: &Section,
    opts: &FormatOptions,
    leading_minus: bool,
) -> Formatted {
    let date_locale = section.locale.unwrap_or(opts.locale);
    let locale = opts.locale;
    let mut layout = LayoutHints::default();
    let mut text = match payload {
        Payload::Text(s) => emit_text(s, section, &mut layout),
        Payload::Number(n) => emit_number(
            n,
            section,
            locale.info(),
            date_locale.info(),
            opts,
            leading_minus,
            &mut layout,
        ),
    };
    expand_fill(&mut text, &layout, opts.width);
    Formatted {
        text,
        color_hint: section.color,
        layout_hints: layout,
    }
}

fn emit_text(s: &str, section: &Section, layout: &mut LayoutHints) -> String {
    let mut out = String::new();
    for tok in &section.tokens {
        match tok {
            Token::TextPlaceholder => out.push_str(s),
            Token::Literal(t) => out.push_str(t),
            Token::Skip(c) => {
                out.push(' ');
                layout.skips.push(*c);
            }
            Token::Fill(c) => apply_fill(&out, layout, *c),
            _ => {}
        }
    }
    out
}

fn emit_number(
    n: f64,
    section: &Section,
    info: &LocaleInfo,
    date_info: &LocaleInfo,
    opts: &FormatOptions,
    leading_minus: bool,
    layout: &mut LayoutHints,
) -> String {
    if section.is_general() {
        let mut body = general_maybe_width(if leading_minus { -n } else { n }, opts.width);
        if leading_minus && !body.starts_with('-') && n != 0.0 {
            body.insert(0, '-');
        }
        return wrap_literals(&body, section, layout);
    }
    if section.is_date()
        || (section.is_time() && !section.is_scientific() && !section.is_fraction())
    {
        return emit_date_time(n, section, date_info, opts, layout);
    }
    if section.is_fraction() {
        return emit_frac(n, section, leading_minus, layout);
    }
    if section.is_scientific() {
        let body = emit_sci(n, section, info);
        return wrap_num(section, &body, layout, leading_minus && n != 0.0);
    }
    emit_plain_number(n, section, info, leading_minus, layout)
}

fn wrap_literals(body: &str, section: &Section, layout: &mut LayoutHints) -> String {
    let mut out = String::new();
    let mut used = false;
    for tok in &section.tokens {
        match tok {
            Token::General => {
                if !used {
                    out.push_str(body);
                    used = true;
                }
            }
            Token::Literal(t) => out.push_str(t),
            Token::Skip(c) => {
                out.push(' ');
                layout.skips.push(*c);
            }
            Token::Fill(c) => apply_fill(&out, layout, *c),
            _ => {}
        }
    }
    if !used {
        out.push_str(body);
    }
    out
}

fn emit_plain_number(
    n: f64,
    section: &Section,
    info: &LocaleInfo,
    leading_minus: bool,
    layout: &mut LayoutHints,
) -> String {
    let mut percent = 0u32;
    let mut grouping = false;
    let mut int_ph: Vec<DigitKind> = Vec::new();
    let mut frac_ph: Vec<DigitKind> = Vec::new();
    let mut in_frac = false;
    let mut scale = 0u32;
    let mut digits_seen = false;
    let mut scale_after = 0u32;
    for tok in &section.tokens {
        match tok {
            Token::Percent => percent += 1,
            Token::Decimal => {
                in_frac = true;
                scale += scale_after;
                scale_after = 0;
            }
            Token::Digit(k) => {
                digits_seen = true;
                scale_after = 0;
                if in_frac {
                    frac_ph.push(*k);
                } else {
                    int_ph.push(*k);
                }
            }
            Token::Grouping => {
                if in_frac {
                    scale += 1;
                    continue;
                }
                if digits_seen {
                    grouping = true;
                    scale_after += 1;
                } else {
                    scale += 1;
                }
            }
            Token::Exp { .. } => break,
            _ => {}
        }
    }
    scale += scale_after;
    // last grouping commas after the last int digit are scale, not grouping-only
    if scale_after > 0 && int_ph.len() <= 1 {
        grouping = false;
    }
    // `#,##0,` has grouping between digits AND trailing scale
    if int_ph.len() > 1 && scale_after > 0 {
        grouping = true;
    }

    let mut value = n.abs();
    for _ in 0..percent {
        value *= 100.0;
    }
    for _ in 0..scale {
        value /= 1000.0;
    }
    let (int_d, frac_d) = split_fixed(value, frac_ph.len());
    let min_zero = int_ph.iter().filter(|k| **k == DigitKind::Zero).count();
    let mut int_d = int_d;
    while int_d.len() < min_zero {
        int_d.insert(0, 0);
    }
    let mut num = if int_ph.is_empty() && frac_ph.is_empty() && !has_dot_token(section) {
        trim_int_str(&int_d)
    } else if int_ph.iter().all(|k| *k == DigitKind::Hash) && int_d.iter().all(|d| *d == 0) {
        String::new()
    } else if grouping {
        group_int(&int_d, info.separators.thousands)
    } else {
        int_d.iter().map(|x| char::from(b'0' + *x)).collect()
    };
    if int_ph.contains(&DigitKind::Question) && num.len() < int_ph.len() {
        num = format!("{num:>width$}", width = int_ph.len());
    }
    let has_dot = has_dot_token(section);
    if has_dot || !frac_ph.is_empty() {
        num.push(info.separators.decimal);
        for (i, kind) in frac_ph.iter().enumerate() {
            let d = frac_d.get(i).copied().unwrap_or(0);
            match kind {
                DigitKind::Zero => num.push(char::from(b'0' + d)),
                DigitKind::Hash => {
                    if d != 0 || frac_d.iter().skip(i).any(|x| *x != 0) {
                        num.push(char::from(b'0' + d));
                    }
                }
                DigitKind::Question => {
                    if d != 0 || frac_d.iter().skip(i).any(|x| *x != 0) {
                        num.push(char::from(b'0' + d));
                    } else {
                        num.push(' ');
                    }
                }
            }
        }
        if frac_ph.iter().all(|k| matches!(k, DigitKind::Hash)) {
            while num.ends_with('0') {
                num.pop();
            }
        }
    }
    wrap_num(section, &num, layout, leading_minus && value != 0.0)
}

fn has_dot_token(section: &Section) -> bool {
    section.tokens.iter().any(|t| matches!(t, Token::Decimal))
}

fn trim_int_str(digits: &[u8]) -> String {
    let s: String = digits.iter().map(|x| char::from(b'0' + *x)).collect();
    s.trim_start_matches('0').to_string()
}

fn wrap_num(section: &Section, num: &str, layout: &mut LayoutHints, leading_minus: bool) -> String {
    let mut out = String::new();
    if leading_minus {
        out.push('-');
    }
    let mut emitted = false;
    for tok in &section.tokens {
        match tok {
            Token::Digit(_)
            | Token::Decimal
            | Token::Grouping
            | Token::Exp { .. }
            | Token::FractionBar => {
                if !emitted {
                    out.push_str(num);
                    emitted = true;
                }
            }
            Token::Percent => {
                if !emitted {
                    out.push_str(num);
                    emitted = true;
                }
                out.push('%');
            }
            Token::Literal(t) => {
                // skip fraction pattern spaces; the body already has them
                if section.is_fraction() && (t == " " || t.chars().all(|c| c.is_ascii_digit())) {
                    continue;
                }
                out.push_str(t);
            }
            Token::Skip(c) => {
                out.push(' ');
                layout.skips.push(*c);
            }
            Token::Fill(c) => apply_fill(&out, layout, *c),
            _ => {}
        }
    }
    let has_num_slot = section.tokens.iter().any(|t| {
        matches!(
            t,
            Token::Digit(_)
                | Token::Decimal
                | Token::Percent
                | Token::Exp { .. }
                | Token::General
                | Token::FractionBar
        )
    });
    let has_fill = section.tokens.iter().any(|t| matches!(t, Token::Fill(_)));
    if !emitted && (has_num_slot || has_fill) {
        out.push_str(num);
    }
    out
}

fn emit_sci(n: f64, section: &Section, info: &LocaleInfo) -> String {
    let mut int_ph = 0usize;
    let mut frac_ph = 0usize;
    let mut in_frac = false;
    let mut plus = true;
    let mut exp_zeros = 0usize;
    let mut after_exp = false;
    for tok in &section.tokens {
        match tok {
            Token::Digit(_) if !after_exp && !in_frac => int_ph += 1,
            Token::Digit(_) if !after_exp && in_frac => frac_ph += 1,
            Token::Digit(_) if after_exp => exp_zeros += 1,
            Token::Decimal if !after_exp => in_frac = true,
            Token::Exp { plus: p } => {
                plus = *p;
                after_exp = true;
            }
            _ => {}
        }
    }
    int_ph = int_ph.max(1);
    let a = n.abs();
    if a == 0.0 {
        let mut body = String::from("0");
        if frac_ph > 0 {
            body.push(info.separators.decimal);
            body.push_str(&"0".repeat(frac_ph));
        }
        return format!("{}{}", body, fmt_exp(0, plus, exp_zeros.max(1)));
    }
    let log = a.log10();
    let mut exp = log.floor() as i32;
    let mut mant = a / 10f64.powi(exp);
    if mant >= 10.0 {
        mant /= 10.0;
        exp += 1;
    }
    let r = exp.rem_euclid(int_ph as i32);
    mant *= 10f64.powi(r);
    exp -= r;
    let mut exp_adj = exp;
    let (mut id, mut fd) = split_fixed(mant, frac_ph);
    if id.len() > int_ph {
        mant /= 10f64.powi(int_ph as i32);
        exp_adj += int_ph as i32;
        (id, fd) = split_fixed(mant, frac_ph);
    }
    let mut body: String = id.iter().map(|d| char::from(b'0' + *d)).collect();
    if frac_ph > 0 {
        body.push(info.separators.decimal);
        body.extend(fd.iter().map(|d| char::from(b'0' + *d)));
    }
    format!("{}{}", body, fmt_exp(exp_adj, plus, exp_zeros.max(1)))
}

fn fmt_exp(exp: i32, plus: bool, width: usize) -> String {
    let sign = if exp < 0 {
        "-"
    } else if plus {
        "+"
    } else {
        ""
    };
    format!(
        "E{sign}{mag:0width$}",
        mag = exp.unsigned_abs(),
        width = width
    )
}

fn emit_frac(n: f64, section: &Section, leading_minus: bool, layout: &mut LayoutHints) -> String {
    let mut num_ph = 0usize;
    let mut den_ph = 0usize;
    let mut int_ph = 0usize;
    let mut seen_bar = false;
    let mut seen_space = false;
    let mut fixed = String::new();
    let mut den_placeholder = false;
    for tok in &section.tokens {
        match tok {
            Token::Literal(s) if s == " " && !seen_bar => {
                seen_space = true;
                int_ph = num_ph;
                num_ph = 0;
            }
            Token::FractionBar => seen_bar = true,
            Token::Digit(DigitKind::Hash | DigitKind::Question) if !seen_bar => num_ph += 1,
            Token::Digit(DigitKind::Zero) if !seen_bar => num_ph += 1,
            Token::Digit(DigitKind::Hash | DigitKind::Question) if seen_bar => {
                den_ph += 1;
                den_placeholder = true;
                fixed.clear();
            }
            Token::Digit(DigitKind::Zero) if seen_bar => {
                den_ph += 1;
                fixed.push('0');
            }
            Token::Literal(t) if seen_bar && t.chars().all(|c| c.is_ascii_digit()) => {
                fixed.push_str(t);
            }
            _ => {}
        }
    }
    let mixed = seen_space || int_ph > 0;
    let fixed_den = if den_placeholder || fixed.is_empty() {
        None
    } else {
        fixed.parse().ok()
    };
    let mut body = render_fraction(n, int_ph, num_ph.max(1), den_ph.max(1), fixed_den, mixed);
    if leading_minus && n != 0.0 && !body.starts_with('-') {
        body.insert(0, '-');
    }
    wrap_num(section, &body, layout, false)
}

fn emit_date_time(
    n: f64,
    section: &Section,
    info: &LocaleInfo,
    opts: &FormatOptions,
    layout: &mut LayoutHints,
) -> String {
    if n < 0.0 && opts.date_system == DateSystem::Excel1900 {
        return OVERFLOW.into();
    }
    let Some((mut day, frac0)) = split_serial(n) else {
        return OVERFLOW.into();
    };
    if section.is_date() {
        let max = match opts.date_system {
            DateSystem::Excel1900 => MAX_SERIAL_1900,
            DateSystem::Excel1904 => MAX_SERIAL_1904,
        };
        if opts.date_system == DateSystem::Excel1900 && day < 0 {
            return OVERFLOW.into();
        }
        if day > max {
            return OVERFLOW.into();
        }
    }
    let subsec = section.subsec_digits();
    let mut tod = time_from_fraction(frac0, subsec);
    if tod.overflow_days > 0 {
        day += tod.overflow_days as i64;
        tod = time_from_fraction(0.0, subsec);
    }
    let date = if section.is_date() {
        match serial_to_date(day, opts.date_system) {
            Some(d) => d,
            None => return OVERFLOW.into(),
        }
    } else {
        CivilDate {
            year: 1900,
            month: 1,
            day: 0,
            lotus_leap: false,
        }
    };
    let wd = weekday_sun0(day, opts.date_system).unwrap_or(0);
    let (eh, em, es) = elapsed(n).unwrap_or((0, 0, 0));
    let ampm = section.has_ampm();
    let mut out = String::new();
    for tok in &section.tokens {
        match tok {
            Token::Year { len, iso, era } => {
                if *era {
                    continue;
                }
                out.push_str(&year_str(date.year, *len, *iso));
            }
            Token::Month { len } => out.push_str(&month_str(date, *len, info)),
            Token::Day { len } => out.push_str(&pad_u32(u32::from(date.day), *len)),
            Token::Weekday { len } => out.push_str(info.weekday_name(wd, *len)),
            Token::Hour { len, elapsed: el } => {
                let h = if *el {
                    eh
                } else if ampm {
                    u64::from(match tod.hour {
                        0 => 12,
                        1..=12 => tod.hour,
                        _ => tod.hour - 12,
                    })
                } else {
                    u64::from(tod.hour)
                };
                out.push_str(&pad_u64(h, *len));
            }
            Token::Minute { len, elapsed: el } => {
                out.push_str(&pad_u64(if *el { em } else { u64::from(tod.minute) }, *len));
            }
            Token::Second { len, elapsed: el } => {
                out.push_str(&pad_u64(if *el { es } else { u64::from(tod.second) }, *len));
            }
            Token::SubSecond { len } => {
                let stored_len = (*len).min(3);
                let scale = 10u32.pow(u32::from(stored_len));
                let v = tod.subsec % scale;
                out.push('.');
                out.push_str(&format!("{v:0width$}", width = usize::from(stored_len)));
                out.extend(std::iter::repeat_n('0', usize::from(*len - stored_len)));
            }
            Token::AmPm { style } => out.push_str(&ampm_str(tod.hour >= 12, *style, info)),
            Token::Literal(t) => out.push_str(t),
            Token::Skip(c) => {
                out.push(' ');
                layout.skips.push(*c);
            }
            Token::Fill(c) => apply_fill(&out, layout, *c),
            Token::Grouping => out.push(','),
            Token::Decimal => out.push('.'),
            _ => {}
        }
    }
    out
}

fn year_str(year: i32, len: u8, iso: bool) -> String {
    if iso || len >= 3 {
        format!("{year:04}")
    } else {
        format!("{:02}", year.rem_euclid(100))
    }
}

fn month_str(date: CivilDate, len: u8, info: &LocaleInfo) -> String {
    match len {
        0 | 1 => date.month.to_string(),
        2 => format!("{:02}", date.month),
        3 => info.month_name(date.month, 3).to_string(),
        5 => info.month_name(date.month, 5).to_string(),
        _ => info.month_name(date.month, 4).to_string(),
    }
}

fn pad_u32(n: u32, len: u8) -> String {
    if len >= 2 {
        format!("{n:02}")
    } else {
        n.to_string()
    }
}
fn pad_u64(n: u64, len: u8) -> String {
    if len >= 2 {
        format!("{n:02}")
    } else {
        n.to_string()
    }
}

fn ampm_str(pm: bool, style: AmPmStyle, info: &LocaleInfo) -> String {
    if info.am != "AM" || info.pm != "PM" {
        return if pm { info.pm } else { info.am }.to_string();
    }
    match (pm, style) {
        (false, AmPmStyle::Upper) => "AM".into(),
        (true, AmPmStyle::Upper) => "PM".into(),
        (false, AmPmStyle::Lower) => "am".into(),
        (true, AmPmStyle::Lower) => "pm".into(),
        (false, AmPmStyle::UpperShort) => "A".into(),
        (true, AmPmStyle::UpperShort) => "P".into(),
        (false, AmPmStyle::LowerShort) => "a".into(),
        (true, AmPmStyle::LowerShort) => "p".into(),
    }
}

fn apply_fill(out: &str, layout: &mut LayoutHints, c: char) {
    layout.fill = Some(c);
    layout.fill_at = Some(out.len());
}

fn expand_fill(text: &mut String, layout: &LayoutHints, width: Option<usize>) {
    let (Some(fill), Some(at), Some(width)) = (layout.fill, layout.fill_at, width) else {
        return;
    };
    let count = width.saturating_sub(text.chars().count());
    if count > 0 {
        let padding: String = std::iter::repeat_n(fill, count).collect();
        text.insert_str(at, &padding);
    }
}

fn general_maybe_width(n: f64, width: Option<usize>) -> String {
    match width {
        Some(w) => general_for_width(n, w),
        None => general(n),
    }
}

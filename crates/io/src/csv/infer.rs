//! Conservative Auto conversion (spec F-9.4). No silent traps.

use omacell_core::dates::{CivilDate, date_to_serial, serial_to_date, time_from_fraction};
use omacell_core::locale::{DateOrder, LocaleId, LocaleInfo};

use super::plan::{ColumnType, ImportPlan};

/// Stored kind after conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertedKind {
    /// Empty input.
    Empty,
    /// IEEE number (including date serials).
    Number,
    /// Boolean.
    Bool,
    /// Date or time serial.
    Date,
    /// Unconverted text.
    Text,
}

/// Result of converting one field.
#[derive(Clone, Debug, PartialEq)]
pub enum Converted {
    /// Empty cell.
    Empty,
    /// Number (not a date).
    Number(f64),
    /// Boolean.
    Bool(bool),
    /// Date/time serial and a suggested builtin `numFmtId`.
    Date {
        /// Excel serial.
        serial: f64,
        /// Builtin `numFmtId` (14 date, 21 time, 22 datetime).
        num_fmt: u32,
    },
    /// Raw text.
    Text(String),
}

impl Converted {
    /// Kind tag.
    #[must_use]
    pub fn kind(&self) -> ConvertedKind {
        match self {
            Self::Empty => ConvertedKind::Empty,
            Self::Number(_) => ConvertedKind::Number,
            Self::Bool(_) => ConvertedKind::Bool,
            Self::Date { .. } => ConvertedKind::Date,
            Self::Text(_) => ConvertedKind::Text,
        }
    }

    /// Preview display. Dates use ISO-like forms so tests are locale-stable.
    #[must_use]
    pub fn preview_text(&self, plan: &ImportPlan) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Number(n) => format_number(*n),
            Self::Bool(true) => "TRUE".into(),
            Self::Bool(false) => "FALSE".into(),
            Self::Date { serial, num_fmt } => format_date_preview(*serial, *num_fmt, plan),
            Self::Text(s) => s.clone(),
        }
    }

    /// True when the stored type is not text/empty.
    #[must_use]
    pub fn changed(&self) -> bool {
        matches!(self, Self::Number(_) | Self::Bool(_) | Self::Date { .. })
    }

    /// Whether Auto refused conversion because of a known trap.
    #[must_use]
    pub fn is_trap_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }
}

/// Convert `raw` using `ty` and the plan's locale/separators.
#[must_use]
pub fn convert_cell(raw: &str, ty: &ColumnType, plan: &ImportPlan) -> Converted {
    if raw.is_empty() {
        return Converted::Empty;
    }
    match ty {
        ColumnType::Text | ColumnType::KeepAsText => Converted::Text(raw.to_string()),
        ColumnType::Number => convert_number(raw, plan, true),
        ColumnType::Boolean => {
            convert_bool(raw).unwrap_or_else(|| Converted::Text(raw.to_string()))
        }
        ColumnType::Date { format } => convert_date_column(raw, format, plan),
        ColumnType::Auto => convert_auto(raw, plan),
    }
}

fn convert_auto(raw: &str, plan: &ImportPlan) -> Converted {
    if let Some(b) = convert_bool(raw) {
        return b;
    }
    match convert_number(raw, plan, false) {
        Converted::Number(n) => return Converted::Number(n),
        Converted::Text(_) | Converted::Empty | Converted::Bool(_) | Converted::Date { .. } => {}
    }
    if let Some(d) = parse_auto_date(raw, plan) {
        return d;
    }
    Converted::Text(raw.to_string())
}

fn convert_date_column(raw: &str, format: &str, plan: &ImportPlan) -> Converted {
    if !format.is_empty() {
        if let Some(d) = parse_format_date(raw, format, plan) {
            return d;
        }
        return Converted::Text(raw.to_string());
    }
    parse_auto_date(raw, plan).unwrap_or_else(|| Converted::Text(raw.to_string()))
}

fn convert_bool(raw: &str) -> Option<Converted> {
    if raw.eq_ignore_ascii_case("TRUE") {
        Some(Converted::Bool(true))
    } else if raw.eq_ignore_ascii_case("FALSE") {
        Some(Converted::Bool(false))
    } else {
        None
    }
}

fn convert_number(raw: &str, plan: &ImportPlan, forced: bool) -> Converted {
    match parse_number(raw, plan.decimal, plan.thousands, forced) {
        Some(n) => Converted::Number(n),
        None => Converted::Text(raw.to_string()),
    }
}

/// Strict number parse. `forced` skips the leading-zero trap but still
/// refuses >15 significant digits (not an exact IEEE value).
fn parse_number(raw: &str, decimal: char, thousands: Option<char>, forced: bool) -> Option<f64> {
    if raw.eq_ignore_ascii_case("nan")
        || raw.eq_ignore_ascii_case("inf")
        || raw.eq_ignore_ascii_case("+inf")
        || raw.eq_ignore_ascii_case("-inf")
        || raw.eq_ignore_ascii_case("infinity")
        || raw.eq_ignore_ascii_case("+infinity")
        || raw.eq_ignore_ascii_case("-infinity")
    {
        return None;
    }
    let normalized;
    let stripped = if decimal == '.' && thousands.is_none_or(|separator| !raw.contains(separator)) {
        raw
    } else {
        normalized = normalize_number(raw, decimal, thousands)?;
        &normalized
    };
    if !forced && has_leading_zero(stripped) {
        return None;
    }
    if !is_strict_number(stripped) {
        return None;
    }
    if significant_digits(stripped) > 15 {
        return None;
    }
    let n: f64 = stripped.parse().ok()?;
    n.is_finite().then_some(n)
}

/// Rewrite `raw` to a `.`-decimal ASCII number. Rejects thousands after the
/// decimal and thousands groups that are not width 3.
fn normalize_number(raw: &str, decimal: char, thousands: Option<char>) -> Option<String> {
    let mut chars: Vec<char> = raw.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut start = 0;
    if matches!(chars[0], '+' | '-') {
        start = 1;
    }
    if start >= chars.len() {
        return None;
    }
    let body = &chars[start..];
    let dec_at = body.iter().position(|c| *c == decimal);
    if body.iter().filter(|c| **c == decimal).count() > 1 {
        return None;
    }
    if let (Some(th), Some(dpos)) = (thousands, dec_at)
        && body[dpos + 1..].contains(&th)
    {
        return None;
    }
    if let Some(th) = thousands {
        let int_part = match dec_at {
            Some(d) => &body[..d],
            None => {
                let exp = body
                    .iter()
                    .position(|c| matches!(c, 'e' | 'E'))
                    .unwrap_or(body.len());
                &body[..exp]
            }
        };
        if int_part.contains(&th) && !thousands_ok(int_part, th) {
            return None;
        }
    }
    let mut out = String::with_capacity(raw.len());
    for (i, c) in chars.drain(..).enumerate() {
        if i < start {
            out.push(c);
            continue;
        }
        if thousands == Some(c) {
            continue;
        }
        if c == decimal {
            out.push('.');
        } else {
            out.push(c);
        }
    }
    Some(out)
}

fn thousands_ok(int_part: &[char], th: char) -> bool {
    let chunks: Vec<&[char]> = int_part.split(|c| *c == th).collect();
    if chunks.len() < 2 {
        return true;
    }
    if chunks[0].is_empty() || !chunks[0].iter().all(|c| c.is_ascii_digit()) {
        return false;
    }
    chunks[1..]
        .iter()
        .all(|g| g.len() == 3 && g.iter().all(|c| c.is_ascii_digit()))
}

fn has_leading_zero(s: &str) -> bool {
    let mut rest = s;
    if let Some(r) = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-')) {
        rest = r;
    }
    let mut chars = rest.chars();
    let Some('0') = chars.next() else {
        return false;
    };
    matches!(chars.next(), Some('0'..='9'))
}

fn is_strict_number(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    if matches!(chars.peek(), Some('+' | '-')) {
        chars.next();
    }
    let mut saw_digit = false;
    while matches!(chars.peek(), Some('0'..='9')) {
        chars.next();
        saw_digit = true;
    }
    if !saw_digit {
        return false;
    }
    if chars.peek() == Some(&'.') {
        chars.next();
        let mut frac = false;
        while matches!(chars.peek(), Some('0'..='9')) {
            chars.next();
            frac = true;
        }
        if !frac {
            return false;
        }
    }
    if matches!(chars.peek(), Some('e' | 'E')) {
        chars.next();
        if matches!(chars.peek(), Some('+' | '-')) {
            chars.next();
        }
        let mut exp = false;
        while matches!(chars.peek(), Some('0'..='9')) {
            chars.next();
            exp = true;
        }
        if !exp {
            return false;
        }
    }
    chars.next().is_none()
}

fn significant_digits(s: &str) -> usize {
    let mut rest = s;
    if let Some(r) = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-')) {
        rest = r;
    }
    let mantissa = rest.split_once(['e', 'E']).map(|(m, _)| m).unwrap_or(rest);
    let mut significant = 0usize;
    let mut saw_nonzero = false;
    let mut saw_digit = false;
    for digit in mantissa.chars().filter(|c| c.is_ascii_digit()) {
        saw_digit = true;
        if digit != '0' {
            saw_nonzero = true;
        }
        if saw_nonzero {
            significant += 1;
        }
    }
    if saw_digit && !saw_nonzero {
        1
    } else {
        significant
    }
}

fn format_number(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

fn format_date_preview(serial: f64, num_fmt: u32, plan: &ImportPlan) -> String {
    let (day, frac) = split_serial_parts(serial);
    let t = time_from_fraction(frac, 0);
    let time = format!("{:02}:{:02}:{:02}", t.hour, t.minute, t.second);
    if num_fmt == 21 || day == 0 {
        return time;
    }
    let Some(d) = serial_to_date(day, plan.date_system) else {
        return format_number(serial);
    };
    let date = format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
    if num_fmt == 22 || frac > 0.0 {
        format!("{date} {time}")
    } else {
        date
    }
}

fn split_serial_parts(serial: f64) -> (i64, f64) {
    if !serial.is_finite() {
        return (0, 0.0);
    }
    let day = serial.floor();
    (day as i64, (serial - day).clamp(0.0, 0.999_999_999_999))
}

fn parse_auto_date(raw: &str, plan: &ImportPlan) -> Option<Converted> {
    if looks_like_gene_token(raw) {
        return None;
    }
    if let Some(c) = parse_iso_datetime(raw, plan) {
        return Some(c);
    }
    if let Some(c) = parse_numeric_date(raw, plan, false) {
        return Some(c);
    }
    if let Some(c) = parse_month_name_date(raw, plan) {
        return Some(c);
    }
    parse_time_only(raw, plan)
}

fn looks_like_gene_token(raw: &str) -> bool {
    let has_letter = raw.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = raw.chars().any(|c| c.is_ascii_digit());
    if !(has_letter && has_digit) {
        return false;
    }
    raw.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !raw.contains([' ', '/', '-', ',', ':'])
}

fn parse_iso_datetime(raw: &str, plan: &ImportPlan) -> Option<Converted> {
    let (date_part, time_part) = if let Some((d, t)) = raw.split_once('T') {
        (d, Some(t))
    } else if let Some((d, t)) = raw.split_once(' ') {
        if d.chars().filter(|c| *c == '-').count() == 2 {
            (d, Some(t))
        } else {
            (raw, None)
        }
    } else {
        (raw, None)
    };
    let mut parts = date_part.split('-');
    let y = parts.next()?.parse::<i32>().ok()?;
    let m = parts.next()?.parse::<u8>().ok()?;
    let d = parts.next()?.parse::<u8>().ok()?;
    if parts.next().is_some() || date_part.len() < 8 || y < 1000 {
        return None;
    }
    let date = civil(y, m, d)?;
    let mut serial = date_to_serial(date, plan.date_system)? as f64;
    let mut num_fmt = 14u32;
    if let Some(t) = time_part {
        let frac = parse_time_fraction(t)?;
        serial += frac;
        num_fmt = 22;
    }
    Some(Converted::Date { serial, num_fmt })
}

fn parse_numeric_date(
    raw: &str,
    plan: &ImportPlan,
    allow_two_digit_year: bool,
) -> Option<Converted> {
    let sep = date_sep(raw)?;
    let parts: Vec<&str> = raw.split(sep).collect();
    if parts.len() != 3 {
        return None;
    }
    let nums: Vec<i32> = parts
        .iter()
        .map(|p| p.parse::<i32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let (y, m, d) = assign_ymd(
        nums[0],
        nums[1],
        nums[2],
        plan.locale.info().date_order,
        allow_two_digit_year,
    )?;
    let date = civil(y, m, d)?;
    let serial = date_to_serial(date, plan.date_system)? as f64;
    Some(Converted::Date {
        serial,
        num_fmt: 14,
    })
}

fn date_sep(raw: &str) -> Option<char> {
    let mut found = None;
    for c in raw.chars() {
        if matches!(c, '/' | '.' | '-') {
            match found {
                None => found = Some(c),
                Some(s) if s != c => return None,
                Some(_) => {}
            }
        } else if !c.is_ascii_digit() {
            return None;
        }
    }
    found
}

fn assign_ymd(
    a: i32,
    b: i32,
    c: i32,
    order: DateOrder,
    allow_two_digit_year: bool,
) -> Option<(i32, u8, u8)> {
    if a >= 1000 {
        return Some((a, b.try_into().ok()?, c.try_into().ok()?));
    }
    let (year_raw, m, d) = match order {
        DateOrder::Mdy => (c, a, b),
        DateOrder::Dmy => (c, b, a),
        DateOrder::Ymd => (a, b, c),
    };
    let year = expand_year(year_raw, allow_two_digit_year)?;
    Some((year, m.try_into().ok()?, d.try_into().ok()?))
}

fn expand_year(y: i32, allow_two_digit: bool) -> Option<i32> {
    if (1000..=9999).contains(&y) {
        Some(y)
    } else if allow_two_digit && (0..100).contains(&y) {
        Some(if y <= 29 { 2000 + y } else { 1900 + y })
    } else {
        None
    }
}

fn parse_month_name_date(raw: &str, plan: &ImportPlan) -> Option<Converted> {
    let info = plan.locale.info();
    let en = LocaleId::EN_US.info();
    let lower = raw.to_ascii_lowercase();
    let (month, matched) = find_month(&lower, *info).or_else(|| find_month(&lower, *en))?;
    let rest = lower.replacen(&matched, " ", 1);
    let tokens: Vec<&str> = rest
        .split([' ', ',', '-', '/', '.'])
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() != 2 {
        return None;
    }
    let (d, y) = if tokens[0].len() == 4 {
        (
            tokens[1].parse::<u8>().ok()?,
            tokens[0].parse::<i32>().ok()?,
        )
    } else if tokens[1].len() == 4 {
        (
            tokens[0].parse::<u8>().ok()?,
            tokens[1].parse::<i32>().ok()?,
        )
    } else {
        return None;
    };
    let date = civil(y, month, d)?;
    let serial = date_to_serial(date, plan.date_system)? as f64;
    Some(Converted::Date {
        serial,
        num_fmt: 14,
    })
}

fn find_month(lower: &str, info: LocaleInfo) -> Option<(u8, String)> {
    let mut best: Option<(u8, String)> = None;
    for (i, name) in info.months_full.iter().enumerate() {
        let n = name.to_ascii_lowercase();
        if n.len() >= 3
            && lower.contains(&n)
            && best.as_ref().is_none_or(|(_, b)| n.len() > b.len())
        {
            best = Some((i as u8 + 1, n));
        }
    }
    for (i, name) in info.months_abbr.iter().enumerate() {
        let n = name.to_ascii_lowercase();
        if n.len() >= 3
            && lower.contains(&n)
            && best.as_ref().is_none_or(|(_, b)| n.len() > b.len())
        {
            best = Some((i as u8 + 1, n));
        }
    }
    best
}

fn parse_time_only(raw: &str, _plan: &ImportPlan) -> Option<Converted> {
    if raw
        .chars()
        .any(|c| c.is_ascii_alphabetic() && !matches!(c, 'a' | 'A' | 'p' | 'P' | 'm' | 'M'))
    {
        return None;
    }
    let frac = parse_time_fraction(raw)?;
    Some(Converted::Date {
        serial: frac,
        num_fmt: 21,
    })
}

fn parse_time_fraction(raw: &str) -> Option<f64> {
    let s = raw.trim();
    let (body, pm) = parse_ampm(s)?;
    let parts: Vec<&str> = body.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let sec: u32 = if parts.len() == 3 {
        parts[2].parse().ok()?
    } else {
        0
    };
    if m > 59 || sec > 59 {
        return None;
    }
    if let Some(is_pm) = pm {
        if h == 0 || h > 12 {
            return None;
        }
        if is_pm && h < 12 {
            h += 12;
        }
        if !is_pm && h == 12 {
            h = 0;
        }
    } else if h > 23 {
        return None;
    }
    Some(f64::from(h * 3600 + m * 60 + sec) / 86_400.0)
}

fn parse_ampm(s: &str) -> Option<(&str, Option<bool>)> {
    let t = s.trim();
    for (suf, pm) in [(" PM", true), (" AM", false), ("PM", true), ("AM", false)] {
        if let Some(head) = t.strip_suffix(suf) {
            return Some((head.trim(), Some(pm)));
        }
        let low = suf.to_ascii_lowercase();
        if let Some(head) = t.strip_suffix(low.as_str()) {
            return Some((head.trim(), Some(pm)));
        }
    }
    Some((t, None))
}

fn parse_format_date(raw: &str, format: &str, plan: &ImportPlan) -> Option<Converted> {
    let tokens = tokenize_format(format);
    let mut rest = raw;
    let mut year = None;
    let mut month = None;
    let mut day = None;
    let mut hour = None;
    let mut minute = None;
    let mut second = None;
    for tok in tokens {
        match tok {
            FmtTok::Lit(lit) => rest = rest.strip_prefix(&lit)?,
            FmtTok::Yyyy => {
                let (n, next) = take_digits(rest, 4, 4)?;
                year = Some(n);
                rest = next;
            }
            FmtTok::Yy => {
                let (n, next) = take_digits(rest, 2, 2)?;
                year = expand_year(n, true);
                rest = next;
            }
            FmtTok::Mmmm | FmtTok::Mmm => {
                let lower = rest.to_ascii_lowercase();
                let (m, matched) = find_month(&lower, *plan.locale.info())
                    .or_else(|| find_month(&lower, *LocaleId::EN_US.info()))?;
                let prefix = rest.get(..matched.len())?;
                if !prefix.eq_ignore_ascii_case(&matched) {
                    return None;
                }
                month = Some(m);
                rest = &rest[prefix.len()..];
            }
            FmtTok::Mm | FmtTok::M => {
                let (n, next) = take_digits(rest, 1, 2)?;
                month = Some(u8::try_from(n).ok()?);
                rest = next;
            }
            FmtTok::Dd | FmtTok::D => {
                let (n, next) = take_digits(rest, 1, 2)?;
                day = Some(u8::try_from(n).ok()?);
                rest = next;
            }
            FmtTok::Hh | FmtTok::H => {
                let (n, next) = take_digits(rest, 1, 2)?;
                hour = Some(n as u32);
                rest = next;
            }
            FmtTok::Ss | FmtTok::S => {
                let (n, next) = take_digits(rest, 1, 2)?;
                second = Some(n as u32);
                rest = next;
            }
            FmtTok::Min => {
                let (n, next) = take_digits(rest, 1, 2)?;
                minute = Some(n as u32);
                rest = next;
            }
        }
    }
    if !rest.is_empty() {
        return None;
    }
    let y = year?;
    let mo = month?;
    let d = day?;
    let date = civil(y, mo, d)?;
    let mut serial = date_to_serial(date, plan.date_system)? as f64;
    let mut num_fmt = 14u32;
    if hour.is_some() || minute.is_some() || second.is_some() {
        let h = hour.unwrap_or(0);
        let mi = minute.unwrap_or(0);
        let s = second.unwrap_or(0);
        if h > 23 || mi > 59 || s > 59 {
            return None;
        }
        serial += f64::from(h * 3600 + mi * 60 + s) / 86_400.0;
        num_fmt = 22;
    }
    Some(Converted::Date { serial, num_fmt })
}

#[derive(Clone, Debug)]
enum FmtTok {
    Yyyy,
    Yy,
    Mmmm,
    Mmm,
    Mm,
    M,
    Dd,
    D,
    Hh,
    H,
    Min,
    Ss,
    S,
    Lit(String),
}

fn is_minute_context(out: &[FmtTok]) -> bool {
    match out.last() {
        Some(FmtTok::H | FmtTok::Hh) => true,
        Some(FmtTok::Lit(s)) if s.ends_with(':') => true,
        _ => false,
    }
}

fn tokenize_format(format: &str) -> Vec<FmtTok> {
    let mut out = Vec::new();
    let lower = format.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if lower[i..].starts_with("yyyy") {
            out.push(FmtTok::Yyyy);
            i += 4;
        } else if lower[i..].starts_with("yy") {
            out.push(FmtTok::Yy);
            i += 2;
        } else if lower[i..].starts_with("mmmm") {
            out.push(FmtTok::Mmmm);
            i += 4;
        } else if lower[i..].starts_with("mmm") {
            out.push(FmtTok::Mmm);
            i += 3;
        } else if lower[i..].starts_with("mm") {
            // `mm` after hour is minutes; before a date day is month. The
            // corpus uses `mm/dd/yyyy` (month). Treat `mm` as month unless the
            // previous token was an hour.
            if is_minute_context(&out) {
                out.push(FmtTok::Min);
            } else {
                out.push(FmtTok::Mm);
            }
            i += 2;
        } else if bytes[i] == b'm' {
            if is_minute_context(&out) {
                out.push(FmtTok::Min);
            } else {
                out.push(FmtTok::M);
            }
            i += 1;
        } else if lower[i..].starts_with("dd") {
            out.push(FmtTok::Dd);
            i += 2;
        } else if bytes[i] == b'd' {
            out.push(FmtTok::D);
            i += 1;
        } else if lower[i..].starts_with("hh") {
            out.push(FmtTok::Hh);
            i += 2;
        } else if bytes[i] == b'h' {
            out.push(FmtTok::H);
            i += 1;
        } else if lower[i..].starts_with("ss") {
            out.push(FmtTok::Ss);
            i += 2;
        } else if bytes[i] == b's' {
            out.push(FmtTok::S);
            i += 1;
        } else {
            let ch = format[i..].chars().next().unwrap_or('?');
            if let Some(FmtTok::Lit(s)) = out.last_mut() {
                s.push(ch);
            } else {
                out.push(FmtTok::Lit(ch.to_string()));
            }
            i += ch.len_utf8();
        }
    }
    out
}

fn take_digits(s: &str, min: usize, max: usize) -> Option<(i32, &str)> {
    let n_digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    if n_digits < min {
        return None;
    }
    let take = n_digits.min(max);
    let (num, rest) = s.split_at(take);
    Some((num.parse().ok()?, rest))
}

fn civil(year: i32, month: u8, day: u8) -> Option<CivilDate> {
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let date = CivilDate {
        year,
        month,
        day,
        lotus_leap: false,
    };
    date_to_serial(date, omacell_core::date_system::DateSystem::Excel1900)?;
    Some(date)
}

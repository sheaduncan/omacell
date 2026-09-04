//! REGEXTEST / REGEXEXTRACT / REGEXREPLACE with size limits.

use std::sync::Arc;

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, RuntimeValue};

use crate::util::{
    MAX_EXCEL_TEXT, boolean, err, optional, text, to_number, to_text, too_long, trunc_i64,
};

const MAX_REGEX_PATTERN: usize = 256;
const REGEX_SIZE_LIMIT: usize = 1 << 20;

pub(crate) fn regextest_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let pat = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let ins = match optional_case(ctx, args, 2) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    if s.chars().count() > MAX_EXCEL_TEXT {
        return err(ErrorKind::Value);
    }
    match compile_regex(&pat, ins) {
        Ok(re) => boolean(re.is_match(&s)),
        Err(e) => err(e),
    }
}

pub(crate) fn regexextract_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let pat = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let mode = match optional(args, 2) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 0,
    };
    let ins = match optional_case(ctx, args, 3) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    if s.chars().count() > MAX_EXCEL_TEXT {
        return err(ErrorKind::Value);
    }
    let re = match compile_regex(&pat, ins) {
        Ok(re) => re,
        Err(e) => return err(e),
    };
    match mode {
        0 => match re.find(&s) {
            Some(m) => text(m.as_str()),
            None => err(ErrorKind::Na),
        },
        1 => {
            let hits: Vec<Scalar> = re
                .find_iter(&s)
                .map(|m| Scalar::Text(Arc::from(m.as_str())))
                .collect();
            if hits.is_empty() {
                err(ErrorKind::Na)
            } else {
                RuntimeValue::array(1, hits.len() as u32, hits)
            }
        }
        2 => {
            let Some(caps) = re.captures(&s) else {
                return err(ErrorKind::Na);
            };
            if caps.len() <= 1 {
                return err(ErrorKind::Na);
            }
            let groups: Vec<Scalar> = caps
                .iter()
                .skip(1)
                .map(|m| match m {
                    Some(mm) => Scalar::Text(Arc::from(mm.as_str())),
                    None => Scalar::Empty,
                })
                .collect();
            RuntimeValue::array(1, groups.len() as u32, groups)
        }
        _ => err(ErrorKind::Value),
    }
}

pub(crate) fn regexreplace_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let pat = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let repl = match to_text(ctx, &args[2]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let occurrence = match optional(args, 3) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 0,
    };
    let ins = match optional_case(ctx, args, 4) {
        Ok(b) => b,
        Err(e) => return err(e),
    };
    if s.chars().count() > MAX_EXCEL_TEXT {
        return err(ErrorKind::Value);
    }
    let re = match compile_regex(&pat, ins) {
        Ok(re) => re,
        Err(e) => return err(e),
    };
    match replace_capped(&re, &s, &repl, occurrence) {
        Ok(out) => text(out),
        Err(e) => err(e),
    }
}

fn replace_capped(
    re: &regex::Regex,
    source: &str,
    replacement: &str,
    occurrence: i64,
) -> Result<String, ErrorKind> {
    let occurrence = if occurrence < 0 {
        let from_end = occurrence.unsigned_abs();
        let match_count =
            u64::try_from(re.find_iter(source).count()).map_err(|_| ErrorKind::Value)?;
        if from_end > match_count {
            let mut out = String::new();
            let mut char_count = 0usize;
            push_capped(&mut out, source, &mut char_count)?;
            return Ok(out);
        }
        i64::try_from(match_count - from_end + 1).map_err(|_| ErrorKind::Value)?
    } else {
        occurrence
    };
    let mut out = String::new();
    let mut char_count = 0usize;
    let mut last_end = 0usize;
    let mut seen = 0i64;
    for captures in re.captures_iter(source) {
        let Some(matched) = captures.get(0) else {
            continue;
        };
        push_capped(
            &mut out,
            source.get(last_end..matched.start()).unwrap_or(""),
            &mut char_count,
        )?;
        seen += 1;
        if occurrence == 0 || seen == occurrence {
            push_expanded_capped(&mut out, replacement, &captures, &mut char_count)?;
        } else {
            push_capped(&mut out, matched.as_str(), &mut char_count)?;
        }
        last_end = matched.end();
        if occurrence > 0 && seen == occurrence {
            push_capped(
                &mut out,
                source.get(last_end..).unwrap_or(""),
                &mut char_count,
            )?;
            return Ok(out);
        }
    }
    push_capped(
        &mut out,
        source.get(last_end..).unwrap_or(""),
        &mut char_count,
    )?;
    Ok(out)
}

fn push_expanded_capped(
    out: &mut String,
    replacement: &str,
    captures: &regex::Captures<'_>,
    char_count: &mut usize,
) -> Result<(), ErrorKind> {
    let mut rest = replacement;
    while !rest.is_empty() {
        let Some(dollar) = rest.find('$') else {
            return push_capped(out, rest, char_count);
        };
        push_capped(out, &rest[..dollar], char_count)?;
        rest = &rest[dollar..];
        if rest.as_bytes().get(1) == Some(&b'$') {
            push_capped(out, "$", char_count)?;
            rest = &rest[2..];
            continue;
        }
        let Some((reference, end)) = parse_capture_reference(rest) else {
            push_capped(out, "$", char_count)?;
            rest = &rest[1..];
            continue;
        };
        let matched = match reference {
            CaptureReference::Index(index) => captures.get(index),
            CaptureReference::Name(name) => captures.name(name),
        };
        if let Some(matched) = matched {
            push_capped(out, matched.as_str(), char_count)?;
        }
        rest = &rest[end..];
    }
    Ok(())
}

enum CaptureReference<'a> {
    Index(usize),
    Name(&'a str),
}

fn parse_capture_reference(replacement: &str) -> Option<(CaptureReference<'_>, usize)> {
    let bytes = replacement.as_bytes();
    if bytes.first() != Some(&b'$') || bytes.len() <= 1 {
        return None;
    }
    let (name, end) = if bytes[1] == b'{' {
        let close = replacement.get(2..)?.find('}')?;
        let end = 2usize.checked_add(close)?.checked_add(1)?;
        (replacement.get(2..end.saturating_sub(1))?, end)
    } else {
        let mut end = 1usize;
        while bytes.get(end).is_some_and(u8::is_ascii_alphanumeric) || bytes.get(end) == Some(&b'_')
        {
            end += 1;
        }
        if end == 1 {
            return None;
        }
        (replacement.get(1..end)?, end)
    };
    let reference = match name.parse::<usize>() {
        Ok(index) => CaptureReference::Index(index),
        Err(_) => CaptureReference::Name(name),
    };
    Some((reference, end))
}

fn push_capped(out: &mut String, value: &str, char_count: &mut usize) -> Result<(), ErrorKind> {
    *char_count = char_count
        .checked_add(value.chars().count())
        .ok_or(ErrorKind::Value)?;
    too_long(*char_count)?;
    out.push_str(value);
    Ok(())
}

fn optional_case(ctx: &mut EvalCtx<'_>, args: &[ArgVal], index: usize) -> Result<bool, ErrorKind> {
    match optional(args, index) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ErrorKind::Value),
        },
        None => Ok(false),
    }
}

fn compile_regex(pat: &str, case_insensitive: bool) -> Result<regex::Regex, ErrorKind> {
    if pat.chars().count() > MAX_REGEX_PATTERN {
        return Err(ErrorKind::Value);
    }
    regex::RegexBuilder::new(pat)
        .case_insensitive(case_insensitive)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(32)
        .build()
        .map_err(|_| ErrorKind::Value)
}

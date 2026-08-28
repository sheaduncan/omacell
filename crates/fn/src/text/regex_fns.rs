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
            Ok(n) if n < 0 => return err(ErrorKind::Value),
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
    let mut i = 0i64;
    let out = re.replace_all(&s, |caps: &regex::Captures<'_>| {
        i += 1;
        if occurrence == 0 || i == occurrence {
            let mut dest = String::new();
            caps.expand(&repl, &mut dest);
            dest
        } else {
            caps.get(0)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default()
        }
    });
    if too_long(out.chars().count()).is_err() {
        return err(ErrorKind::Value);
    }
    text(out.into_owned())
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

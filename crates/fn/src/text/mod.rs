//! Tier-0 text functions (spec §6.4, Appendix D).

mod format;
mod parse;
mod regex_fns;
mod split;

use omacell_core::coerce::{self, Scalar};
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, FnBody, FnRegistry, RuntimeValue};

use crate::metadata::{ArgKind, ArrayBehavior, FunctionSpec};
use crate::util::{
    self, MAX_EXCEL_TEXT, boolean, chars_of, collect_scalars, excel_lower, excel_lower_char,
    excel_upper, excel_upper_char, last_chars, number, optional, scalar, skip_take_chars,
    take_chars, text, to_bool, to_number, to_text, too_long, trunc_i64, trunc_u32_len,
};

pub(crate) use parse::{civil_serial, parse_date_string, parse_time_string};

use format::{arraytotext_impl, dollar_impl, fixed_impl, text_impl, valuetotext_impl};
use parse::{numbervalue_impl, value_impl};
use regex_fns::{regexextract_impl, regexreplace_impl, regextest_impl};
use split::{search_impl, textafter_impl, textbefore_impl, textsplit_impl};

/// Text-function specs in declaration order (JSON output is re-sorted).
pub const TEXT_SPECS: &[FunctionSpec] = &[
    LEN,
    LEFT,
    RIGHT,
    MID,
    FIND,
    SEARCH,
    SUBSTITUTE,
    REPLACE,
    UPPER,
    LOWER,
    PROPER,
    TRIM,
    CLEAN,
    CONCAT,
    TEXTJOIN,
    TEXTSPLIT,
    TEXTBEFORE,
    TEXTAFTER,
    TEXT,
    VALUE,
    NUMBERVALUE,
    FIXED,
    DOLLAR,
    REPT,
    CHAR,
    CODE,
    UNICHAR,
    UNICODE,
    EXACT,
    T,
    ARRAYTOTEXT,
    VALUETOTEXT,
    REGEXTEST,
    REGEXEXTRACT,
    REGEXREPLACE,
];

/// Register text functions (and aliases) onto `registry`.
pub fn register_text(registry: &mut FnRegistry) {
    util::register_specs(registry, TEXT_SPECS);
}

macro_rules! text_fn {
    ($id:ident, $name:literal, $args:expr, $min:expr, $max:expr, $array:expr, $sig:literal, $doc:literal, $body:expr) => {
        crate::define_fn! {
            const $id = {
                name: $name,
                aliases: &[],
                tier: 0,
                category: "text",
                arg_kinds: $args,
                min_args: $min,
                max_args: $max,
                volatile: false,
                array: $array,
                async_node: false,
                signature: $sig,
                doc: $doc,
                body: FnBody::Eager($body),
            };
        }
    };
}

text_fn!(
    LEN,
    "LEN",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "LEN(text)",
    "Number of Unicode scalar values in text. Excel Windows counts UTF-16 code units.",
    len_impl
);
text_fn!(
    LEFT,
    "LEFT",
    &[ArgKind::Text, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "LEFT(text, [num_chars])",
    "Leftmost Unicode scalars. `num_chars` defaults to 1.",
    left_impl
);
text_fn!(
    RIGHT,
    "RIGHT",
    &[ArgKind::Text, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "RIGHT(text, [num_chars])",
    "Rightmost Unicode scalars. `num_chars` defaults to 1.",
    right_impl
);
text_fn!(
    MID,
    "MID",
    &[ArgKind::Text, ArgKind::Number, ArgKind::Number],
    3,
    3,
    ArrayBehavior::LiftAll,
    "MID(text, start_num, num_chars)",
    "Substring of Unicode scalars. `start_num` is 1-based.",
    mid_impl
);
text_fn!(
    FIND,
    "FIND",
    &[ArgKind::Text, ArgKind::Text, ArgKind::Number],
    2,
    3,
    ArrayBehavior::LiftAll,
    "FIND(find_text, within_text, [start_num])",
    "1-based case-sensitive search without wildcards.",
    find_impl
);
text_fn!(
    SEARCH,
    "SEARCH",
    &[ArgKind::Text, ArgKind::Text, ArgKind::Number],
    2,
    3,
    ArrayBehavior::LiftAll,
    "SEARCH(find_text, within_text, [start_num])",
    "1-based case-insensitive search with `*` `?` `~` wildcards.",
    search_impl
);
text_fn!(
    SUBSTITUTE,
    "SUBSTITUTE",
    &[ArgKind::Text, ArgKind::Text, ArgKind::Text, ArgKind::Number],
    3,
    4,
    ArrayBehavior::LiftAll,
    "SUBSTITUTE(text, old_text, new_text, [instance_num])",
    "Case-sensitive replace of `old_text`. Omitted instance replaces all.",
    substitute_impl
);
text_fn!(
    REPLACE,
    "REPLACE",
    &[
        ArgKind::Text,
        ArgKind::Number,
        ArgKind::Number,
        ArgKind::Text
    ],
    4,
    4,
    ArrayBehavior::LiftAll,
    "REPLACE(old_text, start_num, num_chars, new_text)",
    "Replace a Unicode-scalar span. `start_num` is 1-based.",
    replace_impl
);
text_fn!(
    UPPER,
    "UPPER",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "UPPER(text)",
    "Simple one-to-one uppercase (`ß` stays `ß`).",
    upper_impl
);
text_fn!(
    LOWER,
    "LOWER",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "LOWER(text)",
    "Simple one-to-one lowercase.",
    lower_impl
);
text_fn!(
    PROPER,
    "PROPER",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "PROPER(text)",
    "Capitalize the first letter after every non-letter.",
    proper_impl
);
text_fn!(
    TRIM,
    "TRIM",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "TRIM(text)",
    "Trim and collapse ASCII spaces (`U+0020` only).",
    trim_impl
);
text_fn!(
    CLEAN,
    "CLEAN",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "CLEAN(text)",
    "Remove characters `U+0000`–`U+001F`.",
    clean_impl
);
text_fn!(
    CONCAT,
    "CONCAT",
    &[ArgKind::Any],
    1,
    255,
    ArrayBehavior::None,
    "CONCAT(text1, [text2], ...)",
    "Concatenate values, flattening arrays and ranges.",
    concat_impl
);
text_fn!(
    TEXTJOIN,
    "TEXTJOIN",
    &[ArgKind::Text, ArgKind::Logical, ArgKind::Any],
    3,
    255,
    ArrayBehavior::None,
    "TEXTJOIN(delimiter, ignore_empty, text1, [text2], ...)",
    "Join values with a delimiter, optionally skipping empties.",
    textjoin_impl
);
text_fn!(
    TEXTSPLIT,
    "TEXTSPLIT",
    &[
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Logical,
        ArgKind::Number,
        ArgKind::Any
    ],
    2,
    6,
    ArrayBehavior::ReturnsArray,
    "TEXTSPLIT(text, col_delimiter, [row_delimiter], [ignore_empty], [match_mode], [pad_with])",
    "Split text into a spilled array.",
    textsplit_impl
);
text_fn!(
    TEXTBEFORE,
    "TEXTBEFORE",
    &[
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Number,
        ArgKind::Number,
        ArgKind::Number,
        ArgKind::Any
    ],
    2,
    6,
    ArrayBehavior::LiftAll,
    "TEXTBEFORE(text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found])",
    "Text before a delimiter. Negative instance counts from the end.",
    textbefore_impl
);
text_fn!(
    TEXTAFTER,
    "TEXTAFTER",
    &[
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Number,
        ArgKind::Number,
        ArgKind::Number,
        ArgKind::Any
    ],
    2,
    6,
    ArrayBehavior::LiftAll,
    "TEXTAFTER(text, delimiter, [instance_num], [match_mode], [match_end], [if_not_found])",
    "Text after a delimiter. Negative instance counts from the end.",
    textafter_impl
);
text_fn!(
    TEXT,
    "TEXT",
    &[ArgKind::Any, ArgKind::Text],
    2,
    2,
    ArrayBehavior::LiftAll,
    "TEXT(value, format_text)",
    "Format a value with an Excel number-format code (WP-06).",
    text_impl
);
text_fn!(
    VALUE,
    "VALUE",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "VALUE(text)",
    "Parse locale-aware numbers, dates, and times.",
    value_impl
);
text_fn!(
    NUMBERVALUE,
    "NUMBERVALUE",
    &[ArgKind::Text, ArgKind::Text, ArgKind::Text],
    1,
    3,
    ArrayBehavior::LiftAll,
    "NUMBERVALUE(text, [decimal_separator], [group_separator])",
    "Parse a number with explicit or locale separators.",
    numbervalue_impl
);
text_fn!(
    FIXED,
    "FIXED",
    &[ArgKind::Number, ArgKind::Number, ArgKind::Logical],
    1,
    3,
    ArrayBehavior::LiftAll,
    "FIXED(number, [decimals], [no_commas])",
    "Round and format a number as text. Half away from zero.",
    fixed_impl
);
text_fn!(
    DOLLAR,
    "DOLLAR",
    &[ArgKind::Number, ArgKind::Number],
    1,
    2,
    ArrayBehavior::LiftAll,
    "DOLLAR(number, [decimals])",
    "Currency text using the pass locale. Negatives in parentheses.",
    dollar_impl
);
text_fn!(
    REPT,
    "REPT",
    &[ArgKind::Text, ArgKind::Number],
    2,
    2,
    ArrayBehavior::LiftAll,
    "REPT(text, number_times)",
    "Repeat text. Count is truncated toward zero; over 32,767 characters is `#VALUE!`.",
    rept_impl
);
text_fn!(
    CHAR,
    "CHAR",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "CHAR(number)",
    "Latin-1 character for codes 1–255 (not Windows-1252).",
    char_impl
);
text_fn!(
    CODE,
    "CODE",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "CODE(text)",
    "Latin-1 code of the first scalar if it is in 1–255.",
    code_impl
);
text_fn!(
    UNICHAR,
    "UNICHAR",
    &[ArgKind::Number],
    1,
    1,
    ArrayBehavior::LiftAll,
    "UNICHAR(number)",
    "Unicode scalar value. Surrogates and 0 are `#VALUE!`.",
    unichar_impl
);
text_fn!(
    UNICODE,
    "UNICODE",
    &[ArgKind::Text],
    1,
    1,
    ArrayBehavior::LiftAll,
    "UNICODE(text)",
    "Code point of the first Unicode scalar.",
    unicode_impl
);
text_fn!(
    EXACT,
    "EXACT",
    &[ArgKind::Text, ArgKind::Text],
    2,
    2,
    ArrayBehavior::LiftAll,
    "EXACT(text1, text2)",
    "Case-sensitive equality of the two values as text.",
    exact_impl
);
text_fn!(
    T,
    "T",
    &[ArgKind::Any],
    1,
    1,
    ArrayBehavior::LiftAll,
    "T(value)",
    "Return text unchanged; numbers and bools become empty; errors propagate.",
    t_impl
);
text_fn!(
    ARRAYTOTEXT,
    "ARRAYTOTEXT",
    &[ArgKind::Any, ArgKind::Number],
    1,
    2,
    ArrayBehavior::None,
    "ARRAYTOTEXT(array, [format])",
    "Serialize an array: 0 concise, 1 strict.",
    arraytotext_impl
);
text_fn!(
    VALUETOTEXT,
    "VALUETOTEXT",
    &[ArgKind::Any, ArgKind::Number],
    1,
    2,
    ArrayBehavior::None,
    "VALUETOTEXT(value, [format])",
    "Serialize a value: 0 concise, 1 strict.",
    valuetotext_impl
);
text_fn!(
    REGEXTEST,
    "REGEXTEST",
    &[ArgKind::Text, ArgKind::Text, ArgKind::Number],
    2,
    3,
    ArrayBehavior::LiftAll,
    "REGEXTEST(text, pattern, [case_sensitivity])",
    "True if the regex matches. Pattern length and compile size are bounded.",
    regextest_impl
);
text_fn!(
    REGEXEXTRACT,
    "REGEXEXTRACT",
    &[
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Number,
        ArgKind::Number
    ],
    2,
    4,
    ArrayBehavior::LiftAll,
    "REGEXEXTRACT(text, pattern, [return_mode], [case_sensitivity])",
    "Extract regex matches. return_mode 0 first, 1 all, 2 capturing groups.",
    regexextract_impl
);
text_fn!(
    REGEXREPLACE,
    "REGEXREPLACE",
    &[
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Text,
        ArgKind::Number,
        ArgKind::Number
    ],
    3,
    5,
    ArrayBehavior::LiftAll,
    "REGEXREPLACE(text, pattern, replacement, [occurrence], [case_sensitivity])",
    "Replace regex matches. occurrence 0 (default) replaces all.",
    regexreplace_impl
);

fn len_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match to_text(ctx, &args[0]) {
        Ok(s) => number(s.chars().count() as f64),
        Err(e) => util::err(e),
    }
}

fn left_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let n = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_u32_len) {
            Ok(n) => n,
            Err(e) => return util::err(e),
        },
        None => 1,
    };
    text(take_chars(&s, n as usize))
}

fn right_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let n = match optional(args, 1) {
        Some(a) => match to_number(ctx, a).and_then(trunc_u32_len) {
            Ok(n) => n,
            Err(e) => return util::err(e),
        },
        None => 1,
    };
    text(last_chars(&s, n as usize))
}

fn mid_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let start = match to_number(ctx, &args[1]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return util::err(e),
    };
    let n = match to_number(ctx, &args[2]).and_then(trunc_u32_len) {
        Ok(n) => n,
        Err(e) => return util::err(e),
    };
    if start < 1 {
        return util::err(ErrorKind::Value);
    }
    text(skip_take_chars(
        &s,
        (start as usize).saturating_sub(1),
        n as usize,
    ))
}

fn find_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let needle = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let hay = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let start = match optional(args, 2) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return util::err(e),
        },
        None => 1,
    };
    match find_chars(&hay, &needle, start) {
        Ok(i) => number(i as f64),
        Err(e) => util::err(e),
    }
}

pub(crate) fn find_chars(hay: &str, needle: &str, start: i64) -> Result<i64, ErrorKind> {
    if start < 1 {
        return Err(ErrorKind::Value);
    }
    let hay_c: Vec<char> = hay.chars().collect();
    let needle_c: Vec<char> = needle.chars().collect();
    let start_idx = (start as usize).saturating_sub(1);
    if start_idx > hay_c.len() {
        return Err(ErrorKind::Value);
    }
    if needle_c.is_empty() {
        return Ok(start);
    }
    if start_idx >= hay_c.len() || needle_c.len() > hay_c.len() {
        return Err(ErrorKind::Value);
    }
    let nlen = needle_c.len();
    for i in start_idx..=hay_c.len() - nlen {
        if hay_c[i..i + nlen] == needle_c[..] {
            return Ok((i + 1) as i64);
        }
    }
    Err(ErrorKind::Value)
}

fn substitute_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let src = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let old = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let new = match to_text(ctx, &args[2]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let instance = match optional(args, 3) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) if n < 1 => return util::err(ErrorKind::Value),
            Ok(n) => Some(n as u64),
            Err(e) => return util::err(e),
        },
        None => None,
    };
    if old.is_empty() {
        return text(src);
    }
    let mut out = String::new();
    let mut seen = 0u64;
    let mut rest = src.as_ref();
    while let Some(idx) = rest.find(old.as_ref()) {
        seen += 1;
        out.push_str(&rest[..idx]);
        if instance.is_none() || instance == Some(seen) {
            out.push_str(&new);
        } else {
            out.push_str(&old);
        }
        rest = &rest[idx + old.len()..];
    }
    out.push_str(rest);
    if too_long(out.chars().count()).is_err() {
        return util::err(ErrorKind::Value);
    }
    text(out)
}

fn replace_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let src = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let start = match to_number(ctx, &args[1]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return util::err(e),
    };
    let n = match to_number(ctx, &args[2]).and_then(trunc_u32_len) {
        Ok(n) => n,
        Err(e) => return util::err(e),
    };
    let new = match to_text(ctx, &args[3]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    if start < 1 {
        return util::err(ErrorKind::Value);
    }
    let chars = chars_of(&src);
    let start_idx = (start as usize).saturating_sub(1);
    let prefix: String = chars.iter().take(start_idx).collect();
    let suffix: String = chars.iter().skip(start_idx + n as usize).collect();
    let out = format!("{prefix}{new}{suffix}");
    if too_long(out.chars().count()).is_err() {
        return util::err(ErrorKind::Value);
    }
    text(out)
}

fn upper_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match to_text(ctx, &args[0]) {
        Ok(s) => text(excel_upper(&s)),
        Err(e) => util::err(e),
    }
}

fn lower_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match to_text(ctx, &args[0]) {
        Ok(s) => text(excel_lower(&s)),
        Err(e) => util::err(e),
    }
}

fn proper_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let mut cap = true;
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphabetic() {
            out.push(if cap {
                excel_upper_char(c)
            } else {
                excel_lower_char(c)
            });
            cap = false;
        } else {
            out.push(c);
            cap = true;
        }
    }
    text(out)
}

fn trim_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let trimmed = s.trim_matches(' ');
    let mut out = String::new();
    let mut prev_space = false;
    for c in trimmed.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            prev_space = false;
            out.push(c);
        }
    }
    text(out)
}

fn clean_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    text(s.chars().filter(|c| (*c as u32) >= 32).collect::<String>())
}

fn concat_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    join_values(ctx, args, 0, "", false)
}

fn textjoin_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let delim = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let ignore = match to_bool(ctx, &args[1]) {
        Ok(b) => b,
        Err(e) => return util::err(e),
    };
    join_values(ctx, args, 2, &delim, ignore)
}

fn join_values(
    ctx: &mut EvalCtx<'_>,
    args: &[ArgVal],
    start: usize,
    delim: &str,
    ignore_empty: bool,
) -> RuntimeValue {
    let scalars = match collect_scalars(ctx, args, start) {
        Ok(v) => v,
        Err(e) => return util::err(e),
    };
    let mut parts = Vec::new();
    for s in scalars {
        if let Some(e) = s.error() {
            return util::err(e);
        }
        let t = match coerce::to_text(&s) {
            Ok(t) => t,
            Err(e) => return util::err(e),
        };
        if ignore_empty && t.is_empty() {
            continue;
        }
        parts.push(t);
    }
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push_str(delim);
        }
        out.push_str(p);
        if out.chars().count() > MAX_EXCEL_TEXT {
            return util::err(ErrorKind::Value);
        }
    }
    text(out)
}

fn rept_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let n = match to_number(ctx, &args[1]).and_then(trunc_i64) {
        Ok(n) if n < 0 => return util::err(ErrorKind::Value),
        Ok(n) => n as usize,
        Err(e) => return util::err(e),
    };
    let chars = s.chars().count();
    if chars.saturating_mul(n) > MAX_EXCEL_TEXT {
        return util::err(ErrorKind::Value);
    }
    text(s.repeat(n))
}

fn char_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let n = match to_number(ctx, &args[0]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return util::err(e),
    };
    if !(1..=255).contains(&n) {
        return util::err(ErrorKind::Value);
    }
    match char::from_u32(n as u32) {
        Some(c) => text(c.to_string()),
        None => util::err(ErrorKind::Value),
    }
}

fn code_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    match s.chars().next() {
        Some(c) if (1..=255).contains(&(c as u32)) => number(c as u32 as f64),
        Some(_) | None => util::err(ErrorKind::Value),
    }
}

fn unichar_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let n = match to_number(ctx, &args[0]).and_then(trunc_i64) {
        Ok(n) => n,
        Err(e) => return util::err(e),
    };
    if n <= 0 || n > 0x0010_FFFF {
        return util::err(ErrorKind::Value);
    }
    let u = n as u32;
    if (0xD800..=0xDFFF).contains(&u) {
        return util::err(ErrorKind::Value);
    }
    match char::from_u32(u) {
        Some(c) => text(c.to_string()),
        None => util::err(ErrorKind::Value),
    }
}

fn unicode_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let s = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    match s.chars().next() {
        Some(c) => number(c as u32 as f64),
        None => util::err(ErrorKind::Value),
    }
}

fn exact_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let a = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    let b = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return util::err(e),
    };
    boolean(a.as_ref() == b.as_ref())
}

fn t_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    match scalar(ctx, &args[0]) {
        Ok(Scalar::Text(s)) => text(s),
        Ok(Scalar::Error(e)) => util::err(e),
        Ok(_) => text(""),
        Err(e) => util::err(e),
    }
}

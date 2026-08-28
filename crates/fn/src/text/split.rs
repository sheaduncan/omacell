//! SEARCH wildcards, TEXTSPLIT, TEXTBEFORE, TEXTAFTER.

use std::sync::Arc;

use omacell_core::coerce::Scalar;
use omacell_core::error::ErrorKind;
use omacell_core::eval::{ArgVal, EvalCtx, RuntimeArray, RuntimeValue};

use crate::util::{
    MAX_EXCEL_TEXT, err, excel_lower, number, optional, scalar, text, to_bool, to_number, to_text,
    trunc_i64,
};

const SEARCH_REGEX_SIZE_LIMIT: usize = 4 << 20;

pub(crate) fn search_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let needle = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let hay = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let start = match optional(args, 2) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 1,
    };
    match search_wildcard(&hay, &needle, start) {
        Ok(i) => number(i as f64),
        Err(e) => err(e),
    }
}

pub(crate) fn textsplit_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    let src = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    if src.is_empty() {
        return err(ErrorKind::Na);
    }
    if src.chars().count() > MAX_EXCEL_TEXT {
        return err(ErrorKind::Value);
    }
    let col_delim = match optional(args, 1) {
        Some(a) => match to_text(ctx, a) {
            Ok(s) => Some(s),
            Err(e) => return err(e),
        },
        None => None,
    };
    let row_delim = match optional(args, 2) {
        Some(a) => match to_text(ctx, a) {
            Ok(s) => Some(s),
            Err(e) => return err(e),
        },
        None => None,
    };
    if col_delim.is_none() && row_delim.is_none() {
        return err(ErrorKind::Value);
    }
    let ignore_empty = match optional(args, 3) {
        Some(a) => match to_bool(ctx, a) {
            Ok(b) => b,
            Err(e) => return err(e),
        },
        None => false,
    };
    let insensitive = match optional(args, 4) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(0) => false,
            Ok(1) => true,
            Ok(_) => return err(ErrorKind::Value),
            Err(e) => return err(e),
        },
        None => false,
    };
    let pad = match optional(args, 5) {
        Some(a) => match scalar(ctx, a) {
            Ok(s) => s,
            Err(e) => return err(e),
        },
        None => Scalar::Error(ErrorKind::Na),
    };

    let rows: Vec<String> = if let Some(rd) = row_delim.as_deref().filter(|d| !d.is_empty()) {
        split_keep(&src, rd, insensitive, ignore_empty)
    } else {
        vec![src.to_string()]
    };
    let col_d = col_delim.as_deref().filter(|d| !d.is_empty());
    let mut grid: Vec<Vec<Scalar>> = Vec::new();
    let mut max_cols = 1usize;
    for row in &rows {
        let cols = if let Some(cd) = col_d {
            split_keep(row, cd, insensitive, ignore_empty)
        } else {
            vec![row.clone()]
        };
        max_cols = max_cols.max(cols.len().max(1));
        grid.push(
            cols.into_iter()
                .map(|s| Scalar::Text(Arc::from(s)))
                .collect(),
        );
    }
    if grid.is_empty() {
        return err(ErrorKind::Na);
    }
    let Ok(row_count) = u32::try_from(grid.len()) else {
        return err(ErrorKind::Num);
    };
    let Ok(column_count) = u32::try_from(max_cols) else {
        return err(ErrorKind::Num);
    };
    let Ok(value_count) = RuntimeArray::checked_len(row_count, column_count) else {
        return err(ErrorKind::Num);
    };
    let mut values = Vec::with_capacity(value_count);
    for row in &mut grid {
        while row.len() < max_cols {
            row.push(pad.clone());
        }
        values.extend(row.iter().cloned());
    }
    RuntimeValue::array(row_count, column_count, values)
}

pub(crate) fn textbefore_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    split_around(ctx, args, true)
}

pub(crate) fn textafter_impl(ctx: &mut EvalCtx<'_>, args: &[ArgVal]) -> RuntimeValue {
    split_around(ctx, args, false)
}

fn split_around(ctx: &mut EvalCtx<'_>, args: &[ArgVal], before: bool) -> RuntimeValue {
    let src = match to_text(ctx, &args[0]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let delim = match to_text(ctx, &args[1]) {
        Ok(s) => s,
        Err(e) => return err(e),
    };
    let instance = match optional(args, 2) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(0) => return err(ErrorKind::Value),
            Ok(n) => n,
            Err(e) => return err(e),
        },
        None => 1,
    };
    let insensitive = match optional(args, 3) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(0) => false,
            Ok(1) => true,
            Ok(_) => return err(ErrorKind::Value),
            Err(e) => return err(e),
        },
        None => false,
    };
    let match_end = match optional(args, 4) {
        Some(a) => match to_number(ctx, a).and_then(trunc_i64) {
            Ok(0) => false,
            Ok(1) => true,
            Ok(n) => n != 0,
            Err(e) => return err(e),
        },
        None => false,
    };
    let not_found = optional(args, 5).cloned();

    if delim.is_empty() {
        return if before { text("") } else { text(src) };
    }
    let positions = find_all_delim(&src, &delim, insensitive);
    let pick = if instance > 0 {
        positions
            .get((instance as usize).saturating_sub(1))
            .copied()
    } else {
        let idx = positions.len() as i64 + instance;
        if idx < 0 {
            None
        } else {
            positions.get(idx as usize).copied()
        }
    };
    match pick {
        Some((start, end)) => {
            if before {
                text(src.get(..start).unwrap_or(""))
            } else {
                text(src.get(end..).unwrap_or(""))
            }
        }
        None if match_end => {
            if before {
                text(src)
            } else {
                text("")
            }
        }
        None => {
            if let Some(a) = not_found {
                match ctx.materialize(a.value) {
                    RuntimeValue::Scalar(s) => RuntimeValue::Scalar(s),
                    other => other,
                }
            } else {
                err(ErrorKind::Na)
            }
        }
    }
}

fn search_wildcard(hay: &str, pat: &str, start: i64) -> Result<i64, ErrorKind> {
    if start < 1 {
        return Err(ErrorKind::Value);
    }
    let hay_len = hay.chars().count();
    if hay_len > MAX_EXCEL_TEXT || pat.chars().count() > MAX_EXCEL_TEXT {
        return Err(ErrorKind::Value);
    }
    let start_idx = usize::try_from(start - 1).map_err(|_| ErrorKind::Value)?;
    if start_idx > hay_len {
        return Err(ErrorKind::Value);
    }
    let pat_c = decode_wildcard(&excel_lower(pat));
    if pat_c.is_empty() {
        return Ok(start);
    }
    let hay_lower = excel_lower(hay);
    let start_byte = hay_lower
        .char_indices()
        .nth(start_idx)
        .map_or(hay_lower.len(), |(byte, _)| byte);
    let regex = compile_wildcard_regex(&pat_c)?;
    let found = regex
        .find(&hay_lower[start_byte..])
        .ok_or(ErrorKind::Value)?;
    let skipped = hay_lower[start_byte..start_byte + found.start()]
        .chars()
        .count();
    i64::try_from(start_idx + skipped + 1).map_err(|_| ErrorKind::Value)
}

#[derive(Clone, Copy)]
enum Wild {
    Char(char),
    Any,
    Star,
}

fn decode_wildcard(pat: &str) -> Vec<Wild> {
    let mut out = Vec::new();
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '~' {
            if let Some(n) = chars.next() {
                out.push(Wild::Char(n));
            }
        } else if c == '*' {
            out.push(Wild::Star);
        } else if c == '?' {
            out.push(Wild::Any);
        } else {
            out.push(Wild::Char(c));
        }
    }
    out
}

fn compile_wildcard_regex(pat: &[Wild]) -> Result<regex::Regex, ErrorKind> {
    let mut source = String::with_capacity(pat.len());
    let mut literal = String::new();
    let flush_literal = |source: &mut String, literal: &mut String| {
        if !literal.is_empty() {
            source.push_str(&regex::escape(literal));
            literal.clear();
        }
    };
    let mut previous_star = false;
    for token in pat {
        match token {
            Wild::Char(c) => {
                previous_star = false;
                literal.push(*c);
            }
            Wild::Any => {
                flush_literal(&mut source, &mut literal);
                previous_star = false;
                source.push('.');
            }
            Wild::Star if !previous_star => {
                flush_literal(&mut source, &mut literal);
                previous_star = true;
                source.push_str(".*");
            }
            Wild::Star => {}
        }
    }
    flush_literal(&mut source, &mut literal);
    regex::RegexBuilder::new(&source)
        .dot_matches_new_line(true)
        .size_limit(SEARCH_REGEX_SIZE_LIMIT)
        .dfa_size_limit(SEARCH_REGEX_SIZE_LIMIT)
        .nest_limit(32)
        .build()
        .map_err(|_| ErrorKind::Value)
}

fn split_keep(src: &str, delim: &str, insensitive: bool, ignore_empty: bool) -> Vec<String> {
    if delim.is_empty() {
        return vec![src.to_string()];
    }
    let mut parts = Vec::new();
    let hay = if insensitive {
        excel_lower(src)
    } else {
        src.to_string()
    };
    let needle = if insensitive {
        excel_lower(delim)
    } else {
        delim.to_string()
    };
    let mut last = 0;
    let mut search_from = 0;
    while let Some(rel) = hay[search_from..].find(&needle) {
        let idx = search_from + rel;
        let orig_idx = byte_to_orig(src, &hay, idx);
        let piece = src[last..orig_idx].to_string();
        if !(ignore_empty && piece.is_empty()) {
            parts.push(piece);
        }
        let end = idx + needle.len();
        last = byte_to_orig(src, &hay, end);
        search_from = end;
        if needle.is_empty() {
            break;
        }
    }
    let tail = src[last..].to_string();
    if !(ignore_empty && tail.is_empty()) {
        parts.push(tail);
    }
    if parts.is_empty() {
        parts.push(String::new());
    }
    parts
}

fn byte_to_orig(orig: &str, lowered: &str, byte: usize) -> usize {
    if orig.len() == lowered.len() {
        return byte.min(orig.len());
    }
    let mut oi = 0;
    let mut li = 0;
    for (oc, lc) in orig.chars().zip(lowered.chars()) {
        if li == byte {
            return oi;
        }
        oi += oc.len_utf8();
        li += lc.len_utf8();
    }
    orig.len()
}

fn find_all_delim(src: &str, delim: &str, insensitive: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if delim.is_empty() {
        return out;
    }
    let hay = if insensitive {
        excel_lower(src)
    } else {
        src.to_string()
    };
    let needle = if insensitive {
        excel_lower(delim)
    } else {
        delim.to_string()
    };
    let mut from = 0;
    while let Some(rel) = hay[from..].find(&needle) {
        let idx = from + rel;
        let start = byte_to_orig(src, &hay, idx);
        let end = byte_to_orig(src, &hay, idx + needle.len());
        out.push((start, end));
        from = idx + needle.len().max(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::search_wildcard;
    use omacell_core::error::ErrorKind;

    #[test]
    fn search_wildcards_keep_excel_prefix_semantics() {
        assert_eq!(search_wildcard("xxAbczz", "a?c", 1), Ok(3));
        assert_eq!(search_wildcard("xxa*c", "a~*c", 1), Ok(3));
        assert_eq!(search_wildcard("abc", "*b", 1), Ok(1));
        assert_eq!(search_wildcard("abc", "", 4), Ok(4));
    }

    #[test]
    fn search_wildcards_do_not_backtrack_exponentially() {
        let pattern = format!("{}b", "*a".repeat(64));
        let hay = "a".repeat(128);
        assert_eq!(search_wildcard(&hay, &pattern, 1), Err(ErrorKind::Value));
    }

    #[test]
    fn search_rejects_text_above_the_excel_limit() {
        assert_eq!(
            search_wildcard(&"a".repeat(32_768), "a", 1),
            Err(ErrorKind::Value)
        );
        assert_eq!(
            search_wildcard("a", &"a".repeat(32_768), 1),
            Err(ErrorKind::Value)
        );
    }
}

//! Delimiter, encoding, header, and separator sniffer.

use std::io::Read;
use std::path::Path;

use omacell_core::error::CoreError;
use omacell_core::locale::LocaleId;
use serde::{Deserialize, Serialize};

use super::encode::{bom_len, decode_all, sniff_encoding};
use super::infer::convert_cell;
use super::plan::{ColumnPlan, ColumnType, ImportPlan, LineEnding, MAX_SNIFF_BYTES, TextEncoding};
use super::records::parse_records;
use crate::error;

/// Result of sniffing a sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sniff {
    /// Filled-in plan (column types may be Auto or KeepAsText).
    pub plan: ImportPlan,
    /// Decoded sample records (not converted).
    pub sample_rows: Vec<Vec<String>>,
}

/// Sniff in-memory bytes.
pub fn sniff(bytes: &[u8]) -> Result<Sniff, CoreError> {
    sniff_with(bytes, None)
}

/// Sniff a path. Reads at most [`MAX_SNIFF_BYTES`].
pub fn sniff_path(path: &Path) -> Result<Sniff, CoreError> {
    let mut file = std::fs::File::open(path).map_err(|e| error::parse(e.to_string()))?;
    let mut buf = Vec::with_capacity(MAX_SNIFF_BYTES.min(64 * 1024));
    let mut tmp = [0u8; 8192];
    while buf.len() < MAX_SNIFF_BYTES {
        let n = file
            .read(&mut tmp)
            .map_err(|e| error::parse(e.to_string()))?;
        if n == 0 {
            break;
        }
        let take = n.min(MAX_SNIFF_BYTES - buf.len());
        buf.extend_from_slice(&tmp[..take]);
    }
    let hint = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    sniff_with(&buf, hint.as_deref())
}

fn sniff_with(bytes: &[u8], ext: Option<&str>) -> Result<Sniff, CoreError> {
    let sample = if bytes.len() > MAX_SNIFF_BYTES {
        &bytes[..MAX_SNIFF_BYTES]
    } else {
        bytes
    };
    let (encoding, bom) = sniff_encoding(sample);
    let text = decode_sample(sample, encoding)?;
    let line_ending = sniff_line_ending(&text);
    let (delimiter, quote) = sniff_delimiter(&text, ext);
    let mut plan = ImportPlan {
        delimiter,
        quote,
        encoding,
        bom,
        line_ending,
        ..ImportPlan::default()
    };
    let records = match parse_records(text.as_bytes(), &plan) {
        Ok(r) => r,
        Err(_) => {
            plan.delimiter = fallback_delim(ext);
            parse_records(text.as_bytes(), &plan).unwrap_or_default()
        }
    };
    if records.is_empty() {
        return Ok(Sniff {
            plan,
            sample_rows: records,
        });
    }
    let (decimal, thousands) = sniff_separators(&records, plan.locale);
    plan.decimal = decimal;
    plan.thousands = thousands;
    plan.has_header = guess_header(&records);
    plan.columns = infer_columns(&records, &plan);
    Ok(Sniff {
        plan,
        sample_rows: records,
    })
}

fn decode_sample(sample: &[u8], encoding: TextEncoding) -> Result<String, CoreError> {
    if encoding != TextEncoding::Utf8 {
        return decode_all(sample, encoding);
    }
    let skip = bom_len(encoding, sample);
    let body = &sample[skip..];
    match std::str::from_utf8(body) {
        Ok(text) => Ok(text.to_owned()),
        Err(err) if err.error_len().is_none() => {
            Ok(std::str::from_utf8(&body[..err.valid_up_to()])
                .unwrap_or_default()
                .to_owned())
        }
        Err(_) => Err(error::encoding("input is not valid UTF-8")),
    }
}

fn fallback_delim(ext: Option<&str>) -> char {
    match ext {
        Some("tsv") | Some("tab") => '\t',
        _ => ',',
    }
}

fn sniff_line_ending(text: &str) -> LineEnding {
    let mut crlf = 0u32;
    let mut lf = 0u32;
    let mut cr = 0u32;
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
            crlf += 1;
            i += 2;
        } else if b[i] == b'\n' {
            lf += 1;
            i += 1;
        } else if b[i] == b'\r' {
            cr += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    if crlf >= lf && crlf >= cr && crlf > 0 {
        LineEnding::CrLf
    } else if cr > lf && cr > 0 {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

fn sniff_delimiter(text: &str, ext: Option<&str>) -> (char, char) {
    let candidates: &[char] = match ext {
        Some("tsv") | Some("tab") => &['\t', ',', ';', '|'],
        _ => &[',', '\t', ';', '|'],
    };
    let mut best = (fallback_delim(ext), '"', i64::MIN);
    for &delim in candidates {
        for quote in ['"', '\''] {
            let mut plan = ImportPlan {
                delimiter: delim,
                quote,
                ..ImportPlan::default()
            };
            plan.encoding = TextEncoding::Utf8;
            let Ok(rows) = parse_records(text.as_bytes(), &plan) else {
                continue;
            };
            if rows.is_empty() {
                continue;
            }
            let score = score_rows(&rows, delim) + quote_bonus(text, delim, quote);
            if score > best.2 {
                best = (delim, quote, score);
            }
        }
    }
    (best.0, best.1)
}

fn score_rows(rows: &[Vec<String>], _delim: char) -> i64 {
    let counts: Vec<usize> = rows.iter().map(Vec::len).collect();
    let max = counts.iter().copied().max().unwrap_or(0);
    if max < 2 {
        return i64::MIN / 4;
    }
    let mut widths = counts.clone();
    widths.sort_unstable();
    let mut mode = widths[0];
    let mut mode_count = 0usize;
    let mut run_value = widths[0];
    let mut run_count = 0usize;
    for width in widths {
        if width == run_value {
            run_count += 1;
        } else {
            if run_count > mode_count {
                mode = run_value;
                mode_count = run_count;
            }
            run_value = width;
            run_count = 1;
        }
    }
    if run_count > mode_count {
        mode = run_value;
        mode_count = run_count;
    }
    let deviation: usize = counts.iter().map(|width| width.abs_diff(mode)).sum();
    let fields: usize = counts.iter().sum();
    mode_count as i64 * 10_000 + mode as i64 * 100 + fields as i64 - deviation as i64 * 1_000
}

fn quote_bonus(text: &str, delimiter: char, quote: char) -> i64 {
    let bytes = text.as_bytes();
    let quote = quote as u8;
    let delimiter = delimiter as u8;
    let mut starts = 0i64;
    let mut at_field_start = true;
    let mut idx = 0usize;
    while idx < bytes.len() {
        let byte = bytes[idx];
        if at_field_start && byte == quote {
            let Some(end) = valid_quoted_field_end(bytes, idx, delimiter, quote) else {
                at_field_start = false;
                idx += 1;
                continue;
            };
            starts += 1;
            at_field_start = false;
            idx = end + 1;
        } else if byte == delimiter || matches!(byte, b'\r' | b'\n') {
            at_field_start = true;
            idx += 1;
        } else {
            at_field_start = false;
            idx += 1;
        }
    }
    starts * 2_000
}

fn valid_quoted_field_end(bytes: &[u8], start: usize, delimiter: u8, quote: u8) -> Option<usize> {
    let mut idx = start + 1;
    while idx < bytes.len() {
        if bytes[idx] != quote {
            idx += 1;
            continue;
        }
        if bytes.get(idx + 1) == Some(&quote) {
            idx += 2;
            continue;
        }
        return match bytes.get(idx + 1) {
            None => Some(idx),
            Some(next) if *next == delimiter || matches!(*next, b'\r' | b'\n') => Some(idx),
            Some(_) => None,
        };
    }
    None
}

fn sniff_separators(rows: &[Vec<String>], locale: LocaleId) -> (char, Option<char>) {
    let mut eu = 0u32;
    let mut us = 0u32;
    let mut eu_grouped = 0u32;
    let mut us_grouped = 0u32;
    for row in rows {
        for cell in row {
            match number_shape(cell) {
                NumberShape::Eu => {
                    eu += 1;
                    eu_grouped += 1;
                }
                NumberShape::Us => {
                    us += 1;
                    us_grouped += 1;
                }
                NumberShape::CommaDecimal => eu += 1,
                NumberShape::DotDecimal => us += 1,
                NumberShape::Other => {}
            }
        }
    }
    let sep = locale.separators();
    if eu > us && eu > 0 {
        (',', (eu_grouped > 0).then_some('.'))
    } else if us > eu && us > 0 {
        ('.', (us_grouped > 0).then_some(','))
    } else {
        (sep.decimal, None)
    }
}

enum NumberShape {
    Us,
    Eu,
    DotDecimal,
    CommaDecimal,
    Other,
}

fn number_shape(s: &str) -> NumberShape {
    let has_dot = s.contains('.');
    let has_comma = s.contains(',');
    if has_dot && has_comma {
        if s.rfind('.') > s.rfind(',') {
            NumberShape::Us
        } else {
            NumberShape::Eu
        }
    } else if has_dot {
        single_decimal_shape(s, '.').unwrap_or(NumberShape::Other)
    } else if has_comma {
        single_decimal_shape(s, ',').unwrap_or(NumberShape::Other)
    } else {
        NumberShape::Other
    }
}

fn single_decimal_shape(s: &str, separator: char) -> Option<NumberShape> {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let (whole, fraction) = body.split_once(separator)?;
    if whole.is_empty()
        || fraction.is_empty()
        || fraction.len() == 3
        || !whole.chars().all(|c| c.is_ascii_digit())
        || !fraction.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(if separator == ',' {
        NumberShape::CommaDecimal
    } else {
        NumberShape::DotDecimal
    })
}

fn guess_header(rows: &[Vec<String>]) -> bool {
    if rows.len() < 2 {
        return false;
    }
    let first = &rows[0];
    let nonempty: Vec<&str> = first
        .iter()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    if nonempty.is_empty() {
        return false;
    }
    let letterish = |s: &str| s.chars().any(|c| c.is_alphabetic());
    let letters = nonempty.iter().filter(|s| letterish(s)).count();
    if letters * 2 <= nonempty.len() {
        return false;
    }
    let mut seen = Vec::new();
    for s in &nonempty {
        let key = s.to_ascii_lowercase();
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
    }
    let first_num = numeric_frac(first);
    let rest: Vec<&Vec<String>> = rows.iter().skip(1).take(20).collect();
    if rest.is_empty() {
        return false;
    }
    let rest_num: f32 = rest.iter().map(|r| numeric_frac(r)).sum::<f32>() / rest.len() as f32;
    rest_num >= first_num + 0.2 || (first_num == 0.0 && rest_num > 0.0)
}

fn numeric_frac(row: &[String]) -> f32 {
    let n = row.iter().filter(|s| !s.is_empty()).count();
    if n == 0 {
        return 0.0;
    }
    let k = row.iter().filter(|s| numericish(s)).count();
    k as f32 / n as f32
}

fn numericish(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | ' ' | ':' | '/'))
}

fn infer_columns(rows: &[Vec<String>], plan: &ImportPlan) -> Vec<ColumnPlan> {
    let start = usize::from(plan.has_header);
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut cols = Vec::with_capacity(width);
    for i in 0..width {
        let name = if plan.has_header {
            rows.first()
                .and_then(|r| r.get(i))
                .cloned()
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        let mut trap = false;
        for row in rows.iter().skip(start) {
            let raw = row.get(i).map(String::as_str).unwrap_or("");
            if raw.is_empty() {
                continue;
            }
            let converted = convert_cell(raw, &ColumnType::Auto, plan);
            if matches!(converted, super::infer::Converted::Text(_)) && is_trap(raw, plan) {
                trap = true;
                break;
            }
        }
        cols.push(ColumnPlan {
            name,
            ty: if trap {
                ColumnType::KeepAsText
            } else {
                ColumnType::Auto
            },
        });
    }
    cols
}

fn is_trap(raw: &str, plan: &ImportPlan) -> bool {
    let auto = convert_cell(raw, &ColumnType::Auto, plan);
    matches!(auto, super::infer::Converted::Text(_))
        && (raw.chars().any(|c| c.is_ascii_alphabetic()) && raw.chars().any(|c| c.is_ascii_digit())
            || raw.starts_with('0') && raw.chars().any(|c| c.is_ascii_digit())
            || raw.chars().filter(|c| c.is_ascii_digit()).count() > 15)
}

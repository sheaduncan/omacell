//! Flash Fill: learn prefix/suffix/token patterns from filled examples (F-6.6 / WP-18).

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::value::Value;
use crate::workbook::Workbook;

/// Fill empty cells in `range` from the adjacent source column using examples
/// already present in the destination.
pub fn flash_fill(wb: &mut Workbook, sheet: SheetId, range: RangeRef) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    let dest_col = c0;
    let src_col = if c0 > 0 {
        c0 - 1
    } else if c1.saturating_add(1) != c0 {
        c0.saturating_add(1)
    } else {
        return Err(CoreError::new(
            "flashfill.source",
            "flash fill needs an adjacent source column",
        ));
    };
    let mut examples = Vec::new();
    let mut empty = Vec::new();
    for r in r0..=r1 {
        let dest = display(wb, sheet, r, dest_col);
        let src = display(wb, sheet, r, src_col);
        if dest.is_empty() {
            empty.push((r, src));
        } else if !src.is_empty() {
            examples.push((src, dest));
        }
    }
    let Some(pat) = infer(&examples) else {
        return Ok(0);
    };
    let mut filled = 0u32;
    for (row, src) in empty {
        if let Some(out) = apply(&pat, &src) {
            wb.set_text(sheet, row, dest_col, &out)?;
            filled += 1;
        }
    }
    let _ = c1;
    Ok(filled)
}

#[derive(Clone, Debug)]
enum Pattern {
    Prefix(usize),
    Suffix(usize),
    FirstWord,
    LastWord,
    Constant(String),
    Surround { prefix: String, suffix: String },
}

fn infer(examples: &[(String, String)]) -> Option<Pattern> {
    if examples.is_empty() {
        return None;
    }
    if examples.iter().all(|(s, d)| d == s) {
        return Some(Pattern::Surround {
            prefix: String::new(),
            suffix: String::new(),
        });
    }
    if examples.len() >= 2 && examples.iter().all(|(_, d)| d == examples[0].1.as_str()) {
        return Some(Pattern::Constant(examples[0].1.clone()));
    }
    if examples
        .iter()
        .all(|(s, d)| first_word(s) == d.as_str() && !d.is_empty())
    {
        return Some(Pattern::FirstWord);
    }
    if examples
        .iter()
        .all(|(s, d)| last_word(s) == d.as_str() && !d.is_empty())
    {
        return Some(Pattern::LastWord);
    }
    if examples
        .iter()
        .all(|(s, d)| s.starts_with(d.as_str()) && !d.is_empty())
    {
        let n = examples[0].1.chars().count();
        if examples.iter().all(|(_, d)| d.chars().count() == n) {
            return Some(Pattern::Prefix(n));
        }
    }
    if examples
        .iter()
        .all(|(s, d)| s.ends_with(d.as_str()) && !d.is_empty())
    {
        let n = examples[0].1.chars().count();
        if examples.iter().all(|(_, d)| d.chars().count() == n) {
            return Some(Pattern::Suffix(n));
        }
    }
    let (p, sfx) = surround(&examples[0].0, &examples[0].1)?;
    if examples
        .iter()
        .all(|(src, dest)| surround(src, dest) == Some((p.clone(), sfx.clone())))
    {
        return Some(Pattern::Surround {
            prefix: p,
            suffix: sfx,
        });
    }
    None
}

fn apply(pat: &Pattern, src: &str) -> Option<String> {
    match pat {
        Pattern::Prefix(n) => Some(src.chars().take(*n).collect()),
        Pattern::Suffix(n) => {
            let total = src.chars().count();
            Some(src.chars().skip(total.saturating_sub(*n)).collect())
        }
        Pattern::FirstWord => {
            let w = first_word(src);
            if w.is_empty() {
                None
            } else {
                Some(w.to_string())
            }
        }
        Pattern::LastWord => {
            let w = last_word(src);
            if w.is_empty() {
                None
            } else {
                Some(w.to_string())
            }
        }
        Pattern::Constant(s) => Some(s.clone()),
        Pattern::Surround { prefix, suffix } => Some(format!("{prefix}{src}{suffix}")),
    }
}

fn surround(src: &str, dest: &str) -> Option<(String, String)> {
    if dest.len() < src.len() {
        return None;
    }
    let idx = dest.find(src)?;
    Some((dest[..idx].to_string(), dest[idx + src.len()..].to_string()))
}

fn first_word(s: &str) -> &str {
    s.split_whitespace().next().unwrap_or("")
}

fn last_word(s: &str) -> &str {
    s.split_whitespace().next_back().unwrap_or("")
}

fn display(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> String {
    match wb.get(sheet, row, col).ok().flatten().map(|s| s.value) {
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(true)) => "TRUE".into(),
        Some(Value::Bool(false)) => "FALSE".into(),
        Some(Value::Text(id)) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}

//! Find/replace and Go To Special (F-5.8).

use std::time::{Duration, Instant};

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};

use crate::addr::{SheetId, parse_a1};
use crate::error::CoreError;
use crate::value::Value;
use crate::workbook::Workbook;

/// Maximum find regex pattern length.
pub const MAX_PATTERN_CHARS: usize = 256;
/// Compile budget for the `regex` crate.
pub const MAX_REGEX_BYTES: usize = 1024 * 1024;
/// Wall-clock budget for a find/replace scan.
pub const FIND_TIMEOUT: Duration = Duration::from_millis(250);

/// Find options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindSpec {
    /// Needle.
    pub query: String,
    /// Search formula source instead of values.
    #[serde(default)]
    pub formulas: bool,
    /// Whole-cell match.
    #[serde(default)]
    pub whole: bool,
    /// Case-sensitive.
    #[serde(default)]
    pub case: bool,
    /// Treat `query` as a regex.
    #[serde(default)]
    pub regex: bool,
    /// Search the whole workbook (else the active sheet).
    #[serde(default)]
    pub workbook: bool,
}

/// One match.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindHit {
    /// Sheet.
    pub sheet: SheetId,
    /// Row.
    pub row: u32,
    /// Column.
    pub col: u16,
}

/// Go To Special kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GotoKind {
    /// Empty cells in the used range.
    Blanks,
    /// Numeric constants.
    Numbers,
    /// Text constants.
    Text,
    /// Logical constants.
    Logicals,
    /// Error constants.
    Errors,
    /// Any formula.
    Formulas,
    /// Formula cells whose cached value is an error.
    FormulaErrors,
    /// Visible cells only (not hidden).
    Visible,
    /// Cells with conditional formats.
    CondFormats,
    /// Cells with data validation.
    Validation,
}

/// Find cells matching `spec`. Times out with `find.timeout`.
pub fn find_cells(
    wb: &Workbook,
    sheet: SheetId,
    spec: &FindSpec,
) -> Result<Vec<FindHit>, CoreError> {
    let matcher = compile(spec)?;
    let deadline = Instant::now() + FIND_TIMEOUT;
    let mut hits = Vec::new();
    for sh in sheets(wb, sheet, spec.workbook) {
        if Instant::now() > deadline {
            return Err(timeout());
        }
        for (row, col, slot) in sh.store.iter() {
            let text = if spec.formulas {
                slot.formula
                    .and_then(|id| wb.intern().formulas.get(id).map(str::to_string))
                    .unwrap_or_default()
            } else {
                display(wb, slot.value)
            };
            if matcher.matches(&text) {
                hits.push(FindHit {
                    sheet: sh.id,
                    row,
                    col,
                });
            }
        }
    }
    hits.sort_by_key(|h| (h.sheet.index(), h.row, h.col));
    Ok(hits)
}

/// Count replacements that would apply.
pub fn replace_preview(
    wb: &Workbook,
    sheet: SheetId,
    spec: &FindSpec,
    replacement: &str,
) -> Result<u32, CoreError> {
    let _ = replacement;
    Ok(u32::try_from(find_cells(wb, sheet, spec)?.len()).unwrap_or(u32::MAX))
}

/// Apply replacements. Returns the number of cells written.
pub fn replace_apply(
    wb: &mut Workbook,
    sheet: SheetId,
    spec: &FindSpec,
    replacement: &str,
) -> Result<u32, CoreError> {
    let hits = find_cells(wb, sheet, spec)?;
    let matcher = compile(spec)?;
    let mut n = 0u32;
    for hit in hits {
        let text = if spec.formulas {
            wb.get(hit.sheet, hit.row, hit.col)
                .ok()
                .flatten()
                .and_then(|slot| slot.formula)
                .and_then(|id| wb.intern().formulas.get(id).map(str::to_string))
                .unwrap_or_default()
        } else {
            wb.get(hit.sheet, hit.row, hit.col)
                .ok()
                .flatten()
                .map(|slot| display(wb, slot.value))
                .unwrap_or_default()
        };
        let next = matcher.replace(&text, replacement);
        if next != text {
            wb.set_cell_contents(hit.sheet, hit.row, hit.col, &next)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Resolve Go To by A1 or defined name.
pub fn goto_spec(wb: &Workbook, spec: &str) -> Result<(SheetId, u32, u16), CoreError> {
    if let Ok(parsed) = parse_a1(spec) {
        let sheet = parsed
            .sheet
            .as_ref()
            .and_then(|s| wb.sheet_by_name(&s.start).map(|sh| sh.id))
            .unwrap_or_else(|| wb.active_sheet());
        return match parsed.kind {
            crate::addr::RefKind::Cell(c) => Ok((sheet, c.row, c.col)),
            crate::addr::RefKind::Range(r) => Ok((sheet, r.start.row, r.start.col)),
        };
    }
    if let Some(name) = wb.names().resolve(wb.active_sheet(), spec) {
        if let crate::names::NameReferent::Range(r) = &name.referent {
            return Ok((wb.active_sheet(), r.start.row, r.start.col));
        }
    }
    Err(
        CoreError::new("goto.spec", format!("cannot go to {spec:?}"))
            .with_hint("use an A1 address or a defined name"),
    )
}

/// Go To Special cells on `sheet` (or its used range).
pub fn goto_special(
    wb: &Workbook,
    sheet: SheetId,
    kind: GotoKind,
    visible_only: bool,
) -> Result<Vec<FindHit>, CoreError> {
    let Some(s) = wb.sheet(sheet) else {
        return Err(CoreError::sheet_id(format!(
            "unknown sheet {}",
            sheet.index()
        )));
    };
    let mut hits = Vec::new();
    match kind {
        GotoKind::Blanks => {
            let Some(used) = s.used_range() else {
                return Ok(hits);
            };
            for r in used.min_row..=used.max_row {
                for c in used.min_col..=used.max_col {
                    if s.store.get(r, c).ok().flatten().is_none() {
                        hits.push(FindHit {
                            sheet,
                            row: r,
                            col: c,
                        });
                    }
                }
            }
        }
        GotoKind::CondFormats => {
            for rule in &s.cond_formats {
                let (r0, c0, r1, c1) = (
                    rule.range.start.row.min(rule.range.end.row),
                    rule.range.start.col.min(rule.range.end.col),
                    rule.range.start.row.max(rule.range.end.row),
                    rule.range.start.col.max(rule.range.end.col),
                );
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        hits.push(FindHit {
                            sheet,
                            row: r,
                            col: c,
                        });
                    }
                }
            }
        }
        GotoKind::Validation => {
            for dv in &s.validations {
                let (r0, c0, r1, c1) = (
                    dv.range.start.row.min(dv.range.end.row),
                    dv.range.start.col.min(dv.range.end.col),
                    dv.range.start.row.max(dv.range.end.row),
                    dv.range.start.col.max(dv.range.end.col),
                );
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        hits.push(FindHit {
                            sheet,
                            row: r,
                            col: c,
                        });
                    }
                }
            }
        }
        other => {
            for (row, col, slot) in s.store.iter() {
                let keep = match other {
                    GotoKind::Numbers => {
                        slot.formula.is_none() && matches!(slot.value, Value::Number(_))
                    }
                    GotoKind::Text => {
                        slot.formula.is_none() && matches!(slot.value, Value::Text(_))
                    }
                    GotoKind::Logicals => {
                        slot.formula.is_none() && matches!(slot.value, Value::Bool(_))
                    }
                    GotoKind::Errors => matches!(slot.value, Value::Error(_)),
                    GotoKind::Formulas => slot.formula.is_some(),
                    GotoKind::FormulaErrors => {
                        slot.formula.is_some() && matches!(slot.value, Value::Error(_))
                    }
                    GotoKind::Visible => true,
                    _ => false,
                };
                if keep {
                    hits.push(FindHit { sheet, row, col });
                }
            }
        }
    }
    if visible_only || matches!(kind, GotoKind::Visible) {
        hits.retain(|h| {
            !s.geometry.rows.is_hidden(h.row).unwrap_or(false)
                && !s.geometry.cols.is_hidden(u32::from(h.col)).unwrap_or(false)
        });
    }
    hits.sort_by_key(|h| (h.row, h.col));
    hits.dedup_by_key(|h| (h.row, h.col));
    Ok(hits)
}

struct Matcher {
    regex: Option<regex::Regex>,
    needle: String,
    whole: bool,
    case: bool,
}

impl Matcher {
    fn matches(&self, text: &str) -> bool {
        if let Some(re) = &self.regex {
            return re.is_match(text);
        }
        let (hay, needle) = if self.case {
            (text.to_string(), self.needle.clone())
        } else {
            (text.to_lowercase(), self.needle.to_lowercase())
        };
        if self.whole {
            hay == needle
        } else {
            hay.contains(&needle)
        }
    }

    fn replace(&self, text: &str, replacement: &str) -> String {
        if let Some(re) = &self.regex {
            return re.replace_all(text, replacement).into_owned();
        }
        if self.whole {
            if self.matches(text) {
                replacement.to_string()
            } else {
                text.to_string()
            }
        } else if self.case {
            text.replace(&self.needle, replacement)
        } else {
            replace_ignore_case(text, &self.needle, replacement)
        }
    }
}

fn compile(spec: &FindSpec) -> Result<Matcher, CoreError> {
    if spec.regex {
        if spec.query.chars().count() > MAX_PATTERN_CHARS {
            return Err(timeout());
        }
        let re = RegexBuilder::new(&spec.query)
            .case_insensitive(!spec.case)
            .size_limit(MAX_REGEX_BYTES)
            .dfa_size_limit(MAX_REGEX_BYTES)
            .nest_limit(32)
            .build()
            .map_err(|e| {
                CoreError::new("find.regex", e.to_string())
                    .with_hint("shorten the pattern or disable regex")
            })?;
        return Ok(Matcher {
            regex: Some(re),
            needle: spec.query.clone(),
            whole: spec.whole,
            case: spec.case,
        });
    }
    Ok(Matcher {
        regex: None,
        needle: spec.query.clone(),
        whole: spec.whole,
        case: spec.case,
    })
}

fn replace_ignore_case(text: &str, needle: &str, replacement: &str) -> String {
    let lower = text.to_lowercase();
    let n = needle.to_lowercase();
    let mut out = String::new();
    let mut rest = text;
    let mut rest_l = lower.as_str();
    while let Some(at) = rest_l.find(&n) {
        out.push_str(&rest[..at]);
        out.push_str(replacement);
        rest = &rest[at + needle.len()..];
        rest_l = &rest_l[at + n.len()..];
    }
    out.push_str(rest);
    out
}

fn sheets(wb: &Workbook, sheet: SheetId, workbook: bool) -> Vec<&crate::sheet::Sheet> {
    if workbook {
        wb.sheets().collect()
    } else {
        wb.sheet(sheet).into_iter().collect()
    }
}

fn display(wb: &Workbook, v: Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        Value::Error(k) => k.as_str().to_string(),
        _ => String::new(),
    }
}

fn timeout() -> CoreError {
    CoreError::new(
        "find.timeout",
        "find/replace exceeded its time or size budget",
    )
    .with_hint("narrow the scope or simplify the pattern")
}

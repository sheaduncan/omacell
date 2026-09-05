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

const MAX_FIND_RESULTS: usize = 100_000;

/// Find options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindSpec {
    /// Needle.
    pub query: String,
    /// Search formula source for formula cells and displayed contents for constants.
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
    /// Formula cells whose cached value is numeric.
    FormulaNumbers,
    /// Formula cells whose cached value is text.
    FormulaText,
    /// Formula cells whose cached value is logical.
    FormulaLogicals,
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
    let searched_sheets: Vec<_> = if spec.workbook {
        wb.sheets().collect()
    } else {
        vec![
            wb.sheet(sheet)
                .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", sheet.index())))?,
        ]
    };
    for sh in searched_sheets {
        for (row, col, slot) in sh.store.iter() {
            if Instant::now() > deadline {
                return Err(timeout());
            }
            let text = slot_text(wb, &slot, spec.formulas);
            if matcher.matches(&text) {
                push_hit(
                    &mut hits,
                    FindHit {
                        sheet: sh.id,
                        row,
                        col,
                    },
                )?;
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
    let hits = find_cells(wb, sheet, spec)?;
    let matcher = compile(spec)?;
    let mut count = 0u32;
    for hit in hits {
        let Some(text) = replacement_text(wb, &hit, spec.formulas) else {
            continue;
        };
        if matcher.replace(&text, replacement) != text {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
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
        let Some(text) = replacement_text(wb, &hit, spec.formulas) else {
            continue;
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
        let sheet = if let Some(sheet_spec) = &parsed.sheet {
            if sheet_spec.end.is_some() {
                return Err(CoreError::new(
                    "goto.spec",
                    "Go To does not accept a 3-D sheet span",
                ));
            }
            wb.sheet_by_name(&sheet_spec.start)
                .map(|sh| sh.id)
                .ok_or_else(|| {
                    CoreError::new("goto.spec", format!("unknown sheet {:?}", sheet_spec.start))
                })?
        } else {
            wb.active_sheet()
        };
        return match parsed.kind {
            crate::addr::RefKind::Cell(c) => Ok((sheet, c.row, c.col)),
            crate::addr::RefKind::Range(r) => Ok((sheet, r.start.row, r.start.col)),
        };
    }
    if let Ok(parsed_name) = crate::formula::parse(&format!("={spec}"))
        && let crate::formula::ExprKind::Name {
            sheet: name_sheet,
            name: name_text,
        } = &parsed_name.ast.kind
    {
        let lookup_sheet = if let Some(sheet_spec) = name_sheet {
            wb.sheet_by_name(&sheet_spec.start)
                .map(|sheet| sheet.id)
                .ok_or_else(|| {
                    CoreError::new("goto.spec", format!("unknown sheet {:?}", sheet_spec.start))
                })?
        } else {
            wb.active_sheet()
        };
        if let Some(name) = wb.names().resolve(lookup_sheet, name_text)
            && let crate::names::NameReferent::Range(r) = &name.referent
        {
            let sheet = r
                .start
                .sheet
                .or(r.end.sheet)
                .unwrap_or_else(|| match name.scope {
                    crate::names::NameScope::Workbook => wb.active_sheet(),
                    crate::names::NameScope::Sheet(sheet) => sheet,
                });
            if wb.sheet(sheet).is_none() {
                return Err(CoreError::new(
                    "goto.spec",
                    format!("defined name {spec:?} points to an unknown sheet"),
                ));
            }
            return Ok((sheet, r.start.row, r.start.col));
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
                    let blank = s.store.get(r, c).ok().flatten().is_none_or(|slot| {
                        slot.formula.is_none() && matches!(slot.value, Value::Empty)
                    });
                    if blank {
                        push_hit(
                            &mut hits,
                            FindHit {
                                sheet,
                                row: r,
                                col: c,
                            },
                        )?;
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
                        push_hit(
                            &mut hits,
                            FindHit {
                                sheet,
                                row: r,
                                col: c,
                            },
                        )?;
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
                        push_hit(
                            &mut hits,
                            FindHit {
                                sheet,
                                row: r,
                                col: c,
                            },
                        )?;
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
                    GotoKind::Errors => {
                        slot.formula.is_none() && matches!(slot.value, Value::Error(_))
                    }
                    GotoKind::Formulas => slot.formula.is_some(),
                    GotoKind::FormulaErrors => {
                        slot.formula.is_some() && matches!(slot.value, Value::Error(_))
                    }
                    GotoKind::FormulaNumbers => {
                        slot.formula.is_some() && matches!(slot.value, Value::Number(_))
                    }
                    GotoKind::FormulaText => {
                        slot.formula.is_some() && matches!(slot.value, Value::Text(_))
                    }
                    GotoKind::FormulaLogicals => {
                        slot.formula.is_some() && matches!(slot.value, Value::Bool(_))
                    }
                    GotoKind::Visible => true,
                    _ => false,
                };
                if keep {
                    push_hit(&mut hits, FindHit { sheet, row, col })?;
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
    regex_replacement: bool,
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
            if self.regex_replacement {
                return re.replace_all(text, replacement).into_owned();
            }
            let mut out = String::with_capacity(text.len());
            let mut end = 0;
            for matched in re.find_iter(text) {
                out.push_str(&text[end..matched.start()]);
                out.push_str(replacement);
                end = matched.end();
            }
            out.push_str(&text[end..]);
            return out;
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
            // Case-insensitive literal matchers are compiled as escaped regexes.
            text.to_string()
        }
    }
}

fn compile(spec: &FindSpec) -> Result<Matcher, CoreError> {
    if spec.query.is_empty() {
        return Err(CoreError::new("find.query", "find query cannot be empty")
            .with_hint("enter at least one character"));
    }
    if spec.regex {
        if spec.query.chars().count() > MAX_PATTERN_CHARS {
            return Err(timeout());
        }
        let pattern = if spec.whole {
            format!(r"\A(?:{})\z", spec.query)
        } else {
            spec.query.clone()
        };
        let re = RegexBuilder::new(&pattern)
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
            regex_replacement: true,
        });
    }
    if !spec.case {
        let escaped = regex::escape(&spec.query);
        let pattern = if spec.whole {
            format!(r"\A(?:{escaped})\z")
        } else {
            escaped
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .size_limit(MAX_REGEX_BYTES)
            .dfa_size_limit(MAX_REGEX_BYTES)
            .build()
            .map_err(|e| CoreError::new("find.query", e.to_string()))?;
        return Ok(Matcher {
            regex: Some(re),
            needle: spec.query.clone(),
            whole: spec.whole,
            case: spec.case,
            regex_replacement: false,
        });
    }
    Ok(Matcher {
        regex: None,
        needle: spec.query.clone(),
        whole: spec.whole,
        case: spec.case,
        regex_replacement: false,
    })
}

fn replacement_text(wb: &Workbook, hit: &FindHit, formulas: bool) -> Option<String> {
    let slot = wb.get(hit.sheet, hit.row, hit.col).ok().flatten()?;
    if !formulas && slot.formula.is_some() {
        return None;
    }
    Some(slot_text(wb, slot, formulas))
}

fn slot_text(wb: &Workbook, slot: &crate::storage::CellSlot, formulas: bool) -> String {
    if formulas && let Some(id) = slot.formula {
        return wb
            .intern()
            .formulas
            .get(id)
            .map(str::to_string)
            .unwrap_or_default();
    }
    display(wb, slot.value)
}

fn push_hit(hits: &mut Vec<FindHit>, hit: FindHit) -> Result<(), CoreError> {
    if hits.len() >= MAX_FIND_RESULTS {
        return Err(CoreError::new(
            "find.limit",
            format!("find result exceeds {MAX_FIND_RESULTS} cells"),
        )
        .with_hint("narrow the sheet, range, or match criteria"));
    }
    hits.push(hit);
    Ok(())
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

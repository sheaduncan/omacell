//! Workbook card builder (spec A-2).

use std::collections::{BTreeMap, HashSet};

use omacell_core::addr::{RefKind, col_to_letters, parse_a1, quote_sheet_name};
use omacell_core::audit::{dependents_of, precedents_of};
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value as CellValue;
use omacell_core::workbook::Workbook;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{AiError, codes};

/// Card depth.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CardLevel {
    /// Sheets, names, tables, counts.
    #[default]
    Summary,
    /// Adds per-column stats.
    Columns,
    /// Adds representative rows.
    Sample,
    /// A requested range's values.
    Full,
}

impl CardLevel {
    /// Parse.
    pub fn parse(name: &str) -> Result<Self, AiError> {
        match name {
            "summary" => Ok(Self::Summary),
            "columns" => Ok(Self::Columns),
            "sample" => Ok(Self::Sample),
            "full" => Ok(Self::Full),
            other => Err(AiError::new(
                codes::PAYLOAD,
                format!("unknown card level {other}"),
            )),
        }
    }

    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Columns => "columns",
            Self::Sample => "sample",
            Self::Full => "full",
        }
    }
}

/// Card request.
#[derive(Clone, Debug)]
pub struct CardRequest {
    /// Level.
    pub level: CardLevel,
    /// Optional file path.
    pub file: Option<String>,
    /// Optional A1 focus (selection).
    pub selection: Option<String>,
    /// Optional range for `full`.
    pub range: Option<String>,
    /// Sample row cap.
    pub sample_rows: usize,
    /// Token budget (estimated).
    pub token_budget: usize,
    /// Zero-based row offset within a requested full range.
    pub offset: u32,
    /// Maximum rows returned from a requested full range.
    pub limit: u32,
}

impl Default for CardRequest {
    fn default() -> Self {
        Self {
            level: CardLevel::Summary,
            file: None,
            selection: None,
            range: None,
            sample_rows: 8,
            token_budget: 4096,
            offset: 0,
            limit: 128,
        }
    }
}

/// Build a card (values still present; [`crate::policy`] redacts and filters).
pub(crate) fn build(
    wb: &Workbook,
    engine: Option<&RecalcEngine>,
    request: &CardRequest,
) -> Result<Value, AiError> {
    let mut card = summary(wb, request.file.as_deref());
    card["kind"] = json!(request.level.as_str());
    if matches!(
        request.level,
        CardLevel::Columns | CardLevel::Sample | CardLevel::Full
    ) {
        card["columns"] = columns(wb);
    }
    if matches!(request.level, CardLevel::Sample | CardLevel::Full) {
        card["sample_rows"] = sample_rows(wb, request.sample_rows.clamp(1, 20));
    }
    if request.level == CardLevel::Full {
        if let Some(range) = &request.range {
            let (values, page) = range_values(wb, range, request.offset, request.limit)?;
            card["values"] = values;
            card["page"] = page;
        }
    }
    if let Some(sel) = &request.selection {
        if let Some(engine) = engine {
            card["focus"] = focus(wb, engine, sel);
        }
    }
    card["truncated"] = json!(false);
    Ok(card)
}

fn summary(wb: &Workbook, file: Option<&str>) -> Value {
    let mut formula_count = 0u64;
    let mut fn_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut sheets = Vec::new();
    let mut validation = 0u64;
    let mut condfmt = 0u64;
    let mut external = 0u64;
    for sheet in wb.sheets() {
        let used = sheet.used_range();
        let mut formulas = 0u64;
        for (_, _, slot) in sheet.store.iter() {
            if let Some(fid) = slot.formula {
                formulas += 1;
                if let Some(src) = wb.intern().formulas.get(fid) {
                    count_functions(src, &mut fn_counts);
                    if src.contains('[') {
                        external += 1;
                    }
                }
            }
        }
        formula_count += formulas;
        validation += sheet.validations.len() as u64;
        condfmt += sheet.cond_formats.len() as u64;
        sheets.push(json!({
            "name": sheet.name,
            "rows": used.map(|u| u.max_row.saturating_sub(u.min_row) + 1).unwrap_or(0),
            "cols": used.map(|u| u.max_col.saturating_sub(u.min_col) + 1).unwrap_or(0),
            "formulas": formulas,
        }));
    }
    let mut names: Vec<String> = wb.names().iter().map(|n| n.name.clone()).collect();
    names.sort();
    let mut tables: Vec<String> = wb.tables().iter().map(|t| t.name.clone()).collect();
    tables.sort();
    let mut functions: Vec<(u64, String)> = fn_counts.into_iter().map(|(n, c)| (c, n)).collect();
    functions.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let functions: Vec<Value> = functions
        .into_iter()
        .map(|(count, name)| json!({"name": name, "count": count}))
        .collect();
    json!({
        "schema": 1,
        "kind": "summary",
        "file": file,
        "sheets": sheets,
        "names": names,
        "tables": tables,
        "formula_count": formula_count,
        "functions": functions,
        "external_references": external,
        "validations": validation,
        "conditional_formats": condfmt,
    })
}

fn count_functions(src: &str, counts: &mut BTreeMap<String, u64>) {
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'(' {
                let name = src[start..i].to_ascii_uppercase();
                *counts.entry(name).or_insert(0) += 1;
            }
            continue;
        }
        i += 1;
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum DistinctValue {
    Number(u64),
    Bool(bool),
    Text(u32),
    Error(omacell_core::error::ErrorKind),
    Array(u32),
}

#[derive(Default)]
struct ColumnStats {
    nonempty: u64,
    distinct: HashSet<DistinctValue>,
    samples: Vec<String>,
    saw_number: bool,
    saw_text: bool,
    saw_bool: bool,
    min: Option<f64>,
    max: Option<f64>,
}

impl ColumnStats {
    fn add(&mut self, wb: &Workbook, value: &CellValue) {
        let key = match value {
            CellValue::Empty => return,
            CellValue::Number(number) => {
                self.saw_number = true;
                if number.is_finite() {
                    self.min = Some(self.min.map_or(*number, |current| current.min(*number)));
                    self.max = Some(self.max.map_or(*number, |current| current.max(*number)));
                }
                DistinctValue::Number(number.to_bits())
            }
            CellValue::Bool(value) => {
                self.saw_bool = true;
                DistinctValue::Bool(*value)
            }
            CellValue::Text(id) => {
                self.saw_text = true;
                DistinctValue::Text(id.index())
            }
            CellValue::Error(kind) => {
                self.saw_text = true;
                DistinctValue::Error(*kind)
            }
            CellValue::Array(id) => {
                self.saw_text = true;
                DistinctValue::Array(id.index())
            }
        };
        self.nonempty = self.nonempty.saturating_add(1);
        self.distinct.insert(key);
        if self.samples.len() < 5 {
            self.samples.push(format_cell(wb, value));
        }
    }

    fn inferred_type(&self) -> &'static str {
        match (self.saw_number, self.saw_text, self.saw_bool) {
            (true, false, false) => "number",
            (false, true, false) => "text",
            (false, false, true) => "boolean",
            (false, false, false) => "empty",
            _ => "mixed",
        }
    }
}

fn columns(wb: &Workbook) -> Value {
    let mut out = Vec::new();
    for sheet in wb.sheets() {
        let Some(used) = sheet.used_range() else {
            continue;
        };
        let width = usize::from(used.max_col.saturating_sub(used.min_col)) + 1;
        let mut stats: Vec<ColumnStats> = (0..width).map(|_| ColumnStats::default()).collect();
        for (row, col, slot) in sheet.store.iter() {
            if row <= used.min_row {
                continue;
            }
            stats[usize::from(col.saturating_sub(used.min_col))].add(wb, &slot.value);
        }
        let rows = u64::from(used.max_row.saturating_sub(used.min_row));
        for col in used.min_col..=used.max_col {
            let header = cell_text(wb, sheet.id, used.min_row, col);
            let stat = &stats[usize::from(col.saturating_sub(used.min_col))];
            let nulls = rows.saturating_sub(stat.nonempty);
            let inferred = stat.inferred_type();
            let header_row = !header.is_empty() && inferred != "empty";
            let mut entry = json!({
                "sheet": sheet.name,
                "column": col_to_letters(col).unwrap_or_else(|_| "?".into()),
                "name": header,
                "type": inferred,
                "null_share": if rows == 0 { 0.0 } else { nulls as f64 / rows as f64 },
                "distinct": stat.distinct.len(),
                "samples": stat.samples,
                "header_row": header_row,
            });
            if let Some(min) = stat.min {
                entry["min"] = json!(min);
            }
            if let Some(max) = stat.max {
                entry["max"] = json!(max);
            }
            out.push(entry);
        }
    }
    Value::Array(out)
}

fn sample_rows(wb: &Workbook, cap: usize) -> Value {
    let mut out = Vec::new();
    for sheet in wb.sheets() {
        let Some(used) = sheet.used_range() else {
            continue;
        };
        let mut rows = Vec::new();
        for (i, row) in (used.min_row..=used.max_row).enumerate() {
            if i >= cap {
                break;
            }
            let mut line = Vec::new();
            for col in used.min_col..=used.max_col {
                line.push(cell_text(wb, sheet.id, row, col));
            }
            rows.push(line);
        }
        let origin = format!(
            "{}{}",
            col_to_letters(used.min_col).unwrap_or_else(|_| "A".into()),
            used.min_row + 1
        );
        out.push(json!({"sheet": sheet.name, "origin": origin, "rows": rows}));
    }
    Value::Array(out)
}

const MAX_FULL_ROWS: u32 = 1_024;
const MAX_FULL_CELLS: u64 = 65_536;

fn range_values(
    wb: &Workbook,
    range: &str,
    offset: u32,
    limit: u32,
) -> Result<(Value, Value), AiError> {
    let parsed = parse_a1(range).map_err(AiError::from)?;
    let kind = wb.resolve_parsed(parsed).map_err(AiError::from)?;
    let (sheet, r0, c0, r1, c1) = match kind {
        RefKind::Cell(cell) => {
            let sheet = cell.sheet.unwrap_or_else(|| wb.active_sheet());
            (sheet, cell.row, cell.col, cell.row, cell.col)
        }
        RefKind::Range(r) => {
            let sheet = r.start.sheet.unwrap_or_else(|| wb.active_sheet());
            (
                sheet,
                r.start.row.min(r.end.row),
                r.start.col.min(r.end.col),
                r.start.row.max(r.end.row),
                r.start.col.max(r.end.col),
            )
        }
    };
    let total_rows = r1.saturating_sub(r0).saturating_add(1);
    let width = u64::from(c1.saturating_sub(c0)).saturating_add(1);
    let offset = offset.min(total_rows);
    let requested_rows = limit.clamp(1, MAX_FULL_ROWS);
    let cell_limited_rows = (MAX_FULL_CELLS / width).max(1).min(u64::from(u32::MAX)) as u32;
    let returned_rows = total_rows
        .saturating_sub(offset)
        .min(requested_rows)
        .min(cell_limited_rows);
    let start_row = r0.saturating_add(offset);
    let end_row = start_row.saturating_add(returned_rows.saturating_sub(1));
    let mut rows = Vec::with_capacity(returned_rows as usize);
    if returned_rows > 0 {
        for row in start_row..=end_row {
            let mut line = Vec::new();
            for col in c0..=c1 {
                let mut cell = json!({
                    "ref": format!("{}!{}{}", quote_sheet_name(&wb.sheet(sheet).map(|s| s.name.clone()).unwrap_or_default()), col_to_letters(col).unwrap_or_else(|_| "A".into()), row + 1),
                    "value": cell_text(wb, sheet, row, col),
                });
                if let Some(formula) = wb
                    .get(sheet, row, col)
                    .ok()
                    .flatten()
                    .and_then(|slot| slot.formula)
                    .and_then(|id| wb.intern().formulas.get(id))
                {
                    cell["formula"] = json!(formula);
                }
                line.push(cell);
            }
            rows.push(Value::Array(line));
        }
    }
    let truncated = offset.saturating_add(returned_rows) < total_rows;
    Ok((
        Value::Array(rows),
        json!({
            "offset": offset,
            "returned_rows": returned_rows,
            "total_rows": total_rows,
            "truncated": truncated,
        }),
    ))
}

fn focus(wb: &Workbook, engine: &RecalcEngine, selection: &str) -> Value {
    let Ok(parsed) = parse_a1(selection) else {
        return Value::Null;
    };
    let Ok(kind) = wb.resolve_parsed(parsed) else {
        return Value::Null;
    };
    let cell = match kind {
        RefKind::Cell(c) => c,
        RefKind::Range(r) => r.start,
    };
    let sheet = cell.sheet.unwrap_or_else(|| wb.active_sheet());
    let coord = CellCoord::new(sheet, cell.row, cell.col);
    json!({
        "selection": selection,
        "precedents": precedents_of(wb, engine, coord, false),
        "dependents": dependents_of(wb, engine, coord, false),
    })
}

fn cell_text(wb: &Workbook, sheet: omacell_core::addr::SheetId, row: u32, col: u16) -> String {
    wb.get(sheet, row, col)
        .ok()
        .flatten()
        .map(|slot| format_cell(wb, &slot.value))
        .unwrap_or_default()
}

fn format_cell(wb: &Workbook, value: &CellValue) -> String {
    match value {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => n.to_string(),
        CellValue::Bool(true) => "TRUE".into(),
        CellValue::Bool(false) => "FALSE".into(),
        CellValue::Text(id) => wb.intern().strings.get(*id).unwrap_or("").to_string(),
        CellValue::Error(kind) => kind.as_str().to_string(),
        CellValue::Array(_) => String::new(),
    }
}

/// Deterministic token estimate (`ceil(chars/4)`).
#[must_use]
pub fn estimate_tokens(value: &Value) -> usize {
    let n = serde_json::to_string(value).map(|s| s.len()).unwrap_or(0);
    n.div_ceil(4)
}

pub(crate) fn enforce_budget(card: &mut Value, budget: usize) -> Result<(), AiError> {
    if let Some(page_truncated) = card.pointer("/page/truncated").and_then(Value::as_bool)
        && page_truncated
    {
        card["truncated"] = json!(true);
    }
    if budget > 0 {
        card["token_budget"] = json!(budget);
    }
    set_token_estimate(card);
    if budget == 0 || estimate_tokens(card) <= budget {
        return Ok(());
    }
    card["truncated"] = json!(true);
    for key in [
        "values",
        "sample_rows",
        "columns",
        "functions",
        "tables",
        "names",
        "sheets",
    ] {
        while estimate_tokens(card) > budget && halve_array(card.get_mut(key)) {}
    }
    if let Some(returned) = card
        .get("values")
        .and_then(Value::as_array)
        .map(|rows| rows.len() as u64)
    {
        let offset = card
            .pointer("/page/offset")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let total = card
            .pointer("/page/total_rows")
            .and_then(Value::as_u64)
            .unwrap_or(returned);
        if let Some(page) = card.get_mut("page") {
            page["returned_rows"] = json!(returned);
            page["truncated"] = json!(offset.saturating_add(returned) < total);
        }
    }
    if estimate_tokens(card) > budget {
        for key in ["precedents", "dependents"] {
            while estimate_tokens(card) > budget {
                let changed = card
                    .get_mut("focus")
                    .and_then(|focus| focus.get_mut(key))
                    .and_then(Value::as_array_mut)
                    .is_some_and(|items| {
                        if items.is_empty() {
                            false
                        } else {
                            items.truncate(items.len() / 2);
                            true
                        }
                    });
                if !changed {
                    break;
                }
            }
        }
    }
    set_token_estimate(card);
    if estimate_tokens(card) > budget {
        return Err(AiError::new(
            codes::PAYLOAD,
            format!("token budget {budget} is too small for the minimum workbook card"),
        ));
    }
    Ok(())
}

fn halve_array(value: Option<&mut Value>) -> bool {
    let Some(items) = value.and_then(Value::as_array_mut) else {
        return false;
    };
    if items.is_empty() {
        return false;
    }
    let keep = items.len() / 2;
    items.truncate(keep);
    true
}

fn set_token_estimate(card: &mut Value) {
    card["tokens"] = json!(0);
    for _ in 0..3 {
        let tokens = estimate_tokens(card);
        card["tokens"] = json!(tokens);
    }
}

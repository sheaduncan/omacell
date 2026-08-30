//! AutoFilter model and apply (F-6.2).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::value::Value;
use crate::workbook::Workbook;

/// One column's filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterColumn {
    /// 0-based column index inside the filter range.
    pub col_id: u16,
    /// Criteria.
    pub criteria: FilterCriteria,
}

/// Filter criteria.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterCriteria {
    /// Inclusive value list (display text).
    Values(Vec<String>),
    /// `contains` / `begins` / `ends`.
    Text {
        /// Operator.
        op: TextOp,
        /// Needle.
        value: String,
    },
    /// Numeric compare.
    Number {
        /// Operator.
        op: NumOp,
        /// First bound.
        value: f64,
        /// Second bound (between).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value2: Option<f64>,
    },
    /// Top/bottom N (or percent).
    TopN {
        /// Count or percent.
        n: u32,
        /// Percent.
        percent: bool,
        /// Bottom rather than top.
        bottom: bool,
    },
    /// Above/below average.
    Average {
        /// Below rather than above.
        below: bool,
    },
    /// Fill or font colour ARGB.
    Color {
        /// Compare fill (true) or font (false).
        fill: bool,
        /// Packed ARGB.
        argb: u32,
    },
    /// Calendar year and/or month of a date serial.
    Period {
        /// Year (Excel 1900 date system).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        year: Option<i32>,
        /// Month 1–12.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        month: Option<u32>,
    },
}

impl PartialEq for FilterCriteria {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Values(left), Self::Values(right)) => left == right,
            (
                Self::Text {
                    op: left_op,
                    value: left,
                },
                Self::Text {
                    op: right_op,
                    value: right,
                },
            ) => left_op == right_op && left == right,
            (
                Self::Number {
                    op: left_op,
                    value: left,
                    value2: left2,
                },
                Self::Number {
                    op: right_op,
                    value: right,
                    value2: right2,
                },
            ) => {
                left_op == right_op
                    && left.to_bits() == right.to_bits()
                    && option_f64_bits_eq(*left2, *right2)
            }
            (
                Self::TopN {
                    n: left_n,
                    percent: left_percent,
                    bottom: left_bottom,
                },
                Self::TopN {
                    n: right_n,
                    percent: right_percent,
                    bottom: right_bottom,
                },
            ) => left_n == right_n && left_percent == right_percent && left_bottom == right_bottom,
            (Self::Average { below: left }, Self::Average { below: right }) => left == right,
            (
                Self::Color {
                    fill: left_fill,
                    argb: left_argb,
                },
                Self::Color {
                    fill: right_fill,
                    argb: right_argb,
                },
            ) => left_fill == right_fill && left_argb == right_argb,
            (
                Self::Period {
                    year: left_year,
                    month: left_month,
                },
                Self::Period {
                    year: right_year,
                    month: right_month,
                },
            ) => left_year == right_year && left_month == right_month,
            _ => false,
        }
    }
}

impl Eq for FilterCriteria {}

fn option_f64_bits_eq(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.to_bits() == right.to_bits(),
        (None, None) => true,
        _ => false,
    }
}

/// Text match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOp {
    /// Contains.
    Contains,
    /// Begins with.
    Begins,
    /// Ends with.
    Ends,
}

/// Numeric compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumOp {
    /// `>`.
    Greater,
    /// `>=`.
    GreaterEq,
    /// `<`.
    Less,
    /// `<=`.
    LessEq,
    /// `=`.
    Equal,
    /// `!=`.
    NotEqual,
    /// Between.
    Between,
}

/// Saved AutoFilter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoFilter {
    /// Filtered range (usually includes header).
    pub range: RangeRef,
    /// Per-column criteria.
    pub columns: Vec<FilterColumn>,
}

/// Maximum distinct entries returned for a filter dropdown.
pub const MAX_FILTER_VALUE_OPTIONS: usize = 10_000;

/// Distinct display values for a filter column, optionally narrowed by search.
pub fn filter_value_options(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
    col_id: u16,
    search: &str,
) -> Result<Vec<String>, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    let col = c0
        .checked_add(col_id)
        .filter(|col| *col <= c1)
        .ok_or_else(|| CoreError::new("filter.column", "filter column is outside the range"))?;
    wb.sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", sheet.index())))?;
    let needle = search.to_lowercase();
    let mut values = BTreeSet::new();
    for row in r0.saturating_add(1)..=r1 {
        let value = wb
            .get(sheet, row, col)?
            .map_or_else(String::new, |slot| display(wb, slot.value));
        if (needle.is_empty() || value.to_lowercase().contains(&needle))
            && values.len() < MAX_FILTER_VALUE_OPTIONS
        {
            values.insert(value);
        }
    }
    Ok(values.into_iter().collect())
}

/// Apply `filter` by hiding non-matching data rows (header stays visible).
pub fn apply_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    filter: &AutoFilter,
) -> Result<u32, CoreError> {
    let (r0, c0, r1, c1) = norm(filter.range);
    let nums = collect_nums(wb, sheet, r0, c0, r1, c1, filter)?;
    let mut hidden = 0u32;
    let mut hide: Vec<(u32, bool)> = Vec::new();
    for r in r0.saturating_add(1)..=r1 {
        let mut keep = true;
        for col in &filter.columns {
            let c = c0.saturating_add(col.col_id);
            if c > c1 {
                continue;
            }
            if !matches_row(wb, sheet, r, c, col.col_id, &col.criteria, &nums) {
                keep = false;
                break;
            }
        }
        hide.push((r, !keep));
        if !keep {
            hidden += 1;
        }
    }
    let stored = filter.clone();
    wb.mutate_sheet_edit(sheet, |s| {
        for row in std::mem::take(&mut s.filter_hidden_rows) {
            s.geometry.rows.set_hidden(row, false)?;
        }
        s.autofilter = Some(stored);
        for (row, hidden_row) in hide {
            if hidden_row && !s.geometry.rows.is_hidden(row)? {
                s.geometry.rows.set_hidden(row, true)?;
                s.filter_hidden_rows.insert(row);
            }
        }
        Ok(())
    })?;
    Ok(hidden)
}

/// Restore an imported AutoFilter and identify rows hidden by its criteria.
///
/// SpreadsheetML does not label row-hidden flags as manual versus filtered.
/// A hidden row that fails the saved criteria is therefore treated as
/// filter-hidden, while hidden rows that pass remain manual.
pub fn restore_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    filter: &AutoFilter,
) -> Result<(), CoreError> {
    let (r0, c0, r1, c1) = norm(filter.range);
    let nums = collect_nums(wb, sheet, r0, c0, r1, c1, filter)?;
    let mut filtered_rows = Vec::new();
    for row in r0.saturating_add(1)..=r1 {
        let fails = filter.columns.iter().any(|column| {
            let col = c0.saturating_add(column.col_id);
            col <= c1 && !matches_row(wb, sheet, row, col, column.col_id, &column.criteria, &nums)
        });
        if fails {
            filtered_rows.push(row);
        }
    }
    let stored = filter.clone();
    wb.mutate_sheet_edit(sheet, |sheet| {
        sheet.autofilter = Some(stored);
        sheet.filter_hidden_rows.clear();
        for row in filtered_rows {
            sheet.geometry.rows.set_hidden(row, true)?;
            sheet.filter_hidden_rows.insert(row);
        }
        Ok(())
    })
}

/// Clear AutoFilter and unhide its rows.
pub fn clear_filter(wb: &mut Workbook, sheet: SheetId) -> Result<(), CoreError> {
    wb.mutate_sheet_edit(sheet, |s| {
        for row in std::mem::take(&mut s.filter_hidden_rows) {
            s.geometry.rows.set_hidden(row, false)?;
        }
        s.autofilter = None;
        Ok(())
    })
}

/// Toggle: apply an empty value-filter on `range` or clear.
pub fn toggle_filter(
    wb: &mut Workbook,
    sheet: SheetId,
    range: RangeRef,
) -> Result<bool, CoreError> {
    if wb.sheet(sheet).is_some_and(|s| s.autofilter.is_some()) {
        clear_filter(wb, sheet)?;
        Ok(false)
    } else {
        apply_filter(
            wb,
            sheet,
            &AutoFilter {
                range,
                columns: Vec::new(),
            },
        )?;
        Ok(true)
    }
}

fn collect_nums(
    wb: &Workbook,
    sheet: SheetId,
    r0: u32,
    c0: u16,
    r1: u32,
    _c1: u16,
    filter: &AutoFilter,
) -> Result<Vec<(u16, Vec<f64>)>, CoreError> {
    let mut out = Vec::new();
    for col in &filter.columns {
        match col.criteria {
            FilterCriteria::TopN { .. } | FilterCriteria::Average { .. } => {
                let c = c0.saturating_add(col.col_id);
                let mut v = Vec::new();
                for r in r0.saturating_add(1)..=r1 {
                    if let Ok(Some(slot)) = wb.get(sheet, r, c)
                        && let Value::Number(n) = slot.value
                    {
                        v.push(n);
                    }
                }
                out.push((col.col_id, v));
            }
            _ => {}
        }
    }
    Ok(out)
}

fn matches_row(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    col_id: u16,
    crit: &FilterCriteria,
    nums: &[(u16, Vec<f64>)],
) -> bool {
    let slot = wb.get(sheet, row, col).ok().flatten();
    let text = display(wb, slot.map(|s| s.value).unwrap_or(Value::Empty));
    let number = match slot.map(|s| s.value) {
        Some(Value::Number(n)) => Some(n),
        _ => None,
    };
    match crit {
        FilterCriteria::Values(vals) => {
            if vals.is_empty() {
                true
            } else {
                vals.iter().any(|v| v == &text)
            }
        }
        FilterCriteria::Text { op, value } => {
            let t = text.to_lowercase();
            let n = value.to_lowercase();
            match op {
                TextOp::Contains => t.contains(&n),
                TextOp::Begins => t.starts_with(&n),
                TextOp::Ends => t.ends_with(&n),
            }
        }
        FilterCriteria::Number { op, value, value2 } => {
            let Some(n) = number else {
                return false;
            };
            match op {
                NumOp::Greater => n > *value,
                NumOp::GreaterEq => n >= *value,
                NumOp::Less => n < *value,
                NumOp::LessEq => n <= *value,
                NumOp::Equal => (n - *value).abs() < 1e-12,
                NumOp::NotEqual => (n - *value).abs() >= 1e-12,
                NumOp::Between => {
                    let hi = value2.unwrap_or(*value);
                    n >= *value && n <= hi
                }
            }
        }
        FilterCriteria::TopN { n, percent, bottom } => {
            let Some(cur) = number else {
                return false;
            };
            let Some((_, vs)) = nums.iter().find(|(id, _)| *id == col_id) else {
                return true;
            };
            // nums keyed by col_id not absolute col; caller stored col_id.
            let mut sorted = vs.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            if *bottom {
                // keep lowest
            } else {
                sorted.reverse();
            }
            let take = if *percent {
                (sorted.len() as u32 * (*n).min(100)).div_ceil(100).max(1) as usize
            } else {
                (*n as usize).min(sorted.len())
            };
            sorted.iter().take(take).any(|x| (*x - cur).abs() < 1e-12)
        }
        FilterCriteria::Average { below } => {
            let Some(cur) = number else {
                return false;
            };
            let Some((_, vs)) = nums.iter().find(|(id, _)| *id == col_id) else {
                return true;
            };
            if vs.is_empty() {
                return true;
            }
            let avg = vs.iter().sum::<f64>() / vs.len() as f64;
            if *below { cur < avg } else { cur > avg }
        }
        FilterCriteria::Color { fill, argb } => {
            let Some(slot) = slot else {
                return *argb == 0;
            };
            let style = wb.intern().styles.get(slot.style);
            let got = if *fill {
                match style.map(|s| &s.fill) {
                    Some(crate::style::Fill::Solid {
                        fg: crate::style::Color::Rgb { argb: a },
                    })
                    | Some(crate::style::Fill::Pattern {
                        fg: crate::style::Color::Rgb { argb: a },
                        ..
                    }) => *a,
                    _ => 0,
                }
            } else {
                match style.map(|s| s.font.color) {
                    Some(crate::style::Color::Rgb { argb: a }) => a,
                    _ => 0,
                }
            };
            got == *argb
        }
        FilterCriteria::Period { year, month } => {
            let Some(n) = number else {
                return false;
            };
            let serial = n.trunc() as i64;
            let Some(date) = crate::dates::serial_to_date(serial, wb.settings().date_system) else {
                return false;
            };
            year.is_none_or(|y| date.year == y) && month.is_none_or(|m| u32::from(date.month) == m)
        }
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

fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}

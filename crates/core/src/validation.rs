//! Data validation (F-6.4).

use serde::{Deserialize, Serialize};

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::value::Value;
use crate::workbook::Workbook;

/// Validation type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DvType {
    /// Any value.
    #[default]
    Any,
    /// Whole number.
    Whole,
    /// Decimal.
    Decimal,
    /// List (inline or range).
    List,
    /// Date serial.
    Date,
    /// Time fraction.
    Time,
    /// Text length.
    TextLength,
    /// Custom formula (truthy).
    Custom,
}

/// Error alert style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DvErrorStyle {
    /// Stop.
    #[default]
    Stop,
    /// Warning.
    Warning,
    /// Information.
    Information,
}

/// Compare operator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DvOp {
    /// Between.
    #[default]
    Between,
    /// Not between.
    NotBetween,
    /// Equal.
    Equal,
    /// Not equal.
    NotEqual,
    /// Greater.
    Greater,
    /// Less.
    Less,
    /// Greater or equal.
    GreaterEq,
    /// Less or equal.
    LessEq,
}

/// One data-validation rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataValidation {
    /// Target range.
    pub range: RangeRef,
    /// Kind.
    pub kind: DvType,
    /// Operator.
    #[serde(default)]
    pub op: DvOp,
    /// Formula / min / list source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula1: Option<String>,
    /// Max / second bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula2: Option<String>,
    /// Allow blank.
    #[serde(default = "yes")]
    pub allow_blank: bool,
    /// Error style.
    #[serde(default)]
    pub error_style: DvErrorStyle,
    /// Error title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_title: Option<String>,
    /// Error message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Input title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_title: Option<String>,
    /// Input message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_message: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for DataValidation {
    fn default() -> Self {
        Self {
            range: RangeRef::from_corners(
                crate::addr::CellRef::new(0, 0).unwrap(),
                crate::addr::CellRef::new(0, 0).unwrap(),
            ),
            kind: DvType::Any,
            op: DvOp::Between,
            formula1: None,
            formula2: None,
            allow_blank: true,
            error_style: DvErrorStyle::Stop,
            error_title: None,
            error_message: None,
            input_title: None,
            input_message: None,
        }
    }
}

/// Whether `cell` satisfies validations on `sheet`.
pub fn validate_cell(wb: &Workbook, sheet: SheetId, row: u32, col: u16) -> Result<(), CoreError> {
    let Some(s) = wb.sheet(sheet) else {
        return Ok(());
    };
    let slot = wb.get(sheet, row, col).ok().flatten();
    let empty = slot.is_none() || matches!(slot.map(|c| c.value), Some(Value::Empty));
    for dv in &s.validations {
        if !in_range(dv.range, row, col) {
            continue;
        }
        if empty && dv.allow_blank {
            continue;
        }
        if !ok_value(wb, sheet, slot.map(|c| c.value).unwrap_or(Value::Empty), dv) {
            return Err(CoreError::new(
                "validation.failed",
                dv.error_message
                    .clone()
                    .unwrap_or_else(|| "value failed data validation".into()),
            ));
        }
    }
    Ok(())
}

/// Cells that fail validation (circle-invalid).
pub fn invalid_cells(wb: &Workbook, sheet: SheetId) -> Vec<(u32, u16)> {
    let Some(s) = wb.sheet(sheet) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dv in &s.validations {
        let (r0, c0, r1, c1) = norm(dv.range);
        for r in r0..=r1 {
            for c in c0..=c1 {
                if validate_cell(wb, sheet, r, c).is_err() {
                    out.push((r, c));
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn ok_value(wb: &Workbook, sheet: SheetId, value: Value, dv: &DataValidation) -> bool {
    match dv.kind {
        DvType::Any => true,
        DvType::Whole | DvType::Decimal | DvType::Date | DvType::Time => {
            let Value::Number(n) = value else {
                return false;
            };
            if dv.kind == DvType::Whole && n.fract().abs() > 1e-9 {
                return false;
            }
            let lo = parse_num(dv.formula1.as_deref());
            let hi = parse_num(dv.formula2.as_deref());
            cmp_num(n, dv.op, lo, hi)
        }
        DvType::TextLength => {
            let len = match value {
                Value::Text(id) => wb.intern().strings.get(id).map(str::len).unwrap_or(0),
                Value::Empty => 0,
                _ => display(wb, value).len(),
            } as f64;
            cmp_num(
                len,
                dv.op,
                parse_num(dv.formula1.as_deref()),
                parse_num(dv.formula2.as_deref()),
            )
        }
        DvType::List => {
            let text = display(wb, value);
            let Some(src) = dv.formula1.as_deref() else {
                return true;
            };
            let src = src.trim().trim_matches('"');
            if src.contains(',') {
                src.split(',').any(|p| p.trim() == text)
            } else if let Ok(parsed) = crate::addr::parse_a1(src) {
                let range = match parsed.kind {
                    crate::addr::RefKind::Range(rg) => rg,
                    crate::addr::RefKind::Cell(c) => crate::addr::RangeRef::from_corners(c, c),
                };
                let list_sheet = parsed
                    .sheet
                    .as_ref()
                    .and_then(|spec| wb.sheet_by_name(&spec.start).map(|s| s.id))
                    .unwrap_or(sheet);
                list_contains(wb, list_sheet, range, &text)
            } else {
                src == text
            }
        }
        DvType::Custom => formula_truthy(wb, dv.formula1.as_deref().unwrap_or("TRUE")),
    }
}

fn cmp_num(n: f64, op: DvOp, lo: Option<f64>, hi: Option<f64>) -> bool {
    match op {
        DvOp::Between => n >= lo.unwrap_or(n) && n <= hi.unwrap_or(n),
        DvOp::NotBetween => n < lo.unwrap_or(n) || n > hi.unwrap_or(n),
        DvOp::Equal => lo.is_some_and(|x| (n - x).abs() < 1e-12),
        DvOp::NotEqual => lo.is_none_or(|x| (n - x).abs() >= 1e-12),
        DvOp::Greater => lo.is_some_and(|x| n > x),
        DvOp::Less => lo.is_some_and(|x| n < x),
        DvOp::GreaterEq => lo.is_some_and(|x| n >= x),
        DvOp::LessEq => lo.is_some_and(|x| n <= x),
    }
}

fn parse_num(s: Option<&str>) -> Option<f64> {
    s.and_then(|t| t.parse().ok())
}

fn list_contains(wb: &Workbook, sheet: SheetId, range: RangeRef, text: &str) -> bool {
    let (r0, c0, r1, c1) = norm(range);
    for r in r0..=r1 {
        for c in c0..=c1 {
            let slot = wb.get(sheet, r, c).ok().flatten();
            let cell = display(wb, slot.map(|s| s.value).unwrap_or(Value::Empty));
            if cell == text {
                return true;
            }
        }
    }
    false
}

fn formula_truthy(wb: &Workbook, src: &str) -> bool {
    match src.trim() {
        "TRUE" | "1" => return true,
        "FALSE" | "0" | "" => return false,
        _ => {}
    }
    crate::condfmt::eval_truthy(wb, src)
}

fn display(wb: &Workbook, v: Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

fn in_range(r: RangeRef, row: u32, col: u16) -> bool {
    let (r0, c0, r1, c1) = norm(r);
    row >= r0 && row <= r1 && col >= c0 && col <= c1
}

fn norm(r: RangeRef) -> (u32, u16, u32, u16) {
    (
        r.start.row.min(r.end.row),
        r.start.col.min(r.end.col),
        r.start.row.max(r.end.row),
        r.start.col.max(r.end.col),
    )
}

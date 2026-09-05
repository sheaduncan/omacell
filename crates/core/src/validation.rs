//! Data validation (F-6.4).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::addr::{RangeRef, SheetId};
use crate::error::CoreError;
use crate::eval::FnRegistry;
use crate::graph::CellCoord;
use crate::names::{MAX_DEFINED_NAME_DEPTH, NameReferent, NameScope};
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Excel displays at most 255 invalid-data circles at once.
pub const MAX_INVALID_CIRCLES: usize = 255;
/// Maximum values returned to a validation dropdown.
pub const MAX_VALIDATION_LIST_ITEMS: usize = 32_767;

impl Default for DataValidation {
    fn default() -> Self {
        let a1 = crate::addr::CellRef {
            sheet: None,
            row: 0,
            col: 0,
            row_abs: false,
            col_abs: false,
        };
        Self {
            range: RangeRef::from_corners(a1, a1),
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
    let registry = FnRegistry::new();
    validate_cell_with_registry(wb, sheet, row, col, &registry)
}

/// Validate a cell using the application's registered worksheet functions.
pub fn validate_cell_with_registry(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    registry: &FnRegistry,
) -> Result<(), CoreError> {
    let s = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", sheet.index())))?;
    let slot = wb.get(sheet, row, col)?;
    let empty = slot.is_none() || matches!(slot.map(|c| c.value), Some(Value::Empty));
    for dv in &s.validations {
        if !in_range(dv.range, row, col) {
            continue;
        }
        if empty && dv.allow_blank {
            continue;
        }
        if !ok_value(
            wb,
            sheet,
            row,
            col,
            slot.map(|c| c.value).unwrap_or(Value::Empty),
            dv,
            registry,
        ) {
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
    let registry = FnRegistry::new();
    invalid_cells_with_registry(wb, sheet, &registry)
}

/// Cells that fail validation using the application's registered functions.
pub fn invalid_cells_with_registry(
    wb: &Workbook,
    sheet: SheetId,
    registry: &FnRegistry,
) -> Vec<(u32, u16)> {
    let Some(s) = wb.sheet(sheet) else {
        return Vec::new();
    };
    let mut invalid = BTreeSet::new();
    for dv in &s.validations {
        if dv.kind == DvType::Any {
            continue;
        }
        let (r0, c0, r1, c1) = norm(dv.range);
        if dv.allow_blank {
            for (row, col, _) in s.store.iter() {
                if row >= r0
                    && row <= r1
                    && col >= c0
                    && col <= c1
                    && validate_cell_with_registry(wb, sheet, row, col, registry).is_err()
                {
                    invalid.insert((row, col));
                    if invalid.len() >= MAX_INVALID_CIRCLES {
                        break;
                    }
                }
            }
        } else {
            'rows: for row in r0..=r1 {
                for col in c0..=c1 {
                    if validate_cell_with_registry(wb, sheet, row, col, registry).is_err() {
                        invalid.insert((row, col));
                    }
                    if invalid.len() >= MAX_INVALID_CIRCLES {
                        break 'rows;
                    }
                }
            }
        }
        if invalid.len() >= MAX_INVALID_CIRCLES {
            break;
        }
    }
    invalid.into_iter().collect()
}

/// Resolve the list-validation dropdown values for a cell.
///
/// Returns `None` when no list validation applies. Inline comma lists and A1
/// ranges (including a sheet prefix) are supported and bounded to
/// [`MAX_VALIDATION_LIST_ITEMS`].
pub fn validation_list_values(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
) -> Result<Option<Vec<String>>, CoreError> {
    let source = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", sheet.index())))?
        .validations
        .iter()
        .find(|validation| validation.kind == DvType::List && in_range(validation.range, row, col))
        .and_then(|validation| validation.formula1.as_deref());
    source
        .map(|source| resolve_list_source(wb, sheet, source))
        .transpose()
}

fn ok_value(
    wb: &Workbook,
    sheet: SheetId,
    row: u32,
    col: u16,
    value: Value,
    dv: &DataValidation,
    registry: &FnRegistry,
) -> bool {
    match dv.kind {
        DvType::Any => true,
        DvType::Whole | DvType::Decimal | DvType::Date | DvType::Time => {
            let Value::Number(n) = value else {
                return false;
            };
            if dv.kind == DvType::Whole && n.fract().abs() > 1e-9 {
                return false;
            }
            if dv.kind == DvType::Date
                && crate::dates::serial_to_date(n.trunc() as i64, wb.settings().date_system)
                    .is_none()
            {
                return false;
            }
            if dv.kind == DvType::Time && !(0.0..1.0).contains(&n) {
                return false;
            }
            let (origin_row, origin_col, _, _) = norm(dv.range);
            let lo = formula_num(
                wb,
                CellCoord::new(sheet, row, col),
                CellCoord::new(sheet, origin_row, origin_col),
                dv.formula1.as_deref(),
                registry,
            );
            let hi = formula_num(
                wb,
                CellCoord::new(sheet, row, col),
                CellCoord::new(sheet, origin_row, origin_col),
                dv.formula2.as_deref(),
                registry,
            );
            cmp_num(n, dv.op, lo, hi)
        }
        DvType::TextLength => {
            let len = match value {
                Value::Text(id) => wb
                    .intern()
                    .strings
                    .get(id)
                    .map(|text| text.chars().count())
                    .unwrap_or(0),
                Value::Empty => 0,
                _ => display(wb, value).chars().count(),
            } as f64;
            let (origin_row, origin_col, _, _) = norm(dv.range);
            cmp_num(
                len,
                dv.op,
                formula_num(
                    wb,
                    CellCoord::new(sheet, row, col),
                    CellCoord::new(sheet, origin_row, origin_col),
                    dv.formula1.as_deref(),
                    registry,
                ),
                formula_num(
                    wb,
                    CellCoord::new(sheet, row, col),
                    CellCoord::new(sheet, origin_row, origin_col),
                    dv.formula2.as_deref(),
                    registry,
                ),
            )
        }
        DvType::List => {
            let text = display(wb, value);
            let Some(src) = dv.formula1.as_deref() else {
                return false;
            };
            resolve_list_source(wb, sheet, src)
                .is_ok_and(|values| values.iter().any(|candidate| candidate == &text))
        }
        DvType::Custom => formula_truthy_at(
            wb,
            CellCoord::new(sheet, row, col),
            CellCoord::new(sheet, norm(dv.range).0, norm(dv.range).1),
            dv.formula1.as_deref().unwrap_or("TRUE"),
            registry,
        ),
    }
}

fn cmp_num(n: f64, op: DvOp, lo: Option<f64>, hi: Option<f64>) -> bool {
    match op {
        DvOp::Between => lo.zip(hi).is_some_and(|(lo, hi)| n >= lo && n <= hi),
        DvOp::NotBetween => lo.zip(hi).is_some_and(|(lo, hi)| n < lo || n > hi),
        DvOp::Equal => lo.is_some_and(|x| (n - x).abs() < 1e-12),
        DvOp::NotEqual => lo.is_some_and(|x| (n - x).abs() >= 1e-12),
        DvOp::Greater => lo.is_some_and(|x| n > x),
        DvOp::Less => lo.is_some_and(|x| n < x),
        DvOp::GreaterEq => lo.is_some_and(|x| n >= x),
        DvOp::LessEq => lo.is_some_and(|x| n <= x),
    }
}

fn formula_num(
    wb: &Workbook,
    at: CellCoord,
    origin: CellCoord,
    src: Option<&str>,
    registry: &FnRegistry,
) -> Option<f64> {
    crate::condfmt::eval_number_relative_with_registry(wb, at, origin, src?, registry)
}

fn resolve_list_source(
    wb: &Workbook,
    default_sheet: SheetId,
    source: &str,
) -> Result<Vec<String>, CoreError> {
    resolve_list_source_inner(wb, default_sheet, source, &mut Vec::new())
}

fn resolve_list_source_inner(
    wb: &Workbook,
    default_sheet: SheetId,
    source: &str,
    resolving_names: &mut Vec<(NameScope, String)>,
) -> Result<Vec<String>, CoreError> {
    let source = source.trim().trim_start_matches('=');
    if source.starts_with('"') && source.ends_with('"') && source.len() >= 2 {
        return Ok(source[1..source.len() - 1]
            .split(',')
            .map(|value| value.trim().to_string())
            .take(MAX_VALIDATION_LIST_ITEMS)
            .collect());
    }
    if let Ok(parsed) = crate::addr::parse_a1(source) {
        let range = match parsed.kind {
            crate::addr::RefKind::Range(range) => range,
            crate::addr::RefKind::Cell(cell) => crate::addr::RangeRef::from_corners(cell, cell),
        };
        let sheet = if let Some(spec) = parsed.sheet {
            wb.sheet_by_name(&spec.start)
                .map(|sheet| sheet.id)
                .ok_or_else(|| {
                    CoreError::sheet_name(format!("unknown list source sheet {:?}", spec.start))
                })?
        } else {
            default_sheet
        };
        return list_values_from_range(wb, sheet, range);
    }

    let Some(defined) = wb.names().resolve(default_sheet, source) else {
        return Ok(vec![source.to_string()]);
    };
    let key = (defined.scope, defined.name.to_lowercase());
    if resolving_names.len() >= MAX_DEFINED_NAME_DEPTH || resolving_names.contains(&key) {
        return Err(CoreError::new(
            "validation.list",
            format!("defined-name cycle in validation list source {source:?}"),
        ));
    }
    let scope_sheet = match defined.scope {
        NameScope::Workbook => default_sheet,
        NameScope::Sheet(sheet) => sheet,
    };
    let referent = defined.referent.clone();
    resolving_names.push(key);
    let result = match referent {
        NameReferent::Range(range) => {
            list_values_from_range(wb, range.start.sheet.unwrap_or(scope_sheet), range)
        }
        NameReferent::Formula(formula) => {
            resolve_list_source_inner(wb, scope_sheet, &formula, resolving_names)
        }
        NameReferent::Constant(value) => {
            let value = display(wb, value);
            Ok((!value.is_empty()).then_some(value).into_iter().collect())
        }
    };
    resolving_names.pop();
    result
}

fn list_values_from_range(
    wb: &Workbook,
    sheet: SheetId,
    range: RangeRef,
) -> Result<Vec<String>, CoreError> {
    let (r0, c0, r1, c1) = norm(range);
    let sheet = wb
        .sheet(sheet)
        .ok_or_else(|| CoreError::sheet_id(format!("unknown sheet {}", sheet.index())))?;
    let mut values = Vec::new();
    values
        .try_reserve(MAX_VALIDATION_LIST_ITEMS)
        .map_err(|_| CoreError::new("validation.list", "dropdown allocation failed"))?;
    for (_, _, slot) in sheet.store.iter_region(r0, c0, r1, c1) {
        let value = display(wb, slot.value);
        if !value.is_empty() {
            values.push(value);
            if values.len() >= MAX_VALIDATION_LIST_ITEMS {
                break;
            }
        }
    }
    Ok(values)
}

fn formula_truthy_at(
    wb: &Workbook,
    at: CellCoord,
    origin: CellCoord,
    src: &str,
    registry: &FnRegistry,
) -> bool {
    match src.trim() {
        "TRUE" | "1" => return true,
        "FALSE" | "0" | "" => return false,
        _ => {}
    }
    crate::condfmt::eval_truthy_relative_with_registry(wb, at, origin, src, registry)
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

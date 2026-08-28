//! Intern-handle-free restore payloads and inverse helpers.

use omacell_core::changeset::CommandCall;
use omacell_core::command::CommandId;
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::intern::ArrayPayload;
use omacell_core::storage::{CellFlags, CellSlot};
use omacell_core::style::{Fill, Style, StyleId, Underline};
use omacell_core::value::{Array2D, Value};
use omacell_core::workbook::Workbook;
use serde::{Deserialize, Serialize};

use crate::args::{CellRestoreArgs, StyleRestoreArgs, StyleSetArgs};
use crate::error as bus_error;
use crate::resolve::{ResolvedCell, format_cell};

pub(crate) fn call(id: &str, args: serde_json::Value) -> Result<CommandCall, CoreError> {
    Ok(CommandCall {
        id: CommandId::new(id)?,
        args,
    })
}

pub(crate) fn slot_input(wb: &Workbook, slot: &CellSlot) -> String {
    if let Some(fid) = slot.formula {
        return wb.intern().formulas.get(fid).unwrap_or("").to_string();
    }
    match slot.value {
        Value::Empty => String::new(),
        Value::Number(n) => number_input(n),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        Value::Error(kind) => kind.as_str().to_string(),
        Value::Array(_) => String::new(),
    }
}

fn number_input(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{n:.0}")
    } else if let Some(num) = serde_json::Number::from_f64(n) {
        num.to_string()
    } else {
        n.to_string()
    }
}

pub(crate) fn style_of(wb: &Workbook, id: StyleId) -> Style {
    wb.intern().styles.get(id).cloned().unwrap_or_default()
}

pub(crate) fn style_format(wb: &Workbook, style: &Style) -> Option<String> {
    let code = wb.num_fmt_code(style.num_fmt)?;
    if code.as_ref() == "General" && style.num_fmt.index() == 0 {
        None
    } else {
        Some(code.into_owned())
    }
}

pub(crate) fn inverse_contents(
    wb: &Workbook,
    cell: ResolvedCell,
) -> Result<CommandCall, CoreError> {
    match wb.get(cell.sheet, cell.row, cell.col)? {
        None => call(
            "cell.restore",
            serde_json::to_value(CellRestoreArgs {
                cell_ref: format_cell(wb, cell),
                absent: true,
                formula: None,
                value: None,
                style: None,
                format: None,
                flags: 0,
            })
            .map_err(|err| bus_error::args(format!("cannot encode cell inverse: {err}")))?,
        ),
        Some(slot) => {
            let style = style_of(wb, slot.style);
            let format = style_format(wb, &style);
            let style_json = serde_json::to_value(&style)
                .map_err(|err| bus_error::args(format!("cannot encode inverse style: {err}")))?;
            let value = serde_json::to_value(stored_value(wb, slot.value, 0)?)
                .map_err(|err| bus_error::args(format!("cannot encode inverse value: {err}")))?;
            call(
                "cell.restore",
                serde_json::to_value(CellRestoreArgs {
                    cell_ref: format_cell(wb, cell),
                    absent: false,
                    formula: slot
                        .formula
                        .and_then(|id| wb.intern().formulas.get(id).map(str::to_owned)),
                    value: Some(value),
                    style: Some(style_json),
                    format,
                    flags: slot.flags.bits(),
                })
                .map_err(|err| bus_error::args(format!("cannot encode cell inverse: {err}")))?,
            )
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredValue {
    Empty,
    Number {
        bits: u64,
    },
    Bool {
        value: bool,
    },
    Text {
        value: String,
    },
    Error {
        value: ErrorKind,
    },
    Array {
        rows: u32,
        cols: u32,
        values: Vec<StoredValue>,
    },
}

fn stored_value(wb: &Workbook, value: Value, depth: usize) -> Result<StoredValue, CoreError> {
    if depth >= 64 {
        return Err(bus_error::args("cell inverse array nesting exceeds 64"));
    }
    Ok(match value {
        Value::Empty => StoredValue::Empty,
        Value::Number(value) => StoredValue::Number {
            bits: value.to_bits(),
        },
        Value::Bool(value) => StoredValue::Bool { value },
        Value::Text(id) => StoredValue::Text {
            value: wb
                .intern()
                .strings
                .get(id)
                .ok_or_else(|| bus_error::args("cell inverse references missing text"))?
                .to_owned(),
        },
        Value::Error(value) => StoredValue::Error { value },
        Value::Array(id) => {
            let payload = wb
                .intern()
                .arrays
                .get(id)
                .ok_or_else(|| bus_error::args("cell inverse references missing array"))?;
            let values = payload
                .values
                .iter()
                .copied()
                .map(|item| stored_value(wb, item, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            StoredValue::Array {
                rows: payload.shape.rows,
                cols: payload.shape.cols,
                values,
            }
        }
    })
}

pub(crate) fn restore_cell_value(
    wb: &mut Workbook,
    encoded: serde_json::Value,
) -> Result<(Value, Option<RootRef>), CoreError> {
    let stored: StoredValue = serde_json::from_value(encoded)
        .map_err(|err| bus_error::args(format!("invalid stored cell value: {err}")))?;
    restore_value(wb, stored, 0, true)
}

pub(crate) enum RootRef {
    Text(omacell_core::value::StrId),
    Array(omacell_core::value::ArrayId),
}

pub(crate) fn release_root_ref(wb: &mut Workbook, owned: Option<RootRef>) {
    match owned {
        Some(RootRef::Text(id)) => wb.release_text(id),
        Some(RootRef::Array(id)) => wb.release_array(id),
        None => {}
    }
}

fn restore_value(
    wb: &mut Workbook,
    stored: StoredValue,
    depth: usize,
    root: bool,
) -> Result<(Value, Option<RootRef>), CoreError> {
    if depth >= 64 {
        return Err(bus_error::args("stored cell array nesting exceeds 64"));
    }
    Ok(match stored {
        StoredValue::Empty => (Value::Empty, None),
        StoredValue::Number { bits } => (Value::Number(f64::from_bits(bits)), None),
        StoredValue::Bool { value } => (Value::Bool(value), None),
        StoredValue::Text { value } => {
            let id = wb.intern_text(&value);
            (Value::Text(id), root.then_some(RootRef::Text(id)))
        }
        StoredValue::Error { value } => (Value::Error(value), None),
        StoredValue::Array { rows, cols, values } => {
            let shape = Array2D::new(rows, cols)?;
            if values.len() as u32 != shape.len() {
                return Err(bus_error::args(
                    "stored cell array shape does not match values",
                ));
            }
            let mut decoded = Vec::with_capacity(values.len());
            for item in values {
                // Array payloads own the nested interner references for their
                // lifetime; only the top-level cell reference is released.
                decoded.push(restore_value(wb, item, depth + 1, false)?.0);
            }
            let id = wb.intern_array(ArrayPayload::new(shape, decoded)?);
            (Value::Array(id), root.then_some(RootRef::Array(id)))
        }
    })
}

pub(crate) fn decode_cell_flags(bits: u8) -> Result<CellFlags, CoreError> {
    serde_json::from_value(serde_json::json!(bits))
        .map_err(|err| bus_error::args(format!("invalid stored cell flags: {err}")))
}

pub(crate) fn inverse_style(wb: &Workbook, cell: ResolvedCell) -> Result<CommandCall, CoreError> {
    match wb.get(cell.sheet, cell.row, cell.col)? {
        None => call(
            "style.restore",
            serde_json::to_value(StyleRestoreArgs {
                cell_ref: format_cell(wb, cell),
                style: serde_json::to_value(Style::default()).unwrap_or(serde_json::Value::Null),
                format: None,
                absent: true,
            })
            .unwrap_or_else(|_| serde_json::json!({"ref": format_cell(wb, cell), "absent": true})),
        ),
        Some(slot) => {
            let style = style_of(wb, slot.style);
            let format = style_format(wb, &style);
            call(
                "style.restore",
                serde_json::to_value(StyleRestoreArgs {
                    cell_ref: format_cell(wb, cell),
                    style: serde_json::to_value(&style).unwrap_or(serde_json::Value::Null),
                    format,
                    absent: false,
                })
                .unwrap_or_else(|_| serde_json::json!({"ref": format_cell(wb, cell)})),
            )
        }
    }
}

pub(crate) fn apply_style_patch(
    wb: &mut Workbook,
    mut style: Style,
    patch: &StyleSetArgs,
) -> Result<Style, CoreError> {
    if let Some(bold) = patch.bold {
        style.font.bold = bold;
    }
    if let Some(italic) = patch.italic {
        style.font.italic = italic;
    }
    if let Some(underline) = patch.underline {
        style.font.underline = if underline {
            Underline::Single
        } else {
            Underline::None
        };
    }
    if let Some(strike) = patch.strike {
        style.font.strike = strike;
    }
    if let Some(size) = patch.size_pt {
        style.font.size_pt = size;
    }
    if let Some(name) = &patch.font_name {
        style.font.name = name.clone();
    }
    if let Some(argb) = patch.font_color_argb {
        style.font.color = omacell_core::style::Color::Rgb { argb };
    }
    if let Some(argb) = patch.fill_argb {
        style.fill = Fill::Solid {
            fg: omacell_core::style::Color::Rgb { argb },
        };
    }
    if let Some(wrap) = patch.wrap {
        style.alignment.wrap = wrap;
    }
    if let Some(h) = &patch.horizontal {
        style.alignment.horizontal = parse_horizontal(h)?;
    }
    if let Some(v) = &patch.vertical {
        style.alignment.vertical = parse_vertical(v)?;
    }
    if let Some(locked) = patch.locked {
        style.protection.locked = locked;
    }
    if let Some(hidden) = patch.hidden {
        style.protection.hidden = hidden;
    }
    if let Some(code) = &patch.format {
        style.num_fmt = wb.intern_num_fmt(code)?;
    }
    Ok(style)
}

fn parse_horizontal(name: &str) -> Result<omacell_core::style::HorizontalAlign, CoreError> {
    use omacell_core::style::HorizontalAlign;
    Ok(match name {
        "general" => HorizontalAlign::General,
        "left" => HorizontalAlign::Left,
        "center" => HorizontalAlign::Center,
        "right" => HorizontalAlign::Right,
        "fill" => HorizontalAlign::Fill,
        "justify" => HorizontalAlign::Justify,
        "center_continuous" => HorizontalAlign::CenterContinuous,
        "distributed" => HorizontalAlign::Distributed,
        other => {
            return Err(bus_error::args(format!(
                "unknown horizontal alignment {other:?}"
            )));
        }
    })
}

fn parse_vertical(name: &str) -> Result<omacell_core::style::VerticalAlign, CoreError> {
    use omacell_core::style::VerticalAlign;
    Ok(match name {
        "top" => VerticalAlign::Top,
        "center" => VerticalAlign::Center,
        "bottom" => VerticalAlign::Bottom,
        "justify" => VerticalAlign::Justify,
        "distributed" => VerticalAlign::Distributed,
        other => {
            return Err(bus_error::args(format!(
                "unknown vertical alignment {other:?}"
            )));
        }
    })
}

pub(crate) fn apply_stored_style(
    wb: &mut Workbook,
    style_json: serde_json::Value,
    format: Option<&str>,
) -> Result<Style, CoreError> {
    let mut style: Style = serde_json::from_value(style_json)
        .map_err(|err| bus_error::args(format!("invalid style payload: {err}")))?;
    if let Some(code) = format {
        style.num_fmt = wb.intern_num_fmt(code)?;
    }
    Ok(style)
}

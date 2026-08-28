//! Intern-handle-free restore payloads and inverse helpers.

use omacell_core::changeset::CommandCall;
use omacell_core::command::CommandId;
use omacell_core::error::CoreError;
use omacell_core::storage::CellSlot;
use omacell_core::style::{Fill, Style, StyleId, Underline};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

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
                input: None,
                style: None,
                format: None,
            })
            .unwrap_or_else(|_| serde_json::json!({"ref": format_cell(wb, cell), "absent": true})),
        ),
        Some(slot) => {
            let style = style_of(wb, slot.style);
            let format = style_format(wb, &style);
            let style_json = serde_json::to_value(&style).unwrap_or(serde_json::Value::Null);
            call(
                "cell.restore",
                serde_json::to_value(CellRestoreArgs {
                    cell_ref: format_cell(wb, cell),
                    absent: false,
                    input: Some(slot_input(wb, slot)),
                    style: Some(style_json),
                    format,
                })
                .unwrap_or_else(|_| serde_json::json!({"ref": format_cell(wb, cell)})),
            )
        }
    }
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

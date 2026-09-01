//! Native legacy BIFF `.xls` reader.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use calamine::{CellErrorType, Data, Reader, SheetType, SheetVisible, Xls, open_workbook};
use omacell_core::addr::{CellRef, RangeRef, RefKind, parse_a1};
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::sheet::SheetVisibility;
use omacell_core::storage::{CellFlags, CellSlot};
use omacell_core::style::{NumFmtId, Style};
use omacell_core::value::Value;
use omacell_core::workbook::{DateSystem, Workbook};

use crate::error;
use crate::xlsx::peer_lock_blocks;

/// Maximum accepted legacy workbook size before BIFF parsing.
pub const MAX_XLS_BYTES: u64 = 256 * 1024 * 1024;
const MAX_XLS_SHEETS: usize = 1_024;

/// Open a legacy BIFF `.xls` workbook without launching an external converter.
pub fn open_xls(path: &Path) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let len = std::fs::metadata(path)
        .map_err(|err| error::xls_bridge(format!("{}: {err}", path.display())))?
        .len();
    validate_size(len)?;

    let source: Xls<_> =
        open_workbook::<Xls<_>, _>(path).map_err(|err| error::xls_bridge(err.to_string()))?;
    load_xls(source)
}

/// Open legacy BIFF `.xls` bytes without an external converter.
pub fn open_xls_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    validate_size(bytes.len() as u64)?;
    let source = Xls::new(Cursor::new(bytes)).map_err(|err| error::xls_bridge(err.to_string()))?;
    load_xls(source)
}

fn validate_size(len: u64) -> Result<(), CoreError> {
    if len > MAX_XLS_BYTES {
        return Err(error::xlsx_limit(format!(
            "legacy .xls file is {len} bytes; maximum is {MAX_XLS_BYTES}"
        )));
    }
    Ok(())
}

fn load_xls<RS: Read + Seek>(mut source: Xls<RS>) -> Result<Workbook, CoreError> {
    let metadata = source.sheets_metadata().to_vec();
    if metadata.is_empty() {
        return Err(error::xls_bridge("workbook contains no sheets"));
    }
    if metadata.len() > MAX_XLS_SHEETS {
        return Err(error::xlsx_limit(format!(
            "legacy .xls workbook has {} sheets; maximum is {MAX_XLS_SHEETS}",
            metadata.len()
        )));
    }
    if !metadata
        .iter()
        .any(|sheet| sheet.visible == SheetVisible::Visible)
    {
        return Err(error::xls_bridge(
            "workbook must contain at least one visible sheet",
        ));
    }

    let mut workbook = Workbook::new();
    workbook.settings_mut().date_system = if source.has_1904_epoch() {
        DateSystem::Excel1904
    } else {
        DateSystem::Excel1900
    };
    let first = workbook.active_sheet();
    let mut sheets = Vec::with_capacity(metadata.len());
    for (index, sheet) in metadata.iter().enumerate() {
        let id = if index == 0 {
            workbook.rename_sheet(first, &sheet.name)?;
            first
        } else {
            workbook.add_sheet(&sheet.name)?
        };
        sheets.push(id);
    }
    for (sheet, &id) in metadata.iter().zip(&sheets) {
        workbook.set_visibility(id, map_visibility(sheet.visible))?;
    }
    if let Some(id) = metadata
        .iter()
        .zip(&sheets)
        .find(|(sheet, _)| sheet.visible == SheetVisible::Visible)
        .map(|(_, id)| *id)
    {
        workbook.set_active_sheet(id)?;
    }

    let undo_enabled = workbook.undo_log().is_enabled();
    workbook.undo_log_mut().set_enabled(false);
    let result = (|| {
        for (sheet, &id) in metadata.iter().zip(&sheets) {
            if sheet.typ == SheetType::WorkSheet {
                load_sheet(&mut source, &mut workbook, &sheet.name, id)?;
            }
        }
        load_defined_names(&source, &mut workbook);
        Ok(())
    })();
    workbook.undo_log_mut().set_enabled(undo_enabled);
    result?;
    Ok(workbook)
}

fn load_sheet<RS: Read + Seek>(
    source: &mut Xls<RS>,
    workbook: &mut Workbook,
    name: &str,
    sheet: omacell_core::addr::SheetId,
) -> Result<(), CoreError> {
    let values = source
        .worksheet_range(name)
        .map_err(|err| error::xls_bridge(format!("sheet {name:?}: {err}")))?;
    let formulas = source
        .worksheet_formula(name)
        .map_err(|err| error::xls_bridge(format!("sheet {name:?} formulas: {err}")))?;
    let mut formulas_by_cell = BTreeMap::new();
    if let Some((start_row, start_col)) = formulas.start() {
        for (relative_row, relative_col, formula) in formulas.used_cells() {
            if !formula.trim().is_empty() {
                formulas_by_cell.insert(
                    (
                        start_row + relative_row as u32,
                        start_col + relative_col as u32,
                    ),
                    formula.to_string(),
                );
            }
        }
    }

    if let Some((start_row, start_col)) = values.start() {
        for (relative_row, relative_col, value) in values.used_cells() {
            let row = start_row + relative_row as u32;
            let col = start_col + relative_col as u32;
            validate_cell(row, col)?;
            let formula = formulas_by_cell.remove(&(row, col));
            write_cell(workbook, sheet, row, col as u16, value, formula.as_deref())?;
        }
    }
    for ((row, col), formula) in formulas_by_cell {
        validate_cell(row, col)?;
        write_cell(
            workbook,
            sheet,
            row,
            col as u16,
            &Data::Empty,
            Some(&formula),
        )?;
    }

    let merges = source
        .merge_cells_by_sheet_name(name)
        .map_err(|err| error::xls_bridge(format!("sheet {name:?} merges: {err}")))?
        .into_iter()
        .map(|dimensions| {
            validate_cell(dimensions.start.0, dimensions.start.1)?;
            validate_cell(dimensions.end.0, dimensions.end.1)?;
            Ok(RangeRef::from_corners(
                CellRef::new(dimensions.start.0, dimensions.start.1 as u16)?.on_sheet(sheet),
                CellRef::new(dimensions.end.0, dimensions.end.1 as u16)?.on_sheet(sheet),
            ))
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    workbook.set_sheet_merges(sheet, merges)?;
    Ok(())
}

fn write_cell(
    workbook: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: &Data,
    formula: Option<&str>,
) -> Result<(), CoreError> {
    let formula = formula.and_then(normalize_formula);
    if let Some(formula) = formula {
        workbook.set_formula_text(sheet, row, col, &formula)?;
        set_cached_value(workbook, sheet, row, col, value)?;
    } else {
        set_literal_value(workbook, sheet, row, col, value)?;
    }
    apply_date_style(workbook, sheet, row, col, value)
}

fn set_literal_value(
    workbook: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: &Data,
) -> Result<(), CoreError> {
    if matches!(value, Data::Empty) {
        return Ok(());
    }
    let (value, held_text) = core_value(workbook, value)?;
    workbook.set_slot(
        sheet,
        row,
        col,
        CellSlot {
            value,
            formula: None,
            style: omacell_core::style::StyleId::DEFAULT,
            flags: CellFlags::DEFAULT,
        },
    )?;
    if let Some(text) = held_text {
        workbook.release_text(text);
    }
    Ok(())
}

fn set_cached_value(
    workbook: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: &Data,
) -> Result<(), CoreError> {
    let mut slot = workbook
        .get(sheet, row, col)?
        .copied()
        .unwrap_or_else(CellSlot::empty);
    let (value, held_text) = core_value(workbook, value)?;
    slot.value = value;
    workbook.set_slot(sheet, row, col, slot)?;
    if let Some(text) = held_text {
        workbook.release_text(text);
    }
    Ok(())
}

fn core_value(
    workbook: &mut Workbook,
    value: &Data,
) -> Result<(Value, Option<omacell_core::value::StrId>), CoreError> {
    let result = match value {
        Data::Empty => (Value::Empty, None),
        Data::Int(value) => (Value::Number(*value as f64), None),
        Data::Float(value) => (finite_number(*value)?, None),
        Data::Bool(value) => (Value::Bool(*value), None),
        Data::String(value) | Data::DateTimeIso(value) | Data::DurationIso(value) => {
            let id = workbook.intern_text(value);
            (Value::Text(id), Some(id))
        }
        Data::DateTime(value) => (finite_number(value.as_f64())?, None),
        Data::Error(error) => (Value::Error(map_error(error)), None),
    };
    Ok(result)
}

fn finite_number(value: f64) -> Result<Value, CoreError> {
    if value.is_finite() {
        Ok(Value::Number(value))
    } else {
        Err(error::xls_bridge("cell contains a non-finite number"))
    }
}

fn apply_date_style(
    workbook: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: &Data,
) -> Result<(), CoreError> {
    let Data::DateTime(value) = value else {
        return Ok(());
    };
    let num_fmt = if value.is_duration() {
        NumFmtId::new(46)
    } else if value.as_f64().abs() < 1.0 {
        NumFmtId::new(21)
    } else if value.as_f64().fract().abs() > f64::EPSILON {
        NumFmtId::new(22)
    } else {
        NumFmtId::new(14)
    };
    let style = Style {
        num_fmt,
        ..Style::default()
    };
    workbook.set_cell_style(sheet, row, col, style)?;
    Ok(())
}

fn normalize_formula(formula: &str) -> Option<String> {
    let formula = formula.trim();
    if formula.is_empty() || formula.starts_with("Unrecognised formula") {
        return None;
    }
    let formula = if formula.starts_with('=') {
        formula.to_string()
    } else {
        format!("={formula}")
    };
    omacell_core::formula::parse(&formula).ok().map(|_| formula)
}

fn load_defined_names<RS: Read + Seek>(source: &Xls<RS>, workbook: &mut Workbook) {
    for (name, formula) in source.defined_names() {
        let referent = name_referent(workbook, formula);
        let _ = workbook.define_name(DefinedName {
            name: name.clone(),
            scope: NameScope::Workbook,
            referent,
            comment: None,
        });
    }
}

fn name_referent(workbook: &mut Workbook, formula: &str) -> NameReferent {
    let formula = formula.trim().strip_prefix('=').unwrap_or(formula.trim());
    if let Ok(parsed) = parse_a1(formula)
        && let Ok(resolved) = workbook.resolve_parsed(parsed)
    {
        return match resolved {
            RefKind::Cell(cell) => NameReferent::Range(RangeRef::from_corners(cell, cell)),
            RefKind::Range(range) => NameReferent::Range(range),
        };
    }
    if let Some(value) = formula
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
    {
        return NameReferent::Constant(Value::Number(value));
    }
    NameReferent::Formula(formula.to_string())
}

fn map_error(error: &CellErrorType) -> ErrorKind {
    match error {
        CellErrorType::Div0 => ErrorKind::Div0,
        CellErrorType::NA => ErrorKind::Na,
        CellErrorType::Name => ErrorKind::Name,
        CellErrorType::Null => ErrorKind::Null,
        CellErrorType::Num => ErrorKind::Num,
        CellErrorType::Ref => ErrorKind::Ref,
        CellErrorType::Value => ErrorKind::Value,
        CellErrorType::GettingData => ErrorKind::GettingData,
    }
}

fn map_visibility(visible: SheetVisible) -> SheetVisibility {
    match visible {
        SheetVisible::Visible => SheetVisibility::Visible,
        SheetVisible::Hidden => SheetVisibility::Hidden,
        SheetVisible::VeryHidden => SheetVisibility::VeryHidden,
    }
}

fn validate_cell(row: u32, col: u32) -> Result<(), CoreError> {
    if row >= MAX_ROWS || col >= u32::from(MAX_COLS) {
        return Err(error::xlsx_limit(format!(
            "legacy .xls cell r{}c{} exceeds the worksheet grid",
            row + 1,
            col + 1
        )));
    }
    Ok(())
}

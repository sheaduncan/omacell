//! JSON array-of-objects import/export (F-9.5).

use std::collections::BTreeSet;
use std::path::Path;

use omacell_core::error::CoreError;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::workbook::Workbook;
use serde_json::{Map, Value};

use crate::error;
use crate::xlsx::peer_lock_blocks;

const MAX_JSON_BYTES: usize = 64 * 1_048_576;
const MAX_JSON_DEPTH: usize = 64;
const MAX_JSON_TABLE_CELLS: u64 = 1_000_000;

/// Open a JSON file (root array of objects).
pub fn open(path: &Path) -> Result<Workbook, CoreError> {
    open_with_pointer(path, None)
}

/// Open JSON bytes.
pub fn open_bytes(bytes: &[u8]) -> Result<Workbook, CoreError> {
    open_bytes_with_pointer(bytes, None)
}

/// Open a JSON file, selecting a nested array with a jq-style dotted path (`.items`).
pub fn open_with_pointer(path: &Path, pointer: Option<&str>) -> Result<Workbook, CoreError> {
    peer_lock_blocks(path)?;
    let len = std::fs::metadata(path)
        .map_err(|e| error::json_format(e.to_string()))?
        .len();
    if len > MAX_JSON_BYTES as u64 {
        return Err(error::xlsx_limit(format!(
            "JSON is {len} bytes; maximum is {MAX_JSON_BYTES}"
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| error::json_format(e.to_string()))?;
    open_bytes_with_pointer(&bytes, pointer)
}

/// Open JSON bytes with an optional dotted pointer.
pub fn open_bytes_with_pointer(bytes: &[u8], pointer: Option<&str>) -> Result<Workbook, CoreError> {
    if bytes.len() > MAX_JSON_BYTES {
        return Err(error::xlsx_limit(format!(
            "JSON is {} bytes; maximum is {MAX_JSON_BYTES}",
            bytes.len()
        )));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| error::json_format(e.to_string()))?;
    let selected = select(&value, pointer)?;
    let rows = objects(selected)?;
    table_to_workbook(&rows)
}

/// Export the used range as an array of objects (row 1 = keys).
pub fn export(wb: &Workbook) -> Result<Vec<u8>, CoreError> {
    let sheet = wb.active_sheet();
    let Some(used) = wb.used_range(sheet)? else {
        return serde_json::to_vec_pretty(&Vec::<Value>::new())
            .map_err(|e| error::json_format(e.to_string()));
    };
    let row_count = u64::from(used.max_row - used.min_row + 1);
    let col_count = u64::from(used.max_col - used.min_col + 1);
    let cells = row_count.saturating_mul(col_count);
    if cells > MAX_JSON_TABLE_CELLS {
        return Err(error::xlsx_limit(format!(
            "JSON export would visit {cells} cells; maximum is {MAX_JSON_TABLE_CELLS}"
        )));
    }
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();
    for col in used.min_col..=used.max_col {
        let key = cell_text(wb, sheet, used.min_row, col);
        if key.is_empty() {
            return Err(error::json_format(format!(
                "header at column {} is empty",
                usize::from(col) + 1
            )));
        }
        if !seen.insert(key.clone()) {
            return Err(error::json_format(format!(
                "duplicate JSON export header {key:?}"
            )));
        }
        keys.push(key);
    }
    let mut objects = Vec::new();
    for row in used.min_row.saturating_add(1)..=used.max_row {
        let mut map = Map::new();
        for (i, key) in keys.iter().enumerate() {
            let col = used.min_col + i as u16;
            let value = cell_json(wb, sheet, row, col)?;
            map.insert(key.clone(), value);
        }
        objects.push(Value::Object(map));
    }
    let bytes =
        serde_json::to_vec_pretty(&objects).map_err(|e| error::json_format(e.to_string()))?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(error::xlsx_limit(format!(
            "JSON export is {} bytes; maximum is {MAX_JSON_BYTES}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn select<'a>(value: &'a Value, pointer: Option<&str>) -> Result<&'a Value, CoreError> {
    let Some(pointer) = pointer.filter(|p| !p.is_empty() && *p != "." && *p != "$") else {
        return Ok(value);
    };
    let path = pointer
        .trim()
        .trim_start_matches('$')
        .trim_start_matches('.');
    let mut cur = value;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = cur
            .get(part)
            .ok_or_else(|| error::json_format(format!("path {pointer:?} not found")))?;
    }
    Ok(cur)
}

fn objects(value: &Value) -> Result<Vec<Map<String, Value>>, CoreError> {
    let Value::Array(items) = value else {
        return Err(error::json_format(
            "JSON root must be an array of objects (or --jq must select one)",
        ));
    };
    let mut rows = Vec::new();
    if items.len() >= MAX_ROWS as usize {
        return Err(error::xlsx_limit(
            "JSON array plus its header exceeds the row grid",
        ));
    }
    for item in items {
        let Value::Object(map) = item else {
            return Err(error::json_format(
                "every JSON array item must be an object",
            ));
        };
        rows.push(flatten(map)?);
    }
    Ok(rows)
}

fn flatten(map: &Map<String, Value>) -> Result<Map<String, Value>, CoreError> {
    let mut out = Map::new();
    flatten_object(map, "", 0, &mut out)?;
    Ok(out)
}

fn flatten_object(
    map: &Map<String, Value>,
    prefix: &str,
    depth: usize,
    out: &mut Map<String, Value>,
) -> Result<(), CoreError> {
    if depth >= MAX_JSON_DEPTH {
        return Err(error::xlsx_limit(format!(
            "JSON nesting exceeds {MAX_JSON_DEPTH}"
        )));
    }
    for (key, value) in map {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        flatten_value(value, &path, depth + 1, out)?;
    }
    Ok(())
}

fn flatten_value(
    value: &Value,
    path: &str,
    depth: usize,
    out: &mut Map<String, Value>,
) -> Result<(), CoreError> {
    if depth >= MAX_JSON_DEPTH {
        return Err(error::xlsx_limit(format!(
            "JSON nesting exceeds {MAX_JSON_DEPTH}"
        )));
    }
    match value {
        Value::Object(inner) => flatten_object(inner, path, depth, out),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                flatten_value(item, &format!("{path}[{index}]"), depth + 1, out)?;
            }
            Ok(())
        }
        scalar => {
            if out.insert(path.to_string(), scalar.clone()).is_some() {
                return Err(error::json_format(format!(
                    "flattened JSON key {path:?} is ambiguous"
                )));
            }
            if out.len() > usize::from(MAX_COLS) {
                return Err(error::xlsx_limit(
                    "JSON object has more columns than the grid",
                ));
            }
            Ok(())
        }
    }
}

fn table_to_workbook(rows: &[Map<String, Value>]) -> Result<Workbook, CoreError> {
    let mut keys = BTreeSet::new();
    for row in rows {
        keys.extend(row.keys().cloned());
    }
    if keys.len() > usize::from(MAX_COLS) {
        return Err(error::xlsx_limit(
            "JSON object has more columns than the grid",
        ));
    }
    let keys: Vec<String> = keys.into_iter().collect();
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    let undo = wb.undo_log().is_enabled();
    wb.undo_log_mut().set_enabled(false);
    for (c, key) in keys.iter().enumerate() {
        wb.set_text(sheet, 0, c as u16, key)?;
    }
    for (r, row) in rows.iter().enumerate() {
        let row_i = r as u32 + 1;
        for (c, key) in keys.iter().enumerate() {
            if let Some(value) = row.get(key) {
                write_json_cell(&mut wb, sheet, row_i, c as u16, value)?;
            }
        }
    }
    wb.undo_log_mut().set_enabled(undo);
    Ok(wb)
}

fn write_json_cell(
    wb: &mut Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
    value: &Value,
) -> Result<(), CoreError> {
    match value {
        Value::Null => Ok(()),
        Value::Bool(b) => {
            wb.set_cell_contents(sheet, row, col, if *b { "TRUE" } else { "FALSE" })?;
            Ok(())
        }
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                wb.set_number(sheet, row, col, f)?;
            }
            Ok(())
        }
        Value::String(s) => {
            wb.set_text(sheet, row, col, s)?;
            Ok(())
        }
        other => {
            wb.set_text(sheet, row, col, &other.to_string())?;
            Ok(())
        }
    }
}

fn cell_text(wb: &Workbook, sheet: omacell_core::addr::SheetId, row: u32, col: u16) -> String {
    match wb.get(sheet, row, col).ok().flatten() {
        Some(slot) => match slot.value {
            omacell_core::value::Value::Text(id) => {
                wb.intern().strings.get(id).unwrap_or("").to_string()
            }
            omacell_core::value::Value::Number(n) => n.to_string(),
            omacell_core::value::Value::Bool(true) => "TRUE".into(),
            omacell_core::value::Value::Bool(false) => "FALSE".into(),
            _ => String::new(),
        },
        None => String::new(),
    }
}

fn cell_json(
    wb: &Workbook,
    sheet: omacell_core::addr::SheetId,
    row: u32,
    col: u16,
) -> Result<Value, CoreError> {
    match wb.get(sheet, row, col).ok().flatten() {
        Some(slot) => match slot.value {
            omacell_core::value::Value::Number(n) => serde_json::Number::from_f64(n)
                .map(Value::Number)
                .ok_or_else(|| error::json_format("cannot export a non-finite number")),
            omacell_core::value::Value::Bool(b) => Ok(Value::Bool(b)),
            omacell_core::value::Value::Text(id) => Ok(Value::String(
                wb.intern().strings.get(id).unwrap_or("").to_string(),
            )),
            _ => Ok(Value::Null),
        },
        None => Ok(Value::Null),
    }
}

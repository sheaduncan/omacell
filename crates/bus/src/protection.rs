use omacell_core::error::CoreError;
use omacell_core::sheet::{ProtectedRange, ProtectionAllow};
use omacell_core::workbook::Workbook;
use serde_json::Value;

use crate::resolve::{ResolvedRange, resolve_range};

pub(crate) fn check_call(wb: &Workbook, id: &str, args: &Value) -> Result<(), CoreError> {
    if matches!(
        id,
        "sheet.protect"
            | "workbook.protect"
            | "sheet.protectedrange"
            | "edit.undo"
            | "edit.redo"
            | "edit.repeat"
    ) {
        return Ok(());
    }
    if is_structure_command(id) && wb.protection().enabled && wb.protection().lock_structure {
        return Err(workbook_protected(format!(
            "workbook structure protection blocks {id}"
        )));
    }
    if !wb.sheets().any(|sheet| sheet.protection.enabled) {
        return Ok(());
    }

    let mut addresses = Vec::new();
    collect_addresses(args, None, &mut addresses);
    if addresses.is_empty() {
        return check_active_sheet_action(wb, id);
    }
    for address in addresses {
        let Ok(range) = resolve_range(wb, address) else {
            continue;
        };
        check_range(wb, id, args, range)?;
    }
    Ok(())
}

fn is_structure_command(id: &str) -> bool {
    matches!(
        id,
        "sheet.add" | "sheet.remove" | "sheet.rename" | "sheet.reorder" | "sheet.visibility"
    )
}

fn collect_addresses<'a>(value: &'a Value, key: Option<&str>, out: &mut Vec<&'a str>) {
    match value {
        Value::String(text) if key.is_some_and(is_address_key) => out.push(text),
        Value::Array(items) => {
            for item in items {
                collect_addresses(item, key, out);
            }
        }
        Value::Object(map) => {
            for (name, child) in map {
                collect_addresses(child, Some(name), out);
            }
        }
        _ => {}
    }
}

fn is_address_key(key: &str) -> bool {
    matches!(
        key,
        "ref" | "cell_ref" | "range" | "ranges" | "src" | "dest" | "to" | "sources"
    )
}

fn check_active_sheet_action(wb: &Workbook, id: &str) -> Result<(), CoreError> {
    let Some(sheet) = wb.sheet(wb.active_sheet()) else {
        return Ok(());
    };
    if !sheet.protection.enabled {
        return Ok(());
    }
    let allow = &sheet.protection.allow;
    if action_allowed(id, &Value::Null, allow) {
        return Ok(());
    }
    if id.starts_with("filter.")
        || id.starts_with("table.")
        || id.starts_with("validation.")
        || id.starts_with("condfmt.")
        || id.starts_with("chart.")
    {
        return Err(sheet_protected(format!(
            "sheet protection blocks {id} on {:?}",
            sheet.name
        )));
    }
    Ok(())
}

fn check_range(
    wb: &Workbook,
    id: &str,
    args: &Value,
    range: ResolvedRange,
) -> Result<(), CoreError> {
    let Some(sheet) = wb.sheet(range.sheet) else {
        return Ok(());
    };
    if !sheet.protection.enabled || action_allowed(id, args, &sheet.protection.allow) {
        return Ok(());
    }
    for (row, col) in range.cells() {
        let locked = wb
            .get(range.sheet, row, col)?
            .map(|slot| slot.flags.locked())
            .unwrap_or(true);
        if locked && !editable_without_password(&sheet.protection.protected_ranges, row, col) {
            return Err(sheet_protected(format!(
                "sheet protection blocks {id} at row {}, column {}",
                row + 1,
                u32::from(col) + 1
            )));
        }
    }
    Ok(())
}

fn action_allowed(id: &str, args: &Value, allow: &ProtectionAllow) -> bool {
    if id == "range.sort" {
        return allow.sort;
    }
    if id.starts_with("filter.") {
        return allow.auto_filter;
    }
    if id.starts_with("format.") || id == "style.set" {
        return allow.format_cells;
    }
    if id == "edit.insert" {
        return match args.get("shift").and_then(Value::as_str) {
            Some("rows" | "down") => allow.insert_rows,
            Some("cols" | "right") => allow.insert_cols,
            _ => false,
        };
    }
    false
}

fn editable_without_password(ranges: &[ProtectedRange], row: u32, col: u16) -> bool {
    ranges.iter().any(|editable| {
        editable.password.is_none()
            && editable.ranges.iter().any(|range| {
                row >= range.start.row.min(range.end.row)
                    && row <= range.start.row.max(range.end.row)
                    && col >= range.start.col.min(range.end.col)
                    && col <= range.start.col.max(range.end.col)
            })
    })
}

pub(crate) fn sheet_protected(message: impl Into<String>) -> CoreError {
    CoreError::new("sheet.protected", message)
        .with_hint("unprotect the sheet or unlock the target cells before editing")
}

pub(crate) fn workbook_protected(message: impl Into<String>) -> CoreError {
    CoreError::new("workbook.protected", message)
        .with_hint("unprotect workbook structure before changing sheets")
}

//! Shared bus test helpers.

#![allow(dead_code)]

use omacell_bus::Bus;
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;

pub fn bus() -> Bus {
    Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).expect("register core")
}

pub fn exec_ok(bus: &mut Bus, id: &str, args: serde_json::Value) -> serde_json::Value {
    let out = bus.execute(Origin::User, id, args);
    assert!(out.ok, "{id} failed: {:?}", out.error);
    out.result.unwrap_or(serde_json::Value::Null)
}

pub fn exec_err(
    bus: &mut Bus,
    id: &str,
    args: serde_json::Value,
) -> omacell_core::error::CoreError {
    let out = bus.execute(Origin::User, id, args);
    assert!(!out.ok, "{id} unexpectedly succeeded: {:?}", out.result);
    out.error.expect("error payload")
}

pub fn cell_value(bus: &Bus, row: u32, col: u16) -> Option<Value> {
    let sheet = bus.workbook().active_sheet();
    bus.workbook()
        .get(sheet, row, col)
        .ok()
        .flatten()
        .map(|slot| slot.value)
}

pub fn cell_formula(bus: &Bus, row: u32, col: u16) -> Option<String> {
    let sheet = bus.workbook().active_sheet();
    let slot = bus.workbook().get(sheet, row, col).ok().flatten()?;
    slot.formula
        .and_then(|id| bus.workbook().intern().formulas.get(id).map(str::to_string))
}

pub fn logical_dump(bus: &Bus) -> String {
    let wb = bus.workbook();
    let mut lines = Vec::new();
    lines.push(format!("calc={:?}", wb.settings().calc_mode));
    for sheet in wb.sheets() {
        lines.push(format!(
            "sheet {} vis={:?} name={}",
            sheet.id.index(),
            sheet.visibility,
            sheet.name
        ));
        let mut cells: Vec<_> = sheet.store.iter().collect();
        cells.sort_by_key(|(r, c, _)| (*r, *c));
        for (row, col, slot) in cells {
            let input = if let Some(fid) = slot.formula {
                wb.intern().formulas.get(fid).unwrap_or("").to_string()
            } else {
                match slot.value {
                    Value::Empty => String::new(),
                    Value::Number(n) => format!("n:{n}"),
                    Value::Bool(b) => format!("b:{b}"),
                    Value::Text(id) => format!("t:{}", wb.intern().strings.get(id).unwrap_or("")),
                    Value::Error(e) => e.as_str().to_string(),
                    Value::Array(_) => "array".into(),
                }
            };
            let style = wb
                .intern()
                .styles
                .get(slot.style)
                .cloned()
                .unwrap_or_default();
            let fmt = wb.num_fmt_code(style.num_fmt).unwrap_or_default();
            lines.push(format!(
                "  {}{} {input} bold={} fmt={fmt}",
                omacell_core::addr::col_to_letters(col).unwrap_or_else(|_| "?".into()),
                row + 1,
                style.font.bold
            ));
        }
    }
    let mut names: Vec<_> = wb.names().iter().map(|n| n.name.clone()).collect();
    names.sort();
    lines.push(format!("names={names:?}"));
    lines.join("\n")
}

pub fn undo_depth(bus: &Bus) -> (bool, bool) {
    (
        bus.workbook().undo_log().can_undo(),
        bus.workbook().undo_log().can_redo(),
    )
}

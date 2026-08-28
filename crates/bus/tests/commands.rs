//! Per-command normal, invalid, boundary, no-op, inverse, and event tests.

mod common;

use omacell_core::command::Origin;
use omacell_core::event::Event;
use omacell_core::value::Value;
use serde_json::json;

fn subscribe(bus: &mut omacell_bus::Bus) -> omacell_bus::SubscriberId {
    bus.subscribe(64)
}

#[test]
fn cell_set_normal_invalid_boundary_noop_inverse_events() {
    let mut bus = common::bus();
    let sub = subscribe(&mut bus);

    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));

    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "=A1+1"}));
    assert_eq!(common::cell_formula(&bus, 0, 1).as_deref(), Some("=A1+1"));
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(2.0)));

    let err = common::exec_err(&mut bus, "cell.set", json!({"ref": "", "input": "1"}));
    assert_eq!(err.code, "addr.parse");

    let err = common::exec_err(&mut bus, "cell.set", json!({"ref": "XFE1", "input": "1"}));
    assert_eq!(err.code, "addr.ref");

    let before = common::logical_dump(&bus);
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    assert_eq!(common::logical_dump(&bus), before);

    let dump = common::logical_dump(&bus);
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "9"}));
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert_eq!(common::logical_dump(&bus), dump);

    let events = bus.drain(sub);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::CellChanged { row: 0, col: 0, .. })),
        "{events:?}"
    );
}

#[test]
fn cell_clear_contents_only() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "style.set", json!({"range": "A1", "bold": true}));
    common::exec_ok(&mut bus, "cell.clear", json!({"ref": "A1"}));
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Empty));
    let sheet = bus.workbook().active_sheet();
    let slot = bus.workbook().get(sheet, 0, 0).unwrap().unwrap();
    let style = bus.workbook().intern().styles.get(slot.style).unwrap();
    assert!(style.font.bold);
    common::exec_ok(&mut bus, "cell.clear", json!({"ref": "Z9"}));
}

#[test]
fn range_set_and_clear() {
    let mut bus = common::bus();
    common::exec_ok(
        &mut bus,
        "range.set",
        json!({"range": "A1:B2", "values": [["1", "2"], ["3", "4"]]}),
    );
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));
    assert_eq!(common::cell_value(&bus, 1, 1), Some(Value::Number(4.0)));
    let err = common::exec_err(
        &mut bus,
        "range.set",
        json!({"range": "A1", "input": "1", "values": [["1"]]}),
    );
    assert_eq!(err.code, omacell_bus::codes::COMMAND_ARGS);
    common::exec_ok(&mut bus, "range.clear", json!({"range": "A1:B2"}));
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Empty));
}

#[test]
fn range_too_large_is_rejected() {
    let mut bus = common::bus();
    let err = common::exec_err(&mut bus, "range.set", json!({"range": "A:B", "input": "1"}));
    assert_eq!(err.code, omacell_bus::codes::RANGE_SIZE);
}

#[test]
fn sheet_add_rename_visibility() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "sheet.add", json!({"name": "Data"}));
    assert!(bus.workbook().sheet_by_name("Data").is_some());
    common::exec_ok(
        &mut bus,
        "sheet.rename",
        json!({"sheet": "Data", "name": "Inputs"}),
    );
    common::exec_ok(
        &mut bus,
        "sheet.visibility",
        json!({"sheet": "Inputs", "visibility": "hidden"}),
    );
    let err = common::exec_err(
        &mut bus,
        "sheet.visibility",
        json!({"sheet": "Sheet1", "visibility": "hidden"}),
    );
    assert_eq!(err.code, "sheet.name");
    common::exec_ok(&mut bus, "sheet.add", json!({}));
    assert!(bus.workbook().sheet_by_name("Sheet2").is_some());
    let err = common::exec_err(&mut bus, "sheet.add", json!({"name": "Inputs"}));
    assert_eq!(err.code, "sheet.name");
}

#[test]
fn name_define_and_remove() {
    let mut bus = common::bus();
    common::exec_ok(
        &mut bus,
        "name.define",
        json!({"name": "TaxRate", "referent": {"type": "constant", "value": 0.2}}),
    );
    assert!(
        bus.workbook()
            .names()
            .get(omacell_core::names::NameScope::Workbook, "TaxRate")
            .is_some()
    );
    common::exec_ok(&mut bus, "name.remove", json!({"name": "TaxRate"}));
    assert!(bus.workbook().names().is_empty());
    let err = common::exec_err(&mut bus, "name.remove", json!({"name": "Missing"}));
    assert_eq!(err.code, "name.defined");
}

#[test]
fn format_and_style() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(
        &mut bus,
        "format.number",
        json!({"range": "A1", "format": "0.00"}),
    );
    common::exec_ok(&mut bus, "style.set", json!({"range": "A1", "bold": true}));
    let sheet = bus.workbook().active_sheet();
    let slot = bus.workbook().get(sheet, 0, 0).unwrap().unwrap();
    let style = bus.workbook().intern().styles.get(slot.style).unwrap();
    assert!(style.font.bold);
    let code = bus.workbook().num_fmt_code(style.num_fmt).unwrap();
    assert_eq!(code.as_ref(), "0.00");
    common::exec_ok(
        &mut bus,
        "format.number",
        json!({"range": "A1", "format": "0.00"}),
    );
    let err = common::exec_err(&mut bus, "style.set", json!({"range": "A1"}));
    assert_eq!(err.code, omacell_bus::codes::COMMAND_ARGS);
}

#[test]
fn calc_mode_and_recalc() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "calc.mode", json!({"mode": "manual"}));
    assert_eq!(
        bus.workbook().settings().calc_mode,
        omacell_core::workbook::CalcMode::Manual
    );
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "=A1+1"}));
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Empty));
    common::exec_ok(&mut bus, "calc.recalc", json!({}));
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(2.0)));
    common::exec_ok(&mut bus, "calc.mode", json!({"mode": "automatic"}));
}

#[test]
fn undo_redo_are_one_unit() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "2"}));
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));
    common::exec_ok(&mut bus, "edit.redo", json!({}));
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(2.0)));
    let err = common::exec_err(&mut bus, "edit.redo", json!({}));
    assert_eq!(err.code, "undo.empty");
}

#[test]
fn internal_restore_not_direct() {
    let mut bus = common::bus();
    let out = bus.execute(
        Origin::User,
        "cell.restore",
        json!({"ref": "A1", "absent": true}),
    );
    assert!(!out.ok);
    assert_eq!(
        out.error.unwrap().code,
        omacell_bus::codes::COMMAND_INTERNAL
    );
}

#[test]
fn inverse_via_undo_restores_cell() {
    let mut bus = common::bus();
    let start = common::logical_dump(&bus);
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "hello"}));
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert_eq!(common::logical_dump(&bus), start);
}

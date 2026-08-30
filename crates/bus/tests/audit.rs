//! WP-19 audit and find commands.

mod common;

use serde_json::json;

use omacell_core::value::Value;

fn audit_bus() -> omacell_bus::Bus {
    let mut bus = common::bus();
    omacell_bus::register_audit_commands(bus.registry_mut()).unwrap();
    bus
}

#[test]
fn audit_run_returns_schema_one() {
    let mut bus = audit_bus();
    let result = common::exec_ok(&mut bus, "audit.run", json!({}));
    assert_eq!(result["schema"], 1);
    assert!(result["findings"].is_array());
}

#[test]
fn edit_find_counts_matches() {
    let mut bus = audit_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "hello"}));
    let result = common::exec_ok(&mut bus, "edit.findall", json!({"query": "hello"}));
    assert_eq!(result["count"], 1);
    assert_eq!(result["cells"], json!(["Sheet1!A1"]));
}

#[test]
fn workbook_find_results_are_sheet_qualified() {
    let mut bus = audit_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "hit"}));
    common::exec_ok(&mut bus, "sheet.add", json!({"name": "Second Sheet"}));
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "'Second Sheet'!A1", "input": "hit"}),
    );
    let result = common::exec_ok(
        &mut bus,
        "edit.findall",
        json!({"query": "hit", "workbook": true}),
    );
    assert_eq!(result["cells"], json!(["Sheet1!A1", "'Second Sheet'!A1"]));
}

#[test]
fn replace_preview_is_exact_and_replace_recalculates_and_undoes() {
    let mut bus = audit_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "=A1+1"}));
    let preview = common::exec_ok(
        &mut bus,
        "edit.replacepreview",
        json!({"query": "1", "whole": true, "replacement": "5"}),
    );
    assert_eq!(preview, json!({"count": 1, "preview": true}));
    let replaced = common::exec_ok(
        &mut bus,
        "edit.replaceall",
        json!({"query": "1", "whole": true, "replacement": "5"}),
    );
    assert_eq!(replaced["count"], 1);
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(6.0)));
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(2.0)));
}

#[test]
fn replace_requires_an_explicit_replacement_and_rejects_legacy_apply() {
    let mut bus = audit_bus();
    assert_eq!(
        common::exec_err(&mut bus, "edit.replaceall", json!({"query": "x"})).code,
        "command.args"
    );
    assert_eq!(
        common::exec_err(
            &mut bus,
            "edit.replaceall",
            json!({"query": "x", "replacement": "y", "apply": false}),
        )
        .code,
        "command.args"
    );
}

#[test]
fn goto_special_returns_typed_formula_results_and_dependencies() {
    let mut bus = audit_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "2"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "=A1+1"}));
    let formulas = common::exec_ok(
        &mut bus,
        "nav.gotospecial",
        json!({"kind": "formula_numbers"}),
    );
    assert_eq!(formulas["cells"], json!(["Sheet1!B1"]));

    let precedents = common::exec_ok(
        &mut bus,
        "nav.gotospecial",
        json!({"kind": "precedents", "ref": "B1"}),
    );
    assert_eq!(precedents["cells"], json!(["Sheet1!A1"]));
}

#[test]
fn cell_only_commands_reject_trace_options() {
    let mut bus = audit_bus();
    let error = common::exec_err(
        &mut bus,
        "formula.explain",
        json!({"ref": "A1", "transitive": true}),
    );
    assert_eq!(error.code, "command.args");
}

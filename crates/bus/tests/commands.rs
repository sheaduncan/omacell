//! Per-command normal, invalid, boundary, no-op, inverse, and event tests.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use omacell_bus::args::EmptyArgs;
use omacell_bus::{CommandKind, CommandSpec, Effect, Exposure};
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use omacell_core::event::Event;
use omacell_core::names::{DefinedName, NameReferent, NameScope};
use omacell_core::value::Value;
use serde_json::json;

fn subscribe(bus: &mut omacell_bus::Bus) -> omacell_bus::SubscriberId {
    bus.subscribe(64)
}

#[test]
fn external_effects_run_once_after_preflight_and_never_on_dry_run() {
    let mut bus = common::bus();
    let writes = Arc::new(AtomicUsize::new(0));
    let handler_writes = Arc::clone(&writes);
    bus.registry_mut()
        .register::<EmptyArgs, _>(
            CommandSpec {
                id: "test.external",
                doc: "Test-only external effect",
                kind: CommandKind::Mutating,
                changeset_eligible: false,
                exposure: Exposure::Public,
                default_keys: &[],
            },
            move |ctx, _args| {
                if !ctx.is_preflight() {
                    handler_writes.fetch_add(1, Ordering::SeqCst);
                }
                Ok(Effect::query(json!({})))
            },
        )
        .unwrap();

    assert!(bus.execute(Origin::User, "test.external", json!({})).ok);
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(
        bus.dry_run(Origin::User, "test.external", json!({}))
            .unwrap()
            .outcome
            .ok
    );
    assert_eq!(writes.load(Ordering::SeqCst), 1);
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
        "cell.set",
        json!({"ref": "Data!A1", "input": "2"}),
    );
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "Sheet1!A1", "input": "=Data!A1+1"}),
    );
    common::exec_ok(
        &mut bus,
        "sheet.rename",
        json!({"sheet": "Data", "name": "Inputs"}),
    );
    assert_eq!(
        common::cell_formula(&bus, 0, 0).as_deref(),
        Some("=Inputs!A1+1")
    );
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(3.0)));

    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert!(bus.workbook().sheet_by_name("Data").is_some());
    assert_eq!(
        common::cell_formula(&bus, 0, 0).as_deref(),
        Some("=Data!A1+1")
    );
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(3.0)));

    common::exec_ok(&mut bus, "edit.redo", json!({}));
    assert!(bus.workbook().sheet_by_name("Inputs").is_some());
    assert_eq!(
        common::cell_formula(&bus, 0, 0).as_deref(),
        Some("=Inputs!A1+1")
    );
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(3.0)));

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
fn name_remove_changeset_restores_exact_imported_range() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "sheet.add", json!({"name": "Data"}));
    let sheet = bus.workbook().sheet_by_name("Data").unwrap().id;
    let expected = DefinedName {
        name: "ImportedRange".into(),
        scope: NameScope::Sheet(sheet),
        referent: NameReferent::Range(RangeRef::from_corners(
            CellRef::with_abs(2, 1, true, false)
                .unwrap()
                .on_sheet(sheet),
            CellRef::with_abs(4, 3, false, true)
                .unwrap()
                .on_sheet(sheet),
        )),
        comment: Some("preserve exact imported definition".into()),
    };
    bus.workbook_mut().define_name(expected.clone()).unwrap();
    let changeset = bus
        .propose(
            Origin::ExternalAgent,
            vec![CommandCall {
                id: CommandId::new("name.remove").unwrap(),
                args: json!({"name": expected.name, "sheet": "Data"}),
            }],
        )
        .unwrap();

    bus.apply(Origin::User, &changeset.id).unwrap();
    assert!(
        bus.workbook()
            .names()
            .get(expected.scope, &expected.name)
            .is_none()
    );
    bus.revert(Origin::User, &changeset.id).unwrap();
    assert_eq!(
        bus.workbook().names().get(expected.scope, &expected.name),
        Some(&expected)
    );
}

#[test]
fn name_remove_changeset_restores_formula_and_logical_text_constant() {
    let mut bus = common::bus();
    let text = bus.workbook_mut().intern_text("exact text constant");
    let expected = [
        DefinedName {
            name: "ImportedFormula".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Formula("=SUM(Sheet1!$A$1:$A$3)".into()),
            comment: Some("formula comment".into()),
        },
        DefinedName {
            name: "ImportedText".into(),
            scope: NameScope::Workbook,
            referent: NameReferent::Constant(Value::Text(text)),
            comment: Some("constant comment".into()),
        },
    ];
    for definition in &expected {
        bus.workbook_mut().define_name(definition.clone()).unwrap();
    }
    let changeset = bus
        .propose(
            Origin::ExternalAgent,
            expected
                .iter()
                .map(|definition| CommandCall {
                    id: CommandId::new("name.remove").unwrap(),
                    args: json!({"name": definition.name}),
                })
                .collect(),
        )
        .unwrap();

    bus.apply(Origin::User, &changeset.id).unwrap();
    bus.revert(Origin::User, &changeset.id).unwrap();
    for definition in &expected {
        assert_eq!(
            bus.workbook()
                .names()
                .get(definition.scope, &definition.name),
            Some(definition)
        );
    }
}

#[test]
fn name_createfrom_uses_edge_labels_and_undoes_atomically() {
    let mut bus = common::bus();
    for (cell, input) in [
        ("A1", "Region"),
        ("B1", "Net Sales"),
        ("C1", "Margin"),
        ("A2", "North"),
        ("B2", "10"),
        ("C2", "0.25"),
        ("A3", "South"),
        ("B3", "20"),
        ("C3", "0.5"),
    ] {
        common::exec_ok(&mut bus, "cell.set", json!({"ref": cell, "input": input}));
    }

    let result = common::exec_ok(
        &mut bus,
        "name.createfrom",
        json!({"range": "A1:C3", "positions": ["top", "left"]}),
    );
    assert_eq!(result, json!({"created": 4}));

    use omacell_core::names::{NameReferent, NameScope};
    let names = bus.workbook().names();
    let net_sales = names.get(NameScope::Workbook, "Net_Sales").unwrap();
    let north = names.get(NameScope::Workbook, "North").unwrap();
    assert!(matches!(
        net_sales.referent,
        NameReferent::Range(range)
            if (range.start.row, range.start.col, range.end.row, range.end.col) == (1, 1, 2, 1)
                && range.start.row_abs
                && range.start.col_abs
                && range.end.row_abs
                && range.end.col_abs
    ));
    assert!(matches!(
        north.referent,
        NameReferent::Range(range)
            if (range.start.row, range.start.col, range.end.row, range.end.col) == (1, 1, 1, 2)
    ));

    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert!(bus.workbook().names().is_empty());
}

#[test]
fn name_createfrom_rejects_collisions_without_partial_mutation() {
    let mut bus = common::bus();
    for (cell, input) in [
        ("A1", "Duplicate"),
        ("B1", "Duplicate"),
        ("A2", "1"),
        ("B2", "2"),
    ] {
        common::exec_ok(&mut bus, "cell.set", json!({"ref": cell, "input": input}));
    }

    let error = common::exec_err(
        &mut bus,
        "name.createfrom",
        json!({"range": "A1:B2", "positions": ["top"]}),
    );
    assert_eq!(error.code, "name.defined");
    assert!(bus.workbook().names().is_empty());

    let error = common::exec_err(
        &mut bus,
        "name.createfrom",
        json!({"range": "A1:B2", "positions": []}),
    );
    assert_eq!(error.code, omacell_bus::codes::COMMAND_ARGS);
    assert!(bus.workbook().names().is_empty());
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

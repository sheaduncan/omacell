//! WP-18 data-tool commands.

mod common;

use omacell_core::command::Origin;
use omacell_core::value::Value;
use serde_json::json;

fn data_bus() -> omacell_bus::Bus {
    let mut bus = common::bus();
    omacell_bus::register_edit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_data_commands(bus.registry_mut()).unwrap();
    bus
}

#[test]
fn range_sort_orders_numbers() {
    let mut bus = data_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "3"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A3", "input": "2"}));
    common::exec_ok(
        &mut bus,
        "range.sort",
        json!({"range": "A1:A3", "keys": [{"offset": 0}]}),
    );
    assert!(matches!(
        common::cell_value(&bus, 0, 0),
        Some(Value::Number(n)) if n == 1.0
    ));
}

#[test]
fn range_sort_detects_a_text_header_over_numeric_data() {
    let mut bus = data_bus();
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "A1", "input": "Amount"}),
    );
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "2"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A3", "input": "1"}));
    common::exec_ok(&mut bus, "range.sort", json!({"range": "A1:A3"}));
    assert_eq!(common::cell_formula(&bus, 0, 0), None);
    assert!(matches!(
        common::cell_value(&bus, 0, 0),
        Some(Value::Text(_))
    ));
    assert!(matches!(
        common::cell_value(&bus, 1, 0),
        Some(Value::Number(1.0))
    ));
}

#[test]
fn table_create_resize_convert() {
    let mut bus = data_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "Item"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "a"}));
    let result = common::exec_ok(
        &mut bus,
        "table.create",
        json!({"range": "A1:A2", "name": "Sales"}),
    );
    let id = result.get("id").and_then(|v| v.as_u64()).unwrap();
    common::exec_ok(
        &mut bus,
        "table.resize",
        json!({"id": id, "range": "A1:A3"}),
    );
    assert_eq!(
        bus.workbook()
            .tables()
            .get_by_name("Sales")
            .unwrap()
            .end_row,
        2
    );
    common::exec_ok(&mut bus, "table.convert", json!({"id": id}));
    assert!(bus.workbook().tables().get_by_name("Sales").is_none());
}

#[test]
fn filter_toggle_and_clear() {
    let mut bus = data_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "n"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A3", "input": "Apple"}));
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "A4", "input": "banana"}),
    );
    let values = common::exec_ok(
        &mut bus,
        "filter.values",
        json!({"range": "A1:A4", "col_id": 0, "search": "AP"}),
    );
    assert_eq!(values, json!({"values": ["Apple"]}));
    let on = common::exec_ok(&mut bus, "filter.toggle", json!({"range": "A1:A2"}));
    assert_eq!(on.get("on"), Some(&json!(true)));
    common::exec_ok(&mut bus, "filter.clear", json!({}));
    assert!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .autofilter
            .is_none()
    );
}

#[test]
fn filter_criteria_arg_rejects_unknown_fields() {
    let err = serde_json::from_value::<omacell_bus::data::FilterCriteriaArg>(json!({
        "type": "values",
        "values": ["Apple"],
        "extra": true
    }));
    assert!(
        err.is_err(),
        "unknown filter criteria fields must fail closed"
    );
}

#[test]
fn flash_fill_command() {
    let mut bus = data_bus();
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "A1", "input": "Ada Lovelace"}),
    );
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "A2", "input": "Grace Hopper"}),
    );
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "Ada"}));
    common::exec_ok(&mut bus, "edit.flashfill", json!({"range": "B1:B2"}));
    match common::cell_value(&bus, 1, 1) {
        Some(Value::Text(id)) => {
            assert_eq!(bus.workbook().intern().strings.get(id), Some("Grace"));
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn sort_changeset_reverts() {
    let mut bus = data_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "b"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "a"}));
    let cs = bus
        .propose(
            Origin::User,
            vec![omacell_core::changeset::CommandCall {
                id: omacell_core::command::CommandId::new("range.sort").unwrap(),
                args: json!({"range": "A1:A2"}),
            }],
        )
        .unwrap();
    bus.apply(Origin::User, &cs.id).unwrap();
    assert!(matches!(
        common::cell_value(&bus, 0, 0),
        Some(Value::Text(_))
    ));
    bus.revert(Origin::User, &cs.id).unwrap();
    match common::cell_value(&bus, 0, 0) {
        Some(Value::Text(id)) => {
            assert_eq!(bus.workbook().intern().strings.get(id), Some("b"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn table_mutations_have_exact_changeset_inverses() {
    let mut bus = data_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "Item"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "a"}));
    let created = common::exec_ok(
        &mut bus,
        "table.create",
        json!({"range": "A1:A2", "name": "Sales"}),
    );
    let id = created.get("id").and_then(|v| v.as_u64()).unwrap();

    for (command, args) in [
        ("table.resize", json!({"id": id, "range": "A1:A3"})),
        ("table.convert", json!({"id": id})),
        ("table.rename", json!({"id": id, "new_name": "Orders"})),
        (
            "table.totals",
            json!({"id": id, "show": true, "functions": ["sum"]}),
        ),
    ] {
        let before = common::logical_dump(&bus);
        let cs = bus
            .propose(
                Origin::User,
                vec![omacell_core::changeset::CommandCall {
                    id: omacell_core::command::CommandId::new(command).unwrap(),
                    args,
                }],
            )
            .unwrap();
        bus.apply(Origin::User, &cs.id).unwrap();
        bus.revert(Origin::User, &cs.id).unwrap();
        assert_eq!(common::logical_dump(&bus), before, "{command}");
    }
}

#[test]
fn table_rename_updates_formula_and_totals_command_sets_function() {
    let mut bus = data_bus();
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "A1", "input": "Amount"}),
    );
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "2"}));
    let created = common::exec_ok(
        &mut bus,
        "table.create",
        json!({"range": "A1:A2", "name": "Sales"}),
    );
    let id = created.get("id").and_then(|value| value.as_u64()).unwrap();
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "C1", "input": "=SUM(Sales[Amount])"}),
    );
    common::exec_ok(
        &mut bus,
        "table.rename",
        json!({"id": id, "new_name": "Orders"}),
    );
    assert_eq!(
        common::cell_formula(&bus, 0, 2).as_deref(),
        Some("=SUM(Orders[Amount])")
    );

    common::exec_ok(
        &mut bus,
        "table.totals",
        json!({"id": id, "functions": ["sum"]}),
    );
    let table = bus.workbook().tables().get_by_name("Orders").unwrap();
    assert!(table.has_totals);
    assert_eq!(table.columns[0].totals_fn.as_deref(), Some("sum"));
}

#[test]
fn data_commands_reject_unknown_operators_and_build_visual_rules() {
    let mut bus = data_bus();
    let error = common::exec_err(
        &mut bus,
        "validation.set",
        json!({"range": "A1", "kind": "whole", "op": "typo", "formula1": "1"}),
    );
    assert_eq!(error.code, "validation.operator");
    let error = common::exec_err(
        &mut bus,
        "condfmt.add",
        json!({"range": "A1", "kind": "cell_is", "op": "typo", "formula1": "1"}),
    );
    assert_eq!(error.code, "condfmt.operator");

    common::exec_ok(
        &mut bus,
        "condfmt.add",
        json!({
            "range": "A1:A3",
            "kind": "color_scale",
            "colors": [4294901760_u64, 4278255360_u64]
        }),
    );
    let sheet = bus.workbook().sheet(bus.workbook().active_sheet()).unwrap();
    assert!(matches!(
        &sheet.cond_formats[0].kind,
        omacell_core::condfmt::CfKind::ColorScale { colors } if colors.len() == 2
    ));

    common::exec_ok(
        &mut bus,
        "condfmt.add",
        json!({"range": "B1:B3", "kind": "data_bar"}),
    );
    let sheet = bus.workbook().sheet(bus.workbook().active_sheet()).unwrap();
    assert!(sheet.cond_formats.iter().any(|rule| matches!(
        rule.kind,
        omacell_core::condfmt::CfKind::DataBar {
            color: omacell_core::style::Color::Theme {
                theme: 4,
                tint: 0.0
            },
            ..
        }
    )));
}

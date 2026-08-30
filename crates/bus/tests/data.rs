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

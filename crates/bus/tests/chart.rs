//! Chart command schema, undo, and changeset lifecycle regressions.

mod common;

use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use serde_json::json;

fn chart_call() -> CommandCall {
    CommandCall {
        id: CommandId::new("chart.fromselection").unwrap(),
        args: json!({"range": "A1:B2", "kind": "scatter"}),
    }
}

fn chart_bus() -> omacell_bus::Bus {
    let mut bus = common::bus();
    omacell_bus::register_chart_commands(bus.registry_mut()).unwrap();
    bus
}

#[test]
fn chart_changeset_apply_and_revert_are_atomic() {
    let mut bus = chart_bus();
    let changeset = bus.propose(Origin::User, vec![chart_call()]).unwrap();
    assert!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .charts
            .is_empty()
    );
    bus.apply(Origin::User, &changeset.id).unwrap();
    assert_eq!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .charts
            .len(),
        1
    );
    bus.revert(Origin::User, &changeset.id).unwrap();
    assert!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .charts
            .is_empty()
    );
}

#[test]
fn chart_and_sparkline_participate_in_direct_undo_redo() {
    let mut bus = chart_bus();
    common::exec_ok(
        &mut bus,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "line"}),
    );
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .charts
            .is_empty()
    );
    common::exec_ok(&mut bus, "edit.redo", json!({}));
    assert_eq!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .charts
            .len(),
        1
    );

    let changeset = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("sparkline.set").unwrap(),
                args: json!({"range": "A1:A2", "ref": "C1", "kind": "win_loss"}),
            }],
        )
        .unwrap();
    bus.apply(Origin::User, &changeset.id).unwrap();
    assert_eq!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .sparklines
            .len(),
        1
    );
    bus.revert(Origin::User, &changeset.id).unwrap();
    assert!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .sparklines
            .is_empty()
    );
}

#[test]
fn command_schema_rejects_unknown_kinds() {
    let mut bus = chart_bus();
    let outcome = bus.execute(
        Origin::User,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "radar"}),
    );
    assert!(!outcome.ok);
    assert_eq!(
        outcome.error.as_ref().map(|error| error.code.as_str()),
        Some(omacell_bus::codes::COMMAND_ARGS)
    );
}

#[test]
fn release_chart_edits_preserve_identity_and_are_undoable() {
    let mut bus = chart_bus();
    let created = common::exec_ok(
        &mut bus,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "combo"}),
    );
    let id = created["id"].as_u64().unwrap();

    common::exec_ok(&mut bus, "chart.move", json!({"id": id, "to": "C3"}));
    common::exec_ok(
        &mut bus,
        "chart.resize",
        json!({"id": id, "range": "C3:H12"}),
    );
    common::exec_ok(
        &mut bus,
        "chart.title",
        json!({"id": id, "title": "Quarterly sales"}),
    );
    common::exec_ok(
        &mut bus,
        "chart.axistitle",
        json!({"id": id, "axis": "category", "title": "Quarter"}),
    );
    common::exec_ok(
        &mut bus,
        "chart.axistitle",
        json!({"id": id, "axis": "value", "title": "Revenue"}),
    );
    common::exec_ok(
        &mut bus,
        "chart.axistitle",
        json!({"id": id, "axis": "secondary", "title": "Margin"}),
    );

    let sheet = bus.workbook().active_sheet();
    let chart = &bus.workbook().sheet(sheet).unwrap().charts[0];
    assert_eq!(chart.id.index(), id as u32);
    assert_eq!(chart.anchor.from_row, 2);
    assert_eq!(chart.anchor.from_col, 2);
    assert_eq!(chart.anchor.to_row, 11);
    assert_eq!(chart.anchor.to_col, 7);
    assert_eq!(chart.title.as_deref(), Some("Quarterly sales"));
    assert_eq!(chart.category_axis.title.as_deref(), Some("Quarter"));
    assert_eq!(chart.value_axis.title.as_deref(), Some("Revenue"));
    assert_eq!(
        chart
            .secondary_axis
            .as_ref()
            .and_then(|axis| axis.title.as_deref()),
        Some("Margin")
    );

    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert_eq!(
        bus.workbook().sheet(sheet).unwrap().charts[0]
            .secondary_axis
            .as_ref()
            .and_then(|axis| axis.title.as_deref()),
        None
    );
    common::exec_ok(&mut bus, "edit.redo", json!({}));
    assert_eq!(
        bus.workbook().sheet(sheet).unwrap().charts[0]
            .secondary_axis
            .as_ref()
            .and_then(|axis| axis.title.as_deref()),
        Some("Margin")
    );
}

#[test]
fn chart_edits_default_to_first_active_chart_and_revert_as_a_changeset() {
    let mut bus = chart_bus();
    common::exec_ok(
        &mut bus,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "line", "title": "Before"}),
    );

    let changeset = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("chart.title").unwrap(),
                args: json!({"title": "After"}),
            }],
        )
        .unwrap();
    let sheet = bus.workbook().active_sheet();
    assert_eq!(
        bus.workbook().sheet(sheet).unwrap().charts[0]
            .title
            .as_deref(),
        Some("Before")
    );
    bus.apply(Origin::User, &changeset.id).unwrap();
    assert_eq!(
        bus.workbook().sheet(sheet).unwrap().charts[0]
            .title
            .as_deref(),
        Some("After")
    );
    bus.revert(Origin::User, &changeset.id).unwrap();
    assert_eq!(
        bus.workbook().sheet(sheet).unwrap().charts[0]
            .title
            .as_deref(),
        Some("Before")
    );
}

#[test]
fn chart_anchor_changeset_revert_is_independent_of_the_active_sheet() {
    let mut bus = chart_bus();
    let created = common::exec_ok(
        &mut bus,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "line"}),
    );
    let id = created["id"].as_u64().unwrap();
    let chart_sheet = bus.workbook().active_sheet();
    let before = bus.workbook().sheet(chart_sheet).unwrap().charts[0].anchor;
    let changeset = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("chart.move").unwrap(),
                args: json!({"id": id, "to": "C3"}),
            }],
        )
        .unwrap();
    bus.apply(Origin::User, &changeset.id).unwrap();
    let other = bus.workbook_mut().add_sheet("Other").unwrap();
    bus.workbook_mut().set_active_sheet(other).unwrap();

    bus.revert(Origin::User, &changeset.id).unwrap();
    assert_eq!(
        bus.workbook().sheet(chart_sheet).unwrap().charts[0].anchor,
        before
    );

    common::exec_ok(&mut bus, "chart.move", json!({"id": id, "to": "D4"}));
    let moved = bus.workbook().sheet(chart_sheet).unwrap().charts[0].anchor;
    assert_eq!((moved.from_row, moved.from_col), (3, 3));
}

#[test]
fn chart_release_edit_validation_fails_closed() {
    let mut bus = chart_bus();
    common::exec_ok(
        &mut bus,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "line"}),
    );

    let boundary = common::exec_err(&mut bus, "chart.move", json!({"to": "XFD1048576"}));
    assert_eq!(boundary.code, "chart.anchor");
    let whole_column = common::exec_err(&mut bus, "chart.resize", json!({"range": "A:A"}));
    assert_eq!(whole_column.code, "chart.anchor");
    let secondary = common::exec_err(
        &mut bus,
        "chart.axistitle",
        json!({"axis": "secondary", "title": "No axis"}),
    );
    assert_eq!(secondary.code, "chart.axis");
    let missing = common::exec_err(
        &mut bus,
        "chart.title",
        json!({"id": 999, "title": "Missing"}),
    );
    assert_eq!(missing.code, "chart.id");

    let catalog: serde_json::Value = serde_json::from_str(&bus.commands_json().unwrap()).unwrap();
    let ids = catalog["commands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|command| command["id"].as_str())
        .collect::<Vec<_>>();
    for id in [
        "chart.axistitle",
        "chart.move",
        "chart.resize",
        "chart.title",
    ] {
        assert!(ids.contains(&id), "{id}");
    }
}

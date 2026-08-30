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

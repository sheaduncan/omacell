//! Propose / apply / revert lifecycle and atomicity.

mod common;

use omacell_core::changeset::{ChangesetStatus, CommandCall};
use omacell_core::command::{CommandId, Origin};
use omacell_core::event::Event;
use serde_json::json;

fn set(cell: &str, input: &str) -> CommandCall {
    CommandCall {
        id: CommandId::new("cell.set").unwrap(),
        args: json!({"ref": cell, "input": input}),
    }
}

#[test]
fn propose_does_not_mutate_live() {
    let mut bus = common::bus();
    let start = common::logical_dump(&bus);
    let cs = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1"), set("B1", "2")])
        .unwrap();
    assert_eq!(cs.status, ChangesetStatus::Proposed);
    assert!(cs.inverse.is_empty());
    assert_eq!(common::logical_dump(&bus), start);
    assert_eq!(common::undo_depth(&bus), (false, false));
}

#[test]
fn apply_then_revert_restores_logical_state_and_is_one_undo_unit_each() {
    let mut bus = common::bus();
    let start = common::logical_dump(&bus);
    let cs = bus
        .propose(
            Origin::User,
            vec![set("A1", "1"), set("B1", "=A1+1"), set("C1", "hello")],
        )
        .unwrap();
    bus.apply(Origin::User, &cs.id).unwrap();
    assert_eq!(
        common::cell_value(&bus, 0, 0),
        Some(omacell_core::value::Value::Number(1.0))
    );
    assert!(bus.workbook().undo_log().can_undo());
    let applied = bus.get_changeset(&cs.id).unwrap();
    assert_eq!(applied.status, ChangesetStatus::Applied);
    assert!(!applied.inverse.is_empty());
    bus.revert(Origin::User, &cs.id).unwrap();
    assert_eq!(common::logical_dump(&bus), start);
    let reverted = bus.get_changeset(&cs.id).unwrap();
    assert_eq!(reverted.status, ChangesetStatus::Reverted);
}

#[test]
fn apply_is_one_undo_unit() {
    let mut bus = common::bus();
    let cs = bus
        .propose(Origin::User, vec![set("A1", "1"), set("B1", "2")])
        .unwrap();
    bus.apply(Origin::User, &cs.id).unwrap();
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert!(
        common::cell_value(&bus, 0, 0).is_none()
            || common::cell_value(&bus, 0, 0) == Some(omacell_core::value::Value::Empty)
    );
    assert!(
        common::cell_value(&bus, 0, 1).is_none()
            || common::cell_value(&bus, 0, 1) == Some(omacell_core::value::Value::Empty)
    );
}

#[test]
fn failed_batch_is_atomic_and_emits_no_change_events() {
    let mut bus = common::bus();
    let sub = bus.subscribe(32);
    let start = common::logical_dump(&bus);
    let err = bus
        .propose(
            Origin::User,
            vec![
                set("A1", "1"),
                CommandCall {
                    id: CommandId::new("cell.set").unwrap(),
                    args: json!({"ref": "not-a-ref", "input": "2"}),
                },
            ],
        )
        .unwrap_err();
    assert_eq!(err.code, "addr.parse");
    assert_eq!(common::logical_dump(&bus), start);
    let events = bus.drain(sub);
    assert!(
        !events.iter().any(|e| matches!(
            e,
            Event::CellChanged { .. }
                | Event::ChangesetProposed { .. }
                | Event::ChangesetApplied { .. }
        )),
        "{events:?}"
    );
}

#[test]
fn dry_run_leaves_all_session_state_untouched() {
    let mut bus = common::bus();
    let sub = bus.subscribe(8);
    let start = common::logical_dump(&bus);
    let undo = common::undo_depth(&bus);
    let cs_len = bus.list_changesets().len();
    let queued = bus.drain(sub); // empty
    let sub = bus.subscribe(8);
    let dry = bus
        .dry_run(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}))
        .unwrap();
    assert!(dry.outcome.ok);
    assert_eq!(common::logical_dump(&bus), start);
    assert_eq!(common::undo_depth(&bus), undo);
    assert_eq!(bus.list_changesets().len(), cs_len);
    assert!(bus.drain(sub).is_empty());
    let _ = queued;
}

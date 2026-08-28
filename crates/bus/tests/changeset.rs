//! Propose / apply / revert lifecycle and atomicity.

mod common;

use omacell_core::changeset::{ChangesetStatus, CommandCall};
use omacell_core::command::{CommandId, Origin};
use omacell_core::error::ErrorKind;
use omacell_core::eval::FnRegistry;
use omacell_core::event::Event;
use omacell_core::recalc::RecalcEngine;
use omacell_core::storage::CellSlot;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
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

#[test]
fn invalid_lifecycle_transitions_fail_before_mutating_live_state() {
    let mut bus = common::bus();
    let proposed = bus.propose(Origin::User, vec![set("A1", "1")]).unwrap();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "9"}));
    let before_revert = common::logical_dump(&bus);
    let sub = bus.subscribe(8);

    let err = bus.revert(Origin::User, &proposed.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_STATE);
    assert_eq!(common::logical_dump(&bus), before_revert);
    assert!(bus.drain(sub).is_empty());

    let mut bus = common::bus();
    let applied = bus.propose(Origin::User, vec![set("A1", "1")]).unwrap();
    bus.apply(Origin::User, &applied.id).unwrap();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "2"}));
    let before_apply = common::logical_dump(&bus);
    let sub = bus.subscribe(8);

    let err = bus.apply(Origin::User, &applied.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_STATE);
    assert_eq!(common::logical_dump(&bus), before_apply);
    assert!(bus.drain(sub).is_empty());
}

#[test]
fn changeset_inverse_restores_literal_type_and_exact_text() {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook
        .set_slot(
            sheet,
            0,
            0,
            CellSlot {
                value: Value::Error(ErrorKind::Div0),
                ..CellSlot::empty()
            },
        )
        .unwrap();
    workbook.set_text(sheet, 0, 1, "  spaced text  ").unwrap();
    let mut bus = omacell_bus::Bus::new(workbook, RecalcEngine::new(FnRegistry::new())).unwrap();

    let changeset = bus
        .propose(Origin::User, vec![set("A1", "1"), set("B1", "replacement")])
        .unwrap();
    bus.apply(Origin::User, &changeset.id).unwrap();
    bus.revert(Origin::User, &changeset.id).unwrap();

    assert_eq!(
        common::cell_value(&bus, 0, 0),
        Some(Value::Error(ErrorKind::Div0))
    );
    let slot = bus.workbook().get(sheet, 0, 1).unwrap().unwrap();
    let Value::Text(id) = slot.value else {
        panic!("expected restored text cell");
    };
    assert_eq!(
        bus.workbook().intern().strings.get(id),
        Some("  spaced text  ")
    );
}

#[test]
fn oversized_command_range_is_rejected_before_mutation() {
    let mut bus = common::bus();
    let err = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("range.set").unwrap(),
                args: json!({"range": "A1:A100001", "input": "x"}),
            }],
        )
        .unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::RANGE_SIZE);
    assert!(common::cell_value(&bus, 0, 0).is_none());
}

#[test]
fn range_changeset_summary_is_constant_size() {
    let mut bus = common::bus();
    let changeset = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("range.set").unwrap(),
                args: json!({"range": "A1:A1000", "input": "x"}),
            }],
        )
        .unwrap();
    assert_eq!(changeset.summary.text, "set Sheet1!A1:A1000");
    assert!(changeset.summary.text.len() < 64);
}

#[test]
fn apply_rechecks_retained_size_before_live_mutation() {
    let mut bus = common::bus();
    let changeset = bus.propose(Origin::User, vec![set("A1", "new")]).unwrap();
    let existing = "x".repeat(omacell_bus::MAX_CHANGESET_BYTES / 2 + 1_024);
    let outcome = bus.execute(
        Origin::User,
        "cell.set",
        json!({"ref": "A1", "input": existing}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);

    let err = bus.apply(Origin::User, &changeset.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_LIMIT);
    let slot = bus
        .workbook()
        .get(bus.workbook().active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    let Value::Text(id) = slot.value else {
        panic!("expected original large text to remain");
    };
    assert_eq!(
        bus.workbook().intern().strings.get(id).unwrap().len(),
        omacell_bus::MAX_CHANGESET_BYTES / 2 + 1_024
    );
}

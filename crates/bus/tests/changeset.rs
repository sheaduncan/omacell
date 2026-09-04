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
use proptest::prelude::*;
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
fn dry_run_undo_keeps_history_available_and_live_state_untouched() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));

    let dry = bus.dry_run(Origin::User, "edit.undo", json!({})).unwrap();

    assert!(dry.outcome.ok, "{:?}", dry.outcome.error);
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));
    assert!(bus.workbook().undo_log().can_undo());
}

#[test]
fn removed_sheet_inverse_stops_at_the_construction_budget() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "sheet.add", json!({"name": "Victim"}));
    let sheet = bus.workbook().sheet_by_name("Victim").unwrap().id;
    let value = "x".repeat(16 * 1024);
    for row in 0..80 {
        bus.workbook_mut().set_text(sheet, row, 0, &value).unwrap();
    }

    let error = bus
        .propose(
            Origin::ExternalAgent,
            vec![CommandCall {
                id: CommandId::new("sheet.remove").unwrap(),
                args: json!({"sheet": "Victim"}),
            }],
        )
        .unwrap_err();

    assert_eq!(error.code, omacell_bus::codes::CHANGESET_LIMIT);
    assert!(error.message.contains("construction budget"), "{error:?}");
    assert!(bus.list_changesets().is_empty());
    assert!(bus.workbook().sheet_by_name("Victim").is_some());
}

#[test]
fn review_can_revise_a_proposal_before_one_unit_apply() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(
            Origin::PalettePlan,
            vec![set("A1", "rejected"), set("B1", "accepted")],
        )
        .unwrap();

    let revised = bus
        .revise_proposal(Origin::User, &proposed.id, vec![set("B1", "accepted")])
        .unwrap();
    assert_eq!(revised.id, proposed.id);
    assert_eq!(revised.origin, Origin::PalettePlan);
    assert_eq!(revised.status, ChangesetStatus::Proposed);
    assert_eq!(revised.forward, vec![set("B1", "accepted")]);
    assert_eq!(revised.summary.cells, 1);
    assert!(common::cell_value(&bus, 0, 0).is_none());
    assert!(common::cell_value(&bus, 0, 1).is_none());

    bus.apply(Origin::User, &proposed.id).unwrap();
    assert!(common::cell_value(&bus, 0, 0).is_none());
    let slot = bus
        .workbook()
        .get(bus.workbook().active_sheet(), 0, 1)
        .unwrap()
        .unwrap();
    let Value::Text(text) = slot.value else {
        panic!("expected accepted text");
    };
    assert_eq!(bus.workbook().intern().strings.get(text), Some("accepted"));

    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert!(common::cell_value(&bus, 0, 1).is_none());
}

#[test]
fn invalid_revision_is_atomic_and_keeps_the_original_proposal() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::InAppAgent, vec![set("A1", "original")])
        .unwrap();
    let before = bus.get_changeset(&proposed.id).unwrap().clone();

    let err = bus
        .revise_proposal(
            Origin::User,
            &proposed.id,
            vec![CommandCall {
                id: CommandId::new("cell.set").unwrap(),
                args: json!({"ref": "not-a-ref", "input": "invalid"}),
            }],
        )
        .unwrap_err();
    assert_eq!(err.code, "addr.parse");
    assert_eq!(bus.get_changeset(&proposed.id).unwrap(), &before);
    assert!(common::cell_value(&bus, 0, 0).is_none());
}

#[test]
fn review_can_discard_a_proposal_without_mutating() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "discarded")])
        .unwrap();

    let discarded = bus.discard_proposal(Origin::User, &proposed.id).unwrap();
    assert_eq!(discarded, proposed);
    assert!(bus.list_changesets().is_empty());
    assert!(common::cell_value(&bus, 0, 0).is_none());
    let err = bus.apply(Origin::User, &proposed.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_NOT_FOUND);
}

#[test]
fn apply_rejects_a_proposal_after_an_intervening_live_mutation() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1")])
        .unwrap();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "2"}));
    let err = bus.apply(Origin::User, &proposed.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_BASE);
    assert!(common::cell_value(&bus, 0, 0).is_none());
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(2.0)));
}

#[test]
fn apply_succeeds_when_the_workbook_generation_is_unchanged() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1")])
        .unwrap();
    bus.apply(Origin::User, &proposed.id).unwrap();
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));
}

#[test]
fn applying_one_proposal_invalidates_a_sibling_proposed_at_the_same_base() {
    let mut bus = common::bus();
    let first = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1")])
        .unwrap();
    let second = bus
        .propose(Origin::ExternalAgent, vec![set("B1", "2")])
        .unwrap();
    bus.apply(Origin::User, &first.id).unwrap();
    let err = bus.apply(Origin::User, &second.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_BASE);
    assert_eq!(common::cell_value(&bus, 0, 0), Some(Value::Number(1.0)));
    assert!(common::cell_value(&bus, 0, 1).is_none());
}

#[test]
fn review_preview_reports_before_after_cells_without_mutating() {
    let mut bus = common::bus();
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "A1", "input": "before"}),
    );
    let proposed = bus
        .propose(
            Origin::PalettePlan,
            vec![set("A1", "after"), set("B1", "new")],
        )
        .unwrap();

    let preview = bus.preview_changeset(&proposed.id).unwrap();
    assert_eq!(preview.id, proposed.id);
    assert_eq!(preview.origin, Origin::PalettePlan);
    assert_eq!(preview.items.len(), 2);
    assert_eq!(preview.items[0].cells.len(), 1);
    assert_eq!(preview.items[0].cells[0].sheet, "Sheet1");
    assert_eq!(preview.items[0].cells[0].row, 0);
    assert_eq!(preview.items[0].cells[0].col, 0);
    assert_eq!(preview.items[0].cells[0].before.as_deref(), Some("before"));
    assert_eq!(preview.items[0].cells[0].after.as_deref(), Some("after"));
    assert_eq!(preview.items[1].cells[0].before, None);
    assert_eq!(preview.items[1].cells[0].after.as_deref(), Some("new"));

    let slot = bus
        .workbook()
        .get(bus.workbook().active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    let Value::Text(text) = slot.value else {
        panic!("expected original text");
    };
    assert_eq!(bus.workbook().intern().strings.get(text), Some("before"));
    assert!(common::cell_value(&bus, 0, 1).is_none());
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    })]

    #[test]
    fn model_review_and_apply_revert_inverse_hold_for_generated_batches(
        edits in prop::collection::vec((0u32..20, 0u16..10, "[a-z0-9]{0,12}"), 1..20),
    ) {
        let mut bus = common::bus();
        common::exec_ok(&mut bus, "cell.set", json!({"ref":"A1", "input":"seed"}));
        let before = common::logical_dump(&bus);
        let calls = edits
            .into_iter()
            .map(|(row, col, input)| {
                let reference = format!(
                    "{}{}",
                    omacell_core::addr::col_to_letters(col).unwrap(),
                    row + 1,
                );
                set(&reference, &input)
            })
            .collect();
        let proposed = bus.propose(Origin::PalettePlan, calls).unwrap();
        prop_assert_eq!(common::logical_dump(&bus), before.clone());
        bus.apply(Origin::User, &proposed.id).unwrap();
        bus.revert(Origin::User, &proposed.id).unwrap();
        prop_assert_eq!(common::logical_dump(&bus), before);
    }
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
fn apply_base_generation_check_precedes_retained_size_recheck() {
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
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_BASE);
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

#[test]
fn apply_rejects_a_proposal_after_direct_workbook_mutation() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1")])
        .unwrap();
    let sheet = bus.workbook().active_sheet();
    bus.workbook_mut()
        .set_text(sheet, 0, 1, "side channel")
        .unwrap();
    let err = bus.apply(Origin::User, &proposed.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_BASE);
    assert!(common::cell_value(&bus, 0, 0).is_none());
    assert!(matches!(
        common::cell_value(&bus, 0, 1),
        Some(Value::Text(_))
    ));
}

#[test]
fn apply_rejects_a_proposal_after_mutable_engine_access() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1")])
        .unwrap();
    let _ = bus.engine_mut();
    let err = bus.apply(Origin::User, &proposed.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_BASE);
    assert!(common::cell_value(&bus, 0, 0).is_none());
}

#[test]
fn apply_rejects_a_proposal_after_mutable_command_registry_access() {
    let mut bus = common::bus();
    let proposed = bus
        .propose(Origin::ExternalAgent, vec![set("A1", "1")])
        .unwrap();
    let _ = bus.registry_mut();
    let err = bus.apply(Origin::User, &proposed.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::CHANGESET_BASE);
    assert!(common::cell_value(&bus, 0, 0).is_none());
}

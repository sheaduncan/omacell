//! Origin policy matrix.

mod common;

use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use serde_json::json;

fn mutating_call() -> CommandCall {
    CommandCall {
        id: CommandId::new("cell.set").unwrap(),
        args: json!({"ref": "A1", "input": "1"}),
    }
}

#[test]
fn model_origins_cannot_directly_mutate() {
    for origin in [
        Origin::InAppAgent,
        Origin::ExternalAgent,
        Origin::PalettePlan,
    ] {
        let mut bus = common::bus();
        let out = bus.execute(origin, "cell.set", json!({"ref": "A1", "input": "1"}));
        assert!(!out.ok, "{origin:?}");
        assert_eq!(out.error.unwrap().code, omacell_bus::codes::COMMAND_DENIED);
        assert!(common::cell_value(&bus, 0, 0).is_none());
    }
}

#[test]
fn model_origins_cannot_submit_internal_restore() {
    for origin in [
        Origin::InAppAgent,
        Origin::ExternalAgent,
        Origin::PalettePlan,
    ] {
        let mut bus = common::bus();
        let forward = vec![CommandCall {
            id: CommandId::new("cell.restore").unwrap(),
            args: json!({"ref": "A1", "absent": true}),
        }];
        let err = bus.propose(origin, forward).unwrap_err();
        assert_eq!(err.code, omacell_bus::codes::COMMAND_INTERNAL);
    }
}

#[test]
fn model_origins_can_propose_mutating_commands() {
    let mut bus = common::bus();
    let cs = bus
        .propose(Origin::ExternalAgent, vec![mutating_call()])
        .unwrap();
    assert_eq!(
        cs.status,
        omacell_core::changeset::ChangesetStatus::Proposed
    );
    assert!(cs.inverse.is_empty());
    assert!(common::cell_value(&bus, 0, 0).is_none());
}

#[test]
fn model_origins_cannot_apply() {
    let mut bus = common::bus();
    let cs = bus
        .propose(Origin::InAppAgent, vec![mutating_call()])
        .unwrap();
    let err = bus.apply(Origin::InAppAgent, &cs.id).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::COMMAND_DENIED);
}

#[test]
fn user_can_execute_and_apply() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    let cs = bus.propose(Origin::User, vec![mutating_call()]).unwrap();
    bus.apply(Origin::User, &cs.id).unwrap();
}

#[test]
fn undo_is_not_changeset_eligible() {
    let mut bus = common::bus();
    let err = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("edit.undo").unwrap(),
                args: json!({}),
            }],
        )
        .unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::COMMAND_INELIGIBLE);
}

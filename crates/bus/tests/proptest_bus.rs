//! Property tests for inverse, apply/revert, and failed-batch atomicity.

mod common;

use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use proptest::prelude::*;
use serde_json::json;

fn cell_letter(col: u8) -> char {
    char::from(b'A' + col)
}

fn set_call(col: u8, input: &str) -> CommandCall {
    CommandCall {
        id: CommandId::new("cell.set").unwrap(),
        args: json!({"ref": format!("{}1", cell_letter(col)), "input": input}),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    })]

    #[test]
    fn apply_revert_restores(values in prop::collection::vec(0u32..100, 1..4)) {
        let mut bus = common::bus();
        let start = common::logical_dump(&bus);
        let forward: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(i, v)| set_call(i as u8, &v.to_string()))
            .collect();
        let cs = bus.propose(Origin::User, forward).unwrap();
        bus.apply(Origin::User, &cs.id).unwrap();
        bus.revert(Origin::User, &cs.id).unwrap();
        prop_assert_eq!(common::logical_dump(&bus), start);
    }

    #[test]
    fn execute_undo_restores(value in 0u32..10_000u32) {
        let mut bus = common::bus();
        let start = common::logical_dump(&bus);
        common::exec_ok(
            &mut bus,
            "cell.set",
            json!({"ref": "A1", "input": value.to_string()}),
        );
        common::exec_ok(&mut bus, "edit.undo", json!({}));
        prop_assert_eq!(common::logical_dump(&bus), start);
    }

    #[test]
    fn failed_batch_leaves_live_untouched(value in 0u32..50) {
        let mut bus = common::bus();
        let start = common::logical_dump(&bus);
        let err = bus
            .propose(
                Origin::User,
                vec![
                    set_call(0, &value.to_string()),
                    CommandCall {
                        id: CommandId::new("cell.set").unwrap(),
                        args: json!({"ref": "!!!", "input": "x"}),
                    },
                ],
            )
            .unwrap_err();
        prop_assert_eq!(err.code.as_str(), "addr.parse");
        prop_assert_eq!(common::logical_dump(&bus), start);
        prop_assert!(!bus.workbook().undo_log().can_undo());
    }
}

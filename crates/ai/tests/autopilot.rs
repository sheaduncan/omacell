//! Autopilot scope, operation-cap, and forbidden-tool invariants.

use omacell_ai::{AutopilotPolicy, AutopilotScope};
use omacell_core::changeset::CommandCall;
use omacell_core::command::CommandId;
use omacell_core::workbook::Workbook;
use proptest::prelude::*;
use serde_json::json;

fn call(id: &str, args: serde_json::Value) -> CommandCall {
    CommandCall {
        id: CommandId::new(id).unwrap(),
        args,
    }
}

#[test]
fn range_scope_rejects_outside_and_does_not_spend_the_cap() {
    let wb = Workbook::new();
    let sheet = wb.active_sheet();
    let mut policy = AutopilotPolicy::new(
        AutopilotScope::Range {
            sheet,
            min_row: 1,
            min_col: 1,
            max_row: 3,
            max_col: 3,
        },
        2,
    );
    let outside = call("cell.set", json!({"ref":"A1", "input":"outside"}));
    let error = policy
        .authorize_and_record(std::slice::from_ref(&outside), &wb)
        .unwrap_err();
    assert_eq!(error.code, "ai.autopilot");
    assert_eq!(policy.used_ops(), 0);

    let inside = call("cell.set", json!({"ref":"B2", "input":"inside"}));
    policy
        .authorize_and_record(std::slice::from_ref(&inside), &wb)
        .unwrap();
    assert_eq!(policy.used_ops(), 1);
    let error = policy
        .authorize_and_record(&[inside.clone(), inside], &wb)
        .unwrap_err();
    assert!(error.message.contains("operation cap"));
    assert_eq!(policy.used_ops(), 1);
}

#[test]
fn sheet_scope_requires_a_provable_target_on_that_sheet() {
    let mut wb = Workbook::new();
    let data = wb.add_sheet("Data").unwrap();
    let mut policy = AutopilotPolicy::new(AutopilotScope::Sheet(data), 5);
    policy
        .authorize_and_record(
            &[call(
                "range.set",
                json!({"range":"Data!A1:B2", "input":"ok"}),
            )],
            &wb,
        )
        .unwrap();
    assert!(
        policy
            .authorize_and_record(
                &[call(
                    "range.set",
                    json!({"range":"Sheet1!A1:B2", "input":"no"}),
                )],
                &wb,
            )
            .is_err()
    );
    assert!(
        policy
            .authorize_and_record(&[call("sheet.add", json!({"name":"Other"}))], &wb)
            .is_err()
    );
}

#[test]
fn workbook_scope_still_blocks_security_and_policy_commands() {
    let wb = Workbook::new();
    for id in [
        "trust.add",
        "script.source",
        "file.save",
        "network.enable",
        "config.set",
        "macro.replay",
        "workbook.protect",
        "chart.move",
        "chart.resize",
        "chart.title",
        "chart.axistitle",
        "future.command",
    ] {
        let mut policy = AutopilotPolicy::new(AutopilotScope::Workbook, 10);
        let error = policy
            .authorize_and_record(&[call(id, json!({}))], &wb)
            .unwrap_err();
        assert_eq!(error.code, "ai.autopilot", "{id}");
        assert_eq!(policy.used_ops(), 0, "{id}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Off)),
        ..ProptestConfig::default()
    })]

    #[test]
    fn authorized_range_targets_never_escape_scope(
        min_row in 0u32..100,
        min_col in 0u16..20,
        row_delta in 0u32..20,
        col_delta in 0u16..10,
        target_row in 0u32..140,
        target_col in 0u16..35,
    ) {
        let wb = Workbook::new();
        let sheet = wb.active_sheet();
        let max_row = min_row.saturating_add(row_delta);
        let max_col = min_col.saturating_add(col_delta);
        let mut policy = AutopilotPolicy::new(
            AutopilotScope::Range {
                sheet,
                min_row,
                min_col,
                max_row,
                max_col,
            },
            1,
        );
        let reference = format!(
            "{}{}",
            omacell_core::addr::col_to_letters(target_col).unwrap(),
            target_row + 1
        );
        let result = policy.authorize_and_record(
            &[call("cell.set", json!({"ref":reference, "input":"x"}))],
            &wb,
        );
        let inside = target_row >= min_row
            && target_row <= max_row
            && target_col >= min_col
            && target_col <= max_col;
        prop_assert_eq!(result.is_ok(), inside);
        prop_assert_eq!(policy.used_ops(), usize::from(inside));
    }
}

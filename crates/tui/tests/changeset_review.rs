//! Reviewable proposal integration in the terminal frontend.

mod common;

use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use omacell_core::value::Value;
use omacell_ui::{KeyCode, KeyEvent};
use serde_json::json;

fn set(cell: &str, input: &str) -> CommandCall {
    CommandCall {
        id: CommandId::new("cell.set").unwrap(),
        args: json!({"ref": cell, "input": input}),
    }
}

#[test]
fn review_rejects_one_item_and_applies_the_rest_as_one_changeset() {
    let mut harness = common::harness();
    harness
        .tui
        .runner()
        .propose(
            Origin::PalettePlan,
            vec![set("A1", "reject"), set("B1", "accept")],
        )
        .unwrap();
    harness
        .tui
        .execute_cmd("changeset.review", json!({}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);

    let review = harness.tui.ui().changeset_review().unwrap();
    assert_eq!(review.items.len(), 2);
    assert_eq!(
        harness.tui.ui().panel().visible.as_deref(),
        Some("changeset")
    );
    harness.tui.step_key(KeyEvent::new(KeyCode::Space)).unwrap();
    let proposals = harness.tui.runner().list_changesets().unwrap();
    assert_eq!(
        proposals.last().unwrap().forward.len(),
        2,
        "item toggles remain local until the user applies the review"
    );
    harness.tui.step_key(KeyEvent::new(KeyCode::Enter)).unwrap();

    let snapshot = harness.tui.runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert!(snapshot.workbook.get(sheet, 0, 0).unwrap().is_none());
    let slot = snapshot.workbook.get(sheet, 0, 1).unwrap().unwrap();
    let Value::Text(text) = slot.value else {
        panic!("expected accepted text");
    };
    assert_eq!(snapshot.workbook.intern().strings.get(text), Some("accept"));
    assert!(harness.tui.ui().panel().visible.is_none());
}

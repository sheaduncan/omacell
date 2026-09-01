//! CSV import preview and explicit AI-plan acceptance.

mod common;

use omacell_ui::{KeyCode, KeyEvent};
use serde_json::json;

#[test]
fn assistant_proposal_reopens_only_after_enter() {
    let mut harness = common::harness();
    harness
        .tui
        .execute_cmd("file.open", json!({"path":"readings.csv"}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert_eq!(harness.tui.ui().panel().visible.as_deref(), Some("import"));
    assert!(harness.tui.ui().import_review().unwrap().proposed.is_none());

    harness
        .tui
        .step_key(KeyEvent::new(KeyCode::Char('a')))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    let review = harness.tui.ui().import_review().unwrap();
    assert_eq!(
        review.proposed.unwrap().columns[0].name.as_deref(),
        Some("Pressure")
    );

    harness.tui.step_key(KeyEvent::new(KeyCode::Enter)).unwrap();
    common::wait_tasks(&mut harness.tui);
    assert!(harness.tui.ui().panel().visible.is_none());
    assert!(harness.tui.ui().import_review().is_none());
}

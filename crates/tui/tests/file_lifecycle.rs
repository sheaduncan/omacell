//! TUI reconciliation for the composition-owned file lifecycle.

mod common;

use omacell_ui::{KeyCode, KeyEvent};
use serde_json::json;

#[test]
fn file_lifecycle_confirms_discard_and_reconciles_state() {
    let mut harness = common::harness();
    harness
        .tui
        .execute_cmd("cell.set", json!({"ref": "A1", "input": "changed"}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert!(harness.tui.is_dirty());

    harness.tui.execute_cmd("file.new", json!({})).unwrap();
    assert!(harness.tui.message().unwrap().contains("unsaved"));
    assert!(!harness.tui.has_pending_tasks());
    harness.tui.execute_cmd("file.new", json!({})).unwrap();
    common::wait_tasks(&mut harness.tui);
    assert!(!harness.tui.is_dirty());

    harness
        .tui
        .execute_cmd("file.saveas", json!({"path": "/work/renamed.csv"}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert!(!harness.tui.is_dirty());

    harness.tui.execute_cmd("file.close", json!({})).unwrap();
    assert!(harness.tui.quit_requested());
}

#[test]
fn required_argument_key_opens_the_schema_prompt() {
    let mut harness = common::harness();

    harness.tui.step_key(KeyEvent::new(KeyCode::F(12))).unwrap();

    let palette = harness.tui.ui().palette();
    assert!(palette.open);
    assert!(palette.prompt.unwrap().contains("path"));
    assert!(!harness.tui.has_pending_tasks());
}

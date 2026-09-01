//! In-app agent panel and explicit autopilot integration.

mod common;

use omacell_core::command::Origin;
use omacell_core::value::Value;
use omacell_ui::{KeyCode, KeyEvent};
use serde_json::json;

#[test]
fn agent_turn_is_reviewed_then_returns_to_the_panel() {
    let mut harness = common::harness();
    harness
        .tui
        .execute_cmd("ai.agent.turn", json!({"prompt":"set A1", "apply":false}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert_eq!(
        harness.tui.ui().panel().visible.as_deref(),
        Some("changeset")
    );
    let review = harness.tui.ui().changeset_review().unwrap();
    assert_eq!(review.origin, Origin::InAppAgent);
    harness.tui.step_key(KeyEvent::new(KeyCode::Enter)).unwrap();
    assert_eq!(harness.tui.ui().panel().visible.as_deref(), Some("agent"));
    assert!(harness.tui.ui().agent_panel().body().contains("Proposed 1"));
}

#[test]
fn explicit_range_autopilot_applies_inside_and_stops_outside() {
    let mut harness = common::harness_sets(&[
        "ai.agent.review=autopilot_opt_in",
        "ai.agent.autopilot_scope=range",
        "ai.agent.autopilot_max_ops=2",
    ]);
    let mut panel = harness.tui.ui().panel();
    panel.open("agent");
    harness.tui.ui().set_panel(panel);
    harness.tui.step_key(KeyEvent::new(KeyCode::F(8))).unwrap();
    assert!(harness.tui.ui().agent_panel().autopilot);

    harness
        .tui
        .execute_cmd("ai.agent.turn", json!({"prompt":"set A1", "apply":false}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert_eq!(harness.tui.ui().panel().visible.as_deref(), Some("agent"));
    let snapshot = harness.tui.runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    let slot = snapshot.workbook.get(sheet, 0, 0).unwrap().unwrap();
    let Value::Text(text) = slot.value else {
        panic!("expected autopilot text");
    };
    assert_eq!(snapshot.workbook.intern().strings.get(text), Some("agent"));
    assert_eq!(harness.tui.ui().agent_panel().used_ops, 1);

    harness
        .tui
        .execute_cmd("ai.agent.turn", json!({"prompt":"set B1", "apply":false}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert_eq!(
        harness.tui.ui().panel().visible.as_deref(),
        Some("changeset")
    );
    let snapshot = harness.tui.runner().snapshot();
    assert!(snapshot.workbook.get(sheet, 0, 1).unwrap().is_none());
    assert!(
        harness
            .tui
            .ui()
            .agent_panel()
            .body()
            .contains("outside the session scope")
    );

    assert!(!harness.tui.execute_cmd("file.new", json!({})).unwrap().ok);
    assert!(harness.tui.execute_cmd("file.new", json!({})).unwrap().ok);
    common::wait_tasks(&mut harness.tui);
    assert!(!harness.tui.ui().agent_panel().autopilot);
    assert!(
        harness
            .tui
            .ui()
            .agent_panel()
            .body()
            .contains("reset for the new workbook")
    );
}

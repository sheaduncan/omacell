//! Formula-assist result review and application.

mod common;

use omacell_ui::{KeyCode, KeyEvent};
use serde_json::json;

#[test]
fn generated_formula_is_reviewed_before_one_unit_apply() {
    let mut harness = common::harness();
    harness
        .tui
        .execute_cmd(
            "ai.formula.generate",
            json!({"prompt":"sum the inputs", "ref":"E5"}),
        )
        .unwrap();
    common::wait_tasks(&mut harness.tui);

    assert_eq!(harness.tui.ui().panel().visible.as_deref(), Some("formula"));
    let assist = harness.tui.ui().formula_assist().unwrap();
    assert_eq!(assist.scratch.as_deref(), Some("Number(6)"));
    assert_eq!(assist.references.len(), 2);
    assert!(harness.tui.ui().changeset_review().is_some());

    let before = harness.tui.runner().snapshot();
    let sheet = before.workbook.active_sheet();
    assert!(before.workbook.get(sheet, 4, 4).unwrap().is_none());
    harness.tui.step_key(KeyEvent::new(KeyCode::Enter)).unwrap();

    let after = harness.tui.runner().snapshot();
    let slot = after.workbook.get(sheet, 4, 4).unwrap().unwrap();
    assert_eq!(
        after.workbook.intern().formulas.get(slot.formula.unwrap()),
        Some("=SUM(B1:C1)+D2")
    );
}

#[test]
fn explanation_has_no_changeset() {
    let mut harness = common::harness();
    harness
        .tui
        .execute_cmd("ai.formula.explain", json!({"ref":"A1"}))
        .unwrap();
    common::wait_tasks(&mut harness.tui);
    assert_eq!(harness.tui.ui().panel().visible.as_deref(), Some("formula"));
    assert!(harness.tui.ui().changeset_review().is_none());
    assert!(
        harness
            .tui
            .ui()
            .formula_assist()
            .unwrap()
            .body()
            .contains("Adds the selected inputs")
    );
    harness
        .tui
        .step_key(KeyEvent::new(KeyCode::Char('x')))
        .unwrap();
    assert_eq!(harness.tui.ui().panel().visible.as_deref(), Some("formula"));
    harness.tui.step_key(KeyEvent::new(KeyCode::Esc)).unwrap();
    assert!(harness.tui.ui().panel().visible.is_none());
}

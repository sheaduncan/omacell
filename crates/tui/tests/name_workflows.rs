//! Defined-name workflow integration in the terminal frontend.

mod common;

use omacell_ui::{KeyCode, KeyEvent};

#[test]
fn name_keys_open_schema_prompts_with_selection_context() {
    let mut harness = common::harness();

    harness.tui.step_key(KeyEvent::new(KeyCode::F(3))).unwrap();
    let palette = harness.tui.ui().palette();
    assert!(palette.open);
    assert!(palette.prompt.unwrap().contains("name"));
    assert!(palette.query.is_empty());

    harness.tui.step_key(KeyEvent::new(KeyCode::Esc)).unwrap();
    harness
        .tui
        .step_key(KeyEvent {
            code: KeyCode::F(3),
            ctrl: true,
            alt: false,
            shift: true,
        })
        .unwrap();
    let palette = harness.tui.ui().palette();
    assert!(palette.open);
    assert!(palette.prompt.unwrap().contains("positions"));
    assert!(
        palette.query.contains(r#""range":"A1:A1""#),
        "palette query: {:?}",
        palette.query
    );
    assert!(!harness.tui.has_pending_tasks());
}

#[test]
fn ai_assist_key_opens_the_formula_workflow_picker_locally() {
    let mut harness = common::harness();

    harness
        .tui
        .step_key(KeyEvent {
            code: KeyCode::Char('x'),
            ctrl: true,
            alt: false,
            shift: true,
        })
        .unwrap();

    let palette = harness.tui.ui().palette();
    assert!(palette.open);
    assert_eq!(palette.query, "ai.formula.");
    assert!(palette.prompt.unwrap().contains("AI assist"));
    assert!(!harness.tui.has_pending_tasks());
}

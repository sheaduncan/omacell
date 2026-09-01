//! Debounced inline completion lifecycle.

mod common;

use omacell_ui::{EditSurface, KeyCode, KeyEvent, KeyOutcome};

#[test]
fn completion_appears_as_ghost_and_tab_accepts_without_committing() {
    let mut harness = common::harness_sets(&["ai.completion.mode=on", "ai.completion.debounce=1"]);
    harness.tui.ui().begin_edit(EditSurface::InCell, "=SU");
    let started = std::time::Instant::now();
    while harness.tui.ui().edit().ghost.is_none() {
        harness.tui.poll_reload().unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        std::thread::yield_now();
    }
    assert_eq!(harness.tui.ui().edit().ghost.as_deref(), Some("M(A1:A3)"));
    let outcome = harness.tui.step_key(KeyEvent::new(KeyCode::Tab)).unwrap();
    assert_eq!(outcome, KeyOutcome::Pending);
    let edit = harness.tui.ui().edit();
    assert_eq!(edit.buffer, "=SUM(A1:A3)");
    assert!(edit.ghost.is_none());
    assert!(!edit.is_idle());
}

//! Classic and modal keys through the TUI event loop (`step_key`).

mod common;

use common::{draw_text, harness, harness_modal, seed_demo};
use omacell_ui::{KeyCode, KeyEvent, KeyOutcome};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code)
}

fn ch(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c))
}

#[test]
fn classic_arrows_move_the_cursor() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    assert_eq!(h.tui.ui().selection().cursor.col, 0);
    let out = h.tui.step_key(key(KeyCode::Right)).unwrap();
    assert!(matches!(out, KeyOutcome::Command { ref cmd, .. } if cmd == "nav.right"));
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
    h.tui.step_key(key(KeyCode::Down)).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.row, 1);
    let frame = draw_text(&h.tui, 80, 24);
    assert!(frame.contains("B2") || frame.contains("READY"), "{frame}");
}

#[test]
fn classic_f2_edits_and_types() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    h.tui.step_key(key(KeyCode::F(2))).unwrap();
    assert!(!h.tui.ui().edit().is_idle());
    assert!(h.tui.ui().edit().buffer.contains("Hello"));
    h.tui.step_key(ch('!')).unwrap();
    assert!(h.tui.ui().edit().buffer.contains('!'));
}

#[test]
fn modal_hjkl_and_count() {
    let mut h = harness_modal();
    assert_eq!(h.tui.ui().mode().label(), "NORMAL");
    h.tui.step_key(ch('l')).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
    h.tui.step_key(ch('3')).unwrap();
    let out = h.tui.step_key(ch('j')).unwrap();
    match out {
        KeyOutcome::Command { cmd, count, .. } => {
            assert_eq!(cmd, "nav.down");
            assert_eq!(count, 3);
        }
        other => panic!("expected command, got {other:?}"),
    }
    assert_eq!(h.tui.ui().selection().cursor.row, 3);
}

#[test]
fn mouse_click_moves_cursor_when_enabled() {
    let mut h = harness();
    h.tui
        .draw(&mut ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap())
        .unwrap();
    let before = h.tui.ui().selection().cursor;
    h.tui.step_mouse(20, 8);
    let after = h.tui.ui().selection().cursor;
    assert!(
        after.row != before.row || after.col != before.col,
        "click should move off A1, got {after:?}"
    );
}

#[test]
fn palette_opens_on_ctrl_shift_p() {
    let mut h = harness();
    let event = KeyEvent {
        code: KeyCode::Char('p'),
        ctrl: true,
        alt: false,
        shift: true,
    };
    h.tui.step_key(event).unwrap();
    assert!(h.tui.ui().palette().open);
    let frame = draw_text(&h.tui, 80, 24);
    assert!(frame.contains("palette"), "{frame}");
}

//! ReloadEvent::Applied goes through UiSession::apply_config without resetting edit.

mod common;

use common::harness;
use omacell_ui::EditSurface;

#[test]
fn applied_reload_preserves_edit_buffer() {
    let mut h = harness();
    h.tui.ui().begin_edit(EditSurface::InCell, "=A1+B1");
    let before = h.tui.ui().edit().buffer.clone();
    let cursor = h.tui.ui().selection().cursor;

    std::fs::write(
        h._dir.path().join(".config/omacell/config.toml"),
        "[appearance]\ngrid_lines = false\n[tui]\ntruecolor = \"off\"\n",
    )
    .unwrap();
    h.tui.store().reload().unwrap();
    h.tui.poll_reload().unwrap();

    assert_eq!(h.tui.ui().edit().buffer, before);
    assert!(!h.tui.ui().edit().is_idle());
    assert_eq!(h.tui.ui().selection().cursor, cursor);
    assert!(!h.tui.ui().config().appearance.grid_lines);
}

#[test]
fn invalid_reload_keeps_session_and_records_message() {
    let mut h = harness();
    h.tui.ui().begin_edit(EditSurface::FormulaBar, "keep me");
    std::fs::write(
        h._dir.path().join(".config/omacell/config.toml"),
        "this is not toml {{",
    )
    .unwrap();
    assert!(h.tui.store().reload().is_err());
    h.tui.poll_reload().unwrap();
    assert_eq!(h.tui.ui().edit().buffer, "keep me");
    assert!(h.tui.message().is_some(), "expected Invalid message");
}

#[test]
fn invalid_keymap_reload_keeps_the_last_good_session_alive() {
    let mut h = harness();
    let key_dir = h._dir.path().join(".config/omacell/keys");
    std::fs::create_dir_all(&key_dir).unwrap();
    std::fs::write(key_dir.join("classic.toml"), "not valid toml {{").unwrap();

    h.tui.store().reload().unwrap();
    h.tui.poll_reload().unwrap();

    assert!(
        h.tui
            .message()
            .is_some_and(|message| message.contains("keymap"))
    );
    h.tui
        .step_key(omacell_ui::KeyEvent::new(omacell_ui::KeyCode::Right))
        .unwrap();
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
}

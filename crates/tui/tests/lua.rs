//! Retained Lua runtime behavior in the TUI composition.

mod common;

use common::{harness_script, wait_tasks};
use omacell_ui::{KeyCode, KeyEvent, KeyOutcome};
use serde_json::json;

#[test]
fn startup_hooks_save_hooks_keymaps_and_source_are_live() {
    let mut harness = harness_script(
        r#"
        omacell.ui.status("lua loaded")
        omacell.keymap.set("classic", "Ctrl+L", "cell.clear")
        omacell.on_change(function() omacell.ui.status("lua changed") end)
        omacell.on_before_save(function() omacell.ui.status("lua before save") end)
        "#,
    );
    assert_eq!(harness.tui.message(), Some("lua loaded"));

    harness
        .tui
        .execute_cmd("cell.set", json!({"ref": "A1", "input": "1"}))
        .unwrap();
    wait_tasks(&mut harness.tui);
    assert_eq!(harness.tui.message(), Some("lua changed"));
    assert!(matches!(
        harness.tui.step_key(KeyEvent {
            code: KeyCode::Char('l'),
            ctrl: true,
            alt: false,
            shift: false,
        }),
        Ok(KeyOutcome::Command { cmd, .. }) if cmd == "cell.clear"
    ));
    wait_tasks(&mut harness.tui);

    let save = harness
        .tui
        .execute_cmd("file.saveas", json!({"path": "/work/lua.csv"}))
        .unwrap();
    assert!(save.ok, "{:?}", save.error);
    assert!(
        save.result
            .as_ref()
            .is_some_and(|result| result["queued"] == true)
    );
    wait_tasks(&mut harness.tui);
    assert_eq!(harness.tui.message(), Some("lua before save"));

    std::fs::write(
        harness._dir.path().join(".config/omacell/init.lua"),
        r#"
        omacell.ui.status("lua reloaded")
        omacell.keymap.set("classic", "Ctrl+J", "cell.clear")
        "#,
    )
    .unwrap();
    harness.tui.execute_cmd("script.source", json!({})).unwrap();
    wait_tasks(&mut harness.tui);
    assert_eq!(harness.tui.message(), Some("lua reloaded"));
    let keymap = harness.tui.ui().keymap();
    let classic = keymap.table(omacell_ui::Mode::Classic).unwrap();
    assert_ne!(
        classic.get("Ctrl+L").map(|binding| binding.cmd.as_str()),
        Some("cell.clear")
    );
    assert_eq!(classic["Ctrl+J"].cmd, "cell.clear");
}

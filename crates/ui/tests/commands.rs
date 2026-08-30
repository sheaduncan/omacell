//! UI command registration, schemas, and command-bus preflight behavior.

use omacell_bus::Bus;
use omacell_conf::{Paths, load};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_ui::{
    EditSurface, KeyCode, KeyEvent, KeyOutcome, KeymapRoots, Mode, UiSession, register_ui_commands,
};
use serde_json::json;

fn harness() -> (tempfile::TempDir, UiSession, Bus) {
    harness_with_keymap("keys/classic.toml")
}

fn harness_with_keymap(keymap: &str) -> (tempfile::TempDir, UiSession, Bus) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config.join("config.toml"),
        format!("[keys]\nfile = {keymap:?}\n"),
    )
    .unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    let roots = KeymapRoots::new(paths.user_config, paths.default_dir, None);
    let session = UiSession::new(&loaded, &roots).unwrap();

    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(functions)).unwrap();
    omacell_bus::register_chart_commands(bus.registry_mut()).unwrap();
    register_ui_commands(bus.registry_mut(), &session).unwrap();
    (dir, session, bus)
}

fn execute_key(bus: &mut Bus, outcome: KeyOutcome) {
    let KeyOutcome::Command { cmd, args, .. } = outcome else {
        panic!("expected a command outcome");
    };
    let outcome = bus.execute(Origin::User, &cmd, args);
    assert!(outcome.ok, "{:?}", outcome.error);
}

#[test]
fn external_ui_state_changes_once_and_never_on_dry_run() {
    let (_dir, session, mut bus) = harness();
    assert_eq!(session.selection().cursor.col, 0);

    let dry = bus
        .dry_run(Origin::User, "nav.right", json!({"count": 3}))
        .unwrap();
    assert!(dry.outcome.ok);
    assert_eq!(session.selection().cursor.col, 0);

    let outcome = bus.execute(Origin::User, "nav.right", json!({"count": 3}));
    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(session.selection().cursor.col, 3);

    let invalid_f4 = bus
        .dry_run(Origin::User, "edit.cycleanchor", json!({}))
        .unwrap();
    assert!(!invalid_f4.outcome.ok);
    assert!(session.edit().is_idle());
}

#[test]
fn view_zoom_and_select_have_typed_working_schemas() {
    let (_dir, session, mut bus) = harness();

    let zoom = bus.execute(Origin::User, "view.zoom", json!({"factor": 2.0}));
    assert!(zoom.ok, "{:?}", zoom.error);
    assert_eq!(session.viewport().zoom, 2.0);

    let select = bus.execute(Origin::User, "view.select", json!({"range": "C3:D4"}));
    assert!(select.ok, "{:?}", select.error);
    let area = session.selection().active();
    assert_eq!((area.start.row, area.start.col), (2, 2));
    assert_eq!((area.end.row, area.end.col), (3, 3));

    let invalid = bus.execute(
        Origin::User,
        "view.zoom",
        json!({"factor": 2.0, "delta": 0.1}),
    );
    assert!(!invalid.ok);
    assert_eq!(session.viewport().zoom, 2.0);

    let select_schema = serde_json::to_value(
        &bus.registry()
            .get_str("view.select")
            .unwrap()
            .descriptor
            .arg_schema,
    )
    .unwrap();
    assert_eq!(select_schema["required"], json!(["range"]));
    assert!(select_schema["properties"].get("count").is_none());
}

#[test]
fn edit_key_commands_apply_once_and_modal_commit_returns_to_normal() {
    let (_dir, session, mut bus) = harness_with_keymap("keys/modal.toml");
    session.begin_edit(EditSurface::InCell, "=A1");

    let f4 = session.handle_key(KeyEvent::new(KeyCode::F(4)));
    assert_eq!(session.edit().buffer, "=A1");
    execute_key(&mut bus, f4);
    assert_eq!(session.edit().buffer, "=$A$1");

    let enter = session.handle_key(KeyEvent::new(KeyCode::Enter));
    assert!(!session.edit().is_idle());
    execute_key(&mut bus, enter);
    assert!(session.edit().is_idle());
    assert_eq!(session.mode(), Mode::Normal);
    let slot = bus
        .workbook()
        .get(bus.workbook().active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        bus.workbook().intern().formulas.get(slot.formula.unwrap()),
        Some("=$A$1")
    );

    let undo = bus.execute(Origin::User, "edit.undo", json!({}));
    assert!(undo.ok, "{:?}", undo.error);
    let after_undo = bus
        .workbook()
        .get(bus.workbook().active_sheet(), 0, 0)
        .unwrap();
    assert!(after_undo.is_none(), "after undo: {after_undo:?}");
}

#[test]
fn sheet_navigation_and_region_selection_are_real_actions() {
    let (_dir, session, mut bus) = harness();
    assert!(
        bus.execute(Origin::User, "sheet.add", json!({"name": "Sheet2"}))
            .ok
    );
    assert!(bus.execute(Origin::User, "sheet.next", json!({})).ok);
    assert_eq!(session.selection().sheet.index(), 1);
    assert_eq!(bus.workbook().active_sheet().index(), 1);

    for cell in ["A1", "A2", "B2", "J10"] {
        assert!(
            bus.execute(Origin::User, "cell.set", json!({"ref": cell, "input": "1"}),)
                .ok
        );
    }
    assert!(
        bus.execute(Origin::User, "view.select", json!({"range": "B2"}))
            .ok
    );
    assert!(bus.execute(Origin::User, "sel.regionall", json!({})).ok);
    let region = session.selection().active();
    assert_eq!((region.start.row, region.start.col), (0, 0));
    assert_eq!((region.end.row, region.end.col), (1, 1));
    assert!(bus.execute(Origin::User, "sel.regionall", json!({})).ok);
    assert_eq!(session.selection().active().cells(), 1_048_576 * 16_384);
}

#[test]
fn modal_counts_and_data_edges_reach_command_handlers() {
    let (_dir, session, mut bus) = harness_with_keymap("keys/modal.toml");
    for cell in ["A1", "A2", "A3", "A5"] {
        assert!(
            bus.execute(Origin::User, "cell.set", json!({"ref": cell, "input": "1"}),)
                .ok
        );
    }

    let pending = session.handle_key(KeyEvent::new(KeyCode::Char('3')));
    assert_eq!(pending, KeyOutcome::Pending);
    execute_key(
        &mut bus,
        session.handle_key(KeyEvent::new(KeyCode::Char('j'))),
    );
    assert_eq!(session.selection().cursor.row, 3);

    assert!(
        bus.execute(Origin::User, "view.select", json!({"range": "A1"}))
            .ok
    );
    assert!(bus.execute(Origin::User, "nav.edgedown", json!({})).ok);
    assert_eq!(session.selection().cursor.row, 2);
    assert!(bus.execute(Origin::User, "nav.edgedown", json!({})).ok);
    assert_eq!(session.selection().cursor.row, 4);

    assert!(
        bus.execute(Origin::User, "sheet.add", json!({"name": "Sheet2"}))
            .ok
    );
    assert!(
        bus.execute(Origin::User, "sheet.add", json!({"name": "Sheet3"}))
            .ok
    );
    assert_eq!(
        session.handle_key(KeyEvent::new(KeyCode::Char('3'))),
        KeyOutcome::Pending
    );
    assert_eq!(
        session.handle_key(KeyEvent::new(KeyCode::Char('g'))),
        KeyOutcome::Pending
    );
    execute_key(
        &mut bus,
        session.handle_key(KeyEvent::new(KeyCode::Char('t'))),
    );
    assert_eq!(session.selection().sheet.index(), 2);
}

#[test]
fn editing_tab_commits_and_ctrl_enter_keeps_its_fill_binding() {
    let (_dir, session, mut bus) = harness();
    session.begin_edit(EditSurface::InCell, "42");
    let tab = session.handle_key(KeyEvent::new(KeyCode::Tab));
    assert!(matches!(tab, KeyOutcome::Command { ref cmd, .. } if cmd == "nav.tab"));
    execute_key(&mut bus, tab);
    assert!(session.edit().is_idle());
    assert_eq!(session.selection().cursor.col, 1);
    assert_eq!(
        bus.workbook()
            .get(bus.workbook().active_sheet(), 0, 0)
            .unwrap()
            .unwrap()
            .value,
        omacell_core::value::Value::Number(42.0)
    );

    session.begin_edit(EditSurface::InCell, "7");
    let ctrl_enter = session.handle_key(KeyEvent {
        code: KeyCode::Enter,
        ctrl: true,
        alt: false,
        shift: false,
    });
    assert!(matches!(
        ctrl_enter,
        KeyOutcome::Command { cmd, .. } if cmd == "edit.fillselection"
    ));
    assert!(!session.edit().is_idle());
}

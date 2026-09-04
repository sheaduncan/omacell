//! UI command registration, schemas, and command-bus preflight behavior.

use omacell_bus::Bus;
use omacell_conf::{Paths, load};
use omacell_core::command::{Origin, Outcome};
use omacell_core::eval::FnRegistry;
use omacell_core::limits::{MAX_COLS, MAX_ROWS};
use omacell_core::recalc::RecalcEngine;
use omacell_core::sheet::FreezePanes;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_ui::{
    EditSurface, FindScope, KeyCode, KeyEvent, KeyOutcome, KeymapRoots, Mode, UiSession,
    apply_command_panel, apply_local_command, command_changes_workbook, register_ui_commands,
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
    omacell_bus::register_edit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_data_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_audit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_analysis_commands(bus.registry_mut()).unwrap();
    omacell_lua::register_script_commands(bus.registry_mut(), omacell_lua::ScriptGate::default())
        .unwrap();
    register_ui_commands(bus.registry_mut(), &session).unwrap();
    (dir, session, bus)
}

#[test]
fn freeze_and_split_are_mutually_exclusive_in_both_dispatch_paths() {
    let (_dir, session, mut bus) = harness();
    let mut selection = session.selection();
    selection.move_by(2, 3);
    session.set_selection(selection);

    assert!(bus.execute(Origin::User, "view.split", json!({})).ok);
    assert_eq!(session.viewport().freeze, FreezePanes::default());
    assert!(session.viewport().split.is_some());
    assert_eq!(session.viewport().first_row, 0);
    assert_eq!(session.viewport().first_col, 0);
    assert!(bus.execute(Origin::User, "view.freeze", json!({})).ok);
    assert_eq!(session.viewport().freeze, FreezePanes { rows: 2, cols: 3 });
    assert!(session.viewport().split.is_none());

    apply_local_command(&session, bus.workbook(), "view.split", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(session.viewport().freeze, FreezePanes::default());
    let split = session.viewport().split.expect("split");
    assert_eq!(split.y_px, session.viewport().rows.index_to_pixel(2) as u32);
    assert_eq!(split.x_px, session.viewport().cols.index_to_pixel(3) as u32);
    apply_local_command(&session, bus.workbook(), "view.freeze", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(session.viewport().freeze, FreezePanes { rows: 2, cols: 3 });
    assert!(session.viewport().split.is_none());
}

#[test]
fn modal_insert_tab_stays_in_the_edit_buffer() {
    let (_dir, session, _bus) = harness_with_keymap("keys/modal.toml");
    session.begin_edit(EditSurface::InCell, "hello");
    let outcome = session.handle_key(KeyEvent {
        code: KeyCode::Tab,
        ctrl: false,
        alt: false,
        shift: false,
    });
    assert!(matches!(outcome, KeyOutcome::Pending));
    assert_eq!(session.mode(), Mode::Insert);
    assert_eq!(session.edit().buffer, "hello\t");
}

#[test]
fn point_mode_arrows_update_the_ref_and_enter_commits_from_the_origin() {
    let (_dir, session, mut bus) = harness_with_keymap("keys/classic.toml");
    session.begin_edit(EditSurface::InCell, "=");
    assert_eq!(
        session.handle_key(KeyEvent::new(KeyCode::Down)),
        KeyOutcome::Pending
    );
    assert_eq!(session.edit().buffer, "=A2");
    assert_eq!(
        session.handle_key(KeyEvent::new(KeyCode::Right)),
        KeyOutcome::Pending
    );
    assert_eq!(session.edit().buffer, "=B2");
    let outcome = session.handle_key(KeyEvent {
        code: KeyCode::Enter,
        ctrl: false,
        alt: false,
        shift: false,
    });
    assert!(matches!(outcome, KeyOutcome::Command { ref cmd, .. } if cmd == "nav.enter"));
    assert_eq!(session.selection().cursor.row, 0);
    assert_eq!(session.selection().cursor.col, 0);
    execute_key(&mut bus, outcome);
    assert!(session.edit().is_idle());
    assert_eq!(session.selection().cursor.row, 1);
    let slot = bus
        .workbook()
        .get(bus.workbook().active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        bus.workbook().intern().formulas.get(slot.formula.unwrap()),
        Some("=B2")
    );
}

#[test]
fn point_mode_external_selection_updates_the_provisional_ref() {
    let (_dir, session, _bus) = harness_with_keymap("keys/classic.toml");
    session.begin_edit(EditSurface::FormulaBar, "=");
    let mut selection = session.selection();
    selection.move_by(2, 2);
    session.set_selection(selection.clone());
    assert_eq!(session.edit().buffer, "=C3");

    selection.move_by(0, 1);
    session.set_selection(selection);
    assert_eq!(session.edit().buffer, "=D3");
}

#[test]
fn visual_navigation_preserves_shape_and_escape_resets_extension() {
    let (_dir, session, mut bus) = harness_with_keymap("keys/modal.toml");
    let mut selection = session.selection();
    selection.move_by(1, 1);
    session.set_selection(selection);

    for command in ["sel.visual", "nav.down", "nav.top"] {
        apply_local_command(&session, bus.workbook(), command, &json!({}))
            .unwrap()
            .unwrap();
    }
    assert_eq!(session.selection().active().normalized(), (0, 1, 1, 1));
    assert_eq!(session.selection().extend, omacell_ui::ExtendMode::Extend);

    apply_local_command(&session, bus.workbook(), "mode.normal", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(session.selection().extend, omacell_ui::ExtendMode::Replace);
    apply_local_command(&session, bus.workbook(), "nav.right", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(session.selection().active().normalized(), (0, 2, 0, 2));

    let mut selection = session.selection();
    selection.move_by(1, -1);
    session.set_selection(selection);
    for command in ["sel.visualrow", "nav.down", "nav.right"] {
        apply_local_command(&session, bus.workbook(), command, &json!({}))
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        session.selection().active().normalized(),
        (1, 0, 2, MAX_COLS - 1)
    );

    apply_local_command(&session, bus.workbook(), "mode.normal", &json!({}))
        .unwrap()
        .unwrap();
    let mut selection = session.selection();
    selection.replace(omacell_ui::Area::cell(omacell_core::addr::CellRef {
        row: 1,
        col: 1,
        ..selection.cursor
    }));
    session.set_selection(selection);
    for command in ["sel.visualcol", "nav.right", "nav.down"] {
        apply_local_command(&session, bus.workbook(), command, &json!({}))
            .unwrap()
            .unwrap();
    }
    assert_eq!(
        session.selection().active().normalized(),
        (0, 1, MAX_ROWS - 1, 2)
    );

    assert!(bus.execute(Origin::User, "mode.normal", json!({})).ok);
    let mut selection = session.selection();
    selection.replace(omacell_ui::Area::cell(omacell_core::addr::CellRef {
        row: 1,
        col: 1,
        ..selection.cursor
    }));
    session.set_selection(selection);
    for command in ["sel.visual", "nav.down", "nav.top"] {
        let outcome = bus.execute(Origin::User, command, json!({}));
        assert!(outcome.ok, "{command}: {:?}", outcome.error);
    }
    assert_eq!(session.selection().active().normalized(), (0, 1, 1, 1));
    assert_eq!(session.selection().extend, omacell_ui::ExtendMode::Extend);
}

#[test]
fn workbook_panels_use_live_data_and_closed_schemas() {
    let (_dir, session, mut bus) = harness();
    let note = bus.execute(
        Origin::User,
        "edit.note",
        json!({"ref": "C2", "text": "check this", "author": "Ada"}),
    );
    assert!(note.ok, "{:?}", note.error);

    let comments = bus.execute(Origin::User, "comments.panel", json!({}));
    assert!(comments.ok, "{:?}", comments.error);
    assert_eq!(session.panel().visible.as_deref(), Some("comments"));
    assert!(
        session
            .panel()
            .body
            .as_deref()
            .unwrap()
            .contains("C2  note by Ada")
    );

    for id in ["comments.panel", "sort.panel", "filter.panel"] {
        let schema =
            serde_json::to_value(&bus.registry().get_str(id).unwrap().descriptor.arg_schema)
                .unwrap();
        assert_eq!(schema["additionalProperties"], false, "{id}: {schema}");
        assert!(
            schema
                .get("properties")
                .is_none_or(|properties| properties == &json!({})),
            "{id}: {schema}"
        );
    }

    let format = bus.execute(Origin::User, "format.panel", json!({"range": "A1"}));
    assert!(format.ok, "{:?}", format.error);
    assert!(
        apply_command_panel(&session, "format.panel", format.result.as_ref().unwrap()).unwrap()
    );
    assert_eq!(session.panel().visible.as_deref(), Some("format"));
    assert!(
        session
            .panel()
            .body
            .as_deref()
            .unwrap()
            .contains("Number format: General")
    );
}

#[test]
fn workbook_change_policy_distinguishes_session_and_model_mutations() {
    let no_count = Outcome::success(json!({}));
    assert!(command_changes_workbook("view.hiderows", &no_count, true));
    assert!(command_changes_workbook("edit.undo", &no_count, true));
    assert!(!command_changes_workbook("view.zoom", &no_count, true));
    assert!(!command_changes_workbook("file.save", &no_count, true));
    assert!(!command_changes_workbook("cell.set", &no_count, false));

    let explicit_noop = Outcome::success(json!({"changed": 0}));
    assert!(!command_changes_workbook(
        "view.hiderows",
        &explicit_noop,
        true
    ));
    let explicit_change = Outcome::success(json!({"changed": 1}));
    assert!(command_changes_workbook(
        "otherwise.presentation",
        &explicit_change,
        true
    ));
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

#[test]
fn search_commands_advance_wrap_and_follow_workbook_scope() {
    let (_dir, session, mut bus) = harness();
    for cell in ["A1", "C1"] {
        assert!(
            bus.execute(
                Origin::User,
                "cell.set",
                json!({"ref": cell, "input": "needle"}),
            )
            .ok
        );
    }
    assert!(
        bus.execute(Origin::User, "sheet.add", json!({"name": "Other"}))
            .ok
    );
    assert!(
        bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": "Other!B1", "input": "needle"}),
        )
        .ok
    );
    let mut find = session.find_replace();
    find.find = "needle".into();
    find.scope = FindScope::Workbook;
    session.set_find_replace(find);

    assert!(
        bus.execute(Origin::User, "view.select", json!({"range": "A1"}))
            .ok
    );
    assert!(bus.execute(Origin::User, "edit.searchnext", json!({})).ok);
    assert_eq!(session.selection().cursor.col, 2);

    assert!(bus.execute(Origin::User, "edit.searchnext", json!({})).ok);
    assert_eq!(session.selection().sheet.index(), 1);
    assert_eq!(session.selection().cursor.col, 1);
    assert_eq!(bus.workbook().active_sheet().index(), 1);

    assert!(bus.execute(Origin::User, "edit.searchnext", json!({})).ok);
    assert_eq!(session.selection().sheet.index(), 0);
    assert_eq!(session.selection().cursor.col, 0);

    assert!(bus.execute(Origin::User, "edit.searchprev", json!({})).ok);
    assert_eq!(session.selection().sheet.index(), 1);
    assert_eq!(session.selection().cursor.col, 1);

    assert!(
        bus.execute(Origin::User, "edit.searchnext", json!({"count": 2}))
            .ok
    );
    assert_eq!(session.selection().sheet.index(), 0);
    assert_eq!(session.selection().cursor.col, 2);
}

#[test]
fn explain_error_opens_a_panel_for_the_selected_cell() {
    let (_dir, session, mut bus) = harness();
    assert!(
        bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": "A1", "input": "=1/0"}),
        )
        .ok
    );

    let outcome = bus.execute(Origin::User, "edit.explainerror", json!({}));
    assert!(outcome.ok, "{:?}", outcome.error);
    let panel = session.panel();
    assert_eq!(panel.visible.as_deref(), Some("explainerror"));
    assert!(
        panel
            .body
            .as_deref()
            .is_some_and(|body| body.contains("#DIV/0!") && body.contains("divisor 0"))
    );
}

#[test]
fn name_manager_lists_names_and_paste_inserts_into_the_editor() {
    let (_dir, session, mut bus) = harness();
    assert!(
        bus.execute(
            Origin::User,
            "name.define",
            json!({
                "name": "TaxRate",
                "referent": {"type": "constant", "value": 0.2}
            }),
        )
        .ok
    );

    let manager = bus.execute(Origin::User, "name.manager", json!({}));
    assert!(manager.ok, "{:?}", manager.error);
    let panel = session.panel();
    assert_eq!(panel.visible.as_deref(), Some("names"));
    assert!(panel.body.as_deref().is_some_and(|body| {
        body.contains("TaxRate") && body.contains("workbook") && body.contains("0.2")
    }));

    let paste = bus.execute(Origin::User, "name.paste", json!({"name": "taxrate"}));
    assert!(paste.ok, "{:?}", paste.error);
    assert_eq!(session.edit().buffer, "=TaxRate");

    session.begin_edit(EditSurface::FormulaBar, "=SUM(");
    let paste = bus.execute(Origin::User, "name.paste", json!({"name": "TaxRate"}));
    assert!(paste.ok, "{:?}", paste.error);
    assert_eq!(session.edit().buffer, "=SUM(TaxRate");

    let missing = bus.execute(Origin::User, "name.paste", json!({"name": "Missing"}));
    assert!(!missing.ok);
    assert_eq!(missing.error.unwrap().code, "name.defined");
}

#[test]
fn ai_assist_opens_the_formula_workflow_picker() {
    let (_dir, session, mut bus) = harness();

    let outcome = bus.execute(Origin::User, "ai.assist", json!({}));
    assert!(outcome.ok, "{:?}", outcome.error);
    let palette = session.palette();
    assert!(palette.open);
    assert_eq!(palette.query, "ai.formula.");
    assert!(
        palette
            .prompt
            .as_deref()
            .is_some_and(|prompt| prompt.contains("generate") && prompt.contains("refactor"))
    );
}

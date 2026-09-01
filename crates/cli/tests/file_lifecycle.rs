//! File-session lifecycle commands share one typed composition adapter.

use omacell_bus::Bus;
use omacell_cli::{FileSession, register_file_commands};
use omacell_conf::{Paths, load};
use omacell_core::addr::CellRef;
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::sheet::{FreezePanes, SplitView};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_io::xlsx::{SaveOptions, save_workbook};
use omacell_ui::{Area, KeymapRoots, Selection, UiSession};

fn bus_with(workbook: Workbook, session: FileSession) -> Bus {
    let mut bus = Bus::new(workbook, RecalcEngine::new(FnRegistry::new())).unwrap();
    register_file_commands(&mut bus, session).unwrap();
    bus
}

fn cell_text(workbook: &Workbook, row: u32, col: u16) -> String {
    let sheet = workbook.active_sheet();
    let slot = workbook.get(sheet, row, col).unwrap().unwrap();
    let Value::Text(id) = slot.value else {
        panic!("expected text, got {:?}", slot.value);
    };
    workbook.intern().strings.get(id).unwrap().to_owned()
}

fn ui_session(home: &std::path::Path) -> UiSession {
    let paths = Paths::from_home(home);
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    let roots = KeymapRoots::new(paths.user_config, paths.default_dir, None);
    UiSession::new(&loaded, &roots).unwrap()
}

#[test]
fn file_new_replaces_the_workbook_and_detaches_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.csv");
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_cell_contents(sheet, 0, 0, "old").unwrap();
    let session = FileSession::new();
    let mut bus = bus_with(workbook, session.clone());
    let saved = bus.execute(
        Origin::User,
        "file.saveas",
        serde_json::json!({"path": path.display().to_string()}),
    );
    assert!(saved.ok, "{:?}", saved.error);
    assert_eq!(session.current_path().as_deref(), Some(path.as_path()));

    let outcome = bus.execute(Origin::User, "file.new", serde_json::json!({}));

    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(outcome.result, Some(serde_json::json!({"path": null})));
    assert!(session.current_path().is_none());
    assert_eq!(bus.workbook().sheets().count(), 1);
    let sheet = bus.workbook().active_sheet();
    assert!(bus.workbook().get(sheet, 0, 0).unwrap().is_none());
    let save = bus.execute(Origin::User, "file.save", serde_json::json!({}));
    assert_eq!(save.error.unwrap().code, "file.path");
}

#[test]
fn file_saveas_writes_and_becomes_the_active_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("renamed.csv");
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_cell_contents(sheet, 0, 0, "first").unwrap();
    let session = FileSession::new();
    let mut bus = bus_with(workbook, session.clone());

    let outcome = bus.execute(
        Origin::User,
        "file.saveas",
        serde_json::json!({"path": path.display().to_string()}),
    );

    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(session.current_path().as_deref(), Some(path.as_path()));
    assert!(std::fs::read_to_string(&path).unwrap().contains("first"));

    bus.workbook_mut()
        .set_cell_contents(sheet, 0, 0, "second")
        .unwrap();
    let saved = bus.execute(Origin::User, "file.save", serde_json::json!({}));
    assert!(saved.ok, "{:?}", saved.error);
    assert!(std::fs::read_to_string(&path).unwrap().contains("second"));
}

#[test]
fn interactive_save_persists_the_retained_sheet_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("split-view.xlsx");
    let mut workbook = Workbook::new();
    let first = workbook.active_sheet();
    let selected_sheet = workbook.add_sheet("Selected").unwrap();
    let mut original_view = workbook.sheet(selected_sheet).unwrap().view.clone();
    original_view.gridlines = false;
    workbook
        .set_sheet_view(selected_sheet, original_view)
        .unwrap();
    assert_eq!(workbook.active_sheet(), first);

    let ui = ui_session(dir.path());
    let mut start = CellRef::new(2, 1).unwrap();
    start.sheet = Some(selected_sheet);
    let mut end = CellRef::new(5, 4).unwrap();
    end.sheet = Some(selected_sheet);
    let mut selection = Selection::a1(selected_sheet);
    selection.cursor = end;
    selection.areas = vec![Area { start, end }];
    ui.set_selection(selection);
    let mut viewport = ui.viewport();
    viewport.first_row = 18;
    viewport.first_col = 7;
    viewport.zoom = 1.5;
    viewport.split = Some(SplitView {
        x_px: 120,
        y_px: 40,
    });
    ui.set_viewport(viewport);
    ui.set_show_formulas(true);

    let session = FileSession::new();
    session.attach_ui(ui.clone());
    let mut bus = bus_with(workbook, session);
    let outcome = bus.execute(
        Origin::User,
        "file.saveas",
        serde_json::json!({"path": path.display().to_string()}),
    );

    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(bus.workbook().active_sheet(), selected_sheet);
    let live = bus.workbook().sheet(selected_sheet).unwrap().view.clone();
    assert_eq!(live.scroll_row, 18);
    assert_eq!(live.scroll_col, 7);
    assert_eq!(live.zoom, 1.5);
    assert_eq!(live.freeze, FreezePanes::default());
    assert_eq!(
        live.split,
        Some(SplitView {
            x_px: 120,
            y_px: 40
        })
    );
    assert_eq!(live.selection.start, CellRef::new(2, 1).unwrap());
    assert_eq!(live.selection.end, CellRef::new(5, 4).unwrap());
    assert!(!live.gridlines);
    assert!(live.show_formulas);

    let reopened = omacell_io::xlsx::open(&path).unwrap().workbook;
    let reopened_sheet = reopened.active_sheet();
    assert_eq!(reopened.sheet(reopened_sheet).unwrap().view, live);

    let freeze_path = dir.path().join("freeze-view.xlsx");
    let mut viewport = ui.viewport();
    viewport.freeze = FreezePanes { rows: 2, cols: 1 };
    viewport.split = None;
    ui.set_viewport(viewport);
    let outcome = bus.execute(
        Origin::User,
        "file.saveas",
        serde_json::json!({"path": freeze_path.display().to_string()}),
    );

    assert!(outcome.ok, "{:?}", outcome.error);
    let live = bus.workbook().sheet(selected_sheet).unwrap().view.clone();
    assert_eq!(live.freeze, FreezePanes { rows: 2, cols: 1 });
    assert_eq!(live.split, None);
    let reopened = omacell_io::xlsx::open(&freeze_path).unwrap().workbook;
    let reopened_sheet = reopened.active_sheet();
    assert_eq!(reopened.sheet(reopened_sheet).unwrap().view, live);

    let before_failed_save = live;
    let mut viewport = ui.viewport();
    viewport.first_row = 99;
    ui.set_viewport(viewport);
    let missing_parent = dir.path().join("missing").join("failed.xlsx");
    let outcome = bus.execute(
        Origin::User,
        "file.saveas",
        serde_json::json!({"path": missing_parent.display().to_string()}),
    );
    assert!(!outcome.ok);
    assert_eq!(
        bus.workbook().sheet(selected_sheet).unwrap().view,
        before_failed_save
    );
}

#[test]
fn file_close_is_a_non_destructive_frontend_control_result() {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_cell_contents(sheet, 0, 0, "kept").unwrap();
    let mut bus = bus_with(workbook, FileSession::new());

    let outcome = bus.execute(Origin::User, "file.close", serde_json::json!({}));

    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(outcome.result, Some(serde_json::json!({"close": true})));
    assert!(bus.workbook().get(sheet, 0, 0).unwrap().is_some());
}

#[test]
fn file_open_recalculates_with_a_fresh_workbook_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh-session.xlsx");
    let mut opened = Workbook::new();
    let opened_sheet = opened.active_sheet();
    opened
        .set_formula_text(opened_sheet, 1, 1, "=CELL(\"address\")")
        .unwrap();
    save_workbook(
        &opened,
        &path,
        SaveOptions {
            lock: false,
            ..SaveOptions::default()
        },
    )
    .unwrap();

    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(functions)).unwrap();
    register_file_commands(&mut bus, FileSession::new()).unwrap();
    let changed = bus.execute(
        Origin::User,
        "cell.set",
        serde_json::json!({"ref": "D4", "input": "1"}),
    );
    assert!(changed.ok, "{:?}", changed.error);

    let outcome = bus.execute(
        Origin::User,
        "file.open",
        serde_json::json!({"path": path.display().to_string()}),
    );

    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(cell_text(bus.workbook(), 1, 1), "$B$2");
}

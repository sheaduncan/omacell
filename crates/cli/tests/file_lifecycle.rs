//! File-session lifecycle commands share one typed composition adapter.

use omacell_bus::Bus;
use omacell_cli::{FileSession, register_file_commands};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;

fn bus_with(workbook: Workbook, session: FileSession) -> Bus {
    let mut bus = Bus::new(workbook, RecalcEngine::new(FnRegistry::new())).unwrap();
    register_file_commands(&mut bus, session).unwrap();
    bus
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

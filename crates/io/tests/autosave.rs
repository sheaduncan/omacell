use std::path::Path;

use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_io::autosave::AutosaveStore;

fn text_at(workbook: &Workbook, row: u32, col: u16) -> &str {
    let sheet = workbook.active_sheet();
    let slot = workbook.get(sheet, row, col).unwrap().unwrap();
    let Value::Text(id) = slot.value else {
        panic!("expected text")
    };
    workbook.intern().strings.get(id).unwrap()
}

#[test]
fn dirty_snapshot_is_discovered_and_can_be_recovered() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("budget.xlsx");
    std::fs::write(&source, b"original marker").unwrap();
    let store = AutosaveStore::new(temp.path());
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_text(sheet, 0, 0, "recovered edit").unwrap();

    let written = store
        .write_snapshot("live-session", &workbook, Some(&source))
        .unwrap();
    let candidates = store.discover(Some(&source)).unwrap();

    assert_eq!(candidates, vec![written.clone()]);
    let recovered = store.open(&written).unwrap();
    assert_eq!(text_at(&recovered, 0, 0), "recovered edit");

    store.clear("live-session").unwrap();
    assert!(store.discover(Some(&source)).unwrap().is_empty());
    assert!(!written.snapshot.exists());
    assert!(!written.manifest.exists());
}

#[test]
fn a_newer_source_suppresses_stale_recovery_and_snapshots_are_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("budget.xlsx");
    std::fs::write(&source, b"version one").unwrap();
    let store = AutosaveStore::with_limit(temp.path(), 2);
    let workbook = Workbook::new();

    store
        .write_snapshot("session-1", &workbook, Some(&source))
        .unwrap();
    std::fs::write(&source, b"version two is newer").unwrap();
    assert!(store.discover(Some(&source)).unwrap().is_empty());

    store.write_snapshot("session-2", &workbook, None).unwrap();
    store.write_snapshot("session-3", &workbook, None).unwrap();
    store.write_snapshot("session-4", &workbook, None).unwrap();
    let untitled = store.discover(None).unwrap();
    assert_eq!(untitled.len(), 2);
    assert!(untitled.iter().all(|candidate| candidate.source.is_none()));
}

#[test]
fn session_ids_cannot_escape_the_autosave_directory() {
    let temp = tempfile::tempdir().unwrap();
    let store = AutosaveStore::new(temp.path());
    let error = store
        .write_snapshot("../escape", &Workbook::new(), None)
        .unwrap_err();
    assert_eq!(error.code, "autosave.session");
    assert!(!Path::new(temp.path()).join("escape.xlsx").exists());
}

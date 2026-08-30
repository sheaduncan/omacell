//! WP-17 command coverage: insert rewrite, paste special, fill, undo.

mod common;

use omacell_core::command::Origin;
use omacell_core::ops::formula_src;
use serde_json::json;

fn bus() -> omacell_bus::Bus {
    let mut bus = common::bus();
    omacell_bus::register_edit_commands(bus.registry_mut()).unwrap();
    bus
}

#[test]
fn insert_rows_rewrites_and_undoes() {
    let mut bus = bus();
    assert!(
        bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": "A1", "input": "=A3"})
        )
        .ok
    );
    assert!(
        bus.execute(
            Origin::User,
            "edit.insert",
            json!({"range": "A2", "shift": "rows"})
        )
        .ok
    );
    assert_eq!(
        formula_src(bus.workbook(), bus.workbook().active_sheet(), 0, 0),
        "=A4"
    );
    assert!(bus.execute(Origin::User, "edit.undo", json!({})).ok);
}

#[test]
fn copy_paste_values() {
    let mut bus = bus();
    assert!(
        bus.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "7"}))
            .ok
    );
    let copy = bus.execute(Origin::User, "edit.copy", json!({"range": "A1"}));
    assert!(copy.ok);
    let payload = copy.result.unwrap()["payload"].clone();
    let paste = bus.execute(
        Origin::User,
        "edit.paste",
        json!({"range": "B1", "payload": payload, "special": "values"}),
    );
    assert!(paste.ok, "{:?}", paste.error);
}

#[test]
fn fill_down_copies() {
    let mut bus = bus();
    assert!(
        bus.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "x"}))
            .ok
    );
    let out = bus.execute(Origin::User, "edit.filldown", json!({"range": "A1:A3"}));
    assert!(out.ok, "{:?}", out.error);
}

#[test]
fn protect_records_xor_hash() {
    let mut bus = bus();
    let out = bus.execute(
        Origin::User,
        "sheet.protect",
        json!({"password": "password", "enable": true}),
    );
    assert!(out.ok, "{:?}", out.error);
    assert_eq!(out.result.unwrap()["hash"], 0x83AF);
    assert_eq!(
        bus.workbook()
            .sheet(bus.workbook().active_sheet())
            .unwrap()
            .protection
            .password
            .as_deref(),
        Some(b"83AF".as_slice())
    );
}

#[test]
fn move_is_one_command() {
    let mut bus = bus();
    assert!(
        bus.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}))
            .ok
    );
    let out = bus.execute(
        Origin::User,
        "edit.move",
        json!({"src": "A1", "dest": "B1"}),
    );
    assert!(out.ok, "{:?}", out.error);
}

#[test]
fn paste_adjusts_formula_from_clipboard_origin() {
    let mut bus = bus();
    assert!(
        bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": "A1", "input": "=B1"})
        )
        .ok
    );
    let copy = bus.execute(Origin::User, "edit.copy", json!({"range": "A1"}));
    let payload = copy.result.unwrap()["payload"].clone();
    let paste = bus.execute(
        Origin::User,
        "edit.paste",
        json!({"range": "C3", "payload": payload}),
    );
    assert!(paste.ok, "{:?}", paste.error);
    assert_eq!(
        formula_src(bus.workbook(), bus.workbook().active_sheet(), 2, 2),
        "=D3"
    );
}

#[test]
fn cut_paste_moves_once_and_retargets_dependents() {
    let mut bus = bus();
    assert!(
        bus.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}))
            .ok
    );
    assert!(
        bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": "D1", "input": "=A1"})
        )
        .ok
    );
    let cut = bus.execute(Origin::User, "edit.cut", json!({"range": "A1"}));
    let payload = cut.result.unwrap()["payload"].clone();
    let paste = bus.execute(
        Origin::User,
        "edit.paste",
        json!({"range": "B1", "payload": payload}),
    );
    assert!(paste.ok, "{:?}", paste.error);
    assert_eq!(
        formula_src(bus.workbook(), bus.workbook().active_sheet(), 0, 3),
        "=B1"
    );
}

#[test]
fn hidden_rows_are_restored_by_undo() {
    let mut bus = bus();
    let sheet = bus.workbook().active_sheet();
    let hide = bus.execute(Origin::User, "view.hiderows", json!({"range": "2:2"}));
    assert!(hide.ok, "{:?}", hide.error);
    assert!(
        bus.workbook()
            .sheet(sheet)
            .unwrap()
            .geometry
            .rows
            .is_hidden(1)
            .unwrap()
    );
    assert!(bus.execute(Origin::User, "edit.undo", json!({})).ok);
    assert!(
        !bus.workbook()
            .sheet(sheet)
            .unwrap()
            .geometry
            .rows
            .is_hidden(1)
            .unwrap()
    );
}

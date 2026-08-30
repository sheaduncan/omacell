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

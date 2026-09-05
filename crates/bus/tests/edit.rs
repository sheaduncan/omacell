//! WP-17 command coverage: insert rewrite, paste special, fill, undo.

mod common;

use omacell_core::changeset::CommandCall;
use omacell_core::command::CommandId;
use omacell_core::command::Origin;
use omacell_core::ops::formula_src;
use omacell_core::value::Value;
use proptest::prelude::*;
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
fn automatic_fill_detects_each_source_lane_independently() {
    let mut bus = bus();
    for (cell, input) in [("A1", "1"), ("A2", "2"), ("B1", "10"), ("B2", "20")] {
        common::exec_ok(&mut bus, "cell.set", json!({"ref": cell, "input": input}));
    }

    let out = bus.execute(
        Origin::User,
        "edit.fillselection",
        json!({"src": "A1:B2", "dest": "A1:B4"}),
    );

    assert!(out.ok, "{:?}", out.error);
    let sheet = bus.workbook().active_sheet();
    for (row, left, right) in [(2, 3.0, 30.0), (3, 4.0, 40.0)] {
        assert_eq!(
            bus.workbook().get(sheet, row, 0).unwrap().unwrap().value,
            Value::Number(left)
        );
        assert_eq!(
            bus.workbook().get(sheet, row, 1).unwrap().unwrap().value,
            Value::Number(right)
        );
    }
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
fn move_command_supports_atomic_ctrl_drag_copy() {
    let mut bus = bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "=B1"}));
    let copy = bus.execute(
        Origin::User,
        "edit.move",
        json!({"src": "A1", "dest": "C3", "copy": true}),
    );
    assert!(copy.ok, "{:?}", copy.error);
    assert_eq!(
        formula_src(bus.workbook(), bus.workbook().active_sheet(), 0, 0),
        "=B1"
    );
    assert_eq!(
        formula_src(bus.workbook(), bus.workbook().active_sheet(), 2, 2),
        "=D3"
    );
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
        bus.execute(Origin::User, "cell.set", json!({"ref": "B1", "input": "2"}))
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
    assert!(
        bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": "E1", "input": "=B1"})
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
    assert_eq!(
        formula_src(bus.workbook(), bus.workbook().active_sheet(), 0, 4),
        "=#REF!"
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

fn call(id: &str, args: serde_json::Value) -> CommandCall {
    CommandCall {
        id: CommandId::new(id).unwrap(),
        args,
    }
}

#[test]
fn metadata_commands_are_exactly_undoable() {
    let cases = [
        ("range.merge", json!({"range": "A1:B1"})),
        ("edit.group", json!({"range": "2:3"})),
        (
            "edit.note",
            json!({"ref": "A1", "text": "review", "author": "Ada"}),
        ),
        (
            "edit.hyperlink",
            json!({"ref": "A1", "target": "https://example.com"}),
        ),
        (
            "sheet.protect",
            json!({"password": "password", "enable": true}),
        ),
    ];

    for (id, args) in cases {
        let mut bus = bus();
        let before = common::logical_dump(&bus);
        let outcome = bus.execute(Origin::User, id, args);
        assert!(outcome.ok, "{id}: {:?}", outcome.error);
        assert!(bus.execute(Origin::User, "edit.undo", json!({})).ok, "{id}");
        assert_eq!(common::logical_dump(&bus), before, "{id}");
    }
}

#[test]
fn wp17_changeset_apply_and_revert_are_exact() {
    let mut bus = bus();
    assert!(
        bus.execute(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}))
            .ok
    );
    let before = common::logical_dump(&bus);
    let proposed = bus
        .propose(
            Origin::ExternalAgent,
            vec![
                call("edit.move", json!({"src": "A1", "dest": "B2"})),
                call("range.merge", json!({"range": "B2:C2"})),
                call(
                    "edit.note",
                    json!({"ref": "B2", "text": "moved", "author": "Ada"}),
                ),
            ],
        )
        .unwrap();
    assert_eq!(common::logical_dump(&bus), before);
    bus.apply(Origin::User, &proposed.id).unwrap();
    bus.revert(Origin::User, &proposed.id).unwrap();
    assert_eq!(common::logical_dump(&bus), before);
}

#[test]
fn format_changesets_keep_bounded_command_local_inverses() {
    let mut bus = bus();
    let sheet = bus.workbook().active_sheet();
    for row in 0..10_000 {
        bus.workbook_mut()
            .set_number(sheet, row, 19, f64::from(row))
            .unwrap();
    }
    let before = common::logical_dump(&bus);
    let proposed = bus
        .propose(
            Origin::ExternalAgent,
            vec![call("format.bold", json!({"range": "D5"}))],
        )
        .unwrap();

    bus.apply(Origin::User, &proposed.id).unwrap();
    let inverse = &bus.get_changeset(&proposed.id).unwrap().inverse;
    assert_eq!(inverse.len(), 1);
    assert_eq!(inverse[0].id.as_str(), "style.restore");
    assert!(serde_json::to_vec(inverse).unwrap().len() < 2_048);

    bus.revert(Origin::User, &proposed.id).unwrap();
    assert_eq!(common::logical_dump(&bus), before);
}

fn seeded_bus(seed: i16) -> omacell_bus::Bus {
    let mut bus = bus();
    for (cell, input) in [
        ("A1", seed.to_string()),
        ("A2", seed.to_string()),
        ("A3", (i32::from(seed) + 1).to_string()),
        ("B1", "=A1".into()),
        ("C1", "x,y".into()),
        ("C3", "9".into()),
        ("E5", "5".into()),
    ] {
        common::exec_ok(&mut bus, "cell.set", json!({"ref": cell, "input": input}));
    }
    common::exec_ok(&mut bus, "sheet.add", json!({"name": "Extra"}));
    common::exec_ok(
        &mut bus,
        "edit.comment",
        json!({"ref": "H1", "author": "Ada", "text": "review"}),
    );
    common::exec_ok(
        &mut bus,
        "edit.hyperlink",
        json!({"ref": "H2", "target": "https://example.com"}),
    );
    common::exec_ok(
        &mut bus,
        "edit.note",
        json!({"ref": "H3", "author": "Ada", "text": "note"}),
    );
    bus
}

fn wp17_mutating_cases() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("sheet.remove", json!({"sheet": "Extra"})),
        ("sheet.reorder", json!({"sheet": "Extra", "index": 0})),
        ("edit.insert", json!({"range": "E5", "shift": "down"})),
        ("edit.delcells", json!({"range": "A2", "shift": "down"})),
        ("edit.paste", json!({"range": "M1", "payload": null})),
        (
            "edit.pastespecial",
            json!({"range": "M2", "payload": null, "special": {"values": true, "transpose": true}}),
        ),
        ("edit.move", json!({"src": "A1", "dest": "N1"})),
        (
            "edit.fillselection",
            json!({"src": "A1:A2", "dest": "A1:A4", "mode": "linear"}),
        ),
        ("edit.filldown", json!({"range": "A1:A4"})),
        ("edit.fillright", json!({"range": "A1:D1"})),
        ("edit.fillup", json!({"range": "C1:C3"})),
        ("edit.fillleft", json!({"range": "A3:C3"})),
        ("range.merge", json!({"range": "J14:K14"})),
        ("range.mergeacross", json!({"range": "J11:K12"})),
        ("range.unmerge", json!({"range": "J10:K10"})),
        ("view.hiderows", json!({"range": "5:6"})),
        ("view.hidecols", json!({"range": "E:F"})),
        ("view.unhiderows", json!({"range": "7:7"})),
        ("view.unhidecols", json!({"range": "G:G"})),
        ("format.rowheight", json!({"range": "8:9", "px": 31})),
        ("format.colwidth", json!({"range": "H:I", "px": 91})),
        ("format.autofitrows", json!({"range": "1:3"})),
        ("format.autofitcols", json!({"range": "A:C"})),
        ("edit.group", json!({"range": "10:11"})),
        ("edit.ungroup", json!({"range": "12:13"})),
        ("edit.collapse", json!({"range": "14:15", "axis": "rows"})),
        ("edit.expand", json!({"range": "16:17", "axis": "rows"})),
        (
            "edit.note",
            json!({"ref": "H3", "author": "Lin", "text": "changed"}),
        ),
        (
            "edit.comment",
            json!({"ref": "H1", "author": "Lin", "text": "changed"}),
        ),
        (
            "edit.commentreply",
            json!({"ref": "H1", "author": "Lin", "text": "reply"}),
        ),
        (
            "edit.commentresolve",
            json!({"ref": "H1", "resolved": true}),
        ),
        (
            "edit.hyperlink",
            json!({"ref": "H2", "target": "Sheet1!A1", "tooltip": "jump"}),
        ),
        (
            "sheet.protect",
            json!({"password": "password", "allow": {"sort": true}}),
        ),
        (
            "workbook.protect",
            json!({"password": "password", "lock_structure": true}),
        ),
        (
            "sheet.protectedrange",
            json!({"name": "Editable", "ranges": ["B2:C3"], "password": "range"}),
        ),
        (
            "format.protection",
            json!({"range": "D5:E6", "locked": false, "hidden": true}),
        ),
        (
            "edit.texttocolumns",
            json!({"range": "C1", "delimiters": ",", "column_types": ["text", "general"]}),
        ),
        (
            "range.removeduplicates",
            json!({"range": "A1:A3", "columns": [0]}),
        ),
        (
            "range.consolidate",
            json!({"sources": ["A1:A2", "A3:A3"], "dest": "L1"}),
        ),
        ("edit.clearcell", json!({"range": "A1"})),
        ("edit.clear", json!({"range": "H1:H3", "what": "all"})),
        ("edit.delete", json!({"range": "A1:B1"})),
        ("edit.change", json!({"range": "A2:B2"})),
        ("edit.clearrow", json!({"range": "2:2"})),
        ("edit.autosum", json!({"range": "A1:A4"})),
        ("edit.copyformulaabove", json!({"range": "B2"})),
        ("edit.copyvalueabove", json!({"range": "A4"})),
        (
            "edit.insertdate",
            json!({"range": "D1", "serial": 45_000.5}),
        ),
        (
            "edit.inserttime",
            json!({"range": "D2", "serial": 45_000.5}),
        ),
        ("format.bold", json!({"range": "D5"})),
        ("format.italic", json!({"range": "D5"})),
        ("format.underline", json!({"range": "D5"})),
        ("format.indent", json!({"range": "D5"})),
        ("format.outdent", json!({"range": "D6"})),
        ("format.general", json!({"range": "D7"})),
        ("format.numberstyle", json!({"range": "D5"})),
        ("format.time", json!({"range": "D5"})),
        ("format.date", json!({"range": "D5"})),
        ("format.currency", json!({"range": "D5"})),
        ("format.percent", json!({"range": "D5"})),
        ("format.scientific", json!({"range": "D5"})),
        ("format.borderoutline", json!({"range": "D5:E6"})),
        ("format.bordernone", json!({"range": "D8:E9"})),
    ]
}

fn prepare_case(bus: &mut omacell_bus::Bus, id: &str, args: &mut serde_json::Value) {
    match id {
        "edit.paste" | "edit.pastespecial" => {
            let copy = common::exec_ok(bus, "edit.copy", json!({"range": "A1:B2"}));
            args["payload"] = copy["payload"].clone();
        }
        "range.unmerge" => {
            common::exec_ok(bus, "range.merge", json!({"range": "J10:K10"}));
        }
        "view.unhiderows" => {
            common::exec_ok(bus, "view.hiderows", json!({"range": "7:7"}));
        }
        "view.unhidecols" => {
            common::exec_ok(bus, "view.hidecols", json!({"range": "G:G"}));
        }
        "format.autofitrows" => {
            common::exec_ok(bus, "format.rowheight", json!({"range": "1:3", "px": 1}));
        }
        "format.autofitcols" => {
            common::exec_ok(bus, "format.colwidth", json!({"range": "A:C", "px": 1}));
        }
        "edit.ungroup" => {
            common::exec_ok(bus, "edit.group", json!({"range": "12:13"}));
        }
        "edit.expand" => {
            common::exec_ok(
                bus,
                "edit.collapse",
                json!({"range": "16:17", "axis": "rows"}),
            );
        }
        "format.outdent" => {
            common::exec_ok(bus, "format.indent", json!({"range": "D6"}));
        }
        "format.general" => {
            common::exec_ok(bus, "format.numberstyle", json!({"range": "D7"}));
        }
        "format.bordernone" => {
            common::exec_ok(bus, "format.borderoutline", json!({"range": "D8:E9"}));
        }
        _ => {}
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4))]

    #[test]
    fn every_wp17_mutating_command_has_exact_direct_undo(seed in any::<i16>()) {
        for (id, mut args) in wp17_mutating_cases() {
            let mut bus = seeded_bus(seed);
            prepare_case(&mut bus, id, &mut args);
            let before = common::logical_dump(&bus);
            let outcome = bus.execute(Origin::User, id, args.clone());
            prop_assert!(outcome.ok, "{id}: {:?}", outcome.error);
            let undo = bus.execute(Origin::User, "edit.undo", json!({}));
            prop_assert!(undo.ok, "undo {id}: {:?}", undo.error);
            prop_assert_eq!(common::logical_dump(&bus), before.as_str(), "{}", id);

            let proposed = bus
                .propose(Origin::ExternalAgent, vec![call(id, args)])
                .map_err(|error| TestCaseError::fail(format!("propose {id}: {error}")))?;
            prop_assert_eq!(common::logical_dump(&bus), before.as_str(), "propose {}", id);
            bus.apply(Origin::User, &proposed.id)
                .map_err(|error| TestCaseError::fail(format!("apply {id}: {error}")))?;
            bus.revert(Origin::User, &proposed.id)
                .map_err(|error| TestCaseError::fail(format!("revert {id}: {error}")))?;
            prop_assert_eq!(common::logical_dump(&bus), before.as_str(), "revert {}", id);
        }
    }
}

#[test]
fn repeat_replays_the_last_mutation_as_one_undo_unit() {
    let mut bus = seeded_bus(3);
    common::exec_ok(
        &mut bus,
        "edit.insert",
        json!({"range": "A2", "shift": "rows"}),
    );
    let after_first = common::logical_dump(&bus);
    let repeat = bus.execute(Origin::User, "edit.repeat", json!({"count": 2}));
    assert!(repeat.ok, "{:?}", repeat.error);
    assert_ne!(common::logical_dump(&bus), after_first);
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    assert_eq!(common::logical_dump(&bus), after_first);
}

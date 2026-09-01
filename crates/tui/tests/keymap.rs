//! Classic and modal keys through the TUI event loop (`step_key`).

mod common;

use common::{
    draw_text, harness, harness_modal, harness_sets, harness_workbook, seed_demo, wait_tasks,
};
use omacell_core::workbook::Workbook;
use omacell_ui::{FindScope, KeyCode, KeyEvent, KeyOutcome};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code)
}

fn ch(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c))
}

#[test]
fn classic_arrows_move_the_cursor() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    assert_eq!(h.tui.ui().selection().cursor.col, 0);
    let out = h.tui.step_key(key(KeyCode::Right)).unwrap();
    assert!(matches!(out, KeyOutcome::Command { ref cmd, .. } if cmd == "nav.right"));
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
    h.tui.step_key(key(KeyCode::Down)).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.row, 1);
    let frame = draw_text(&h.tui, 80, 24);
    assert!(frame.contains("B2") || frame.contains("READY"), "{frame}");
}

#[test]
fn classic_f2_edits_and_types() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    h.tui.step_key(key(KeyCode::F(2))).unwrap();
    assert!(!h.tui.ui().edit().is_idle());
    assert!(h.tui.ui().edit().buffer.contains("Hello"));
    h.tui.step_key(ch('!')).unwrap();
    assert!(h.tui.ui().edit().buffer.contains('!'));
}

#[test]
fn modal_hjkl_and_count() {
    let mut h = harness_modal();
    assert_eq!(h.tui.ui().mode().label(), "NORMAL");
    h.tui.step_key(ch('l')).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
    h.tui.step_key(ch('3')).unwrap();
    let out = h.tui.step_key(ch('j')).unwrap();
    match out {
        KeyOutcome::Command { cmd, count, .. } => {
            assert_eq!(cmd, "nav.down");
            assert_eq!(count, 3);
        }
        other => panic!("expected command, got {other:?}"),
    }
    assert_eq!(h.tui.ui().selection().cursor.row, 3);
}

#[test]
fn mouse_click_moves_cursor_when_enabled() {
    let mut h = harness();
    h.tui
        .draw(&mut ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap())
        .unwrap();
    let before = h.tui.ui().selection().cursor;
    h.tui.step_mouse(20, 8);
    let after = h.tui.ui().selection().cursor;
    assert!(
        after.row != before.row || after.col != before.col,
        "click should move off A1, got {after:?}"
    );
}

#[test]
fn palette_opens_on_ctrl_shift_p() {
    let mut h = harness();
    let event = KeyEvent {
        code: KeyCode::Char('p'),
        ctrl: true,
        alt: false,
        shift: true,
    };
    h.tui.step_key(event).unwrap();
    assert!(h.tui.ui().palette().open);
    let frame = draw_text(&h.tui, 80, 24);
    assert!(frame.contains("palette"), "{frame}");
}

#[test]
fn modal_count_does_not_corrupt_the_frozen_undo_schema() {
    let mut h = harness_modal();
    seed_demo(&mut h.tui);
    h.tui.step_key(ch('3')).unwrap();
    let outcome = h.tui.step_key(ch('u')).unwrap();
    assert!(matches!(
        outcome,
        KeyOutcome::Command { ref cmd, count: 3, .. } if cmd == "edit.undo"
    ));
    common::wait_tasks(&mut h.tui);
    assert!(h.tui.message().is_none(), "{:?}", h.tui.message());
    assert!(!draw_text(&h.tui, 80, 24).contains("#DIV/0!"));
}

#[test]
fn ctrl_c_remains_copy_and_ctrl_q_guards_unsaved_work() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    let copy = KeyEvent {
        code: KeyCode::Char('c'),
        ctrl: true,
        alt: false,
        shift: false,
    };
    let outcome = h.tui.step_key(copy).unwrap();
    assert!(matches!(
        outcome,
        KeyOutcome::Command { ref cmd, .. } if cmd == "edit.copy"
    ));
    wait_tasks(&mut h.tui);
    assert_eq!(
        h.tui.ui().clipboard().map(|clipboard| clipboard.tsv),
        Some("Hello".into())
    );
    assert!(!h.tui.quit_requested());

    let quit = KeyEvent {
        code: KeyCode::Char('q'),
        ctrl: true,
        alt: false,
        shift: false,
    };
    h.tui.step_key(quit).unwrap();
    assert!(h.tui.is_dirty());
    assert!(!h.tui.quit_requested());
    assert!(
        h.tui
            .message()
            .is_some_and(|message| message.contains("unsaved"))
    );
    h.tui.step_key(quit).unwrap();
    assert!(h.tui.quit_requested());
}

#[test]
fn internal_and_bracketed_text_paste_flow_through_the_bus() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    h.tui
        .execute_cmd("view.select", serde_json::json!({"range": "A2"}))
        .unwrap();
    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Char('c'),
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    wait_tasks(&mut h.tui);
    h.tui
        .execute_cmd("view.select", serde_json::json!({"range": "C3"}))
        .unwrap();
    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Char('v'),
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    wait_tasks(&mut h.tui);
    let snapshot = h.tui.runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    let copied = snapshot.workbook.get(sheet, 2, 2).unwrap().unwrap();
    assert_eq!(
        copied
            .formula
            .and_then(|id| snapshot.workbook.intern().formulas.get(id)),
        Some("=D2*2")
    );

    h.tui
        .execute_cmd("view.select", serde_json::json!({"range": "E5"}))
        .unwrap();
    h.tui.paste_text("7\t8\n9\t10").unwrap();
    wait_tasks(&mut h.tui);
    let snapshot = h.tui.runner().snapshot();
    assert_eq!(
        snapshot.workbook.get(sheet, 4, 4).unwrap().unwrap().value,
        omacell_core::value::Value::Number(7.0)
    );
    assert_eq!(
        snapshot.workbook.get(sheet, 5, 5).unwrap().unwrap().value,
        omacell_core::value::Value::Number(10.0)
    );
}

#[test]
fn ctrl_enter_fills_the_active_selection_from_its_cursor() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    h.tui
        .execute_cmd("view.select", serde_json::json!({"range": "A1:A3"}))
        .unwrap();
    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Enter,
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    wait_tasks(&mut h.tui);

    let snapshot = h.tui.runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    for row in 0..=2 {
        let slot = snapshot.workbook.get(sheet, row, 0).unwrap().unwrap();
        let omacell_core::value::Value::Text(id) = slot.value else {
            panic!("expected filled text in A{}, got {:?}", row + 1, slot.value);
        };
        assert_eq!(snapshot.workbook.intern().strings.get(id), Some("Hello"));
    }
}

#[test]
fn keyboard_cut_payload_moves_only_when_it_is_pasted() {
    let mut h = harness();
    seed_demo(&mut h.tui);
    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Char('x'),
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    wait_tasks(&mut h.tui);
    let before_paste = h.tui.runner().snapshot();
    let sheet = before_paste.workbook.active_sheet();
    assert!(before_paste.workbook.get(sheet, 0, 0).unwrap().is_some());

    h.tui
        .execute_cmd("view.select", serde_json::json!({"range": "B2"}))
        .unwrap();
    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Char('v'),
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    wait_tasks(&mut h.tui);

    let snapshot = h.tui.runner().snapshot();
    assert!(snapshot.workbook.get(sheet, 0, 0).unwrap().is_none());
    let moved = snapshot.workbook.get(sheet, 1, 1).unwrap().unwrap();
    let omacell_core::value::Value::Text(id) = moved.value else {
        panic!("expected moved text, got {:?}", moved.value);
    };
    assert_eq!(snapshot.workbook.intern().strings.get(id), Some("Hello"));
}

#[test]
fn goto_and_palette_argument_prompts_execute_real_commands() {
    let mut h = harness();
    let goto = KeyEvent {
        code: KeyCode::Char('g'),
        ctrl: true,
        alt: false,
        shift: false,
    };
    h.tui.step_key(goto).unwrap();
    h.tui.step_key(ch('B')).unwrap();
    h.tui.step_key(ch('2')).unwrap();
    h.tui.step_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.row, 1);
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
    assert!(h.tui.ui().panel().visible.is_none());

    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Char('p'),
            ctrl: true,
            alt: false,
            shift: true,
        })
        .unwrap();
    for character in "view.select".chars() {
        h.tui.step_key(ch(character)).unwrap();
    }
    h.tui.step_key(key(KeyCode::Enter)).unwrap();
    assert!(h.tui.ui().palette().prompt.is_some());
    for character in r#"{"range":"C3"}"#.chars() {
        h.tui.step_key(ch(character)).unwrap();
    }
    h.tui.step_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.row, 2);
    assert_eq!(h.tui.ui().selection().cursor.col, 2);
    assert!(!h.tui.ui().palette().open);
}

#[test]
fn find_panel_enter_selects_the_next_match() {
    let mut h = harness();
    for cell in ["A1", "C1"] {
        let outcome = h
            .tui
            .execute_cmd(
                "cell.set",
                serde_json::json!({"ref": cell, "input": "needle"}),
            )
            .unwrap();
        assert!(outcome.ok, "{:?}", outcome.error);
        wait_tasks(&mut h.tui);
    }
    let outcome = h
        .tui
        .execute_cmd("sheet.add", serde_json::json!({"name": "Other"}))
        .unwrap();
    assert!(outcome.ok, "{:?}", outcome.error);
    wait_tasks(&mut h.tui);
    let outcome = h
        .tui
        .execute_cmd(
            "cell.set",
            serde_json::json!({"ref": "Other!B1", "input": "needle"}),
        )
        .unwrap();
    assert!(outcome.ok, "{:?}", outcome.error);
    wait_tasks(&mut h.tui);
    let mut find = h.tui.ui().find_replace();
    find.scope = FindScope::Workbook;
    h.tui.ui().set_find_replace(find);
    h.tui
        .step_key(KeyEvent {
            code: KeyCode::Char('f'),
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    for character in "needle".chars() {
        h.tui.step_key(ch(character)).unwrap();
    }
    h.tui.step_key(key(KeyCode::Enter)).unwrap();
    wait_tasks(&mut h.tui);

    assert_eq!(h.tui.ui().selection().cursor.col, 2);
    assert!(h.tui.ui().panel().visible.is_none());

    let outcome = h
        .tui
        .execute_cmd("edit.searchnext", serde_json::json!({}))
        .unwrap();
    assert!(outcome.ok, "{:?}", outcome.error);
    wait_tasks(&mut h.tui);
    assert_eq!(h.tui.ui().selection().sheet.index(), 1);
    assert_eq!(h.tui.ui().selection().cursor.col, 1);
}

#[test]
fn modal_command_line_executes_and_restores_normal_mode() {
    let mut h = harness_modal();
    h.tui.step_key(ch(':')).unwrap();
    assert_eq!(h.tui.ui().mode().label(), "COMMAND");
    h.tui.step_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(h.tui.ui().mode().label(), "NORMAL");

    h.tui.step_key(ch(':')).unwrap();
    for character in "goto D4".chars() {
        let event = if character == ' ' {
            key(KeyCode::Space)
        } else {
            ch(character)
        };
        h.tui.step_key(event).unwrap();
    }
    h.tui.step_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(h.tui.ui().selection().cursor.row, 3);
    assert_eq!(h.tui.ui().selection().cursor.col, 3);
    assert_eq!(h.tui.ui().mode().label(), "NORMAL");
}

#[test]
fn mouse_uses_the_last_rendered_layout_for_add_and_drag() {
    let mut h = harness();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    h.tui.draw(&mut terminal).unwrap();

    h.tui.step_mouse_with_modifiers(20, 8, false, false);
    let anchor = h.tui.ui().selection().cursor;
    h.tui.step_mouse_with_modifiers(30, 10, true, false);
    assert_eq!(h.tui.ui().selection().areas.len(), 2);
    h.tui.step_mouse_with_modifiers(38, 12, false, true);
    let selection = h.tui.ui().selection();
    assert_ne!(selection.cursor, anchor);
    assert_ne!(selection.active().start, selection.active().end);

    h.tui.draw(&mut terminal).unwrap();
    assert_eq!(
        terminal.backend().buffer()[(20, 8)].bg,
        ratatui::style::Color::Indexed(4),
        "the first Ctrl-added area must remain visibly selected"
    );
}

#[test]
fn mouse_wheel_scrolls_and_ctrl_wheel_zooms_without_moving_the_cursor() {
    let mut h = harness();
    let cursor = h.tui.ui().selection().cursor;
    h.tui.step_scroll(0, 1, false).unwrap();
    assert_eq!(h.tui.ui().viewport().first_row, 3);
    assert_eq!(h.tui.ui().selection().cursor, cursor);

    let zoom = h.tui.ui().viewport().zoom;
    h.tui.step_scroll(0, -1, true).unwrap();
    assert!(h.tui.ui().viewport().zoom > zoom);
    assert!(!h.tui.is_dirty());
}

#[test]
fn bottom_sheet_tabs_follow_the_loaded_appearance_setting() {
    let h = harness_sets(&["appearance.sheet_tabs_position=bottom"]);
    let frame = draw_text(&h.tui, 120, 24);
    let lines = frame.lines().collect::<Vec<_>>();
    assert!(!lines[0].contains("Sheet1"), "{frame}");
    assert!(lines[lines.len() - 2].contains("Sheet1"), "{frame}");
}

#[test]
fn extreme_freeze_remains_virtualized() {
    let mut h = harness();
    assert!(
        h.tui
            .execute_cmd("view.select", serde_json::json!({"range": "XFD1048576"}))
            .unwrap()
            .ok
    );
    assert!(
        h.tui
            .execute_cmd("view.freeze", serde_json::json!({}))
            .unwrap()
            .ok
    );
    let started = std::time::Instant::now();
    let frame = draw_text(&h.tui, 80, 24);
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert!(frame.lines().count() <= 24);
}

#[test]
fn saved_sheet_view_seeds_the_tui_session() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/xlsx/l2_merges_freeze.xlsx");
    let workbook = omacell_io::xlsx::open(&path).unwrap().workbook;
    let h = harness_workbook(workbook);
    let viewport = h.tui.ui().viewport();
    assert_eq!(viewport.freeze.rows, 1);
    assert_eq!(viewport.freeze.cols, 1);
    assert!((viewport.zoom - 1.5).abs() < f64::EPSILON);
}

#[test]
fn hidden_sheets_do_not_appear_as_tabs() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/xlsx/l2_hidden_sheet.xlsx");
    let workbook = omacell_io::xlsx::open(&path).unwrap().workbook;
    let mut h = harness_workbook(workbook);
    let frame = draw_text(&h.tui, 120, 24);
    assert!(frame.contains("Visible"), "{frame}");
    assert!(!frame.contains("Hidden"), "{frame}");
    let active = h.tui.ui().selection().sheet;
    assert!(
        h.tui
            .execute_cmd("sheet.prev", serde_json::json!({}))
            .unwrap()
            .ok
    );
    common::wait_tasks(&mut h.tui);
    assert_eq!(h.tui.ui().selection().sheet, active);
}

#[test]
fn sheet_navigation_does_not_mark_the_workbook_dirty() {
    let mut workbook = Workbook::new();
    workbook.add_sheet("Sheet2").unwrap();
    let mut h = harness_workbook(workbook);
    assert!(!h.tui.is_dirty());
    assert!(
        h.tui
            .execute_cmd("sheet.next", serde_json::json!({}))
            .unwrap()
            .ok
    );
    common::wait_tasks(&mut h.tui);
    assert!(!h.tui.is_dirty());
}

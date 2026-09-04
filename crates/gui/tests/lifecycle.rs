//! GUI lifecycle and toolkit-event integration regressions.

mod common;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{graphics_adapter_available, launch_opts, launch_script, launch_theme};
use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
#[cfg(unix)]
use omacell_bus::ipc::{default_runtime_dir, discover_focused};
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CondFormat};
use omacell_core::style::Color;
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use omacell_gui::Gui;
use omacell_ui::{EditSurface, FindScope, KeyCode, KeyEvent};

#[cfg(unix)]
#[test]
fn native_window_focus_publishes_the_default_ipc_target() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, true, &cc.egui_ctx).unwrap());
    let runtime = default_runtime_dir();
    let pid = std::process::id();

    harness.step();
    assert_eq!(discover_focused(&runtime).unwrap().unwrap().pid, pid);

    harness.input_mut().focused = false;
    harness.step();
    assert_ne!(
        discover_focused(&runtime)
            .unwrap()
            .map(|instance| instance.pid),
        Some(pid)
    );

    harness.input_mut().focused = true;
    harness.step();
    assert_eq!(discover_focused(&runtime).unwrap().unwrap().pid, pid);

    drop(harness);
    assert!(!runtime.join(format!("{pid}.focus")).exists());
}

#[test]
fn startup_file_is_opened_through_the_task_runner() {
    let mut parts = launch_theme(None);
    let open_count = parts.open_count.clone();
    let file = parts._dir.path().join("book.csv");
    parts.launch.file = Some(file.clone());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());

    let started = Instant::now();
    while harness.state().runner().is_busy() {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    harness.step();

    assert_eq!(open_count.load(Ordering::SeqCst), 1);
    assert!(harness.state().title().contains("book.csv"));
}

#[test]
fn grid_paints_worker_resolved_conditional_format_fill() {
    if !graphics_adapter_available() {
        return;
    }
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_number(sheet, 1, 1, 3.0).unwrap();
    workbook
        .set_cond_formats(
            sheet,
            vec![CondFormat {
                range: RangeRef::from_corners(
                    CellRef::new(1, 1).unwrap(),
                    CellRef::new(1, 1).unwrap(),
                ),
                priority: 1,
                stop_if_true: true,
                kind: CfKind::CellIs {
                    op: CfOp::Greater,
                    formula1: "0".into(),
                    formula2: None,
                },
                dxf: CfDxf {
                    fill: Some(Color::Rgb { argb: 0xFF12_3456 }),
                    font: None,
                },
            }],
        )
        .unwrap();
    let parts = launch_opts(None, workbook, false);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());

    harness.step();
    let snapshot = harness.state().runner().snapshot();
    let started = Instant::now();
    while harness
        .state()
        .runner()
        .conditional_formats(&snapshot, sheet)
        .is_none()
    {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    harness.step();
    let image = harness.render().unwrap();
    assert!(
        image
            .pixels()
            .any(|pixel| pixel.0[..3] == [0x12, 0x34, 0x56]),
        "conditional-format fill was not painted"
    );
}

#[test]
fn retained_lua_runtime_loads_hooks_keymaps_and_source() {
    let parts = launch_script(
        r#"
        omacell.ui.status("lua loaded")
        omacell.keymap.set("classic", "Ctrl+L", "cell.clear")
        omacell.on_change(function() omacell.ui.status("lua changed") end)
        "#,
    );
    let script = parts.launch.paths.user_config.join("init.lua");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    assert_eq!(harness.state().message(), Some("lua loaded"));

    harness
        .state_mut()
        .execute_cmd("cell.set", serde_json::json!({"ref": "A1", "input": "1"}))
        .unwrap();
    let started = Instant::now();
    while harness.state().runner().is_busy() {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    harness.step();
    assert_eq!(harness.state().message(), Some("lua changed"));
    assert!(matches!(
        harness.state().ui_session().handle_key(KeyEvent {
            code: KeyCode::Char('l'),
            ctrl: true,
            alt: false,
            shift: false,
        }),
        omacell_ui::KeyOutcome::Command { cmd, .. } if cmd == "cell.clear"
    ));

    std::fs::write(
        script,
        r#"
        omacell.ui.status("lua reloaded")
        omacell.keymap.set("classic", "Ctrl+J", "cell.clear")
        "#,
    )
    .unwrap();
    harness
        .state_mut()
        .execute_cmd("script.source", serde_json::json!({}))
        .unwrap();
    let started = Instant::now();
    while harness.state().runner().is_busy() {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    harness.step();
    assert_eq!(harness.state().message(), Some("lua reloaded"));
    let keymap = harness.state().ui_session().keymap();
    let classic = keymap.table(omacell_ui::Mode::Classic).unwrap();
    assert_ne!(
        classic.get("Ctrl+L").map(|binding| binding.cmd.as_str()),
        Some("cell.clear")
    );
    assert_eq!(classic["Ctrl+J"].cmd, "cell.clear");
}

#[test]
fn key_and_text_events_insert_one_character_in_cell() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state()
        .ui_session()
        .begin_edit(EditSurface::InCell, "");
    harness.input_mut().events.extend([
        Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        },
        Event::Text("a".into()),
    ]);

    harness.step();

    assert_eq!(harness.state().ui_session().edit().buffer, "a");
}

#[test]
fn toolkit_clipboard_events_copy_internal_cells_and_paste_external_tables() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness.input_mut().events.push(Event::Copy);
    harness.step();
    wait_tasks(&mut harness);
    let clipboard = harness
        .state()
        .ui_session()
        .clipboard()
        .expect("copy should retain the rich clipboard payload");
    assert_eq!(clipboard.tsv, "Hello");

    harness
        .state_mut()
        .execute_cmd("view.select", serde_json::json!({"range": "B2"}))
        .unwrap();
    harness
        .input_mut()
        .events
        .push(Event::Paste("7\t8\n9\t10".into()));
    harness.step();
    wait_tasks(&mut harness);

    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert_eq!(
        snapshot.workbook.get(sheet, 1, 1).unwrap().unwrap().value,
        Value::Number(7.0)
    );
    assert_eq!(
        snapshot.workbook.get(sheet, 2, 2).unwrap().unwrap().value,
        Value::Number(10.0)
    );
}

#[test]
fn fill_handle_and_drag_move_execute_atomic_edit_commands() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    let a1 = harness.get_by_label_contains("cell A1").rect();
    let handle = egui::pos2(a1.right() - 2.0, a1.bottom() - 2.0);
    let a3 = egui::pos2(a1.center().x, a1.center().y + a1.height() * 2.0);
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: handle,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerMoved(a3),
        Event::PointerButton {
            pos: a3,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    harness.step();
    wait_tasks(&mut harness);
    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    for row in 0..=2 {
        let slot = snapshot.workbook.get(sheet, row, 0).unwrap().unwrap();
        let Value::Text(id) = slot.value else {
            panic!("expected filled text in A{}, got {:?}", row + 1, slot.value);
        };
        assert_eq!(snapshot.workbook.intern().strings.get(id), Some("Hello"));
    }

    harness
        .state_mut()
        .execute_cmd("view.select", serde_json::json!({"range": "A1:B1"}))
        .unwrap();
    harness.step();
    let a1 = harness.get_by_label_contains("cell A1").rect();
    let press = a1.center();
    let c3 = press + egui::vec2(a1.width() * 2.0, a1.height() * 2.0);
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: press,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerMoved(c3),
        Event::PointerButton {
            pos: c3,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    harness.step();
    wait_tasks(&mut harness);
    let snapshot = harness.state().runner().snapshot();
    assert!(snapshot.workbook.get(sheet, 0, 0).unwrap().is_none());
    assert!(snapshot.workbook.get(sheet, 0, 1).unwrap().is_none());
    assert!(snapshot.workbook.get(sheet, 2, 2).unwrap().is_some());
    assert!(snapshot.workbook.get(sheet, 2, 3).unwrap().is_some());
}

#[test]
fn header_resize_persists_through_writer_commands() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    let a1 = harness.get_by_label_contains("cell A1").rect();
    let col_edge = egui::pos2(a1.right(), a1.top() - 10.0);
    let wider = col_edge + egui::vec2(24.0, 0.0);
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: col_edge,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerMoved(wider),
        Event::PointerButton {
            pos: wider,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    harness.step();
    wait_tasks(&mut harness);
    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert_eq!(
        snapshot
            .workbook
            .sheet(sheet)
            .unwrap()
            .geometry
            .cols
            .size(0)
            .unwrap(),
        88
    );

    harness.step();
    let a1 = harness.get_by_label_contains("cell A1").rect();
    let row_edge = egui::pos2(a1.left() - 24.0, a1.bottom());
    let taller = row_edge + egui::vec2(0.0, 12.0);
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: row_edge,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerMoved(taller),
        Event::PointerButton {
            pos: taller,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    harness.step();
    wait_tasks(&mut harness);
    let snapshot = harness.state().runner().snapshot();
    assert_eq!(
        snapshot
            .workbook
            .sheet(sheet)
            .unwrap()
            .geometry
            .rows
            .size(0)
            .unwrap(),
        32
    );

    harness
        .state_mut()
        .execute_cmd("edit.undo", serde_json::json!({}))
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(harness.state().ui_session().viewport().row_px(0), 20);
    harness
        .state_mut()
        .execute_cmd("edit.undo", serde_json::json!({}))
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(harness.state().ui_session().viewport().col_px(0), 64);
}

#[test]
fn ctrl_drag_copies_the_selected_range_without_clearing_it() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state_mut()
        .execute_cmd("view.select", serde_json::json!({"range": "A1:B1"}))
        .unwrap();
    harness.step();
    let a1 = harness.get_by_label_contains("cell A1").rect();
    let press = a1.center();
    let c3 = press + egui::vec2(a1.width() * 2.0, a1.height() * 2.0);
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: press,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::CTRL,
        },
        Event::PointerMoved(c3),
        Event::PointerButton {
            pos: c3,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::CTRL,
        },
    ]);
    harness.step();
    wait_tasks(&mut harness);

    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert!(snapshot.workbook.get(sheet, 0, 0).unwrap().is_some());
    assert!(snapshot.workbook.get(sheet, 0, 1).unwrap().is_some());
    assert!(snapshot.workbook.get(sheet, 2, 2).unwrap().is_some());
    assert!(snapshot.workbook.get(sheet, 2, 3).unwrap().is_some());
}

#[test]
fn multi_cell_fill_handle_extends_the_selected_series_from_its_bottom_right() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state_mut()
        .execute_cmd("cell.set", serde_json::json!({"ref": "A1", "input": "1"}))
        .unwrap();
    harness
        .state_mut()
        .execute_cmd("cell.set", serde_json::json!({"ref": "A2", "input": "2"}))
        .unwrap();
    wait_tasks(&mut harness);
    harness
        .state_mut()
        .execute_cmd("view.select", serde_json::json!({"range": "A1:A2"}))
        .unwrap();
    harness.step();
    let a1 = harness.get_by_label_contains("cell A1").rect();
    let handle = egui::pos2(a1.right() - 2.0, a1.bottom() + a1.height() - 2.0);
    let a4 = egui::pos2(a1.center().x, a1.center().y + a1.height() * 3.0);
    harness.input_mut().events.extend([
        Event::PointerButton {
            pos: handle,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
        Event::PointerMoved(a4),
        Event::PointerButton {
            pos: a4,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        },
    ]);
    harness.step();
    wait_tasks(&mut harness);

    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert_eq!(
        snapshot.workbook.get(sheet, 2, 0).unwrap().unwrap().value,
        Value::Number(3.0)
    );
    assert_eq!(
        snapshot.workbook.get(sheet, 3, 0).unwrap().unwrap().value,
        Value::Number(4.0)
    );
}

#[test]
fn find_panel_accepts_text_and_enter_selects_the_next_match() {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_cell_contents(sheet, 1, 1, "Hello").unwrap();
    let other = workbook.add_sheet("Other").unwrap();
    workbook.set_cell_contents(other, 0, 2, "Hello").unwrap();
    let parts = launch_opts(None, workbook, false);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    let mut find = harness.state().ui_session().find_replace();
    find.scope = FindScope::Workbook;
    harness.state().ui_session().set_find_replace(find);

    harness
        .state_mut()
        .step_key(KeyEvent {
            code: KeyCode::Char('f'),
            ctrl: true,
            alt: false,
            shift: false,
        })
        .unwrap();
    harness.input_mut().events.push(Event::Text("Hello".into()));
    harness.step();
    assert_eq!(harness.state().ui_session().find_replace().find, "Hello");
    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("find")
    );
    assert_eq!(harness.state().ui_session().selection().cursor.col, 0);
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Enter))
        .unwrap();
    assert!(harness.state().ui_session().panel().visible.is_none());
    let selection = harness.state().ui_session().selection();
    assert!(
        harness.state().runner().tracked_tasks() != 0
            || (selection.cursor.row, selection.cursor.col) == (1, 1),
        "the search task must be pending or already applied"
    );
    let started = Instant::now();
    while harness.state().runner().is_busy()
        || harness
            .state()
            .message()
            .is_some_and(|text| text == "queued…")
    {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    assert_eq!(
        (
            harness.state().ui_session().selection().cursor.row,
            harness.state().ui_session().selection().cursor.col,
        ),
        (1, 1),
        "message: {:?}",
        harness.state().message()
    );
    assert!(harness.state().ui_session().panel().visible.is_none());

    let outcome = harness
        .state_mut()
        .execute_cmd("edit.searchnext", serde_json::json!({}))
        .unwrap();
    assert!(outcome.ok, "{:?}", outcome.error);
    let started = Instant::now();
    while harness.state().runner().is_busy()
        || harness
            .state()
            .message()
            .is_some_and(|text| text == "queued…")
    {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    assert_eq!(harness.state().ui_session().selection().sheet, other);
    assert_eq!(harness.state().ui_session().selection().cursor.col, 2);
}

#[test]
fn dropping_the_gui_persists_session_state() {
    let parts = launch_theme(None);
    let state_file = parts.launch.paths.state_dir.join("session.toml");
    let harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());

    drop(harness);

    assert!(state_file.is_file());
}

#[test]
fn clicking_a_sheet_tab_updates_the_saved_workbook_active_sheet() {
    let mut workbook = Workbook::new();
    let data = workbook.add_sheet("Data").unwrap();
    let parts = launch_opts(None, workbook, false);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1024.0, 600.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness.get_by_label("Data").click();
    harness.run();
    let started = Instant::now();
    while harness.state().runner().is_busy() {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    assert_eq!(
        harness.state().runner().snapshot().workbook.active_sheet(),
        data
    );
    assert_eq!(harness.state().ui_session().selection().sheet, data);
}

#[test]
fn file_lifecycle_commands_confirm_discard_and_reconcile_frontend_state() {
    let parts = launch_theme(None);
    let save_as = parts._dir.path().join("renamed.csv");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .execute_cmd(
            "cell.set",
            serde_json::json!({"ref": "A1", "input": "changed"}),
        )
        .unwrap();
    wait_tasks(&mut harness);
    assert!(harness.state().title().starts_with('•'));

    harness
        .state_mut()
        .execute_cmd("file.new", serde_json::json!({}))
        .unwrap();
    assert!(harness.state().message().unwrap().contains("unsaved"));
    assert!(!harness.state().runner().is_busy());
    harness
        .state_mut()
        .execute_cmd("file.new", serde_json::json!({}))
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(harness.state().title(), "untitled — Omacell");
    let snapshot = harness.state().runner().snapshot();
    assert!(
        snapshot
            .workbook
            .get(snapshot.workbook.active_sheet(), 0, 0)
            .unwrap()
            .is_none()
    );

    harness
        .state_mut()
        .execute_cmd(
            "cell.set",
            serde_json::json!({"ref": "A1", "input": "saved"}),
        )
        .unwrap();
    wait_tasks(&mut harness);
    harness
        .state_mut()
        .execute_cmd(
            "file.saveas",
            serde_json::json!({"path": save_as.display().to_string()}),
        )
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(harness.state().title(), "renamed.csv — Omacell");

    harness
        .state_mut()
        .execute_cmd("file.close", serde_json::json!({}))
        .unwrap();
    assert!(harness.state().close_requested());
}

#[test]
fn required_argument_key_opens_the_schema_prompt() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::F(12)))
        .unwrap();

    let palette = harness.state().ui_session().palette();
    assert!(palette.open);
    assert!(palette.prompt.unwrap().contains("path"));
    assert!(!harness.state().runner().is_busy());
}

#[test]
fn workbook_panel_commands_open_live_shared_content() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .execute_cmd(
            "edit.note",
            serde_json::json!({"ref": "C2", "text": "check this", "author": "Ada"}),
        )
        .unwrap();
    wait_tasks(&mut harness);
    harness
        .state_mut()
        .execute_cmd("comments.panel", serde_json::json!({}))
        .unwrap();
    assert!(
        harness
            .state()
            .ui_session()
            .panel()
            .body
            .as_deref()
            .unwrap()
            .contains("C2  note by Ada")
    );

    harness
        .state_mut()
        .execute_cmd("format.panel", serde_json::json!({"range": "A1"}))
        .unwrap();
    wait_tasks(&mut harness);
    let panel = harness.state().ui_session().panel();
    assert_eq!(panel.visible.as_deref(), Some("format"));
    assert!(
        panel
            .body
            .as_deref()
            .unwrap()
            .contains("Number format: General")
    );
}

#[test]
fn name_keys_open_schema_prompts_with_selection_context() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::F(3)))
        .unwrap();
    let palette = harness.state().ui_session().palette();
    assert!(palette.open);
    assert!(palette.prompt.unwrap().contains("name"));
    assert!(palette.query.is_empty());

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Esc))
        .unwrap();
    harness
        .state_mut()
        .step_key(KeyEvent {
            code: KeyCode::F(3),
            ctrl: true,
            alt: false,
            shift: true,
        })
        .unwrap();
    let palette = harness.state().ui_session().palette();
    assert!(palette.open);
    assert!(palette.prompt.unwrap().contains("positions"));
    assert!(
        palette.query.contains(r#""range":"A1:A1""#),
        "palette query: {:?}",
        palette.query
    );
    assert!(!harness.state().runner().is_busy());
}

#[test]
fn ai_assist_key_opens_the_formula_workflow_picker_locally() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .step_key(KeyEvent {
            code: KeyCode::Char('x'),
            ctrl: true,
            alt: false,
            shift: true,
        })
        .unwrap();

    let palette = harness.state().ui_session().palette();
    assert!(palette.open);
    assert_eq!(palette.query, "ai.formula.");
    assert!(palette.prompt.unwrap().contains("AI assist"));
    assert!(!harness.state().runner().is_busy());
}

#[test]
fn changeset_review_toggles_one_plan_item_before_apply() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    let call = |cell: &str, input: &str| CommandCall {
        id: CommandId::new("cell.set").unwrap(),
        args: serde_json::json!({"ref": cell, "input": input}),
    };
    harness
        .state()
        .runner()
        .propose(
            Origin::PalettePlan,
            vec![call("D5", "reject"), call("E5", "accept")],
        )
        .unwrap();
    harness
        .state_mut()
        .execute_cmd("changeset.review", serde_json::json!({}))
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("changeset")
    );
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Space))
        .unwrap();
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Enter))
        .unwrap();

    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    assert!(snapshot.workbook.get(sheet, 4, 3).unwrap().is_none());
    let slot = snapshot.workbook.get(sheet, 4, 4).unwrap().unwrap();
    let Value::Text(text) = slot.value else {
        panic!("expected accepted text");
    };
    assert_eq!(snapshot.workbook.intern().strings.get(text), Some("accept"));
    assert!(harness.state().ui_session().panel().visible.is_none());
}

#[test]
fn agent_turn_opens_review_and_returns_to_retained_panel() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state_mut()
        .execute_cmd(
            "ai.agent.turn",
            serde_json::json!({"prompt":"set D6", "apply":false}),
        )
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("changeset")
    );
    assert_eq!(
        harness
            .state()
            .ui_session()
            .changeset_review()
            .unwrap()
            .origin,
        Origin::InAppAgent
    );
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Enter))
        .unwrap();
    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("agent")
    );
    assert!(
        harness
            .state()
            .ui_session()
            .agent_panel()
            .body()
            .contains("Proposed 1")
    );
}

#[test]
fn formula_assist_opens_a_reviewable_validated_proposal() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state_mut()
        .execute_cmd(
            "ai.formula.generate",
            serde_json::json!({"prompt":"sum the inputs", "ref":"E5"}),
        )
        .unwrap();
    wait_tasks(&mut harness);

    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("formula")
    );
    let assist = harness.state().ui_session().formula_assist().unwrap();
    assert_eq!(assist.scratch.as_deref(), Some("Number(6)"));
    assert_eq!(assist.references.len(), 2);
    assert!(harness.state().ui_session().changeset_review().is_some());

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Enter))
        .unwrap();
    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    let slot = snapshot.workbook.get(sheet, 4, 4).unwrap().unwrap();
    assert_eq!(
        snapshot
            .workbook
            .intern()
            .formulas
            .get(slot.formula.unwrap()),
        Some("=SUM(B1:C1)+D2")
    );
}

#[test]
fn csv_import_assist_is_reviewed_before_reopening_with_the_plan() {
    let parts = launch_theme(None);
    let open_count = parts.open_count.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state_mut()
        .execute_cmd("file.open", serde_json::json!({"path":"readings.csv"}))
        .unwrap();
    wait_tasks(&mut harness);

    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("import")
    );
    assert!(
        harness
            .state()
            .ui_session()
            .import_review()
            .unwrap()
            .proposed
            .is_none()
    );

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Char('a')))
        .unwrap();
    wait_tasks(&mut harness);
    let review = harness.state().ui_session().import_review().unwrap();
    assert_eq!(
        review.proposed.unwrap().columns[0].name.as_deref(),
        Some("Pressure")
    );

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Enter))
        .unwrap();
    wait_tasks(&mut harness);
    assert_eq!(open_count.load(Ordering::SeqCst), 2);
    assert!(harness.state().ui_session().panel().visible.is_none());
    assert!(harness.state().ui_session().import_review().is_none());
}

#[test]
fn inline_completion_is_debounced_and_tab_accepts_the_ghost() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    harness
        .state()
        .ui_session()
        .begin_edit(omacell_ui::EditSurface::FormulaBar, "=SU");
    let started = Instant::now();
    while harness.state().ui_session().edit().ghost.is_none() {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::yield_now();
    }
    assert_eq!(
        harness.state().ui_session().edit().ghost.as_deref(),
        Some("M(A1:A3)")
    );
    let outcome = harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Tab))
        .unwrap();
    assert_eq!(outcome, omacell_ui::KeyOutcome::Pending);
    let edit = harness.state().ui_session().edit();
    assert_eq!(edit.buffer, "=SUM(A1:A3)");
    assert!(!edit.is_idle());
}

fn wait_tasks(harness: &mut Harness<'_, Gui>) {
    let started = Instant::now();
    while harness.state().runner().tracked_tasks() != 0 {
        harness.step();
        assert!(started.elapsed() < Duration::from_secs(5));
        // The task runner completes work on another thread. Yield explicitly so
        // this polling loop cannot starve it on a single-core or loaded CI host.
        std::thread::yield_now();
    }
    harness.step();
}

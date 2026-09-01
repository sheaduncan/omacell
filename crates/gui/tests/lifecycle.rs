//! GUI lifecycle and toolkit-event integration regressions.

mod common;

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::{launch_opts, launch_theme};
use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use omacell_core::workbook::Workbook;
use omacell_gui::Gui;
use omacell_ui::{EditSurface, FindScope, KeyCode, KeyEvent};

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
    assert_ne!(harness.state().runner().tracked_tasks(), 0);
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

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
use omacell_ui::EditSurface;

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

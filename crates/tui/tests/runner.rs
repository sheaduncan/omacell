//! TUI interaction while the writer is held.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use omacell_bus::{Bus, LongOps, register_hold_command};
use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_tui::{Launch, Tui};
use omacell_ui::{KeyCode, KeyEvent, KeymapRoots, UiSession, register_ui_commands};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

fn tui_with_hold(start: Arc<Barrier>, release: Arc<AtomicBool>) -> (tempfile::TempDir, Tui) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let store = ConfigStore::load_with(paths.clone(), LoadOptions::default()).unwrap();
    let loaded = store.snapshot();
    let roots = KeymapRoots::new(paths.user_config.clone(), paths.default_dir.clone(), None);
    let ui = UiSession::new(&loaded, &roots).unwrap();
    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(functions)).unwrap();
    omacell_bus::register_chart_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_edit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_data_commands(bus.registry_mut()).unwrap();
    register_ui_commands(bus.registry_mut(), &ui).unwrap();
    register_hold_command(bus.registry_mut(), start, release).unwrap();
    let tui = Tui::new(
        Launch {
            paths,
            store,
            bus,
            ui,
            roots,
            long_ops: LongOps::production(),
        },
        false,
    )
    .unwrap();
    (dir, tui)
}

#[test]
fn paint_and_nav_stay_responsive_while_writer_held() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let (_dir, mut tui) = tui_with_hold(Arc::clone(&start), Arc::clone(&release));
    tui.execute_cmd("test.hold", json!({})).unwrap();
    start.wait();

    let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
    let t0 = Instant::now();
    tui.draw(&mut terminal).unwrap();
    let paint_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let paint_limit = if cfg!(debug_assertions) { 100.0 } else { 16.0 };
    assert!(
        paint_ms < paint_limit,
        "paint while writer held took {paint_ms:.2} ms (limit {paint_limit})"
    );

    let before = tui.ui().selection().cursor.col;
    let t1 = Instant::now();
    tui.step_key(KeyEvent::new(KeyCode::Right)).unwrap();
    let nav_ms = t1.elapsed().as_secs_f64() * 1000.0;
    assert!(nav_ms < 50.0, "nav while writer held took {nav_ms:.2} ms");
    assert_eq!(tui.ui().selection().cursor.col, before + 1);

    let t2 = Instant::now();
    tui.step_key(KeyEvent {
        code: KeyCode::Char('='),
        ctrl: true,
        alt: true,
        shift: false,
    })
    .ok();
    let _ = tui.ui().viewport();
    let zoom_ms = t2.elapsed().as_secs_f64() * 1000.0;
    assert!(zoom_ms < 50.0, "zoom path took {zoom_ms:.2} ms");

    tui.step_key(KeyEvent::new(KeyCode::Esc)).unwrap();
    release.store(true, Ordering::SeqCst);
}

#[test]
fn esc_cancels_without_closing_help_panel() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let (_dir, mut tui) = tui_with_hold(Arc::clone(&start), Arc::clone(&release));
    tui.execute_cmd("help.keys", json!({})).unwrap();
    assert_eq!(tui.ui().panel().visible.as_deref(), Some("keys"));
    tui.execute_cmd("test.hold", json!({})).unwrap();
    start.wait();
    tui.step_key(KeyEvent::new(KeyCode::Esc)).unwrap();
    assert_eq!(tui.ui().panel().visible.as_deref(), Some("keys"));
    common::wait_tasks(&mut tui);
    assert_eq!(tui.message(), Some("operation cancelled"));
    release.store(true, Ordering::SeqCst);
}

#[test]
fn completed_and_failed_tasks_reconcile_status() {
    let start = Arc::new(Barrier::new(1));
    let release = Arc::new(AtomicBool::new(true));
    let (_dir, mut tui) = tui_with_hold(start, release);
    let queued = tui
        .execute_cmd("cell.set", json!({"ref": "A1", "input": "7"}))
        .unwrap();
    assert!(queued.ok);
    assert!(tui.has_pending_tasks());
    common::wait_tasks(&mut tui);
    assert!(tui.message().is_none());
    assert!(tui.is_dirty());

    tui.execute_cmd("does.not.exist", json!({})).unwrap();
    common::wait_tasks(&mut tui);
    assert!(
        tui.message()
            .is_some_and(|message| message.contains("unknown"))
    );
}

#[test]
fn resize_and_shutdown_with_task_in_flight() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let (_dir, mut tui) = tui_with_hold(Arc::clone(&start), Arc::clone(&release));
    tui.execute_cmd("test.hold", json!({})).unwrap();
    start.wait();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    tui.draw(&mut terminal).unwrap();
    drop(terminal);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    tui.draw(&mut terminal).unwrap();
    drop(tui);
    release.store(true, Ordering::SeqCst);
}

//! Frame budget: redraw a 200×60 window over a 1M-row sheet.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use omacell_bus::Bus;
use omacell_bus::LongOps;
use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_tui::{Launch, Tui};
use omacell_ui::{KeymapRoots, UiSession, register_ui_commands};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

fn setup() -> (tempfile::TempDir, Tui, Terminal<TestBackend>) {
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
    omacell_bus::register_audit_commands(bus.registry_mut()).unwrap();
    register_ui_commands(bus.registry_mut(), &ui).unwrap();
    let mut tui = Tui::new(
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
    let result = tui
        .execute_cmd("cell.set", json!({"ref": "A1048576", "input": "1"}))
        .unwrap();
    assert!(result.ok, "{:?}", result.error);
    let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
    tui.draw(&mut terminal).unwrap();
    (dir, tui, terminal)
}

fn redraw_200x60_1m(c: &mut Criterion) {
    let (_dir, tui, mut terminal) = setup();
    c.bench_function("tui_redraw_200x60_1m", |b| {
        b.iter(|| {
            tui.draw(black_box(&mut terminal)).unwrap();
        });
    });
}

criterion_group!(benches, redraw_200x60_1m);
criterion_main!(benches);

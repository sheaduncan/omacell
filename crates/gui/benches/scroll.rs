//! Software-raster scroll budget on a 1M-row sheet.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use egui_kittest::Harness;
use omacell_bus::{Bus, LongOps};
use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_gui::{Gui, Launch};
use omacell_ui::{KeymapRoots, UiSession, register_ui_commands};
use serde_json::json;

fn setup() -> Harness<'static, Gui> {
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
    let result = bus.execute(
        omacell_core::command::Origin::User,
        "cell.set",
        json!({"ref": "A1048576", "input": "1"}),
    );
    assert!(result.ok, "{:?}", result.error);
    let launch = Launch {
        paths,
        store,
        bus,
        ui,
        roots,
        long_ops: LongOps::production(),
        file: None,
        use_shell_font: false,
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_eframe(|cc| Gui::new(launch, false, &cc.egui_ctx).unwrap());
    harness.step();
    black_box(harness.render().expect("warm software render"));
    std::mem::forget(dir);
    harness
}

fn scroll_1m_software(c: &mut Criterion) {
    let mut harness = setup();
    let mut row = 0u32;
    c.bench_function("gui_scroll_1m_software", |b| {
        b.iter(|| {
            row = row.wrapping_add(7_919) % (omacell_core::limits::MAX_ROWS - 32);
            let mut viewport = harness.state().ui_session().viewport();
            viewport.first_row = row;
            harness.state().ui_session().set_viewport(viewport);
            harness.step();
            black_box(harness.render().expect("software render"));
            black_box(row);
        });
    });
}

criterion_group!(benches, scroll_1m_software);
criterion_main!(benches);

//! AccessKit tree exposes the focused cell.

mod common;

use common::launch_theme;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use omacell_gui::Gui;

#[test]
fn focused_cell_is_in_the_accesskit_tree() {
    let parts = launch_theme(Some("nord"));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    let node = harness.get_by_label_contains("cell A1");
    let text = node
        .accesskit_node()
        .value()
        .or_else(|| node.accesskit_node().label())
        .unwrap_or_default();
    assert!(text.contains("A1"), "{text}");
    assert!(text.contains("Hello") || text.contains("value"), "{text}");
}

#[test]
fn first_frame_stays_within_software_ci_regression_budget() {
    // The product's 300 ms target is gated on the fixed integrated-GPU
    // reference host (WP-28). This catches large regressions on lavapipe CI.
    let start = std::time::Instant::now();
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    let _ = harness.render().expect("render first frame");
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("gui_first_frame_ms={ms:.2}");
    assert!(
        ms < 500.0,
        "first software kittest frame took {ms:.2} ms (500 ms CI regression budget)"
    );
}

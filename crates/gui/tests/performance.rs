//! CI software-render scrolling gate across the full row address space.

mod common;

use std::time::{Duration, Instant};

use common::{fixed_gpu_setup, graphics_adapter_available, launch_theme};
use egui_kittest::Harness;
use omacell_core::limits::MAX_ROWS;
use omacell_gui::Gui;

#[test]
#[ignore = "nightly wall-clock performance gate"]
fn scrolling_the_million_row_space_stays_within_the_regression_budget() {
    if !graphics_adapter_available() {
        return;
    }
    let parts = launch_theme(None);
    let mut builder = Harness::builder().with_size(egui::vec2(800.0, 600.0));
    if let Some(setup) = fixed_gpu_setup() {
        builder = builder.wgpu_setup(setup);
    }
    let mut harness =
        builder.build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    for row in [0, 1_000, 2_000, 3_000] {
        let mut viewport = harness.state().ui_session().viewport();
        viewport.first_row = row;
        harness.state().ui_session().set_viewport(viewport);
        harness.step();
        let _ = harness.render().expect("warm software render");
    }
    let release_budget = Duration::from_micros(36_300); // 33 ms target + 10%.
    // Unoptimized wgpu plus synchronous screenshot readback is not a product
    // frame measurement. Keep it bounded in `just check`; release owns the
    // actual software-render target.
    let budget = if cfg!(debug_assertions) {
        release_budget * 7
    } else {
        release_budget
    };
    let mut frames = Vec::new();

    for sample in 0..24_u64 {
        let row = (sample * u64::from(MAX_ROWS - 32) / 23) as u32;
        let mut viewport = harness.state().ui_session().viewport();
        viewport.first_row = row;
        harness.state().ui_session().set_viewport(viewport);
        let started = Instant::now();
        harness.step();
        let _ = harness.render().expect("render scrolled frame");
        frames.push(started.elapsed());
    }

    frames.sort_unstable();
    let mean = frames.iter().sum::<Duration>() / frames.len() as u32;
    let p95 = frames[(frames.len() * 95).div_ceil(100) - 1];
    eprintln!(
        "gui_scroll_software_mean_ms={:.2} p95_ms={:.2}",
        mean.as_secs_f64() * 1_000.0,
        p95.as_secs_f64() * 1_000.0,
    );
    assert!(
        mean < budget,
        "mean software frame took {:.2} ms (budget {:.2} ms)",
        mean.as_secs_f64() * 1_000.0,
        budget.as_secs_f64() * 1_000.0,
    );
    assert!(
        p95 < budget * 2,
        "p95 software frame took {:.2} ms (guard {:.2} ms)",
        p95.as_secs_f64() * 1_000.0,
        (budget * 2).as_secs_f64() * 1_000.0,
    );
}

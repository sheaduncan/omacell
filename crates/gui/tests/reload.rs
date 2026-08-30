//! Theme reload mid-edit preserves the buffer and stays under 100 ms.

mod common;

use std::time::{Duration, Instant};

use common::{fixture_theme, launch_theme, launch_watched};
use egui_kittest::Harness;
use omacell_gui::Gui;
use omacell_ui::EditSurface;
use serde_json::json;

#[test]
fn theme_reload_preserves_edit_and_is_fast() {
    let parts = launch_theme(Some("tokyo-night"));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| {
            let gui = Gui::new(parts.launch, false, &cc.egui_ctx).unwrap();
            gui.ui_session()
                .begin_edit(EditSurface::FormulaBar, "=A1+B1");
            gui
        });
    harness.run();
    assert_eq!(harness.state().ui_session().edit().buffer, "=A1+B1");
    assert!(!harness.state().ui_session().edit().is_idle());
    let before_theme = harness.state().theme_name().to_string();
    let theme_dir = harness.state().paths().omarchy_state.join("current/theme");

    std::fs::copy(fixture_theme("nord"), theme_dir.join("colors.toml")).unwrap();
    std::fs::write(
        harness
            .state()
            .paths()
            .omarchy_state
            .join("current/theme.name"),
        "nord",
    )
    .unwrap();

    let ctx = harness.ctx.clone();
    let t0 = Instant::now();
    harness.state().store().reload().unwrap();
    harness.state_mut().poll(&ctx);
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert!(ms < 100.0, "theme reload took {ms:.2} ms (gate 100 ms)");
    assert_eq!(harness.state().ui_session().edit().buffer, "=A1+B1");
    assert!(!harness.state().ui_session().edit().is_idle());
    assert_ne!(harness.state().theme_name(), before_theme);
}

#[test]
fn watcher_ipc_and_direct_reload_share_one_store() {
    let parts = launch_watched(Some("nord"));
    let paths = parts.launch.paths.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| {
            let gui = Gui::new(parts.launch, false, &cc.egui_ctx).unwrap();
            gui.ui_session().begin_edit(EditSurface::InCell, "keep-me");
            gui
        });
    harness.run();
    let first = harness.state().store().snapshot().theme.roles.clone();

    harness.state().store().reload().unwrap();
    let ctx = harness.ctx.clone();
    harness.state_mut().poll(&ctx);
    let direct = harness.state().store().snapshot().theme.roles.clone();
    assert_eq!(first, direct);
    assert_eq!(harness.state().ui_session().edit().buffer, "keep-me");

    let _ = harness
        .state_mut()
        .execute_cmd("theme.reload", json!({}))
        .unwrap();
    wait_idle(&mut harness);
    let via_command = harness.state().store().snapshot().theme.roles.clone();
    assert_eq!(first, via_command);
    assert_eq!(harness.state().ui_session().edit().buffer, "keep-me");

    std::fs::write(
        paths.user_config.join("config.toml"),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    let saw = wait_for_applied(&mut harness);
    assert!(saw, "expected watcher reload to apply");
    assert_eq!(harness.state().ui_session().edit().buffer, "keep-me");
    assert!(!harness.state().ui_session().config().appearance.grid_lines);
}

fn wait_idle(harness: &mut Harness<'_, Gui>) {
    let started = Instant::now();
    loop {
        harness.step();
        if !harness.state().runner().is_busy() {
            let ctx = harness.ctx.clone();
            harness.state_mut().poll(&ctx);
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "theme.reload task did not finish"
        );
        std::thread::yield_now();
    }
}

fn wait_for_applied(harness: &mut Harness<'_, Gui>) -> bool {
    let ctx = harness.ctx.clone();
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
        harness.state_mut().poll(&ctx);
        if !harness.state().ui_session().config().appearance.grid_lines {
            return true;
        }
    }
    false
}

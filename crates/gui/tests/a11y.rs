//! AccessKit tree exposes the focused cell.

mod common;

use common::{fixed_gpu_setup, launch_theme};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use omacell_gui::Gui;
use omacell_ui::{KeyCode, KeyEvent};

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
fn palette_panel_and_status_actions_are_in_the_accesskit_tree() {
    let parts = launch_theme(Some("nord"));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .execute_cmd("palette.open", serde_json::json!({}))
        .unwrap();
    harness.step();
    assert_eq!(
        harness
            .get_by_label("Command palette")
            .accesskit_node()
            .role(),
        Role::Window
    );

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Esc))
        .unwrap();
    harness
        .state_mut()
        .execute_cmd("nav.goto", serde_json::json!({}))
        .unwrap();
    harness.step();
    assert!(harness.get_by_label("Go to").accesskit_node().role() == Role::Label);

    let zoom = harness.get_by_label("Zoom 100%");
    assert_eq!(zoom.accesskit_node().role(), Role::Button);
}

#[test]
fn keyboard_only_grid_palette_and_panel_walkthrough() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Right))
        .unwrap();
    assert_eq!(harness.state().ui_session().selection().cursor.col, 1);
    harness
        .state_mut()
        .step_key(KeyEvent {
            code: KeyCode::Char('p'),
            ctrl: true,
            alt: false,
            shift: true,
        })
        .unwrap();
    assert!(harness.state().ui_session().palette().open);
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Esc))
        .unwrap();
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::F(1)))
        .unwrap();
    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("keys")
    );
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Esc))
        .unwrap();
    assert!(harness.state().ui_session().panel().visible.is_none());
}

#[test]
fn print_preview_opens_a_keyboard_accessible_printer_panel() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    harness
        .state_mut()
        .execute_cmd("file.print", serde_json::json!({}))
        .unwrap();
    for _ in 0..100 {
        harness.step();
        if harness.state().ui_session().panel().visible.as_deref() == Some("print") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        harness.state().ui_session().panel().visible.as_deref(),
        Some("print")
    );
    assert!(
        harness
            .state()
            .ui_session()
            .panel()
            .body
            .as_deref()
            .is_some_and(|body| body.contains("> lab (last used)"))
    );
    assert_eq!(
        harness.get_by_label("Print").accesskit_node().role(),
        Role::Label
    );
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Up))
        .unwrap();
    assert!(
        harness
            .state()
            .ui_session()
            .panel()
            .body
            .as_deref()
            .is_some_and(|body| body.contains("> office"))
    );
    harness
        .state_mut()
        .step_key(KeyEvent::new(KeyCode::Esc))
        .unwrap();
    assert!(harness.state().ui_session().panel().visible.is_none());
}

#[test]
fn chart_release_edits_are_reachable_through_the_gui_command_surface() {
    let parts = launch_theme(None);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(640.0, 400.0))
        .build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();

    for (command, args) in [
        (
            "chart.fromselection",
            serde_json::json!({"range": "A1:B2", "kind": "combo"}),
        ),
        ("chart.resize", serde_json::json!({"range": "C3:H12"})),
        (
            "chart.title",
            serde_json::json!({"title": "Quarterly sales"}),
        ),
        (
            "chart.axistitle",
            serde_json::json!({"axis": "category", "title": "Quarter"}),
        ),
    ] {
        harness.state_mut().execute_cmd(command, args).unwrap();
        for _ in 0..200 {
            harness.step();
            if !harness.state().runner().is_busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!harness.state().runner().is_busy(), "{command}");
    }

    let snapshot = harness.state().runner().snapshot();
    let sheet = snapshot.workbook.active_sheet();
    let chart = &snapshot.workbook.sheet(sheet).unwrap().charts[0];
    assert_eq!(chart.anchor.from_row, 2);
    assert_eq!(chart.anchor.from_col, 2);
    assert_eq!(chart.title.as_deref(), Some("Quarterly sales"));
    assert_eq!(chart.category_axis.title.as_deref(), Some("Quarter"));
}

#[test]
#[ignore = "nightly wall-clock smoke bound; shared CI software runners are nondeterministic"]
fn first_frame_renders_within_software_ci_smoke_budget() {
    // The product's 300 ms target is gated on the fixed integrated-GPU
    // reference host (WP-28). GitHub's cold lavapipe initialization takes
    // seconds, so this is only a render-completion smoke bound.
    let start = std::time::Instant::now();
    let parts = launch_theme(None);
    let mut builder = Harness::builder().with_size(egui::vec2(640.0, 400.0));
    if let Some(setup) = fixed_gpu_setup() {
        builder = builder.wgpu_setup(setup);
    }
    let mut harness =
        builder.build_eframe(|cc| Gui::new(parts.launch, false, &cc.egui_ctx).unwrap());
    harness.run();
    let _ = harness.render().expect("render first frame");
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("gui_first_frame_ms={ms:.2}");
    assert!(
        ms < 5_000.0,
        "first software kittest frame took {ms:.2} ms (5 s CI smoke budget)"
    );
}

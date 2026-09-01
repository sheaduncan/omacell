//! Composition-root `chart.export` command coverage.

use omacell_bus::Bus;
use omacell_cli::{FileSession, register_file_commands};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use serde_json::json;

fn scratch_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(".scratch-chart-command-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
}

fn bus_with_chart() -> Bus {
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    omacell_bus::register_chart_commands(bus.registry_mut()).unwrap();
    register_file_commands(&mut bus, FileSession::new()).unwrap();
    for (cell, input) in [
        ("A1", "Month"),
        ("B1", "Sales"),
        ("A2", "Jan"),
        ("B2", "12"),
    ] {
        let outcome = bus.execute(
            Origin::User,
            "cell.set",
            json!({"ref": cell, "input": input}),
        );
        assert!(outcome.ok, "{:?}", outcome.error);
    }
    let outcome = bus.execute(
        Origin::User,
        "chart.fromselection",
        json!({"range": "A1:B2", "kind": "column", "title": "Sales"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    bus
}

#[test]
fn chart_export_writes_svg_atomically_and_dry_run_does_not_write() {
    let dir = scratch_dir();
    let svg = dir.path().join("sales.svg");
    let dry = dir.path().join("dry.svg");
    let mut bus = bus_with_chart();

    let outcome = bus.execute(
        Origin::User,
        "chart.export",
        json!({"path": svg.display().to_string(), "width": 320, "height": 200}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    assert_eq!(outcome.result.as_ref().unwrap()["chart"], 1);
    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(text.starts_with("<svg"), "{text}");
    assert!(!std::fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".sales.svg.omacell-")
    }));

    let dry_run = bus
        .dry_run(
            Origin::User,
            "chart.export",
            json!({"path": dry.display().to_string()}),
        )
        .unwrap();
    assert!(dry_run.outcome.ok, "{:?}", dry_run.outcome.error);
    assert!(!dry.exists());
}

#[test]
fn chart_export_rejects_unknown_chart_and_output_format() {
    let dir = scratch_dir();
    let mut bus = bus_with_chart();

    let missing = bus.execute(
        Origin::User,
        "chart.export",
        json!({"path": dir.path().join("missing.svg").display().to_string(), "id": 99}),
    );
    assert!(!missing.ok);
    assert_eq!(missing.error.unwrap().code, "chart.export");

    let format = bus.execute(
        Origin::User,
        "chart.export",
        json!({"path": dir.path().join("sales.jpg").display().to_string()}),
    );
    assert!(!format.ok);
    assert_eq!(format.error.unwrap().code, "chart.export");
}

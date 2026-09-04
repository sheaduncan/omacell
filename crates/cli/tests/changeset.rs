//! Changeset CLI lifecycle and dry-run coverage.

use assert_cmd::Command;
use omacell_bus::Bus;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use tempfile::TempDir;

fn run_json(xdg: &TempDir, home: &TempDir, args: &[&str]) -> serde_json::Value {
    let output = Command::cargo_bin("omacell")
        .unwrap()
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", xdg.path())
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[cfg(unix)]
#[test]
fn changeset_list_show_export_apply_revert_and_dry_run() {
    let xdg = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let runtime = xdg.path().join("omacell");
    let bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    let _server = omacell_bus::ipc::serve(runtime, bus).unwrap();

    let proposed = run_json(
        &xdg,
        &home,
        &["ipc", "cell.set", r#"{"ref":"A1","input":"7"}"#],
    );
    let id = proposed["id"].as_str().unwrap();

    let listed = run_json(&xdg, &home, &["changeset", "list"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let shown = run_json(&xdg, &home, &["changeset", "show", id]);
    assert_eq!(shown["status"], "proposed");

    let extra = run_json(
        &xdg,
        &home,
        &["ipc", "cell.set", r#"{"ref":"B1","input":"8"}"#],
    );
    let extra_id = extra["id"].as_str().unwrap();
    let discarded = run_json(&xdg, &home, &["changeset", "discard", extra_id]);
    assert_eq!(discarded["id"], extra_id);
    assert_eq!(
        run_json(&xdg, &home, &["changeset", "list"])
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let dry_export = home.path().join("dry.omc");
    let dry = run_json(
        &xdg,
        &home,
        &[
            "--dry-run",
            "changeset",
            "export",
            id,
            "--omc",
            dry_export.to_str().unwrap(),
        ],
    );
    assert_eq!(dry["dry_run"], true);
    assert!(!dry_export.exists());

    let exported = home.path().join("change.omc");
    run_json(
        &xdg,
        &home,
        &[
            "changeset",
            "export",
            id,
            "--omc",
            exported.to_str().unwrap(),
        ],
    );
    assert!(exported.is_file());

    let dry_discard = run_json(&xdg, &home, &["--dry-run", "changeset", "discard", id]);
    assert_eq!(dry_discard["dry_run"], true);
    assert_eq!(
        run_json(&xdg, &home, &["changeset", "show", id])["status"],
        "proposed"
    );

    let dry_apply = run_json(&xdg, &home, &["--dry-run", "changeset", "apply", id]);
    assert_eq!(dry_apply["dry_run"], true);
    assert_eq!(
        run_json(&xdg, &home, &["changeset", "show", id])["status"],
        "proposed"
    );

    assert_eq!(
        run_json(&xdg, &home, &["changeset", "apply", id])["status"],
        "applied"
    );
    assert_eq!(
        run_json(&xdg, &home, &["changeset", "revert", id])["status"],
        "reverted"
    );
}

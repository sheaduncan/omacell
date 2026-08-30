//! `assert_cmd` coverage for every subcommand, including stubs.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("omacell").unwrap()
}

fn home() -> (TempDir, Command) {
    let dir = TempDir::new().unwrap();
    let mut cmd = bin();
    cmd.env("HOME", dir.path());
    cmd.env("XDG_RUNTIME_DIR", dir.path().join("run"));
    (dir, cmd)
}

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx")
}

#[test]
fn version_and_usage() {
    bin()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("omacell"));
    bin().arg("--not-a-flag").assert().code(2);

    let output = bin().args(["--json", "--not-a-flag"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "cli.usage");
    assert!(error["message"].as_str().unwrap().contains("not-a-flag"));
}

#[test]
fn ipc_all_and_socket_are_mutually_exclusive() {
    bin()
        .args(["ipc", "ping", "--all", "--socket", "/does/not/matter.sock"])
        .assert()
        .code(2);
}

#[test]
fn stubs_exit_three_with_hint() {
    for args in [
        vec!["run", "x.lua", "x.xlsx"],
        vec!["ai", "setup"],
        vec!["agent", "hello"],
        vec!["mcp"],
    ] {
        let (_home, mut cmd) = home();
        cmd.args(&args)
            .assert()
            .code(3)
            .stderr(predicate::str::contains("arrives in WP-"));
    }
    let (_home, mut cmd) = home();
    cmd.env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .arg("book.xlsx")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "requires a Wayland or X11 display",
        ));
}

#[test]
fn gui_rejects_ambiguous_or_ignored_arguments_before_display_setup() {
    for args in [vec!["one.xlsx", "two.xlsx"], vec!["--dry-run"]] {
        let (_home, mut cmd) = home();
        cmd.env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cli.usage"));
    }
}

#[test]
fn tui_without_tty_exits_error() {
    let (_home, mut cmd) = home();
    cmd.args(["--tui"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("requires a terminal"));
}

#[test]
fn tui_rejects_ambiguous_or_ignored_arguments_before_tty_setup() {
    for args in [
        vec!["--tui", "one.xlsx", "two.xlsx"],
        vec!["--tui", "config", "check"],
        vec!["--tui", "--dry-run"],
    ] {
        let (_home, mut cmd) = home();
        cmd.args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("cli.usage"));
    }
}

#[test]
fn json_error_shape() {
    let (_home, mut cmd) = home();
    cmd.args(["--json", "run", "x.lua", "x.xlsx"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("\"code\""))
        .stderr(predicate::str::contains("\"message\""))
        .stderr(predicate::str::contains("\"hint\""));
}

#[test]
fn fn_list_and_doc() {
    let (_home, mut cmd) = home();
    cmd.args(["--json", "fn", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema\""))
        .stdout(predicate::str::contains("SUM"));
    let (_home, mut cmd) = home();
    cmd.args(["fn", "doc", "XLOOKUP"])
        .assert()
        .success()
        .stdout(predicate::str::contains("XLOOKUP"));
}

#[test]
fn commands_json_includes_file_and_theme() {
    let (_home, mut cmd) = home();
    cmd.args(["--json", "commands"])
        .assert()
        .success()
        .stdout(predicate::str::contains("file.open"))
        .stdout(predicate::str::contains("file.save"))
        .stdout(predicate::str::contains("file.export"))
        .stdout(predicate::str::contains("file.print"))
        .stdout(predicate::str::contains("theme.reload"));
}

#[test]
fn query_eval_set_recalc_convert() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["--json", "query", book.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rows"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["eval", book.to_str().unwrap(), "=1+1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["set", book.to_str().unwrap(), "Z1", "7"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["set", book.to_str().unwrap(), "Z2", "-1"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["recalc", book.to_str().unwrap()])
        .assert()
        .success();

    let out = dir.path().join("out.csv");
    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["convert", book.to_str().unwrap(), out.to_str().unwrap()])
        .assert()
        .success();
    assert!(out.is_file());

    let pdf = dir.path().join("out.pdf");
    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["convert", book.to_str().unwrap(), pdf.to_str().unwrap()])
        .assert()
        .success();
    let pdf_bytes = std::fs::read(&pdf).unwrap();
    assert!(pdf_bytes.starts_with(b"%PDF-"));
}

#[test]
fn query_human_csv_and_markdown_outputs_escape_cells() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.csv");
    std::fs::write(
        &book,
        "\"a,b\",\"quote\"\"here\",\"pipe|slash\\\",\"line\nbreak\"\n",
    )
    .unwrap();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["query", book.to_str().unwrap(), "A1:D1", "--format", "csv"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"a,b\""))
        .stdout(predicate::str::contains("\"quote\"\"here\""))
        .stdout(predicate::str::contains("\"line\nbreak\""));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["query", book.to_str().unwrap(), "A1:D1", "--format", "md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("pipe\\|slash\\\\"))
        .stdout(predicate::str::contains("line<br>break"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["query", book.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("["))
        .stdout(predicate::str::contains("a,b"));
}

#[test]
fn convert_accepts_the_shared_csv_import_plan() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("input.csv");
    let plan = dir.path().join("plan.json");
    let output = dir.path().join("output.xlsx");
    std::fs::write(&input, "00123\n").unwrap();
    std::fs::write(
        &plan,
        r#"{"delimiter":",","columns":[{"ty":{"kind":"text"}}]}"#,
    )
    .unwrap();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args([
            "convert",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "--plan",
            plan.to_str().unwrap(),
        ])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["--json", "query", output.to_str().unwrap(), "A1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("00123"));
}

#[test]
fn theme_show_and_keys_check_and_setup() {
    let (dir, mut cmd) = home();
    cmd.args(["--json", "theme", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("theme"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["keys", "check"])
        .assert()
        .success();

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["setup", "omarchy", "--show-hyprland"])
        .assert()
        .success()
        .stdout(predicate::str::contains("o.bind"));

    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["setup", "omarchy"])
        .assert()
        .success();
    assert!(
        dir.path()
            .join(".config/omarchy/hooks/theme-set.d/omacell")
            .is_file()
    );
}

#[test]
fn diff_two_identical_copies() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.xlsx");
    let b = dir.path().join("b.xlsx");
    std::fs::copy(corpus_xlsx(), &a).unwrap();
    std::fs::copy(corpus_xlsx(), &b).unwrap();
    let mut cmd = bin();
    cmd.env("HOME", dir.path())
        .args(["--json", "diff", a.to_str().unwrap(), b.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"empty\": true"));
}

#[test]
fn audit_json_matches_schema_shape() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();
    let output = bin()
        .env("HOME", dir.path())
        .args(["--json", "audit", book.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["schema"], 1);
    let findings = json["findings"].as_array().expect("findings");
    for finding in findings {
        let id = finding["id"].as_str().unwrap();
        assert!(id.starts_with("audit."));
        let sev = finding["severity"].as_str().unwrap();
        assert!(matches!(sev, "error" | "warning" | "info"));
        assert!(finding["sheet"].as_str().is_some());
        assert!(finding["ref"].as_str().is_some());
        assert!(finding["message"].as_str().is_some());
    }
}

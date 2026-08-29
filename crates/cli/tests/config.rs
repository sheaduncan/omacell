//! `--config`, `--theme`, env, `--set`, and workbook overlay reach one `LoadOptions`.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    Command::cargo_bin("omacell").unwrap()
}

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx")
}

#[test]
fn layers_reach_one_load() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    fs::create_dir_all(home.join(".config/omacell")).unwrap();
    fs::write(
        home.join(".config/omacell/config.toml"),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();

    let explicit = home.join("profile.toml");
    fs::write(&explicit, "[layout]\npanel_width = 321\n").unwrap();

    let theme = home.join("theme.toml");
    fs::write(&theme, "[state]\ncursor = \"#abcdef\"\n").unwrap();

    bin()
        .env("HOME", home)
        .env("OMACELL_APPEARANCE__GRID_LINES", "true")
        .args([
            "--json",
            "--config",
            explicit.to_str().unwrap(),
            "--theme",
            theme.to_str().unwrap(),
            "--set",
            "layout.panel_width=111",
            "--set",
            "appearance.grid_lines=false",
            "config",
            "show",
            "--all",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"panel_width\": 111"))
        .stdout(predicate::str::contains("#abcdef"));

    bin()
        .env("HOME", home)
        .env("OMACELL_APPEARANCE__GRID_LINES", "true")
        .args([
            "--json",
            "--config",
            explicit.to_str().unwrap(),
            "--set",
            "appearance.grid_lines=false",
            "config",
            "show",
            "appearance.grid_lines",
            "--explain",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"layer\": \"cli\""));
}

#[test]
fn from_workbook_applies_settings_overlay() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    fs::copy(corpus_xlsx(), &book).unwrap();
    bin()
        .env("HOME", dir.path())
        .args([
            "--json",
            "--from-workbook",
            book.to_str().unwrap(),
            "config",
            "show",
            "calc.mode",
            "--explain",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("workbook"));
}

#[test]
fn config_reset_named_and_default() {
    let dir = TempDir::new().unwrap();
    let home = dir.path();
    let user = home.join(".config/omacell");
    fs::create_dir_all(user.join("profiles")).unwrap();
    fs::write(
        user.join("config.toml"),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    fs::write(
        user.join("profiles/work.toml"),
        "[layout]\npanel_width = 9\n",
    )
    .unwrap();

    bin()
        .env("HOME", home)
        .args(["config", "reset", "profiles/work.toml"])
        .assert()
        .success();
    assert!(!user.join("profiles/work.toml").is_file());

    bin()
        .env("HOME", home)
        .args(["config", "reset"])
        .assert()
        .success();
    assert!(!user.join("config.toml").is_file());
}

#[test]
fn config_edit_dry_run_prints_path() {
    let dir = TempDir::new().unwrap();
    bin()
        .env("HOME", dir.path())
        .args(["--dry-run", "config", "edit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("config.toml"));
}

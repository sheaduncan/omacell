//! `--dry-run` on writes does not change files.

use std::path::PathBuf;

use assert_cmd::Command;
use omacell_bus::Bus;
use omacell_cli::{FileSession, register_file_commands};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use tempfile::TempDir;

fn corpus_xlsx() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/xlsx/l1_values.xlsx")
}

#[test]
fn convert_dry_run_does_not_write() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("in.xlsx");
    std::fs::copy(corpus_xlsx(), &input).unwrap();
    let output = dir.path().join("out.csv");
    Command::cargo_bin("omacell")
        .unwrap()
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", dir.path())
        .args([
            "--dry-run",
            "convert",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(!output.exists());
}

#[test]
fn set_dry_run_does_not_change_workbook() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();
    let before = std::fs::read(&book).unwrap();
    Command::cargo_bin("omacell")
        .unwrap()
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", dir.path())
        .args(["--dry-run", "set", book.to_str().unwrap(), "A1", "999"])
        .assert()
        .success();
    let after = std::fs::read(&book).unwrap();
    assert_eq!(before, after);
}

#[test]
fn setup_dry_run_writes_nothing() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("omacell")
        .unwrap()
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", dir.path())
        .args(["--dry-run", "setup", "omarchy"])
        .assert()
        .success();
    assert!(!dir.path().join(".config/omarchy").exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn configured_backup_is_the_original_not_a_second_preflight_save() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();
    let original = std::fs::read(&book).unwrap();

    Command::cargo_bin("omacell")
        .unwrap()
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env("HOME", dir.path())
        .args([
            "--set",
            "files.keep_backups=1",
            "set",
            book.to_str().unwrap(),
            "A1",
            "999",
        ])
        .assert()
        .success();

    assert_eq!(
        std::fs::read(book.with_extension("xlsx.bak.1")).unwrap(),
        original
    );
}

#[test]
fn file_open_dry_run_does_not_attach_the_live_file_session() {
    let dir = TempDir::new().unwrap();
    let book = dir.path().join("book.xlsx");
    std::fs::copy(corpus_xlsx(), &book).unwrap();
    let original = std::fs::read(&book).unwrap();
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    register_file_commands(&mut bus, FileSession::new()).unwrap();

    let dry = bus
        .dry_run(
            Origin::User,
            "file.open",
            serde_json::json!({"path": book.display().to_string()}),
        )
        .unwrap();
    assert!(dry.outcome.ok);
    let save = bus.execute(Origin::User, "file.save", serde_json::json!({}));
    assert!(!save.ok);
    assert_eq!(save.error.unwrap().code, "file.path");
    assert_eq!(std::fs::read(book).unwrap(), original);
}

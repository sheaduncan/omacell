//! `--dry-run` on writes does not change files.

use std::path::PathBuf;

use assert_cmd::Command;
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
        .env("HOME", dir.path())
        .args(["--dry-run", "setup", "omarchy"])
        .assert()
        .success();
    assert!(!dir.path().join(".config/omarchy").exists());
}

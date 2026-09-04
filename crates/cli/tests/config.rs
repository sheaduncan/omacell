//! `--config`, `--theme`, env, `--set`, and workbook overlay reach one `LoadOptions`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn bin() -> Command {
    let mut command = Command::cargo_bin("omacell").unwrap();
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME");
    command
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
    let explicit = dir.path().join("missing/profile.toml");
    bin()
        .env("HOME", dir.path())
        .args([
            "--dry-run",
            "--config",
            explicit.to_str().unwrap(),
            "config",
            "edit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("profile.toml"));
    assert!(!explicit.exists());
}

#[test]
fn config_edit_validates_the_editors_result() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("profile.toml");
    let editor = dir.path().join("invalid-editor");
    fs::write(&editor, "#!/bin/sh\nprintf '%s\\n' '[[[invalid' > \"$1\"\n").unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o700)).unwrap();

    bin()
        .env("HOME", dir.path())
        .env("EDITOR", &editor)
        .args([
            "--json",
            "--config",
            config.to_str().unwrap(),
            "config",
            "edit",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("invalid"));
    assert_eq!(fs::read_to_string(config).unwrap(), "[[[invalid\n");
}

#[test]
fn config_edit_uses_xdg_config_home() {
    let dir = TempDir::new().unwrap();
    let config_home = dir.path().join("xdg-config");
    bin()
        .env("HOME", dir.path().join("home"))
        .env("XDG_CONFIG_HOME", &config_home)
        .args(["--dry-run", "config", "edit"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            config_home
                .join("omacell/config.toml")
                .display()
                .to_string(),
        ));
}

#[test]
fn invalid_config_can_still_be_reset() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join(".config/omacell/config.toml");
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, "[[[invalid").unwrap();

    bin()
        .env("HOME", dir.path())
        .args(["config", "reset"])
        .assert()
        .success();
    assert!(!config.exists());
    let backups = dir.path().join(".local/state/omacell/backups");
    assert!(backups.read_dir().unwrap().next().is_some());
}

#[test]
fn config_reset_dry_run_still_rejects_an_unsafe_path() {
    let dir = TempDir::new().unwrap();
    bin()
        .env("HOME", dir.path())
        .args(["--dry-run", "config", "reset", "../escape.toml"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("relative file"));
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn config_diff_uses_the_explicit_config_path() {
    let dir = TempDir::new().unwrap();
    let explicit = dir.path().join("profiles/work.toml");
    fs::create_dir_all(explicit.parent().unwrap()).unwrap();
    fs::write(&explicit, "[layout]\npanel_width = 444\n").unwrap();

    bin()
        .env("HOME", dir.path())
        .args([
            "--json",
            "--config",
            explicit.to_str().unwrap(),
            "config",
            "diff",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("layout.panel_width"))
        .stdout(predicate::str::contains("444"))
        .stdout(predicate::str::contains("profiles/work.toml"));
}

//! `theme.reload` locally, over IPC `--all`, and through SIGUSR1.

use std::time::{Duration, Instant};

use assert_cmd::Command;
use omacell_bus::Bus;
use omacell_cli::{register_theme_reload, spawn_sigusr1_reloader};
use omacell_conf::layer::LoadOptions;
use omacell_conf::{ConfigStore, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn local_theme_reload() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("omacell")
        .unwrap()
        .env("HOME", dir.path())
        .args(["--json", "theme", "reload"])
        .assert()
        .success()
        .stdout(predicate::str::contains("name"));
}

#[cfg(unix)]
#[test]
fn ipc_theme_reload_all() {
    let xdg = TempDir::new().unwrap();
    let runtime = xdg.path().join("omacell");
    let home = TempDir::new().unwrap();
    let paths = Paths::from_home(home.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let store = ConfigStore::load_and_watch_with(paths, LoadOptions::default()).unwrap();
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    register_theme_reload(&mut bus, store.handle()).unwrap();
    let _server = omacell_bus::ipc::serve(runtime, bus).unwrap();

    Command::cargo_bin("omacell")
        .unwrap()
        .env("HOME", home.path())
        .env("XDG_RUNTIME_DIR", xdg.path())
        .args(["--json", "ipc", "theme.reload", "--all", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("instances"));
}

#[cfg(unix)]
#[test]
fn sigusr1_reloads_retained_config() {
    let dir = TempDir::new().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config_toml(),
        "[layout]\npanel_width = 100\n[config]\nlive_reload = false\n",
    )
    .unwrap();
    let store = ConfigStore::load_and_watch_with(paths.clone(), LoadOptions::default()).unwrap();
    assert_eq!(store.snapshot().config.layout.panel_width, 100);
    let _guard = spawn_sigusr1_reloader(store.handle()).unwrap();
    std::fs::write(
        paths.user_config_toml(),
        "[layout]\npanel_width = 222\n[config]\nlive_reload = false\n",
    )
    .unwrap();
    signal_hook::low_level::raise(signal_hook::consts::SIGUSR1).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if store.snapshot().config.layout.panel_width == 222 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "SIGUSR1 did not reload; width={}",
        store.snapshot().config.layout.panel_width
    );
}

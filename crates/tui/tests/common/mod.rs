//! Shared TUI test harness (no second config load; no watcher).
#![allow(dead_code)]

use omacell_bus::Bus;
use omacell_conf::{ConfigStore, LoadOptions, Paths};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;
use omacell_tui::{Launch, Tui};
use omacell_ui::{KeymapRoots, UiSession, register_ui_commands};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;

pub struct Harness {
    pub _dir: tempfile::TempDir,
    pub tui: Tui,
}

pub fn fixture_theme(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/omarchy-themes")
        .join(name)
        .join("colors.toml")
}

fn install_omarchy_theme(paths: &Paths, name: &str) {
    let theme_dir = paths.omarchy_state.join("current/theme");
    std::fs::create_dir_all(&theme_dir).unwrap();
    std::fs::copy(fixture_theme(name), theme_dir.join("colors.toml")).unwrap();
    std::fs::write(paths.omarchy_state.join("current/theme.name"), name).unwrap();
}

pub fn harness() -> Harness {
    harness_opts(None, "keys/classic.toml", "off")
}

pub fn harness_theme(theme: &str) -> Harness {
    harness_opts(Some(theme), "keys/classic.toml", "off")
}

pub fn harness_modal() -> Harness {
    harness_opts(None, "keys/modal.toml", "off")
}

pub fn harness_opts(theme: Option<&str>, keymap: &str, truecolor: &str) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config.join("config.toml"),
        format!("[keys]\nfile = {keymap:?}\n[tui]\ntruecolor = {truecolor:?}\n"),
    )
    .unwrap();
    if let Some(theme) = theme {
        install_omarchy_theme(&paths, theme);
    }
    let options = LoadOptions {
        cli_sets: vec![
            format!("keys.file={keymap}"),
            format!("tui.truecolor={truecolor}"),
        ],
        ..LoadOptions::default()
    };
    let store = ConfigStore::load_with(paths.clone(), options).unwrap();
    let loaded = store.snapshot();
    let roots = KeymapRoots::new(paths.user_config.clone(), paths.default_dir.clone(), None);
    let ui = UiSession::new(&loaded, &roots).unwrap();

    let mut functions = FnRegistry::new();
    register_all(&mut functions);
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(functions)).unwrap();
    register_ui_commands(bus.registry_mut(), &ui).unwrap();

    let tui = Tui::new(
        Launch {
            paths,
            store,
            bus,
            ui,
            roots,
        },
        false,
    )
    .unwrap();
    Harness { _dir: dir, tui }
}

pub fn seed_demo(tui: &mut Tui) {
    for (cell, input) in [
        ("A1", "Hello"),
        ("B1", "1234.5"),
        ("C1", "TRUE"),
        ("A2", "=B1*2"),
        ("D1", "overflows into next"),
        ("A3", "=1/0"),
    ] {
        let result = tui
            .execute_cmd("cell.set", json!({"ref": cell, "input": input}))
            .unwrap();
        assert!(result.ok, "{cell}: {:?}", result.error);
    }
}

pub fn draw_text(tui: &Tui, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    tui.draw(&mut terminal).unwrap();
    omacell_tui::buffer_text(&terminal)
}

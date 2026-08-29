//! Last-good config survives an invalid write.

use std::time::Duration;

use omacell_conf::layer::LoadOptions;
use omacell_conf::paths::Paths;
use omacell_conf::watch::{ConfigStore, ReloadEvent};

fn wait_for(store: &ConfigStore, predicate: impl Fn(&ReloadEvent) -> bool) -> bool {
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
        if store.drain_events().iter().any(&predicate) {
            return true;
        }
    }
    false
}

#[test]
fn invalid_write_keeps_last_good() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    let store = ConfigStore::load_and_watch(paths.clone()).unwrap();
    assert!(!store.snapshot().config.appearance.grid_lines);

    std::fs::write(paths.user_config_toml(), "[[[not toml").unwrap();
    let saw_invalid = wait_for(&store, |ev| matches!(ev, ReloadEvent::Invalid { .. }));
    assert!(saw_invalid, "expected invalid reload event");
    assert!(
        !store.snapshot().config.appearance.grid_lines,
        "last good must remain"
    );
}

#[test]
fn reload_preserves_workbook_env_and_cli_layers() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_lines = true\n[calc]\nmode = \"manual\"\n",
    )
    .unwrap();
    let workbook = toml::from_str("[calc]\nmode = \"automatic\"\n").unwrap();
    let options = LoadOptions {
        cli_sets: vec!["appearance.grid_lines=false".into()],
        workbook: Some(workbook),
        env: vec![("OMACELL_BEHAVIOR__ENTER_MOVES".into(), "right".into())],
        theme_override: None,
    };
    let store = ConfigStore::load_and_watch_with(paths.clone(), options).unwrap();

    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_lines = true\n[calc]\nmode = \"manual\"\n[layout]\npanel_width = 400\n",
    )
    .unwrap();
    assert!(wait_for(&store, |ev| matches!(
        ev,
        ReloadEvent::Applied { .. }
    )));
    let snapshot = store.snapshot();
    assert!(!snapshot.config.appearance.grid_lines);
    assert_eq!(snapshot.config.calc.mode, "automatic");
    assert_eq!(snapshot.config.behavior.enter_moves, "right");
    assert_eq!(snapshot.config.layout.panel_width, 400);
}

#[test]
fn live_reload_false_does_not_start_a_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config_toml(),
        "[config]\nlive_reload = false\n[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    let store = ConfigStore::load_and_watch(paths.clone()).unwrap();
    assert!(!store.is_watching());

    std::fs::write(
        paths.user_config_toml(),
        "[config]\nlive_reload = false\n[appearance]\ngrid_lines = true\n",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!store.snapshot().config.appearance.grid_lines);
    assert!(store.drain_events().is_empty());
}

#[test]
fn active_theme_changes_reload_and_emit_theme_event() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let theme = paths.omarchy_state.join("current/theme");
    std::fs::create_dir_all(&theme).unwrap();
    let original = include_str!("../../../tests/fixtures/omarchy-themes/tokyo-night/colors.toml");
    std::fs::write(theme.join("colors.toml"), original).unwrap();
    let store = ConfigStore::load_and_watch(paths.clone()).unwrap();
    assert_eq!(store.snapshot().theme.roles["state.cursor"], "#7aa2f7");

    let changed = original.replace("accent = \"#7aa2f7\"", "accent = \"#123456\"");
    std::fs::write(theme.join("colors.toml"), changed).unwrap();
    assert!(wait_for(&store, |ev| matches!(
        ev,
        ReloadEvent::ThemeChanged { .. }
    )));
    assert_eq!(store.snapshot().theme.roles["state.cursor"], "#123456");
}

#[test]
fn explicit_theme_override_is_watched() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let override_path = dir.path().join("override.toml");
    std::fs::write(&override_path, "[state]\ncursor = \"#123456\"\n").unwrap();
    let store = ConfigStore::load_and_watch_with(
        paths,
        LoadOptions {
            theme_override: Some(override_path.clone()),
            ..LoadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(store.snapshot().theme.roles["state.cursor"], "#123456");

    std::fs::write(&override_path, "[state]\ncursor = \"#654321\"\n").unwrap();
    assert!(wait_for(&store, |ev| matches!(
        ev,
        ReloadEvent::ThemeChanged { .. }
    )));
    assert_eq!(store.snapshot().theme.roles["state.cursor"], "#654321");
}

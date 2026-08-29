//! Last-good config survives an invalid write.

use std::time::Duration;

use omacell_conf::paths::Paths;
use omacell_conf::watch::{ConfigStore, ReloadEvent};

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
    let mut saw_invalid = false;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(20));
        for ev in store.drain_events() {
            if matches!(ev, ReloadEvent::Invalid { .. }) {
                saw_invalid = true;
            }
        }
        if saw_invalid {
            break;
        }
    }
    assert!(saw_invalid, "expected invalid reload event");
    assert!(
        !store.snapshot().config.appearance.grid_lines,
        "last good must remain"
    );
}

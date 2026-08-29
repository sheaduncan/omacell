//! `setup omarchy` writes only under the fake $HOME.

use omacell_conf::paths::Paths;
use omacell_conf::setup::{SetupOptions, setup_omarchy};

#[test]
fn setup_without_menu_writes_template_and_hook_only() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    let report = setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: false,
            link_skill: true,
        },
    )
    .unwrap();
    let tpl = paths.omarchy_config.join("themed/omacell.toml.tpl");
    let hook = paths.omarchy_config.join("hooks/theme-set.d/omacell");
    assert!(tpl.is_file(), "{report:?}");
    assert!(hook.is_file());
    assert!(
        !paths
            .omarchy_config
            .join("extensions/omarchy-menu.jsonc")
            .exists()
    );
    assert!(report.skipped.iter().any(|s| s.contains("menu")));
    // nothing under /usr/share/omarchy
    assert!(report.written.iter().all(|p| p.starts_with(dir.path())));
}

#[test]
fn setup_with_menu_confirmation_writes_jsonc() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    setup_omarchy(
        &paths,
        SetupOptions {
            confirm_menu: true,
            link_skill: false,
        },
    )
    .unwrap();
    assert!(
        paths
            .omarchy_config
            .join("extensions/omarchy-menu.jsonc")
            .is_file()
    );
}

//! `apply_config` must not reset live edit, selection, viewport, or session.

use omacell_bus::{CommandRegistry, register_core};
use omacell_conf::{Paths, load};
use omacell_ui::{EditSurface, KeymapRoots, Mode, UiSession, register_ui_commands};

fn roots_and_config(dir: &tempfile::TempDir) -> (omacell_conf::LoadedConfig, KeymapRoots) {
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    let roots = KeymapRoots::new(paths.user_config, paths.default_dir, None);
    (loaded, roots)
}

#[test]
fn apply_config_preserves_interaction_state() {
    let dir = tempfile::tempdir().unwrap();
    let (loaded, roots) = roots_and_config(&dir);
    let session = UiSession::new(&loaded, &roots).unwrap();
    let mut registry = CommandRegistry::new();
    register_core(&mut registry).unwrap();
    omacell_bus::register_chart_commands(&mut registry).unwrap();
    omacell_bus::register_edit_commands(&mut registry).unwrap();
    register_ui_commands(&mut registry, &session).unwrap();
    session.begin_edit(EditSurface::InCell, "=A1+1");
    let before_edit = session.edit();
    let before_sel = session.selection();
    let before_view = session.viewport();
    let loaded2 = load(
        &Paths::from_home(dir.path()),
        &["appearance.grid_lines=false".into()],
        None,
    )
    .unwrap();
    session.apply_config(&loaded2, &roots, &registry).unwrap();
    let after = session.edit();
    assert_eq!(after.buffer, before_edit.buffer);
    assert!(!after.is_idle());
    assert_eq!(session.selection().cursor, before_sel.cursor);
    assert_eq!(session.viewport().first_row, before_view.first_row);
    assert!(!session.config().appearance.grid_lines);

    let modal = load(
        &Paths::from_home(dir.path()),
        &["keys.file=keys/modal.toml".into()],
        None,
    )
    .unwrap();
    session.apply_config(&modal, &roots, &registry).unwrap();
    assert_eq!(session.mode(), Mode::Insert);
    assert_eq!(session.edit().buffer, before_edit.buffer);

    let user_map = roots.user_config.join("keys/modal.toml");
    std::fs::create_dir_all(user_map.parent().unwrap()).unwrap();
    std::fs::write(&user_map, "[bindings.normal]\nq='unknown.command'\n").unwrap();
    assert!(session.apply_config(&modal, &roots, &registry).is_err());
    assert_eq!(session.mode(), Mode::Insert);
    assert_eq!(session.edit().buffer, before_edit.buffer);
}

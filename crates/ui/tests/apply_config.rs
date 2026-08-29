//! `apply_config` must not reset live edit, selection, viewport, or session.

use omacell_conf::{Paths, load};
use omacell_ui::{EditSurface, KeymapRoots, UiSession};

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
    session.apply_config(&loaded2, &roots).unwrap();
    let after = session.edit();
    assert_eq!(after.buffer, before_edit.buffer);
    assert!(!after.is_idle());
    assert_eq!(session.selection().cursor, before_sel.cursor);
    assert_eq!(session.viewport().first_row, before_view.first_row);
}

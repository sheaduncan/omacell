//! Session persistence defaults and validation.

use omacell_ui::SessionState;

#[test]
fn default_and_round_trip_keep_a_valid_zoom() {
    let dir = tempfile::tempdir().unwrap();
    let state = SessionState::default();
    assert_eq!(state.zoom, 1.0);
    state.save(dir.path()).unwrap();
    assert_eq!(SessionState::load(dir.path()).unwrap(), state);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(SessionState::path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn invalid_zoom_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.toml"), "zoom = nan\n").unwrap();
    assert!(SessionState::load(dir.path()).is_err());
}

#[cfg(unix)]
#[test]
fn session_load_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside.toml");
    std::fs::write(&outside, "zoom = 1.0\n").unwrap();
    symlink(&outside, SessionState::path(dir.path())).unwrap();
    assert!(SessionState::load(dir.path()).is_err());
}

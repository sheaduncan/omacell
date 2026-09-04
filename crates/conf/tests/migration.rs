//! Version migration and reset backups never lose or escape user data.

use omacell_conf::layer::{load, reset_user_file};
use omacell_conf::paths::Paths;

#[test]
fn schema_zero_is_backed_up_before_migration() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(
        paths.user_config_toml(),
        "schema = 0\n[appearance]\ngrid_lines = false\n",
    )
    .unwrap();

    let loaded = load(&paths, &[], None).unwrap();
    assert!(!loaded.config.appearance.grid_lines);
    let rewritten = std::fs::read_to_string(paths.user_config_toml()).unwrap();
    assert!(rewritten.contains("schema = 1"), "{rewritten}");

    let backup_root = paths.state_dir.join("backups");
    let backups: Vec<_> = std::fs::read_dir(backup_root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("config.toml"))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read_to_string(&backups[0]).unwrap(),
        "schema = 0\n[appearance]\ngrid_lines = false\n"
    );
}

#[test]
fn migration_preserves_comments_formatting_mode_and_symlink() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let target = dir.path().join("shared-config.toml");
    let original = "# keep this heading\nschema=0 # keep this note\n[appearance]\ngrid_lines = false\n";
    std::fs::write(&target, original).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
    symlink(&target, paths.user_config_toml()).unwrap();

    let loaded = load(&paths, &[], None).unwrap();
    assert!(!loaded.config.appearance.grid_lines);
    assert!(
        std::fs::symlink_metadata(paths.user_config_toml())
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "# keep this heading\nschema=1 # keep this note\n[appearance]\ngrid_lines = false\n"
    );
    assert_eq!(std::fs::metadata(&target).unwrap().mode() & 0o777, 0o640);
}

#[test]
fn future_schema_is_rejected_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    let original = "schema = 999\n[appearance]\ngrid_lines = false\n";
    std::fs::write(paths.user_config_toml(), original).unwrap();

    assert!(load(&paths, &[], None).is_err());
    assert_eq!(
        std::fs::read_to_string(paths.user_config_toml()).unwrap(),
        original
    );
    assert!(!paths.state_dir.join("backups").exists());
}

#[test]
fn reset_rejects_a_path_like_stamp() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(paths.user_config_toml(), "[appearance]\n").unwrap();

    assert!(reset_user_file(&paths, "../../escape").is_err());
    assert!(reset_user_file(&paths, "..").is_err());
    assert!(reset_user_file(&paths, ".").is_err());
    assert!(paths.user_config_toml().is_file());
    assert!(!dir.path().join("escape").exists());
}

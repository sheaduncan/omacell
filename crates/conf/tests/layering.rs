//! Layering precedence and `explain`.

use omacell_conf::layer::{Layer, LoadOptions, load, load_with_env, load_with_options};
use omacell_conf::paths::Paths;
use omacell_conf::schema::package_defaults;

fn temp_paths() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    let paths = Paths::from_home(&home);
    std::fs::create_dir_all(&paths.user_config).unwrap();
    (dir, paths)
}

#[test]
fn package_defaults_parse() {
    let c = package_defaults().unwrap();
    assert_eq!(c.schema, 1);
    assert!(c.appearance.grid_lines);
    assert!(!c.network.enabled);
    assert!(!c.ai.enabled);
}

#[test]
fn user_overrides_package() {
    let (_t, paths) = temp_paths();
    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    let loaded = load(&paths, &[], None).unwrap();
    assert!(!loaded.config.appearance.grid_lines);
    let e = loaded.explain("appearance.grid_lines").unwrap();
    assert_eq!(e.layer, Layer::User);
}

#[test]
fn env_overrides_user() {
    let (_t, paths) = temp_paths();
    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    let loaded = load_with_env(
        &paths,
        &[],
        None,
        [("OMACELL_APPEARANCE__GRID_LINES".into(), "true".into())],
    )
    .unwrap();
    assert!(loaded.config.appearance.grid_lines);
    let e = loaded.explain("appearance.grid_lines").unwrap();
    assert_eq!(e.layer, Layer::Env);
}

#[test]
fn cli_overrides_env() {
    let (_t, paths) = temp_paths();
    let loaded = load_with_env(
        &paths,
        &["appearance.grid_lines=false".into()],
        None,
        [("OMACELL_APPEARANCE__GRID_LINES".into(), "true".into())],
    )
    .unwrap();
    assert!(!loaded.config.appearance.grid_lines);
    assert_eq!(
        loaded.explain("appearance.grid_lines").unwrap().layer,
        Layer::Cli
    );
}

#[test]
fn workbook_overrides_user_but_not_env() {
    let (_t, paths) = temp_paths();
    std::fs::write(paths.user_config_toml(), "[calc]\nmode = \"manual\"\n").unwrap();
    let wb: toml::Value = toml::from_str("[calc]\nmode = \"automatic\"\n").unwrap();
    let loaded = load(&paths, &[], Some(&wb)).unwrap();
    assert_eq!(loaded.config.calc.mode, "automatic");
    assert_eq!(loaded.explain("calc.mode").unwrap().layer, Layer::Workbook);
}

#[test]
fn invalid_user_toml_is_line_error() {
    let (_t, paths) = temp_paths();
    std::fs::write(paths.user_config_toml(), "[appearance\n").unwrap();
    let err = load(&paths, &[], None).unwrap_err();
    assert_eq!(err.code, omacell_conf::error::codes::CONFIG_PARSE);
}

#[test]
fn unknown_keys_are_rejected() {
    let (_t, paths) = temp_paths();
    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_linse = false\n",
    )
    .unwrap();
    let err = load(&paths, &[], None).unwrap_err();
    assert_eq!(err.code, omacell_conf::error::codes::CONFIG_SCHEMA);
    assert!(err.message.contains("grid_linse"), "{err}");
}

#[test]
fn invalid_enums_and_ranges_are_rejected() {
    let (_t, paths) = temp_paths();
    std::fs::write(
        paths.user_config_toml(),
        "[calc]\nmode = \"sometimes\"\nmax_change = -1\n",
    )
    .unwrap();
    let err = load(&paths, &[], None).unwrap_err();
    assert_eq!(err.code, omacell_conf::error::codes::CONFIG_SCHEMA);
    assert!(err.message.contains("calc.mode"), "{err}");
}

#[test]
fn cli_unknown_key_is_rejected() {
    let (_t, paths) = temp_paths();
    let err = load(&paths, &["appearance.typo=true".into()], None).unwrap_err();
    assert_eq!(err.code, omacell_conf::error::codes::CONFIG_SCHEMA);
}

#[test]
fn cli_theme_override_wins_over_environment_theme() {
    let (dir, paths) = temp_paths();
    let env_theme = dir.path().join("env-theme.toml");
    let cli_theme = dir.path().join("cli-theme.toml");
    std::fs::write(&env_theme, "[state]\ncursor = \"#111111\"\n").unwrap();
    std::fs::write(&cli_theme, "[state]\ncursor = \"#222222\"\n").unwrap();
    let loaded = load_with_options(
        &paths,
        &LoadOptions {
            cli_sets: Vec::new(),
            workbook: None,
            env: vec![(
                "OMACELL_THEME".into(),
                env_theme.to_string_lossy().into_owned(),
            )],
            theme_override: Some(cli_theme),
        },
    )
    .unwrap();
    assert_eq!(loaded.theme.roles["state.cursor"], "#222222");
}

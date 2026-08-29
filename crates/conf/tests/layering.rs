//! Layering precedence and `explain`.

use omacell_conf::layer::{Layer, LoadOptions, load, load_with_env, load_with_options};
use omacell_conf::paths::Paths;
use omacell_conf::schema::package_defaults;
use omacell_conf::workbook_settings_overlay;
use omacell_core::date_system::DateSystem;
use omacell_core::workbook::{CalcMode, Iteration, WorkbookSettings};

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
            config_file: None,
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

#[test]
fn explicit_config_file_replaces_the_default_user_file() {
    let (dir, paths) = temp_paths();
    std::fs::write(
        paths.user_config_toml(),
        "[appearance]\ngrid_lines = false\n",
    )
    .unwrap();
    let explicit = dir.path().join("profiles/review.toml");
    std::fs::create_dir_all(explicit.parent().unwrap()).unwrap();
    std::fs::write(&explicit, "[layout]\npanel_width = 444\n").unwrap();

    let loaded = load_with_options(
        &paths,
        &LoadOptions {
            config_file: Some(explicit),
            ..LoadOptions::default()
        },
    )
    .unwrap();
    assert!(loaded.config.appearance.grid_lines);
    assert_eq!(loaded.config.layout.panel_width, 444);
    assert_eq!(
        loaded.explain("layout.panel_width").unwrap().layer,
        Layer::User
    );

    let missing = dir.path().join("missing.toml");
    assert!(
        load_with_options(
            &paths,
            &LoadOptions {
                config_file: Some(missing),
                ..LoadOptions::default()
            }
        )
        .is_err()
    );
}

#[test]
fn workbook_settings_have_one_canonical_config_overlay() {
    let (_dir, paths) = temp_paths();
    let settings = WorkbookSettings {
        date_system: DateSystem::Excel1904,
        calc_mode: CalcMode::Manual,
        iteration: Iteration {
            enabled: true,
            max_iterations: 23,
            max_change: 0.25,
        },
        precision_as_displayed: true,
    };
    let loaded = load_with_options(
        &paths,
        &LoadOptions {
            workbook: Some(workbook_settings_overlay(&settings)),
            ..LoadOptions::default()
        },
    )
    .unwrap();

    assert_eq!(loaded.config.behavior.date_system, 1904);
    assert!(loaded.config.behavior.precision_as_displayed);
    assert_eq!(loaded.config.calc.mode, "manual");
    assert!(loaded.config.calc.iterative);
    assert_eq!(loaded.config.calc.max_iterations, 23);
    assert_eq!(loaded.config.calc.max_change, 0.25);
    assert_eq!(loaded.explain("calc.mode").unwrap().layer, Layer::Workbook);
}

#[test]
fn named_user_reset_stays_under_user_config() {
    use omacell_conf::reset_user_rel;

    let (_t, paths) = temp_paths();
    let profile = paths.user_config.join("profiles/work.toml");
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, "[layout]\npanel_width = 9\n").unwrap();

    assert!(reset_user_rel(&paths, "stamp1", "../escape.toml").is_err());
    assert!(reset_user_rel(&paths, "stamp1", "/etc/passwd").is_err());
    let dest = reset_user_rel(&paths, "stamp1", "profiles/work.toml")
        .unwrap()
        .unwrap();
    assert!(!profile.is_file());
    assert!(dest.starts_with(&paths.state_dir));
    assert!(
        std::fs::read_to_string(&dest)
            .unwrap()
            .contains("panel_width")
    );
}

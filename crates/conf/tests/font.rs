//! Omarchy shell/font token resolution.

use omacell_conf::font::{ShellTokens, resolve_font_path, shell_tokens};
use omacell_conf::schema::AutoNum;

#[test]
fn shell_base_size_spacing_and_corner_tokens_parse() {
    let parsed = ShellTokens::parse(
        r#"
[font]
base-size = 13
[spacing]
scale = 1.25
[window]
corner-style = "sharp"
"#,
    )
    .unwrap();
    assert_eq!(parsed.font_base_size, Some(13.0));
    assert!(parsed.ui_font_path.is_none());
    assert_eq!(parsed.spacing_scale, 1.25);
    assert_eq!(parsed.corner_style.as_deref(), Some("sharp"));
}

#[test]
fn user_shell_tokens_overlay_the_active_theme() {
    let dir = tempfile::tempdir().unwrap();
    let paths = omacell_conf::Paths::from_home(dir.path());
    let theme = paths.omarchy_state.join("current/theme");
    std::fs::create_dir_all(&theme).unwrap();
    std::fs::write(
        theme.join("colors.toml"),
        include_str!("../../../tests/fixtures/omarchy-themes/tokyo-night/colors.toml"),
    )
    .unwrap();
    std::fs::write(
        theme.join("shell.toml"),
        "[font]\nbase-size = 12\n[spacing]\nscale = 1.1\n",
    )
    .unwrap();
    std::fs::create_dir_all(&paths.omarchy_config).unwrap();
    std::fs::write(
        paths.omarchy_config.join("shell.toml"),
        "[font]\nbase-size = 14\n",
    )
    .unwrap();

    let tokens = shell_tokens(&paths, &AutoNum::Token("system".into()), "system").unwrap();
    assert_eq!(tokens.ui_font_size_pt, 14.0);
    assert_eq!(tokens.spacing_scale, 1.1);
}

#[test]
fn dimensionless_font_scale_is_applied_to_the_default_size() {
    let parsed = ShellTokens::parse("[font]\nscale = 1.25\n").unwrap();
    assert_eq!(parsed.font_base_size, Some(13.75));
}

#[test]
fn resolved_font_path_is_loadable_when_fontconfig_exposes_one() {
    if let Some(path) = resolve_font_path("monospace") {
        assert!(path.is_file(), "{}", path.display());
    }
}

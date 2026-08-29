//! Template-vs-code mapping, mix, light mode, contrast.

use omacell_conf::theme::{
    ColorsToml, ROLE_MAP, Rgb, RoleSrc, ThemeRoles, contrast_ratio, mix,
    resolve_roles_with_override, template_placeholders,
};

fn fixtures() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/omarchy-themes");
    let mut out = Vec::new();
    for dir in ["", "community"] {
        let base = if dir.is_empty() {
            root.clone()
        } else {
            root.join(dir)
        };
        if !base.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&base).unwrap() {
            let entry = entry.unwrap();
            let colors = entry.path().join("colors.toml");
            if colors.is_file() {
                out.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(colors).unwrap(),
                ));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn template_agrees_with_role_map() {
    let tpl = template_placeholders();
    for (role, src) in ROLE_MAP {
        let expr = tpl
            .get(*role)
            .unwrap_or_else(|| panic!("template is missing {role}"));
        match src {
            RoleSrc::Key(k) => {
                assert!(
                    expr.contains(&format!("{{{{ {k} }}}}"))
                        || expr.contains(&format!("{{{{{k}}}}}")),
                    "{role} tpl={expr} key={k}"
                );
            }
            RoleSrc::Mix(a, b, p) => {
                assert!(
                    expr.contains("mix")
                        && expr.contains(a)
                        && expr.contains(b)
                        && expr.contains(&format!("{p}%")),
                    "{role} tpl={expr}"
                );
            }
        }
    }
    for i in 0..8 {
        assert!(tpl.contains_key(&format!("references.{i}")));
        assert!(tpl.contains_key(&format!("charts.palette.{i}")));
    }
}

#[test]
fn every_fixture_theme_maps() {
    for (name, text) in fixtures() {
        let colors = ColorsToml::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let roles =
            ThemeRoles::from_colors(&name, &colors, true).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            roles.roles.contains_key("surfaces.background"),
            "{name} missing background"
        );
        assert!(roles.roles.contains_key("text.foreground"), "{name}");
        assert!(roles.roles.contains_key("state.cursor"), "{name}");
    }
}

#[test]
fn template_renders_same_roles_for_every_fixture() {
    let template = template_placeholders();
    for (name, text) in fixtures() {
        let colors = ColorsToml::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let roles = ThemeRoles::from_colors(&name, &colors, false)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        for (role, expression) in &template {
            if role == "mode" {
                continue;
            }
            let expected = evaluate_template_expression(expression, &colors)
                .unwrap_or_else(|| panic!("{name}: unsupported template expression {expression}"));
            assert_eq!(
                roles.roles.get(role),
                Some(&expected.hex()),
                "{name}: {role}"
            );
        }
    }
}

fn evaluate_template_expression(expression: &str, colors: &ColorsToml) -> Option<Rgb> {
    let expression = expression
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim();
    let parts: Vec<_> = expression.split_whitespace().collect();
    match parts.as_slice() {
        [key] => colors.get(key),
        ["mix", first, second, percent] => {
            let percent = percent.strip_suffix('%')?.parse().ok()?;
            Some(mix(colors.get(first)?, colors.get(second)?, percent))
        }
        _ => None,
    }
}

#[test]
fn mix_is_linear() {
    let a = Rgb::parse("#000000").unwrap();
    let b = Rgb::parse("#ffffff").unwrap();
    let m = mix(a, b, 50);
    assert_eq!(m.r, 128);
    assert_eq!(m.g, 128);
    assert_eq!(m.b, 128);
}

#[test]
fn light_mode_grid_is_darker_than_background() {
    let text = include_str!("../../../tests/fixtures/omarchy-themes/catppuccin-latte/colors.toml");
    let colors = ColorsToml::parse(text).unwrap();
    assert_eq!(colors.mode, "light");
    let roles = ThemeRoles::from_colors("latte", &colors, false).unwrap();
    let bg = Rgb::parse(&roles.roles["surfaces.background"]).unwrap();
    let grid = Rgb::parse(&roles.roles["structure.grid_line"]).unwrap();
    assert!(
        omacell_conf::theme::relative_luminance(grid) < omacell_conf::theme::relative_luminance(bg),
        "light grid should be darker than bg"
    );
}

#[test]
fn low_contrast_is_nudged() {
    let text =
        include_str!("../../../tests/fixtures/omarchy-themes/community/low-contrast/colors.toml");
    let colors = ColorsToml::parse(text).unwrap();
    let off = ThemeRoles::from_colors("lc", &colors, false).unwrap();
    let on = ThemeRoles::from_colors("lc", &colors, true).unwrap();
    assert!(!on.nudged.is_empty(), "expected a contrast nudge");
    let bg = Rgb::parse(&on.roles["surfaces.background"]).unwrap();
    let muted = Rgb::parse(&on.roles["text.muted"]).unwrap();
    assert!(
        contrast_ratio(muted, bg)
            >= contrast_ratio(Rgb::parse(&off.roles["text.muted"]).unwrap(), bg)
    );
}

#[test]
fn reference_and_chart_palettes_match_appendix_c() {
    let text = include_str!("../../../tests/fixtures/omarchy-themes/tokyo-night/colors.toml");
    let colors = ColorsToml::parse(text).unwrap();
    let roles = ThemeRoles::from_colors("tokyo-night", &colors, false).unwrap();
    let expected_refs = [
        "color4", "color2", "color5", "color3", "color6", "color1", "accent", "color7",
    ];
    let expected_charts = [
        "accent", "color2", "color3", "color5", "color6", "color1", "color4", "color7",
    ];
    for (i, key) in expected_refs.iter().enumerate() {
        assert_eq!(
            roles.roles[&format!("references.{i}")],
            colors.get(key).unwrap().hex()
        );
    }
    for (i, key) in expected_charts.iter().enumerate() {
        assert_eq!(
            roles.roles[&format!("charts.palette.{i}")],
            colors.get(key).unwrap().hex()
        );
    }
}

#[test]
fn rendered_theme_is_a_partial_overlay_and_keeps_array_roles() {
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
        theme.join("omacell.toml"),
        r##"
[state]
cursor = "#010203"
[references]
colors = ["#000001", "#000002", "#000003", "#000004", "#000005", "#000006", "#000007", "#000008"]
[charts]
palette = ["#100001", "#100002", "#100003", "#100004", "#100005", "#100006", "#100007", "#100008"]
"##,
    )
    .unwrap();

    let roles = resolve_roles_with_override(&paths, None, false, false).unwrap();
    assert_eq!(roles.roles["state.cursor"], "#010203");
    assert!(roles.roles.contains_key("surfaces.background"));
    assert_eq!(roles.roles["references.7"], "#000008");
    assert_eq!(roles.roles["charts.palette.6"], "#100007");
}

#[test]
fn explicit_theme_override_wins_and_is_validated() {
    let dir = tempfile::tempdir().unwrap();
    let paths = omacell_conf::Paths::from_home(dir.path());
    let override_path = dir.path().join("override.toml");
    std::fs::write(&override_path, "[state]\ncursor = \"#123456\"\n").unwrap();
    let roles = resolve_roles_with_override(&paths, Some(&override_path), false, false).unwrap();
    assert_eq!(roles.roles["state.cursor"], "#123456");

    std::fs::write(&override_path, "[state]\ncursor = \"#not-a-color\"\n").unwrap();
    assert!(resolve_roles_with_override(&paths, Some(&override_path), false, false).is_err());
}

#[test]
fn invalid_theme_mode_is_rejected() {
    assert!(ColorsToml::parse("mode = \"sepia\"\n").is_err());
    assert!(ColorsToml::parse("mode = 1\n").is_err());
}

#[test]
fn contrast_is_enforced_after_user_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let paths = omacell_conf::Paths::from_home(dir.path());
    std::fs::create_dir_all(&paths.user_config).unwrap();
    std::fs::write(paths.user_theme_toml(), "[text]\nmuted = \"#111111\"\n").unwrap();
    let roles = resolve_roles_with_override(&paths, None, true, false).unwrap();
    let bg = Rgb::parse(&roles.roles["surfaces.background"]).unwrap();
    let muted = Rgb::parse(&roles.roles["text.muted"]).unwrap();
    assert!(contrast_ratio(muted, bg) >= 4.5);
}

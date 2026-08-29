//! Template-vs-code mapping, mix, light mode, contrast.

use omacell_conf::theme::{
    ColorsToml, ROLE_MAP, Rgb, RoleSrc, ThemeRoles, contrast_ratio, mix, template_placeholders,
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
        let Some(expr) = tpl.get(*role) else {
            continue;
        };
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

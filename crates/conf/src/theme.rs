//! Omarchy `colors.toml` → Omacell color roles (spec §7.1, Appendix C).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error;
use crate::paths::Paths;

/// One mapping from `colors.toml` into a role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleSrc {
    /// Direct key (`{{ background }}`).
    Key(&'static str),
    /// `mix a b p%`.
    Mix(&'static str, &'static str, u8),
}

/// Appendix C mapping (code and `omacell.toml.tpl` must agree).
pub const ROLE_MAP: &[(&str, RoleSrc)] = &[
    ("surfaces.background", RoleSrc::Key("background")),
    ("surfaces.surface", RoleSrc::Key("lighter_background")),
    (
        "surfaces.header_background",
        RoleSrc::Key("dark_background"),
    ),
    (
        "surfaces.popup_background",
        RoleSrc::Key("darker_background"),
    ),
    ("text.foreground", RoleSrc::Key("foreground")),
    ("text.muted", RoleSrc::Key("muted")),
    ("text.header_foreground", RoleSrc::Key("dark_foreground")),
    ("text.bright", RoleSrc::Key("bright_foreground")),
    (
        "structure.grid_line",
        RoleSrc::Mix("background", "foreground", 12),
    ),
    (
        "structure.pane_divider",
        RoleSrc::Mix("background", "foreground", 35),
    ),
    ("structure.frozen_edge", RoleSrc::Key("accent")),
    ("state.cursor", RoleSrc::Key("accent")),
    ("state.selection", RoleSrc::Key("selection")),
    ("state.selection_border", RoleSrc::Key("accent")),
    ("state.active_header", RoleSrc::Key("accent")),
    ("state.hover", RoleSrc::Mix("background", "foreground", 6)),
    ("state.stale", RoleSrc::Mix("background", "muted", 50)),
    ("semantic.error", RoleSrc::Key("red")),
    ("semantic.warning", RoleSrc::Key("color3")),
    ("semantic.success", RoleSrc::Key("color2")),
    ("semantic.info", RoleSrc::Key("color4")),
    ("semantic.link", RoleSrc::Key("blue")),
    ("charts.axis", RoleSrc::Key("dark_foreground")),
    (
        "charts.gridline",
        RoleSrc::Mix("background", "foreground", 10),
    ),
    ("conditional.scale_low", RoleSrc::Key("red")),
    ("conditional.scale_mid", RoleSrc::Key("color3")),
    ("conditional.scale_high", RoleSrc::Key("color2")),
    ("conditional.data_bar", RoleSrc::Key("accent")),
];

/// sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// Parse `#rgb` or `#rrggbb`.
    pub fn parse(s: &str) -> Result<Self, omacell_core::error::CoreError> {
        let s = s.trim();
        let h = s.strip_prefix('#').unwrap_or(s);
        match h.len() {
            3 => {
                let r = u8::from_str_radix(&h[0..1], 16)
                    .map_err(|_| error::theme(format!("invalid color {s}")))?;
                let g = u8::from_str_radix(&h[1..2], 16)
                    .map_err(|_| error::theme(format!("invalid color {s}")))?;
                let b = u8::from_str_radix(&h[2..3], 16)
                    .map_err(|_| error::theme(format!("invalid color {s}")))?;
                Ok(Self {
                    r: r * 17,
                    g: g * 17,
                    b: b * 17,
                })
            }
            6 => {
                let n = u32::from_str_radix(h, 16)
                    .map_err(|_| error::theme(format!("invalid color {s}")))?;
                Ok(Self {
                    r: ((n >> 16) & 0xFF) as u8,
                    g: ((n >> 8) & 0xFF) as u8,
                    b: (n & 0xFF) as u8,
                })
            }
            _ => Err(error::theme(format!("invalid color {s}"))),
        }
    }

    /// `#rrggbb`.
    #[must_use]
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// Blend `a` toward `b` by `percent` (0–100) in sRGB.
#[must_use]
pub fn mix(a: Rgb, b: Rgb, percent: u8) -> Rgb {
    let t = f64::from(percent.clamp(0, 100)) / 100.0;
    let ch =
        |x: u8, y: u8| -> u8 { (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8 };
    Rgb {
        r: ch(a.r, b.r),
        g: ch(a.g, b.g),
        b: ch(a.b, b.b),
    }
}

/// WCAG relative luminance.
#[must_use]
pub fn relative_luminance(c: Rgb) -> f64 {
    let lin = |u: u8| {
        let s = f64::from(u) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// Contrast ratio of two colours.
#[must_use]
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (l1, l2) = (relative_luminance(a), relative_luminance(b));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

/// Nudge `color` along `lo`↔`hi` until contrast vs `against` is at least `min`.
#[must_use]
pub fn nudge_contrast(color: Rgb, against: Rgb, lo: Rgb, hi: Rgb, min: f64) -> Rgb {
    if contrast_ratio(color, against) >= min {
        return color;
    }
    let mut best = color;
    let mut best_c = contrast_ratio(color, against);
    for pct in 0..=100u8 {
        let cand = mix(lo, hi, pct);
        let c = contrast_ratio(cand, against);
        if c > best_c {
            best = cand;
            best_c = c;
        }
        if c >= min {
            return cand;
        }
    }
    best
}

/// Parsed Omarchy `colors.toml`.
#[derive(Clone, Debug, Default)]
pub struct ColorsToml {
    /// `light` or `dark`.
    pub mode: String,
    /// Canonical and extra keys (hex).
    pub keys: BTreeMap<String, Rgb>,
}

impl ColorsToml {
    /// Parse TOML text.
    pub fn parse(text: &str) -> Result<Self, omacell_core::error::CoreError> {
        let value: toml::Value =
            toml::from_str(text).map_err(|e| error::theme(format!("colors.toml: {e}")))?;
        let mut keys = BTreeMap::new();
        let mut mode = String::from("dark");
        if let toml::Value::Table(t) = value {
            for (k, v) in t {
                if k == "mode" {
                    if let Some(s) = v.as_str() {
                        mode = s.to_string();
                    }
                    continue;
                }
                if let Some(s) = v.as_str()
                    && let Ok(rgb) = Rgb::parse(s)
                {
                    keys.insert(k, rgb);
                }
            }
        }
        Ok(Self { mode, keys })
    }

    /// Lookup with `color1`…`color7` aliases.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Rgb> {
        if let Some(c) = self.keys.get(key) {
            return Some(*c);
        }
        let alias = match key {
            "color1" => "red",
            "color2" => "green",
            "color3" => "yellow",
            "color4" => "blue",
            "color5" => "magenta",
            "color6" => "cyan",
            "color7" => "bright_foreground",
            _ => return None,
        };
        self.keys.get(alias).copied()
    }
}

/// Resolved Omacell roles.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ThemeRoles {
    /// Theme name or `"neutral"`.
    pub name: String,
    /// `light` / `dark`.
    pub mode: String,
    /// Role → hex.
    pub roles: BTreeMap<String, String>,
    /// Roles that were contrast-nudged.
    pub nudged: Vec<String>,
}

impl ThemeRoles {
    /// Built-in mapping applied to `colors`.
    pub fn from_colors(
        name: &str,
        colors: &ColorsToml,
        enforce_contrast: bool,
    ) -> Result<Self, omacell_core::error::CoreError> {
        let mut roles = BTreeMap::new();
        for (role, src) in ROLE_MAP {
            let rgb = match src {
                RoleSrc::Key(k) => colors
                    .get(k)
                    .ok_or_else(|| error::theme(format!("missing colors.toml key {k}")))?,
                RoleSrc::Mix(a, b, p) => {
                    let ca = colors
                        .get(a)
                        .ok_or_else(|| error::theme(format!("missing colors.toml key {a}")))?;
                    let cb = colors
                        .get(b)
                        .ok_or_else(|| error::theme(format!("missing colors.toml key {b}")))?;
                    mix(ca, cb, *p)
                }
            };
            roles.insert((*role).to_string(), rgb);
        }
        // Reference / chart palettes.
        let cycle = [
            "color4", "color2", "color5", "color3", "color6", "color1", "accent", "color7",
        ];
        for (i, k) in cycle.iter().enumerate() {
            if let Some(c) = colors.get(k) {
                roles.insert(format!("references.{i}"), c);
                roles.insert(format!("charts.palette.{i}"), c);
            }
        }
        if let Some(c) = colors.get("accent") {
            roles.insert("charts.palette.0".into(), c);
            roles.insert("references.6".into(), c);
        }
        let mut nudged = Vec::new();
        if enforce_contrast {
            let bg = *roles
                .get("surfaces.background")
                .unwrap_or(&Rgb { r: 0, g: 0, b: 0 });
            let fg = *roles.get("text.foreground").unwrap_or(&Rgb {
                r: 255,
                g: 255,
                b: 255,
            });
            let header_bg = *roles.get("surfaces.header_background").unwrap_or(&bg);
            let pairs: &[(&str, Rgb, f64)] = &[
                ("text.muted", bg, 4.5),
                ("text.header_foreground", header_bg, 4.5),
                ("text.bright", bg, 4.5),
                ("structure.grid_line", bg, 1.5),
                ("structure.pane_divider", bg, 1.5),
            ];
            for (role, against, min) in pairs {
                if let Some(cur) = roles.get(*role).copied() {
                    let next = nudge_contrast(cur, *against, bg, fg, *min);
                    if next != cur {
                        tracing::debug!(role, from = %cur.hex(), to = %next.hex(), "contrast nudge");
                        roles.insert((*role).to_string(), next);
                        nudged.push((*role).to_string());
                    }
                }
            }
        }
        let hex = roles.into_iter().map(|(k, v)| (k, v.hex())).collect();
        Ok(Self {
            name: name.to_string(),
            mode: colors.mode.clone(),
            roles: hex,
            nudged,
        })
    }
}

/// Locate the active Omarchy theme directory.
#[must_use]
pub fn active_theme_dir(paths: &Paths) -> Option<PathBuf> {
    let v4 = paths.omarchy_state.join("current/theme");
    if v4.join("colors.toml").is_file() {
        return Some(v4);
    }
    let v3 = paths.omarchy_config.join("current/theme");
    if v3.join("colors.toml").is_file() {
        return Some(v3);
    }
    None
}

/// Neutral palette when no Omarchy theme is present.
#[must_use]
pub fn neutral_colors(light: bool) -> ColorsToml {
    let text = if light {
        include_str!("../../../tests/fixtures/omarchy-themes/community/community-light/colors.toml")
    } else {
        include_str!("../../../tests/fixtures/omarchy-themes/tokyo-night/colors.toml")
    };
    ColorsToml::parse(text).expect("shipped fixture")
}

/// Resolve roles: user theme.toml overlay, then Omarchy `omacell.toml`, then mapping.
pub fn resolve_roles(
    paths: &Paths,
    enforce_contrast: bool,
    portal_light: bool,
) -> Result<ThemeRoles, omacell_core::error::CoreError> {
    if let Some(dir) = active_theme_dir(paths) {
        let name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("theme")
            .to_string();
        let colors = ColorsToml::parse(
            &std::fs::read_to_string(dir.join("colors.toml"))
                .map_err(|e| error::theme(e.to_string()))?,
        )?;
        let mut roles = if dir.join("omacell.toml").is_file() {
            roles_from_omacell_toml(
                &std::fs::read_to_string(dir.join("omacell.toml"))
                    .map_err(|e| error::theme(e.to_string()))?,
            )?
        } else {
            ThemeRoles::from_colors(&name, &colors, false)?
        };
        if enforce_contrast {
            let rebuilt = ThemeRoles::from_colors(&name, &colors, true)?;
            roles.nudged = rebuilt.nudged;
            for k in [
                "text.muted",
                "text.header_foreground",
                "text.bright",
                "structure.grid_line",
                "structure.pane_divider",
            ] {
                if let Some(v) = rebuilt.roles.get(k) {
                    roles.roles.insert(k.to_string(), v.clone());
                }
            }
        }
        overlay_user_theme(paths, &mut roles)?;
        roles.name = theme_name(&dir, &name);
        return Ok(roles);
    }
    let colors = neutral_colors(portal_light);
    let mut roles = ThemeRoles::from_colors("neutral", &colors, enforce_contrast)?;
    overlay_user_theme(paths, &mut roles)?;
    Ok(roles)
}

fn theme_name(dir: &Path, fallback: &str) -> String {
    let name_file = dir.parent().map(|p| p.join("theme.name"));
    if let Some(p) = name_file
        && let Ok(s) = std::fs::read_to_string(p)
    {
        return s.trim().to_string();
    }
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(fallback)
        .to_string()
}

fn overlay_user_theme(
    paths: &Paths,
    roles: &mut ThemeRoles,
) -> Result<(), omacell_core::error::CoreError> {
    let p = paths.user_theme_toml();
    if !p.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(&p).map_err(|e| error::io(e.to_string()))?;
    let v: toml::Value =
        toml::from_str(&text).map_err(|e| error::parse(format!("{}: {e}", p.display())))?;
    flatten_hex(&v, "", &mut roles.roles);
    Ok(())
}

fn flatten_hex(v: &toml::Value, prefix: &str, out: &mut BTreeMap<String, String>) {
    match v {
        toml::Value::Table(t) => {
            for (k, child) in t {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_hex(child, &next, out);
            }
        }
        toml::Value::String(s) if s.starts_with('#') => {
            out.insert(prefix.to_string(), s.clone());
        }
        _ => {}
    }
}

fn roles_from_omacell_toml(text: &str) -> Result<ThemeRoles, omacell_core::error::CoreError> {
    let v: toml::Value = toml::from_str(text).map_err(|e| error::theme(e.to_string()))?;
    let mut roles = BTreeMap::new();
    flatten_hex(&v, "", &mut roles);
    let mode = v
        .get("mode")
        .and_then(|x| x.as_str())
        .unwrap_or("dark")
        .to_string();
    Ok(ThemeRoles {
        name: "omacell.toml".into(),
        mode,
        roles,
        nudged: Vec::new(),
    })
}

/// Desktop-portal color-scheme: `true` = prefer light. Fails open to dark.
#[must_use]
pub fn portal_prefers_light() -> bool {
    read_portal_prefers_light().unwrap_or(false)
}

fn read_portal_prefers_light() -> Option<bool> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let m = conn
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Settings"),
            "Read",
            &("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?;
    let value: u32 = m.body().deserialize().ok()?;
    Some(value == 2)
}

/// Shipped Appendix C template.
pub const TEMPLATE: &str = include_str!("../../../default/themed/omacell.toml.tpl");

/// Extract `{{ … }}` expressions assigned in the template, keyed by role path.
#[must_use]
pub fn template_placeholders() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut section = String::new();
    for raw in TEMPLATE.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']).to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim().trim_matches('"').to_string();
            if !val.contains("{{") {
                continue;
            }
            let path = if section.is_empty() {
                key.to_string()
            } else {
                format!("{section}.{key}")
            };
            out.insert(path, val);
        }
    }
    out
}

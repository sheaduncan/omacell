//! Fontconfig alias resolution and file-font substitutions (spec §7.2).

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::OnceLock;

use serde::Serialize;

use crate::error;
use crate::paths::Paths;
use crate::theme::active_theme_dir;

/// Excel/Office family → metric-compatible free family.
#[must_use]
pub fn substitutions() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("Calibri", "Carlito"),
        ("Aptos", "Carlito"),
        ("Arial", "Liberation Sans"),
        ("Times New Roman", "Liberation Serif"),
        ("Cambria", "Caladea"),
    ])
}

/// Substitute a per-cell font from a file, or return `name` unchanged.
#[must_use]
pub fn substitute_file_font(name: &str) -> &str {
    substitutions()
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| *v)
        .unwrap_or(name)
}

/// Resolve the fontconfig `monospace` (or other) alias via fontdb.
#[must_use]
pub fn resolve_family(alias: &str) -> String {
    if let Ok(output) = Command::new("fc-match")
        .args(["-f", "%{family}\n", alias])
        .output()
        && output.status.success()
        && let Some(name) = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .map(str::trim)
        && !name.is_empty()
    {
        return name.split(',').next().unwrap_or(name).to_string();
    }
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let family = match alias {
        "sans-serif" => fontdb::Family::SansSerif,
        "serif" => fontdb::Family::Serif,
        _ => fontdb::Family::Monospace,
    };
    let families = [family];
    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight::NORMAL,
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    };
    if let Some(id) = db.query(&query)
        && let Some(face) = db.face(id)
        && let Some(name) = face.families.first()
    {
        return name.0.clone();
    }
    alias.to_string()
}

/// UI text size in pt: explicit number, else shell scale, else 11.
#[must_use]
pub fn ui_font_size_pt(configured: &crate::schema::AutoNum, shell_scale: Option<f64>) -> f64 {
    match configured {
        crate::schema::AutoNum::Num(n) if *n > 0.0 => *n,
        _ => shell_scale
            .filter(|s| *s > 0.0)
            .or_else(|| gtk_text_scale().map(|scale| 11.0 * scale))
            .unwrap_or(11.0),
    }
}

/// Read `[font] size` / `scale` from an Omarchy `shell.toml` if present.
#[must_use]
pub fn shell_font_scale(shell_toml: &str) -> Option<f64> {
    let v: toml::Value = toml::from_str(shell_toml).ok()?;
    let font = v.get("font")?;
    let absolute = font
        .get("base-size")
        .or_else(|| font.get("base_size"))
        .or_else(|| font.get("size"))
        .and_then(number);
    absolute.or_else(|| font.get("scale").and_then(number).map(|scale| 11.0 * scale))
}

/// Resolved typography and shell layout tokens.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ShellTokens {
    /// Font size supplied by Omarchy before the explicit Omacell override.
    pub font_base_size: Option<f64>,
    /// Resolved fontconfig family for UI chrome.
    pub ui_font_family: String,
    /// Effective UI size in points.
    pub ui_font_size_pt: f64,
    /// Shared spacing multiplier.
    pub spacing_scale: f64,
    /// Shell corner style when supplied.
    pub corner_style: Option<String>,
}

impl ShellTokens {
    /// Parse one merged Omarchy `shell.toml` document.
    pub fn parse(text: &str) -> Result<Self, omacell_core::error::CoreError> {
        let value: toml::Value =
            toml::from_str(text).map_err(|e| error::parse(format!("shell.toml: {e}")))?;
        Self::from_value(&value)
    }

    fn from_value(value: &toml::Value) -> Result<Self, omacell_core::error::CoreError> {
        let font_base_size = value.get("font").and_then(|font| {
            font.get("base-size")
                .or_else(|| font.get("base_size"))
                .or_else(|| font.get("size"))
                .and_then(number)
                .or_else(|| font.get("scale").and_then(number).map(|scale| 11.0 * scale))
        });
        if font_base_size.is_some_and(|size| !size.is_finite() || size <= 0.0) {
            return Err(error::schema(
                "shell.toml font size/scale must be a finite positive number",
            ));
        }
        let spacing_scale = value
            .get("spacing")
            .and_then(|spacing| spacing.get("scale"))
            .and_then(number)
            .unwrap_or(1.0);
        if !spacing_scale.is_finite() || spacing_scale <= 0.0 {
            return Err(error::schema(
                "shell.toml spacing.scale must be a finite positive number",
            ));
        }
        let corner_style = ["window", "appearance", "style"]
            .iter()
            .find_map(|section| {
                value.get(*section).and_then(|table| {
                    table
                        .get("corner-style")
                        .or_else(|| table.get("corner_style"))
                        .and_then(toml::Value::as_str)
                })
            })
            .map(ToOwned::to_owned);
        if let Some(style) = &corner_style
            && !matches!(style.as_str(), "rounded" | "sharp")
        {
            return Err(error::schema(
                "shell.toml corner style must be rounded or sharp",
            ));
        }
        Ok(Self {
            font_base_size,
            ui_font_family: String::new(),
            ui_font_size_pt: font_base_size.unwrap_or(11.0),
            spacing_scale,
            corner_style,
        })
    }
}

/// Resolve active-theme and user `shell.toml` tokens plus Omacell overrides.
pub fn shell_tokens(
    paths: &Paths,
    configured_size: &crate::schema::AutoNum,
    configured_corner: &str,
) -> Result<ShellTokens, omacell_core::error::CoreError> {
    shell_tokens_for_font(paths, configured_size, "monospace", configured_corner)
}

/// Resolve shell tokens using the configured UI font alias.
pub fn shell_tokens_for_font(
    paths: &Paths,
    configured_size: &crate::schema::AutoNum,
    configured_font: &str,
    configured_corner: &str,
) -> Result<ShellTokens, omacell_core::error::CoreError> {
    let mut value = toml::Value::Table(toml::map::Map::new());
    if let Some(theme) = active_theme_dir(paths) {
        merge_shell_file(&mut value, &theme.join("shell.toml"))?;
    }
    merge_shell_file(&mut value, &paths.omarchy_config.join("shell.toml"))?;
    let mut tokens = ShellTokens::from_value(&value)?;
    let alias = if configured_font == "system" {
        "monospace"
    } else {
        configured_font
    };
    tokens.ui_font_family = resolve_family(alias);
    tokens.ui_font_size_pt = ui_font_size_pt(configured_size, tokens.font_base_size);
    if configured_corner != "system" {
        tokens.corner_style = Some(configured_corner.to_string());
    }
    Ok(tokens)
}

fn merge_shell_file(
    value: &mut toml::Value,
    path: &std::path::Path,
) -> Result<(), omacell_core::error::CoreError> {
    if !path.is_file() {
        return Ok(());
    }
    let text = std::fs::read_to_string(path).map_err(|e| error::io(e.to_string()))?;
    let overlay: toml::Value =
        toml::from_str(&text).map_err(|e| error::parse(format!("{}: {e}", path.display())))?;
    merge_value(value, overlay);
    Ok(())
}

fn merge_value(dst: &mut toml::Value, src: toml::Value) {
    match (dst, src) {
        (toml::Value::Table(dst), toml::Value::Table(src)) => {
            for (key, value) in src {
                match dst.get_mut(&key) {
                    Some(existing) => merge_value(existing, value),
                    None => {
                        dst.insert(key, value);
                    }
                }
            }
        }
        (dst, src) => *dst = src,
    }
}

fn number(value: &toml::Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|number| number as f64))
}

fn gtk_text_scale() -> Option<f64> {
    static SCALE: OnceLock<Option<f64>> = OnceLock::new();
    *SCALE.get_or_init(|| {
        let output = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "text-scaling-factor"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
    })
}

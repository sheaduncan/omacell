//! Fontconfig alias resolution and file-font substitutions (spec §7.2).

use std::collections::BTreeMap;

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
        _ => shell_scale.filter(|s| *s > 0.0).unwrap_or(11.0),
    }
}

/// Read `[font] size` / `scale` from an Omarchy `shell.toml` if present.
#[must_use]
pub fn shell_font_scale(shell_toml: &str) -> Option<f64> {
    let v: toml::Value = toml::from_str(shell_toml).ok()?;
    let font = v.get("font")?;
    font.get("scale")
        .or_else(|| font.get("size"))
        .and_then(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64)))
}

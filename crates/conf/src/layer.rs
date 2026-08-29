//! Layered load, provenance, env, and CLI `--set`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::json;
use toml::Value;

use crate::error;
use crate::paths::Paths;
use crate::schema::{Config, DEFAULT_TOML};
use crate::theme::{ThemeRoles, portal_prefers_light, resolve_roles};

/// Configuration layer (lowest → highest).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// `/usr/share/omacell/default`.
    Package,
    /// Active Omarchy theme (roles only; not config keys).
    Theme,
    /// `~/.config/omacell/`.
    User,
    /// Workbook-stored settings.
    Workbook,
    /// `OMACELL_*`.
    Env,
    /// CLI `--set`.
    Cli,
}

impl Layer {
    /// Stable name for `--explain` / JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Package => "package",
            Self::Theme => "theme",
            Self::User => "user",
            Self::Workbook => "workbook",
            Self::Env => "env",
            Self::Cli => "cli",
        }
    }
}

/// Where a dotted key was last set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// Layer.
    pub layer: Layer,
    /// File path, env var, or `--set`.
    pub source: String,
}

/// Effective configuration plus provenance.
#[derive(Clone, Debug)]
pub struct LoadedConfig {
    /// Typed config.
    pub config: Config,
    /// Dotted-key provenance.
    pub provenance: BTreeMap<String, Provenance>,
    /// Resolved color roles.
    pub theme: ThemeRoles,
}

impl LoadedConfig {
    /// Effective JSON value of `dotted` key (`appearance.grid_lines`).
    #[must_use]
    pub fn get_json(&self, dotted: &str) -> Option<serde_json::Value> {
        let v = serde_json::to_value(&self.config).ok()?;
        walk_json(&v, dotted)
    }

    /// `omacell config show <key> --explain`.
    #[must_use]
    pub fn explain(&self, dotted: &str) -> Option<Explain> {
        let value = self.get_json(dotted)?;
        let prov = self.provenance.get(dotted).cloned().unwrap_or(Provenance {
            layer: Layer::Package,
            source: "<package-default>".into(),
        });
        Some(Explain {
            key: dotted.to_string(),
            value,
            layer: prov.layer,
            source: prov.source,
        })
    }
}

/// Explain payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Explain {
    /// Dotted key.
    pub key: String,
    /// Effective JSON value.
    pub value: serde_json::Value,
    /// Winning layer.
    pub layer: Layer,
    /// File / env / flag.
    pub source: String,
}

/// Load all layers.
pub fn load(
    paths: &Paths,
    cli_sets: &[String],
    workbook: Option<&Value>,
) -> Result<LoadedConfig, omacell_core::error::CoreError> {
    let mut value: Value =
        toml::from_str(DEFAULT_TOML).map_err(|e| error::parse(format!("package defaults: {e}")))?;
    let mut provenance = BTreeMap::new();
    mark_leaves(
        &value,
        "",
        Layer::Package,
        "<package-default>",
        &mut provenance,
    );

    let user = paths.user_config_toml();
    if user.is_file() {
        let text = std::fs::read_to_string(&user).map_err(|e| error::io(e.to_string()))?;
        let overlay = parse_user_toml(&text, &user)?;
        merge(
            &mut value,
            overlay,
            Layer::User,
            &user.display().to_string(),
            "",
            &mut provenance,
        );
    }

    if let Some(wb) = workbook {
        merge(
            &mut value,
            wb.clone(),
            Layer::Workbook,
            "<workbook>",
            "",
            &mut provenance,
        );
    }

    merge_env_pairs(&mut value, std::env::vars(), &mut provenance)?;
    merge_cli(&mut value, cli_sets, &mut provenance)?;
    finish_load(paths, value, provenance)
}

/// Load with an explicit environment (tests).
pub fn load_with_env(
    paths: &Paths,
    cli_sets: &[String],
    workbook: Option<&Value>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<LoadedConfig, omacell_core::error::CoreError> {
    let mut value: Value =
        toml::from_str(DEFAULT_TOML).map_err(|e| error::parse(format!("package defaults: {e}")))?;
    let mut provenance = BTreeMap::new();
    mark_leaves(
        &value,
        "",
        Layer::Package,
        "<package-default>",
        &mut provenance,
    );
    let user = paths.user_config_toml();
    if user.is_file() {
        let text = std::fs::read_to_string(&user).map_err(|e| error::io(e.to_string()))?;
        let overlay = parse_user_toml(&text, &user)?;
        merge(
            &mut value,
            overlay,
            Layer::User,
            &user.display().to_string(),
            "",
            &mut provenance,
        );
    }
    if let Some(wb) = workbook {
        merge(
            &mut value,
            wb.clone(),
            Layer::Workbook,
            "<workbook>",
            "",
            &mut provenance,
        );
    }
    merge_env_pairs(&mut value, env, &mut provenance)?;
    merge_cli(&mut value, cli_sets, &mut provenance)?;
    finish_load(paths, value, provenance)
}

fn finish_load(
    paths: &Paths,
    value: Value,
    provenance: BTreeMap<String, Provenance>,
) -> Result<LoadedConfig, omacell_core::error::CoreError> {
    let config: Config =
        Config::deserialize(value).map_err(|e| error::schema(format!("merged config: {e}")))?;
    if config.schema != 1 {
        return Err(error::schema(format!(
            "unsupported schema {}; expected 1",
            config.schema
        )));
    }
    let theme = resolve_roles(
        paths,
        config.appearance.enforce_contrast,
        portal_prefers_light(),
    )?;
    Ok(LoadedConfig {
        config,
        provenance,
        theme,
    })
}

fn parse_user_toml(text: &str, path: &Path) -> Result<Value, omacell_core::error::CoreError> {
    toml::from_str(text).map_err(|e| {
        let loc = e
            .span()
            .map(|s| format!("{}:{}", path.display(), line_of(text, s.start)))
            .unwrap_or_else(|| path.display().to_string());
        error::parse(format!("{loc}: {e}"))
    })
}

fn line_of(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

fn merge(
    dst: &mut Value,
    src: Value,
    layer: Layer,
    source: &str,
    path: &str,
    prov: &mut BTreeMap<String, Provenance>,
) {
    match (dst, src) {
        (Value::Table(dt), Value::Table(st)) => {
            for (k, sv) in st {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match dt.get_mut(&k) {
                    Some(dv) => merge(dv, sv, layer, source, &child, prov),
                    None => {
                        mark_leaves(&sv, &child, layer, source, prov);
                        dt.insert(k, sv);
                    }
                }
            }
        }
        (dst, src) => {
            *dst = src;
            if !path.is_empty() {
                prov.insert(
                    path.to_string(),
                    Provenance {
                        layer,
                        source: source.into(),
                    },
                );
            }
        }
    }
}

fn mark_leaves(
    v: &Value,
    path: &str,
    layer: Layer,
    source: &str,
    prov: &mut BTreeMap<String, Provenance>,
) {
    match v {
        Value::Table(t) => {
            for (k, child) in t {
                let next = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                mark_leaves(child, &next, layer, source, prov);
            }
        }
        _ if !path.is_empty() => {
            prov.insert(
                path.to_string(),
                Provenance {
                    layer,
                    source: source.into(),
                },
            );
        }
        _ => {}
    }
}

fn merge_env_pairs(
    dst: &mut Value,
    env: impl IntoIterator<Item = (String, String)>,
    prov: &mut BTreeMap<String, Provenance>,
) -> Result<(), omacell_core::error::CoreError> {
    let mut pairs: Vec<(String, String)> = env
        .into_iter()
        .filter(|(k, _)| {
            k.starts_with("OMACELL_") && k != "OMACELL_THEME" && k != "OMACELL_DEFAULT_DIR"
        })
        .collect();
    pairs.sort();
    for (k, v) in pairs {
        let dotted = env_key_to_dotted(&k);
        if dotted.is_empty() {
            continue;
        }
        set_dotted(dst, &dotted, parse_scalar(&v), Layer::Env, &k, prov)?;
    }
    Ok(())
}

fn env_key_to_dotted(k: &str) -> String {
    k.trim_start_matches("OMACELL_")
        .to_ascii_lowercase()
        .replace("__", ".")
}

fn merge_cli(
    dst: &mut Value,
    sets: &[String],
    prov: &mut BTreeMap<String, Provenance>,
) -> Result<(), omacell_core::error::CoreError> {
    for spec in sets {
        let (key, val) = spec
            .split_once('=')
            .ok_or_else(|| error::schema(format!("--set expects key=value, got {spec}")))?;
        set_dotted(
            dst,
            key.trim(),
            parse_scalar(val.trim()),
            Layer::Cli,
            spec,
            prov,
        )?;
    }
    Ok(())
}

fn parse_scalar(s: &str) -> Value {
    if s == "true" {
        return Value::Boolean(true);
    }
    if s == "false" {
        return Value::Boolean(false);
    }
    if let Ok(i) = s.parse::<i64>() {
        return Value::Integer(i);
    }
    if let Ok(f) = s.parse::<f64>() {
        return Value::Float(f);
    }
    Value::String(s.to_string())
}

fn set_dotted(
    root: &mut Value,
    dotted: &str,
    val: Value,
    layer: Layer,
    source: &str,
    prov: &mut BTreeMap<String, Provenance>,
) -> Result<(), omacell_core::error::CoreError> {
    let parts: Vec<&str> = dotted.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err(error::schema("empty config key"));
    }
    let mut cur = root;
    for (i, p) in parts.iter().enumerate() {
        if i + 1 == parts.len() {
            if let Value::Table(t) = cur {
                t.insert((*p).to_string(), val);
                prov.insert(
                    dotted.to_string(),
                    Provenance {
                        layer,
                        source: source.into(),
                    },
                );
                return Ok(());
            }
            return Err(error::schema(format!("{dotted} is not a table path")));
        }
        match cur {
            Value::Table(t) => {
                cur = t
                    .entry((*p).to_string())
                    .or_insert_with(|| Value::Table(toml::map::Map::new()));
            }
            _ => return Err(error::schema(format!("{dotted} is not a table path"))),
        }
    }
    Ok(())
}

fn walk_json(v: &serde_json::Value, dotted: &str) -> Option<serde_json::Value> {
    let mut cur = v;
    for p in dotted.split('.') {
        cur = cur.get(p)?;
    }
    Some(cur.clone())
}

/// Dump the effective config as JSON (WP-13 `config show --all --json`).
#[must_use]
pub fn show_all_json(loaded: &LoadedConfig) -> serde_json::Value {
    json!({
        "config": loaded.config,
        "provenance": loaded.provenance.iter().map(|(k, p)| {
            (k.clone(), json!({"layer": p.layer.as_str(), "source": p.source}))
        }).collect::<BTreeMap<_, _>>(),
        "theme": loaded.theme,
    })
}

/// Move `user` to a timestamped backup and restore package defaults (no-op if absent).
pub fn reset_user_file(
    paths: &Paths,
    stamp: &str,
) -> Result<Option<std::path::PathBuf>, omacell_core::error::CoreError> {
    let src = paths.user_config_toml();
    if !src.is_file() {
        return Ok(None);
    }
    let dir = paths.backup_dir(stamp);
    std::fs::create_dir_all(&dir).map_err(|e| error::io(e.to_string()))?;
    let dest = dir.join("config.toml");
    std::fs::rename(&src, &dest).map_err(|e| error::io(e.to_string()))?;
    Ok(Some(dest))
}

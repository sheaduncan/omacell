//! Layered load, provenance, env, and CLI `--set`.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::json;
use toml::Value;

use omacell_core::workbook::{CalcMode, WorkbookSettings};

use crate::error;
use crate::font::{ShellTokens, shell_tokens_for_font};
use crate::paths::Paths;
use crate::schema::{CURRENT_SCHEMA, Config, DEFAULT_TOML};
use crate::theme::{ThemeRoles, portal_prefers_light, resolve_roles_with_override};

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
    /// Resolved Omarchy shell/font tokens.
    pub shell: ShellTokens,
    /// User-file migrations performed during this load.
    pub migrations: Vec<Migration>,
}

/// Sources retained across live reloads.
#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// Explicit user config file (`--config`); replaces the default `config.toml`.
    pub config_file: Option<PathBuf>,
    /// CLI `--set key=value` overlays.
    pub cli_sets: Vec<String>,
    /// Workbook-stored configuration overlay.
    pub workbook: Option<Value>,
    /// Environment snapshot, including an optional `OMACELL_THEME`.
    pub env: Vec<(String, String)>,
    /// Explicit CLI `--theme` path; wins over `OMACELL_THEME`.
    pub theme_override: Option<PathBuf>,
}

impl LoadOptions {
    /// Capture the current process environment with no workbook or CLI overlays.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            env: std::env::vars().collect(),
            ..Self::default()
        }
    }
}

/// One backed-up schema migration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    /// Previous schema number.
    pub from: u32,
    /// Resulting schema number.
    pub to: u32,
    /// Backup created before rewriting.
    pub backup: PathBuf,
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
    load_with_options(
        paths,
        &LoadOptions {
            config_file: None,
            cli_sets: cli_sets.to_vec(),
            workbook: workbook.cloned(),
            env: std::env::vars().collect(),
            theme_override: None,
        },
    )
}

/// Load with an explicit environment (tests).
pub fn load_with_env(
    paths: &Paths,
    cli_sets: &[String],
    workbook: Option<&Value>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<LoadedConfig, omacell_core::error::CoreError> {
    load_with_options(
        paths,
        &LoadOptions {
            config_file: None,
            cli_sets: cli_sets.to_vec(),
            workbook: workbook.cloned(),
            env: env.into_iter().collect(),
            theme_override: None,
        },
    )
}

/// Load all layers from a reusable source snapshot.
pub fn load_with_options(
    paths: &Paths,
    options: &LoadOptions,
) -> Result<LoadedConfig, omacell_core::error::CoreError> {
    let mut value: Value =
        toml::from_str(DEFAULT_TOML).map_err(|e| error::parse(format!("package defaults: {e}")))?;
    let mut provenance = BTreeMap::new();
    let mut migrations = Vec::new();
    mark_leaves(
        &value,
        "",
        Layer::Package,
        "<package-default>",
        &mut provenance,
    );
    let user = options
        .config_file
        .clone()
        .unwrap_or_else(|| paths.user_config_toml());
    if options.config_file.is_some() && !user.is_file() {
        return Err(error::io(format!(
            "explicit config file does not exist: {}",
            user.display()
        )));
    }
    if user.is_file() {
        let (overlay, migration) = read_and_migrate_user(paths, &user)?;
        if let Some(migration) = migration {
            migrations.push(migration);
        }
        merge(
            &mut value,
            overlay,
            Layer::User,
            &user.display().to_string(),
            "",
            &mut provenance,
        );
    }
    if let Some(wb) = &options.workbook {
        merge(
            &mut value,
            wb.clone(),
            Layer::Workbook,
            "<workbook>",
            "",
            &mut provenance,
        );
    }
    merge_env_pairs(&mut value, options.env.iter().cloned(), &mut provenance)?;
    merge_cli(&mut value, &options.cli_sets, &mut provenance)?;
    let env_theme = options
        .env
        .iter()
        .find(|(key, _)| key == "OMACELL_THEME")
        .map(|(_, value)| PathBuf::from(value));
    let theme_override = options.theme_override.as_ref().or(env_theme.as_ref());
    finish_load(paths, value, provenance, theme_override, migrations)
}

/// Translate frozen workbook settings into the configuration keys they override.
#[must_use]
pub fn workbook_settings_overlay(settings: &WorkbookSettings) -> Value {
    let mut behavior = toml::map::Map::new();
    behavior.insert(
        "date_system".into(),
        Value::Integer(i64::from(settings.date_system.epoch_year())),
    );
    behavior.insert(
        "precision_as_displayed".into(),
        Value::Boolean(settings.precision_as_displayed),
    );

    let mode = match settings.calc_mode {
        CalcMode::Automatic => "automatic",
        CalcMode::AutomaticExceptTables => "automatic_except_tables",
        CalcMode::Manual => "manual",
    };
    let mut calc = toml::map::Map::new();
    calc.insert("mode".into(), Value::String(mode.into()));
    calc.insert(
        "iterative".into(),
        Value::Boolean(settings.iteration.enabled),
    );
    calc.insert(
        "max_iterations".into(),
        Value::Integer(i64::from(settings.iteration.max_iterations)),
    );
    calc.insert(
        "max_change".into(),
        Value::Float(settings.iteration.max_change),
    );

    Value::Table(toml::map::Map::from_iter([
        ("behavior".into(), Value::Table(behavior)),
        ("calc".into(), Value::Table(calc)),
    ]))
}

fn finish_load(
    paths: &Paths,
    value: Value,
    provenance: BTreeMap<String, Provenance>,
    theme_override: Option<&PathBuf>,
    migrations: Vec<Migration>,
) -> Result<LoadedConfig, omacell_core::error::CoreError> {
    let config: Config =
        Config::deserialize(value).map_err(|e| error::schema(format!("merged config: {e}")))?;
    config.validate()?;
    let theme = resolve_roles_with_override(
        paths,
        theme_override.map(PathBuf::as_path),
        config.appearance.enforce_contrast,
        portal_prefers_light(),
    )?;
    let shell = shell_tokens_for_font(
        paths,
        &config.appearance.ui_font_size,
        &config.appearance.ui_font,
        &config.appearance.corner_style,
    )?;
    Ok(LoadedConfig {
        config,
        provenance,
        theme,
        shell,
        migrations,
    })
}

fn read_and_migrate_user(
    paths: &Paths,
    path: &Path,
) -> Result<(Value, Option<Migration>), omacell_core::error::CoreError> {
    let original = std::fs::read_to_string(path).map_err(|e| error::io(e.to_string()))?;
    let mut value = parse_user_toml(&original, path)?;
    let Some(schema) = value.get("schema").and_then(Value::as_integer) else {
        return Ok((value, None));
    };
    let schema = u32::try_from(schema)
        .map_err(|_| error::schema("schema must be a non-negative integer"))?;
    if schema > CURRENT_SCHEMA {
        return Err(error::schema(format!(
            "unsupported schema {schema}; expected at most {CURRENT_SCHEMA}"
        )));
    }
    if schema == CURRENT_SCHEMA {
        return Ok((value, None));
    }

    let backup = backup_user_text(paths, &original)?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| error::schema("user config root must be a table"))?;
    table.insert("schema".into(), Value::Integer(i64::from(CURRENT_SCHEMA)));
    let rewritten = toml::to_string_pretty(&value)
        .map_err(|e| error::schema(format!("migrated config: {e}")))?;
    atomic_write(path, rewritten.as_bytes())?;
    Ok((
        value,
        Some(Migration {
            from: schema,
            to: CURRENT_SCHEMA,
            backup,
        }),
    ))
}

fn backup_user_text(paths: &Paths, text: &str) -> Result<PathBuf, omacell_core::error::CoreError> {
    let stamp = unique_stamp()?;
    let dir = paths.backup_dir(&stamp);
    std::fs::create_dir_all(&dir).map_err(|e| error::io(e.to_string()))?;
    let backup = dir.join("config.toml");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)
        .map_err(|e| error::io(e.to_string()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| error::io(e.to_string()))?;
    Ok(backup)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), omacell_core::error::CoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| error::io("config path has no parent"))?;
    let stamp = unique_stamp()?;
    let temp = parent.join(format!(".config.toml.omacell-new-{stamp}"));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|e| error::io(e.to_string()))
}

fn unique_stamp() -> Result<String, omacell_core::error::CoreError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| error::io(e.to_string()))?;
    Ok(format!(
        "{}-{:09}-{}",
        elapsed.as_secs(),
        elapsed.subsec_nanos(),
        std::process::id()
    ))
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
        "shell": loaded.shell,
        "migrations": loaded.migrations.iter().map(|migration| json!({
            "from": migration.from,
            "to": migration.to,
            "backup": migration.backup,
        })).collect::<Vec<_>>(),
    })
}

/// Move `user` to a timestamped backup and restore package defaults (no-op if absent).
pub fn reset_user_file(
    paths: &Paths,
    stamp: &str,
) -> Result<Option<std::path::PathBuf>, omacell_core::error::CoreError> {
    reset_user_rel(paths, stamp, "config.toml")
}

/// Reset a file beneath [`Paths::user_config`] using the same backup policy as
/// [`reset_user_file`]. `relative` must contain only normal path components.
pub fn reset_user_rel(
    paths: &Paths,
    stamp: &str,
    relative: &str,
) -> Result<Option<std::path::PathBuf>, omacell_core::error::CoreError> {
    validate_backup_stamp(stamp)?;
    let relative = validate_user_rel(relative)?;
    let src = paths.user_config.join(&relative);
    let metadata = match std::fs::symlink_metadata(&src) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(error::io(err.to_string())),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error::schema(
            "reset path must name a regular file, not a link or directory",
        ));
    }
    let root = std::fs::canonicalize(&paths.user_config).map_err(|e| error::io(e.to_string()))?;
    let parent = src
        .parent()
        .ok_or_else(|| error::schema("reset path has no parent"))?;
    let resolved_parent = std::fs::canonicalize(parent).map_err(|e| error::io(e.to_string()))?;
    if !resolved_parent.starts_with(&root) {
        return Err(error::schema(
            "reset path escaped the user config directory",
        ));
    }
    let dir = paths.backup_dir(stamp);
    let dest = dir.join(relative);
    if dest.exists() {
        return Err(error::io(format!(
            "backup already exists: {}",
            dest.display()
        )));
    }
    let dest_parent = dest
        .parent()
        .ok_or_else(|| error::schema("backup path has no parent"))?;
    std::fs::create_dir_all(dest_parent).map_err(|e| error::io(e.to_string()))?;
    std::fs::rename(&src, &dest).map_err(|e| error::io(e.to_string()))?;
    Ok(Some(dest))
}

/// Validate a user-config-relative file path without touching the filesystem.
pub fn validate_user_rel(relative: &str) -> Result<PathBuf, omacell_core::error::CoreError> {
    let relative = PathBuf::from(relative);
    if relative.is_absolute()
        || relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(error::schema(
            "reset path must be a relative file under the user config directory",
        ));
    }
    Ok(relative)
}

fn validate_backup_stamp(stamp: &str) -> Result<(), omacell_core::error::CoreError> {
    let mut components = Path::new(stamp).components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
        || !stamp
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(error::schema("backup stamp contains unsafe characters"));
    }
    Ok(())
}

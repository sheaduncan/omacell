//! Keymap loader, chord matching, and counts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use omacell_core::command::CommandId;
use omacell_core::error::CoreError;
use serde::Deserialize;
use serde_json::Value;

use crate::deferred;
use crate::error;
use crate::event::{KeyCode, KeyEvent};
use crate::mode::{KeyModel, Mode};

/// Search roots for `Config.keys.file` (composition root supplies these).
#[derive(Clone, Debug)]
pub struct KeymapRoots {
    /// `Paths::user_config`.
    pub user_config: PathBuf,
    /// Package `default/` directory.
    pub default_dir: PathBuf,
    /// Parent of `--config` when set.
    pub config_file_parent: Option<PathBuf>,
}

impl KeymapRoots {
    /// Build from a `Paths` value the caller already has — never `from_env`.
    #[must_use]
    pub fn new(user_config: PathBuf, default_dir: PathBuf, config_file: Option<&Path>) -> Self {
        Self {
            user_config,
            default_dir,
            config_file_parent: config_file.and_then(Path::parent).map(Path::to_path_buf),
        }
    }
}

/// Resolve `keys.file` as a safe relative path.
pub fn resolve_keymap_path(file: &str, roots: &KeymapRoots) -> Result<PathBuf, CoreError> {
    let rel = Path::new(file);
    if rel.is_absolute()
        || file.is_empty()
        || rel
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(error::keymap(
            "keys.file must be a relative path with only normal components",
        ));
    }
    let candidates = [
        roots.config_file_parent.as_ref().map(|p| p.join(rel)),
        Some(roots.user_config.join(rel)),
        Some(roots.default_dir.join(rel)),
    ];
    for path in candidates.into_iter().flatten() {
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(error::keymap(format!("keymap not found: {file}")))
}

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// Command id.
    pub cmd: String,
    /// Optional JSON args.
    pub args: Value,
}

/// Loaded keymap.
#[derive(Clone, Debug)]
pub struct Keymap {
    /// Display name.
    pub name: String,
    /// Classic vs modal.
    pub model: KeyModel,
    /// Leader chord (`Space`).
    pub leader: Option<String>,
    /// Mode → chord → binding. Classic uses the `classic` table.
    tables: BTreeMap<String, IndexMap<String, Binding>>,
    /// In-progress multi-key chord.
    pending: String,
    /// Modal count prefix.
    count: u32,
}

#[derive(Deserialize)]
struct FileMeta {
    #[serde(default)]
    name: Option<String>,
    model: String,
    #[serde(default)]
    leader: Option<String>,
}

#[derive(Deserialize)]
struct KeyFile {
    meta: FileMeta,
    #[serde(default = "empty_table")]
    bindings: toml::Value,
}

fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

impl Keymap {
    /// Parse a keymap TOML document.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        let file: KeyFile =
            toml::from_str(text).map_err(|e| error::keymap(format!("keymap toml: {e}")))?;
        let model = match file.meta.model.as_str() {
            "classic" => KeyModel::Classic,
            "modal" => KeyModel::Modal,
            other => {
                return Err(error::keymap(format!(
                    "unknown keymap model {other}; expected classic or modal"
                )));
            }
        };
        let mut tables = BTreeMap::new();
        let leader = file.meta.leader.clone();
        match file.bindings {
            toml::Value::Table(map) if model == KeyModel::Classic => {
                tables.insert(
                    "classic".into(),
                    parse_table(&toml::Value::Table(map), leader.as_deref())?,
                );
            }
            toml::Value::Table(map) => {
                for (mode, value) in map {
                    tables.insert(mode, parse_table(&value, leader.as_deref())?);
                }
            }
            other => {
                return Err(error::keymap(format!(
                    "bindings must be a table, got {other}"
                )));
            }
        }
        reject_duplicate_chords(&tables)?;
        Ok(Self {
            name: file.meta.name.unwrap_or_else(|| file.meta.model.clone()),
            model,
            leader: file.meta.leader,
            tables,
            pending: String::new(),
            count: 0,
        })
    }

    /// Load from a resolved path, overlaying an optional user file.
    pub fn load(path: &Path, user_overlay: Option<&Path>) -> Result<Self, CoreError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| error::keymap(format!("{}: {e}", path.display())))?;
        let mut map = Self::parse(&text)?;
        if let Some(user) = user_overlay.filter(|p| p.is_file()) {
            let overlay = std::fs::read_to_string(user)
                .map_err(|e| error::keymap(format!("{}: {e}", user.display())))?;
            let extra = Self::parse(&overlay)?;
            if extra.model != map.model {
                return Err(error::keymap(
                    "user keymap model must match the package map",
                ));
            }
            for (mode, table) in extra.tables {
                let dest = map.tables.entry(mode).or_default();
                for (chord, binding) in table {
                    dest.insert(chord, binding);
                }
            }
            reject_duplicate_chords(&map.tables)?;
        }
        Ok(map)
    }

    /// Bindings for a mode.
    #[must_use]
    pub fn table(&self, mode: Mode) -> Option<&IndexMap<String, Binding>> {
        self.tables.get(mode.table())
    }

    /// Iterate every (mode, chord, binding).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str, &Binding)> {
        self.tables.iter().flat_map(|(mode, table)| {
            table
                .iter()
                .map(move |(chord, binding)| (mode.as_str(), chord.as_str(), binding))
        })
    }

    /// Reset pending chord / count.
    pub fn reset_pending(&mut self) {
        self.pending.clear();
        self.count = 0;
    }

    /// Feed a key. `Partial` means wait for more keys (`g` of `gg`).
    pub fn dispatch(&mut self, mode: Mode, event: KeyEvent) -> KeyOutcome {
        if self.model == KeyModel::Modal
            && mode == Mode::Normal
            && let KeyCode::Char(c) = event.code
            && c.is_ascii_digit()
            && !event.ctrl
            && !event.alt
            && self.pending.is_empty()
            && !(c == '0' && self.count == 0)
        {
            self.count = self
                .count
                .saturating_mul(10)
                .saturating_add(u32::from(c as u8 - b'0'));
            return KeyOutcome::Pending;
        }
        let token = event.chord();
        let candidate = if self.pending.is_empty() {
            token.clone()
        } else {
            format!("{}{token}", self.pending)
        };
        let Some(table) = self.tables.get(mode.table()) else {
            self.reset_pending();
            return KeyOutcome::Unbound;
        };
        if let Some(binding) = table.get(&candidate) {
            let count = if self.count == 0 { 1 } else { self.count };
            let cmd = binding.cmd.clone();
            let mut args = binding.args.clone();
            if count > 1 {
                if let Value::Object(map) = &mut args {
                    map.insert("count".into(), Value::from(count));
                } else if args.is_null() {
                    args = serde_json::json!({"count": count});
                }
            }
            self.reset_pending();
            return KeyOutcome::Command { cmd, args, count };
        }
        let prefix = table
            .keys()
            .any(|k| k.starts_with(&candidate) && k.len() > candidate.len());
        if prefix {
            self.pending = candidate;
            return KeyOutcome::Pending;
        }
        if !self.pending.is_empty() {
            self.reset_pending();
        }
        KeyOutcome::Unbound
    }
}

/// Result of [`Keymap::dispatch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Fire this command.
    Command {
        /// Id.
        cmd: String,
        /// Args (may include `count`).
        args: Value,
        /// Repeat count (at least 1).
        count: u32,
    },
    /// Multi-key chord or count digit.
    Pending,
    /// No binding.
    Unbound,
}

fn parse_table(
    value: &toml::Value,
    leader: Option<&str>,
) -> Result<IndexMap<String, Binding>, CoreError> {
    let Some(table) = value.as_table() else {
        return Err(error::keymap("bindings table expected"));
    };
    let mut out = IndexMap::new();
    for (chord, spec) in table {
        let chord = expand_leader(chord, leader);
        let normalized = normalize_chord(&chord);
        if out.contains_key(&normalized) {
            return Err(error::keymap(format!("duplicate chord {normalized}")));
        }
        let binding = match spec {
            toml::Value::String(cmd) => Binding {
                cmd: cmd.clone(),
                args: Value::Null,
            },
            toml::Value::Table(t) => {
                let cmd = t
                    .get("cmd")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| error::keymap(format!("{chord}: missing cmd")))?;
                let args = t.get("args").map(toml_to_json).unwrap_or(Value::Null);
                Binding {
                    cmd: cmd.to_string(),
                    args,
                }
            }
            _ => {
                return Err(error::keymap(format!(
                    "{chord}: expected command string or table"
                )));
            }
        };
        CommandId::new(&binding.cmd).map_err(|e| error::keymap(e.to_string()))?;
        out.insert(normalized, binding);
    }
    Ok(out)
}

fn reject_duplicate_chords(
    tables: &BTreeMap<String, IndexMap<String, Binding>>,
) -> Result<(), CoreError> {
    for (mode, table) in tables {
        let mut seen = BTreeMap::new();
        for chord in table.keys() {
            if let Some(prev) = seen.insert(chord.as_str(), mode.as_str()) {
                return Err(error::keymap(format!(
                    "duplicate chord {chord} in {prev}/{mode}"
                )));
            }
        }
    }
    Ok(())
}

fn expand_leader(chord: &str, leader: Option<&str>) -> String {
    if let Some(rest) = chord.strip_prefix("<leader>") {
        let leader = leader.unwrap_or("Space");
        if rest.is_empty() {
            leader.to_string()
        } else {
            format!("{leader}{rest}")
        }
    } else {
        chord.to_string()
    }
}

fn normalize_chord(raw: &str) -> String {
    if raw.eq_ignore_ascii_case("<leader>") {
        return "<leader>".into();
    }
    raw.split('+')
        .map(|p| {
            let t = p.trim();
            match t.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => "Ctrl".into(),
                "alt" | "a" | "mod1" => "Alt".into(),
                "shift" | "s" => "Shift".into(),
                "space" => "Space".into(),
                "esc" | "escape" => "Esc".into(),
                "return" => "Enter".into(),
                "pgup" | "pageup" => "PgUp".into(),
                "pgdn" | "pagedown" => "PgDn".into(),
                other => {
                    if other.starts_with('f')
                        && other.len() > 1
                        && other[1..].bytes().all(|b| b.is_ascii_digit())
                    {
                        format!("F{}", &other[1..])
                    } else {
                        t.to_string()
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn toml_to_json(v: &toml::Value) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// True when `id` is registered or listed in the deferred table.
#[must_use]
pub fn command_is_known(id: &str, registered: &[&str]) -> bool {
    registered.contains(&id) || deferred::owner(id).is_some()
}

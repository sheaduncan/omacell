//! Keymap loader, chord matching, and counts.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use omacell_bus::CommandRegistry;
use omacell_core::command::CommandId;
use omacell_core::error::CoreError;
use serde::Deserialize;
use serde_json::Value;

use crate::deferred;
use crate::error;
use crate::event::{KeyCode, KeyEvent};
use crate::mode::{KeyModel, Mode};

const MAX_KEYMAP_BYTES: u64 = 1024 * 1024;

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
    let primary = roots
        .config_file_parent
        .as_deref()
        .unwrap_or(roots.user_config.as_path());
    for root in [primary, roots.default_dir.as_path()] {
        if let Some(path) = resolve_candidate(root, rel)? {
            return Ok(path);
        }
    }
    Err(error::keymap(format!("keymap not found: {file}")))
}

fn resolve_candidate(root: &Path, relative: &Path) -> Result<Option<PathBuf>, CoreError> {
    let candidate = root.join(relative);
    match std::fs::symlink_metadata(&candidate) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(error::keymap(format!("{}: {err}", candidate.display()))),
    }
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|err| error::keymap(format!("{}: {err}", root.display())))?;
    let canonical = std::fs::canonicalize(&candidate)
        .map_err(|err| error::keymap(format!("{}: {err}", candidate.display())))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(error::keymap(format!(
            "keymap escaped its search root: {}",
            candidate.display()
        )));
    }
    if !canonical.is_file() {
        return Err(error::keymap(format!(
            "keymap is not a regular file: {}",
            candidate.display()
        )));
    }
    Ok(Some(canonical))
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
#[serde(deny_unknown_fields)]
struct FileMeta {
    #[serde(default)]
    name: Option<String>,
    model: String,
    #[serde(default)]
    leader: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    meta: FileMeta,
    #[serde(default = "empty_table")]
    bindings: toml::Value,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlayMeta {
    #[serde(default, rename = "name")]
    _name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    leader: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlayFile {
    #[serde(default)]
    meta: OverlayMeta,
    #[serde(default = "empty_table")]
    bindings: toml::Value,
}

fn empty_table() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

impl Keymap {
    /// Parse a keymap TOML document.
    pub fn parse(text: &str) -> Result<Self, CoreError> {
        Self::parse_with_leader(text, None)
    }

    fn parse_with_leader(text: &str, leader_override: Option<&str>) -> Result<Self, CoreError> {
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
        let leader = leader_override
            .map(str::to_string)
            .or_else(|| file.meta.leader.clone());
        let tables = parse_tables(file.bindings, model, leader.as_deref())?;
        reject_duplicate_chords(&tables)?;
        Ok(Self {
            name: file.meta.name.unwrap_or_else(|| file.meta.model.clone()),
            model,
            leader,
            tables,
            pending: String::new(),
            count: 0,
        })
    }

    /// Load from a resolved path, overlaying an optional user file.
    pub fn load(path: &Path, user_overlay: Option<&Path>) -> Result<Self, CoreError> {
        let text = read_bounded(path)?;
        let overlay = user_overlay.map(read_bounded).transpose()?;
        let parsed_overlay = overlay.as_deref().map(parse_overlay).transpose()?;
        let leader_override = parsed_overlay
            .as_ref()
            .and_then(|overlay| overlay.meta.leader.as_deref());
        let mut map = Self::parse_with_leader(&text, leader_override)?;
        if let Some(extra) = parsed_overlay {
            if let Some(model) = extra.meta.model.as_deref()
                && model != model_name(map.model)
            {
                return Err(error::keymap(
                    "user keymap model must match the package map",
                ));
            }
            let tables = parse_tables(extra.bindings, map.model, map.leader.as_deref())?;
            for (mode, table) in tables {
                let dest = map.tables.entry(mode).or_default();
                for (chord, binding) in table {
                    dest.insert(chord, binding);
                }
            }
            reject_duplicate_chords(&map.tables)?;
        }
        Ok(map)
    }

    /// Resolve a configured map and apply a sparse higher-priority override to
    /// the shipped package map when one exists.
    pub fn load_from_roots(file: &str, roots: &KeymapRoots) -> Result<Self, CoreError> {
        let selected = resolve_keymap_path(file, roots)?;
        let relative = Path::new(file);
        let package = resolve_candidate(&roots.default_dir, relative)?;
        match package {
            Some(package) if package != selected => Self::load(&package, Some(&selected)),
            _ => Self::load(&selected, None),
        }
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

    /// Reject bindings that are neither registered, composition-provided, nor
    /// assigned to a later WP.
    pub fn validate_commands(&self, registry: &CommandRegistry) -> Result<(), CoreError> {
        for (mode, chord, binding) in self.iter() {
            if registry.get_str(&binding.cmd).is_err()
                && deferred::owner(&binding.cmd).is_none()
                && !deferred::is_composition_command(&binding.cmd)
            {
                return Err(error::keymap(format!(
                    "unowned command {} for {mode} chord {chord}",
                    binding.cmd
                )));
            }
        }
        Ok(())
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
            if self.count > 0 && accepts_count_argument(&cmd) {
                match &mut args {
                    Value::Null => args = serde_json::json!({"count": count}),
                    Value::Object(fields) => {
                        fields.insert("count".into(), Value::from(count));
                    }
                    _ => {}
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
        self.reset_pending();
        KeyOutcome::Unbound
    }
}

fn accepts_count_argument(command: &str) -> bool {
    command.starts_with("nav.")
        || command.starts_with("sel.")
        || matches!(command, "sheet.next" | "sheet.prev")
}

fn read_bounded(path: &Path) -> Result<String, CoreError> {
    let file = std::fs::File::open(path)
        .map_err(|err| error::keymap(format!("{}: {err}", path.display())))?;
    let mut bytes = Vec::new();
    file.take(MAX_KEYMAP_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| error::keymap(format!("{}: {err}", path.display())))?;
    if bytes.len() as u64 > MAX_KEYMAP_BYTES {
        return Err(error::keymap(format!(
            "{} exceeds the {MAX_KEYMAP_BYTES}-byte keymap limit",
            path.display()
        )));
    }
    String::from_utf8(bytes)
        .map_err(|err| error::keymap(format!("{} is not UTF-8: {err}", path.display())))
}

fn parse_overlay(text: &str) -> Result<OverlayFile, CoreError> {
    toml::from_str(text).map_err(|err| error::keymap(format!("keymap overlay toml: {err}")))
}

fn model_name(model: KeyModel) -> &'static str {
    match model {
        KeyModel::Classic => "classic",
        KeyModel::Modal => "modal",
    }
}

fn parse_tables(
    bindings: toml::Value,
    model: KeyModel,
    leader: Option<&str>,
) -> Result<BTreeMap<String, IndexMap<String, Binding>>, CoreError> {
    let mut tables = BTreeMap::new();
    match bindings {
        toml::Value::Table(map) if model == KeyModel::Classic => {
            tables.insert(
                "classic".into(),
                parse_table(&toml::Value::Table(map), leader)?,
            );
        }
        toml::Value::Table(map) => {
            for (mode, value) in map {
                if !matches!(mode.as_str(), "normal" | "insert" | "visual" | "command") {
                    return Err(error::keymap(format!(
                        "unknown modal bindings table {mode}"
                    )));
                }
                tables.insert(mode, parse_table(&value, leader)?);
            }
        }
        other => {
            return Err(error::keymap(format!(
                "bindings must be a table, got {other}"
            )));
        }
    }
    Ok(tables)
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
                let args = t
                    .get("args")
                    .map(toml_to_json)
                    .transpose()?
                    .unwrap_or(Value::Null);
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
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut keys = Vec::new();
    let parts = raw.split('+').collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        let token = part.trim();
        let modifier = index + 1 < parts.len();
        match (modifier, token.to_ascii_lowercase().as_str()) {
            (true, "ctrl" | "control" | "c") => ctrl = true,
            (true, "alt" | "a" | "mod1") => alt = true,
            (true, "shift" | "s") => shift = true,
            _ => keys.push(normalize_key(token)),
        }
    }
    let mut out = String::new();
    if ctrl {
        out.push_str("Ctrl+");
    }
    if alt {
        out.push_str("Alt+");
    }
    if shift {
        out.push_str("Shift+");
    }
    let mut key = keys.join("+");
    if (ctrl || alt) && key.len() == 1 && key.is_ascii() {
        key.make_ascii_uppercase();
    }
    out.push_str(&key);
    out
}

fn normalize_key(token: &str) -> String {
    match token.to_ascii_lowercase().as_str() {
        "space" => "Space".into(),
        "esc" | "escape" => "Esc".into(),
        "return" => "Enter".into(),
        "pgup" | "pageup" => "PgUp".into(),
        "pgdn" | "pagedown" => "PgDn".into(),
        other
            if other.starts_with('f')
                && other.len() > 1
                && other[1..].bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            format!("F{}", &other[1..])
        }
        _ => token.to_string(),
    }
}

fn toml_to_json(v: &toml::Value) -> Result<Value, CoreError> {
    serde_json::to_value(v)
        .map_err(|err| error::keymap(format!("keymap arguments are not valid JSON: {err}")))
}

/// True when `id` is registered or listed in the deferred table.
#[must_use]
pub fn command_is_known(id: &str, registered: &[&str]) -> bool {
    registered.contains(&id)
        || deferred::owner(id).is_some()
        || deferred::is_composition_command(id)
}

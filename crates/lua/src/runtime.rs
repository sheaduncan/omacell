//! Lua 5.4 runtime, sandbox profiles, and the `omacell` API.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use mlua::{HookTriggers, Lua, Table, Value as LuaValue, VmState};
use omacell_core::addr::{RefKind, parse_a1};
use omacell_core::coerce::Scalar;
use omacell_core::error::CoreError;
use omacell_core::eval::{ArgVal, ArrayLift, DynamicFn, DynamicFnBody, RuntimeValue};
use omacell_core::value::Value;
use serde_json::Value as Json;
use std::sync::Mutex;

use crate::host::ScriptHost;
use crate::trust::{TrustStore, hash_path, sha256_hex, trust_path};

/// Instruction budget for embedded scripts (hook every 1000).
pub const EMBEDDED_INSTRUCTION_LIMIT: u32 = 1_000_000;
/// Memory budget for embedded scripts.
pub const EMBEDDED_MEMORY_LIMIT: usize = 8 * 1024 * 1024;
/// Custom-part path for a workbook-embedded script.
pub const EMBEDDED_PART: &str = "xl/omacell/scripts/main.lua";

/// Sandbox profile (F-10.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Config-dir / explicit CLI scripts: full standard library.
    User,
    /// Workbook-embedded scripts: no io/os/package/debug, caps.
    Embedded,
}

/// How embedded scripts are treated (`[scripting] embedded_scripts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddedMode {
    /// Trusted files run sandboxed.
    Sandbox,
    /// Never run embedded scripts.
    Deny,
}

impl EmbeddedMode {
    /// Parse the config string. `ask` is deny in non-interactive hosts.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "sandbox" => Self::Sandbox,
            _ => Self::Deny,
        }
    }

    /// Stricter of two modes (`deny` wins).
    #[must_use]
    pub fn stricter(self, other: Self) -> Self {
        match (self, other) {
            (Self::Deny, _) | (_, Self::Deny) => Self::Deny,
            (Self::Sandbox, Self::Sandbox) => Self::Sandbox,
        }
    }
}

/// Policy consumed from `LoadedConfig` (never parse TOML here).
#[derive(Clone, Debug)]
pub struct ScriptPolicy {
    /// Master switch.
    pub enabled: bool,
    /// Embedded mode.
    pub embedded: EmbeddedMode,
    /// Canonical trusted directories (missing entries dropped).
    pub trusted_dirs: Vec<std::path::PathBuf>,
}

impl ScriptPolicy {
    /// From conf + path canonicalization.
    #[must_use]
    pub fn from_config(enabled: bool, embedded_scripts: &str, trusted_dirs: &[String]) -> Self {
        let dirs = trusted_dirs
            .iter()
            .filter_map(|d| canonicalize_dir(d))
            .collect();
        Self {
            enabled,
            embedded: EmbeddedMode::parse(embedded_scripts),
            trusted_dirs: dirs,
        }
    }

    /// Apply a possibly stricter reload.
    pub fn tighten(&mut self, other: &Self) {
        self.enabled &= other.enabled;
        self.embedded = self.embedded.stricter(other.embedded);
        if other.trusted_dirs.len() < self.trusted_dirs.len() {
            self.trusted_dirs.clone_from(&other.trusted_dirs);
        }
    }
}

fn canonicalize_dir(spec: &str) -> Option<std::path::PathBuf> {
    let expanded = if let Some(rest) = spec.strip_prefix("~/") {
        let home = std::env::var_os("HOME")?;
        std::path::PathBuf::from(home).join(rest)
    } else if spec == "~" {
        std::path::PathBuf::from(std::env::var_os("HOME")?)
    } else {
        std::path::PathBuf::from(spec)
    };
    let canon = std::fs::canonicalize(&expanded).ok()?;
    canon.is_dir().then_some(canon)
}

/// Running Lua VM plus host.
pub struct Runtime {
    lua: Arc<Mutex<Lua>>,
    #[allow(dead_code)]
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    profile: Profile,
}

impl Runtime {
    /// Construct a VM for `profile`.
    pub fn new(profile: Profile, host: Box<dyn ScriptHost>) -> Result<Self, CoreError> {
        let lua = match profile {
            Profile::User => Lua::new(),
            Profile::Embedded => embedded_lua()?,
        };
        let host = Arc::new(Mutex::new(host));
        let lua = Arc::new(Mutex::new(lua));
        install_api(&lock_mutex(&lua), &host)?;
        Ok(Self { lua, host, profile })
    }

    /// Profile in force.
    #[must_use]
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Run a host command (e.g. `file.save` after a script).
    pub fn execute_cmd(&self, id: &str, args: Json) -> Result<Json, CoreError> {
        lock_mutex(&self.host).execute(id, args)
    }

    /// Execute a chunk. Errors include file:line when Lua reports it.
    pub fn exec(&self, source: &str, name: &str) -> Result<(), CoreError> {
        let lua = lock_mutex(&self.lua);
        lua.load(source)
            .set_name(name)
            .exec()
            .map_err(|e| lua_error(e, name))
    }

    /// Fire a named hook (`on_open`, …). Missing hooks are no-ops.
    pub fn emit_hook(&self, name: &str) -> Result<(), CoreError> {
        let lua = lock_mutex(&self.lua);
        let globals = lua.globals();
        let omacell: Table = globals
            .get("omacell")
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
        let hook: Option<mlua::Function> = omacell.get(name).unwrap_or(None);
        if let Some(hook) = hook {
            hook.call::<()>(()).map_err(|e| lua_error(e, name))?;
        }
        Ok(())
    }
}

fn embedded_lua() -> Result<Lua, CoreError> {
    let lua = Lua::new();
    lua.set_memory_limit(EMBEDDED_MEMORY_LIMIT)
        .map_err(|e| CoreError::new("lua.sandbox", e.to_string()))?;
    let counter = Arc::new(AtomicU32::new(0));
    let limit = EMBEDDED_INSTRUCTION_LIMIT / 1000;
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1000),
        move |_lua, _debug| {
            let n = counter.fetch_add(1, Ordering::Relaxed);
            if n >= limit {
                return Err(mlua::Error::RuntimeError(
                    "instruction limit exceeded".into(),
                ));
            }
            Ok(VmState::Continue)
        },
    );
    strip_embedded(&lua)?;
    Ok(lua)
}

fn strip_embedded(lua: &Lua) -> Result<(), CoreError> {
    let globals = lua.globals();
    for name in [
        "io",
        "os",
        "package",
        "debug",
        "require",
        "load",
        "loadfile",
        "dofile",
        "loadstring",
    ] {
        globals
            .set(name, LuaValue::Nil)
            .map_err(|e| CoreError::new("lua.sandbox", e.to_string()))?;
    }
    Ok(())
}

fn install_api(lua: &Lua, host: &Arc<Mutex<Box<dyn ScriptHost>>>) -> Result<(), CoreError> {
    let omacell = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let host_cmd = Arc::clone(host);
    let cmd = lua
        .create_function(move |_, (id, args): (String, LuaValue)| {
            let json = lua_to_json(&args).map_err(mlua::Error::external)?;
            let mut host = lock_mutex(&host_cmd);
            host.execute(&id, json).map_err(mlua::Error::external)?;
            Ok(())
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("cmd", cmd)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;

    let host_fn = Arc::clone(host);
    let register = lua
        .create_function(
            move |lua, (name, spec, func): (String, Table, mlua::Function)| {
                register_lua_fn(lua, &host_fn, name, spec, func).map_err(mlua::Error::external)
            },
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("fn", register)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;

    install_ui(lua, &omacell, host)?;
    install_events(lua, &omacell)?;
    install_keymap(lua, &omacell, host)?;
    install_ai(lua, &omacell)?;
    install_book(lua, &omacell, host)?;

    lua.globals()
        .set("omacell", omacell)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    Ok(())
}

fn install_ui(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
) -> Result<(), CoreError> {
    let ui = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    ui.set(
        "status",
        lua.create_function(move |_, msg: String| {
            lock_mutex(&h).status(&msg);
            Ok(())
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
    )
    .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    ui.set(
        "notify",
        lua.create_function(move |_, msg: String| {
            lock_mutex(&h).notify(&msg);
            Ok(())
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
    )
    .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    ui.set(
        "prompt",
        lua.create_function(move |_, msg: String| {
            lock_mutex(&h).prompt(&msg).map_err(mlua::Error::external)
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
    )
    .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("ui", ui)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

fn install_events(lua: &Lua, omacell: &Table) -> Result<(), CoreError> {
    for name in [
        "on_open",
        "on_change",
        "on_before_save",
        "on_recalc",
        "on_theme_change",
    ] {
        let key = name.to_string();
        let setter = lua
            .create_function(move |lua, func: mlua::Function| {
                let g = lua.globals();
                let omacell: Table = g.get("omacell")?;
                omacell.set(key.as_str(), func)?;
                Ok(())
            })
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
        omacell
            .set(name, setter)
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    }
    Ok(())
}

fn install_keymap(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
) -> Result<(), CoreError> {
    let keymap = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    keymap
        .set(
            "set",
            lua.create_function(move |_, (mode, keys, cmd): (String, String, String)| {
                lock_mutex(&h).keymap_set(&mode, &keys, &cmd);
                Ok(())
            })
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("keymap", keymap)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

fn install_ai(lua: &Lua, omacell: &Table) -> Result<(), CoreError> {
    let ai = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let stub = lua
        .create_function(|_, (): ()| -> mlua::Result<()> {
            Err(mlua::Error::RuntimeError(
                "omacell.ai is reserved for WP-23".into(),
            ))
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    ai.set("task", stub.clone())
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    ai.set("fn", stub)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("ai", ai)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

fn install_book(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
) -> Result<(), CoreError> {
    let h = Arc::clone(host);
    let getter = lua
        .create_function(move |lua, (): ()| {
            let host = lock_mutex(&h);
            let wb = host.workbook();
            let name = wb
                .sheet(wb.active_sheet())
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Sheet1".into());
            drop(host);
            LuaBook {
                host: Arc::clone(&h),
                active: name,
            }
            .into_lua_owned(lua)
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("book", getter)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

struct LuaBook {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    active: String,
}

struct LuaSheet {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    name: String,
}

struct LuaCell {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    sheet: String,
    a1: String,
}

struct LuaRange {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    sheet: String,
    a1: String,
}

impl mlua::UserData for LuaBook {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("sheet", |_, this, name: Option<String>| {
            let name = name.unwrap_or_else(|| this.active.clone());
            Ok(LuaSheet {
                host: Arc::clone(&this.host),
                name,
            })
        });
    }
}

impl mlua::UserData for LuaSheet {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cell", |_, this, a1: String| {
            Ok(LuaCell {
                host: Arc::clone(&this.host),
                sheet: this.name.clone(),
                a1,
            })
        });
        methods.add_method("range", |_, this, a1: String| {
            Ok(LuaRange {
                host: Arc::clone(&this.host),
                sheet: this.name.clone(),
                a1,
            })
        });
        methods.add_method("name", |_, this, (): ()| Ok(this.name.clone()));
    }
}

impl mlua::UserData for LuaCell {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("value", |lua, this| {
            let host = lock_mutex(&this.host);
            cell_lua_value(lua, host.workbook(), &this.sheet, &this.a1)
        });
        fields.add_field_method_get("input", |_, this| {
            let host = lock_mutex(&this.host);
            Ok(cell_input(host.workbook(), &this.sheet, &this.a1))
        });
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set", |_, this, input: String| {
            let mut host = lock_mutex(&this.host);
            let r#ref = qualify(&this.sheet, &this.a1);
            host.execute(
                "cell.set",
                serde_json::json!({"ref": r#ref, "input": input}),
            )
            .map_err(mlua::Error::external)?;
            Ok(())
        });
    }
}

impl mlua::UserData for LuaRange {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cells", |lua, this, (): ()| {
            let host = lock_mutex(&this.host);
            let cells = range_cells(host.workbook(), &this.sheet, &this.a1)
                .map_err(mlua::Error::external)?;
            drop(host);
            let table = lua.create_table()?;
            for (i, a1) in cells.into_iter().enumerate() {
                table.set(
                    i + 1,
                    LuaCell {
                        host: Arc::clone(&this.host),
                        sheet: this.sheet.clone(),
                        a1,
                    },
                )?;
            }
            Ok(table)
        });
    }
}

fn qualify(sheet: &str, a1: &str) -> String {
    if a1.contains('!') {
        a1.to_string()
    } else {
        format!("{sheet}!{a1}")
    }
}

fn cell_input(wb: &omacell_core::workbook::Workbook, sheet: &str, a1: &str) -> String {
    let Ok(parsed) = parse_a1(&qualify(sheet, a1)) else {
        return String::new();
    };
    let Ok(kind) = wb.resolve_parsed(parsed) else {
        return String::new();
    };
    let (sheet_id, row, col) = match kind {
        RefKind::Cell(c) => (c.sheet.unwrap_or(wb.active_sheet()), c.row, c.col),
        RefKind::Range(r) => (
            r.start.sheet.unwrap_or(wb.active_sheet()),
            r.start.row,
            r.start.col,
        ),
    };
    let Ok(Some(slot)) = wb.get(sheet_id, row, col) else {
        return String::new();
    };
    if let Some(fid) = slot.formula {
        return wb.intern().formulas.get(fid).unwrap_or("").to_string();
    }
    match slot.value {
        Value::Empty => String::new(),
        Value::Number(n) => n.to_string(),
        Value::Bool(true) => "TRUE".into(),
        Value::Bool(false) => "FALSE".into(),
        Value::Text(id) => wb.intern().strings.get(id).unwrap_or("").to_string(),
        Value::Error(e) => e.as_str().to_string(),
        Value::Array(_) => String::new(),
    }
}

fn cell_lua_value(
    lua: &Lua,
    wb: &omacell_core::workbook::Workbook,
    sheet: &str,
    a1: &str,
) -> mlua::Result<LuaValue> {
    let parsed = parse_a1(&qualify(sheet, a1)).map_err(mlua::Error::external)?;
    let kind = wb.resolve_parsed(parsed).map_err(mlua::Error::external)?;
    let (sheet_id, row, col) = match kind {
        RefKind::Cell(c) => (c.sheet.unwrap_or(wb.active_sheet()), c.row, c.col),
        RefKind::Range(r) => (
            r.start.sheet.unwrap_or(wb.active_sheet()),
            r.start.row,
            r.start.col,
        ),
    };
    let Some(slot) = wb.get(sheet_id, row, col).map_err(mlua::Error::external)? else {
        return Ok(LuaValue::Nil);
    };
    match slot.value {
        Value::Empty => Ok(LuaValue::Nil),
        Value::Number(n) => Ok(LuaValue::Number(n)),
        Value::Bool(b) => Ok(LuaValue::Boolean(b)),
        Value::Text(id) => {
            let t = wb.intern().strings.get(id).unwrap_or("");
            Ok(LuaValue::String(lua.create_string(t)?))
        }
        Value::Error(e) => Ok(LuaValue::String(lua.create_string(e.as_str())?)),
        Value::Array(_) => Ok(LuaValue::Nil),
    }
}

fn range_cells(
    wb: &omacell_core::workbook::Workbook,
    sheet: &str,
    a1: &str,
) -> Result<Vec<String>, CoreError> {
    let parsed = parse_a1(&qualify(sheet, a1))?;
    let kind = wb.resolve_parsed(parsed)?;
    let (sheet_id, r0, c0, r1, c1) = match kind {
        RefKind::Cell(c) => {
            let s = c.sheet.unwrap_or(wb.active_sheet());
            (s, c.row, c.col, c.row, c.col)
        }
        RefKind::Range(r) => {
            let s = r.start.sheet.unwrap_or(wb.active_sheet());
            (
                s,
                r.start.row.min(r.end.row),
                r.start.col.min(r.end.col),
                r.start.row.max(r.end.row),
                r.start.col.max(r.end.col),
            )
        }
    };
    let _ = sheet_id;
    let mut out = Vec::new();
    for r in r0..=r1 {
        for c in c0..=c1 {
            let col = omacell_core::addr::col_to_letters(c).unwrap_or_else(|_| "?".into());
            out.push(format!("{col}{}", r + 1));
        }
    }
    Ok(out)
}

struct LuaBody {
    func: mlua::Function,
}

impl DynamicFnBody for LuaBody {
    fn eval(&self, args: &[ArgVal]) -> RuntimeValue {
        let mut values = Vec::new();
        for arg in args {
            values.push(runtime_to_lua_light(&arg.value));
        }
        match self
            .func
            .call::<LuaValue>(mlua::MultiValue::from_vec(values))
        {
            Ok(v) => lua_to_runtime(&v),
            Err(_) => RuntimeValue::error(omacell_core::error::ErrorKind::Value),
        }
    }
}

fn runtime_to_lua_light(value: &RuntimeValue) -> LuaValue {
    match value {
        RuntimeValue::Scalar(Scalar::Empty) => LuaValue::Nil,
        RuntimeValue::Scalar(Scalar::Number(n)) => LuaValue::Number(*n),
        RuntimeValue::Scalar(Scalar::Bool(b)) => LuaValue::Boolean(*b),
        _ => LuaValue::Nil,
    }
}

fn lua_to_runtime(value: &LuaValue) -> RuntimeValue {
    match value {
        LuaValue::Nil => RuntimeValue::Scalar(Scalar::Empty),
        LuaValue::Boolean(b) => RuntimeValue::Scalar(Scalar::Bool(*b)),
        LuaValue::Integer(i) => RuntimeValue::Scalar(Scalar::Number(*i as f64)),
        LuaValue::Number(n) => RuntimeValue::Scalar(Scalar::Number(*n)),
        LuaValue::String(s) => match s.to_str() {
            Ok(t) => RuntimeValue::Scalar(Scalar::Text(std::sync::Arc::<str>::from(t.as_ref()))),
            Err(_) => RuntimeValue::error(omacell_core::error::ErrorKind::Value),
        },
        _ => RuntimeValue::error(omacell_core::error::ErrorKind::Value),
    }
}

fn register_lua_fn(
    lua: &Lua,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    name: String,
    spec: Table,
    func: mlua::Function,
) -> Result<(), CoreError> {
    if !name.contains('.') {
        return Err(
            CoreError::new("lua.fn", "custom functions must be namespaced (USER.NAME)")
                .with_hint("register omacell.fn(\"USER.DOUBLE\", spec, fn)"),
        );
    }
    let min_args: u8 = spec.get("min").unwrap_or(0);
    let max_args: u8 = spec.get("max").unwrap_or(min_args.max(1));
    let volatile: bool = spec.get("volatile").unwrap_or(false);
    let lift: String = spec.get("array_lift").unwrap_or_else(|_| "none".into());
    let array_lift = if lift == "all" {
        ArrayLift::All
    } else {
        ArrayLift::None
    };
    let globals = lua.globals();
    let omacell: Table = globals
        .get("omacell")
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let fns: Table = match omacell.get("_fns") {
        Ok(t) => t,
        Err(_) => {
            let t = lua
                .create_table()
                .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
            omacell
                .set("_fns", t.clone())
                .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
            t
        }
    };
    let body_fn = func.clone();
    fns.set(name.as_str(), func)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let def = DynamicFn {
        name: name.clone(),
        min_args,
        max_args,
        volatile,
        array_lift,
        body: Arc::new(LuaBody { func: body_fn }),
    };
    lock_mutex(host).register_function(def)
}

fn lua_to_json(value: &LuaValue) -> Result<Json, CoreError> {
    match value {
        LuaValue::Nil => Ok(Json::Null),
        LuaValue::Boolean(b) => Ok(Json::Bool(*b)),
        LuaValue::Integer(i) => Ok(Json::from(*i)),
        LuaValue::Number(n) => serde_json::Number::from_f64(*n)
            .map(Json::Number)
            .ok_or_else(|| CoreError::new("lua.args", "non-finite number")),
        LuaValue::String(s) => Ok(Json::String(
            s.to_str()
                .map_err(|e| CoreError::new("lua.args", e.to_string()))?
                .to_string(),
        )),
        LuaValue::Table(t) => {
            let mut map = serde_json::Map::new();
            t.for_each(|k: LuaValue, v: LuaValue| {
                if let LuaValue::String(s) = k {
                    let key = s.to_str().map_err(mlua::Error::external)?.to_string();
                    let json = lua_to_json(&v).map_err(mlua::Error::external)?;
                    map.insert(key, json);
                }
                Ok(())
            })
            .map_err(|e| CoreError::new("lua.args", e.to_string()))?;
            Ok(Json::Object(map))
        }
        _ => Err(CoreError::new("lua.args", "unsupported Lua value")),
    }
}

fn lua_error(err: mlua::Error, name: &str) -> CoreError {
    CoreError::new("lua.exec", format!("{name}: {err}"))
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Decide whether an embedded workbook script may run.
pub fn allow_embedded(
    policy: &ScriptPolicy,
    store: &TrustStore,
    book: &Path,
    bytes: &[u8],
) -> Result<(), CoreError> {
    if !policy.enabled {
        return Err(CoreError::new("lua.disabled", "scripting is disabled"));
    }
    if policy.embedded == EmbeddedMode::Deny {
        return Err(CoreError::new(
            "lua.embedded",
            "embedded scripts are denied by policy",
        ));
    }
    let hash = if book.is_file() {
        hash_path(book)?
    } else {
        sha256_hex(bytes)
    };
    if !store.contains_hash(&hash) {
        return Err(CoreError::new(
            "lua.untrusted",
            format!("workbook {} is not in the trust store", book.display()),
        )
        .with_hint("omacell trust add <file>"));
    }
    Ok(())
}

/// Load the trust store from a state directory.
pub fn load_trust(state_dir: &Path) -> Result<TrustStore, CoreError> {
    TrustStore::load(&trust_path(state_dir))
}

trait IntoLuaOwned {
    fn into_lua_owned(self, lua: &Lua) -> mlua::Result<LuaValue>;
}

impl IntoLuaOwned for LuaBook {
    fn into_lua_owned(self, lua: &Lua) -> mlua::Result<LuaValue> {
        Ok(LuaValue::UserData(lua.create_userdata(self)?))
    }
}

//! Lua 5.4 runtime, sandbox profiles, and the `omacell` API.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use mlua::{HookTriggers, Lua, Table, Value as LuaValue, Variadic, VmState};
use omacell_ai::{AiError, AiHookRequest, AiHookResponse, AiHooks, AiTaskSpec, ToolSpec};
use omacell_core::addr::{RefKind, parse_a1, quote_sheet_name};
use omacell_core::coerce::Scalar;
use omacell_core::error::{CoreError, ErrorKind};
use omacell_core::eval::{ArgVal, ArrayLift, DynamicFn, DynamicFnBody, RuntimeValue};
use omacell_core::value::Value;
use serde::Deserialize;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, TryLockError};

use crate::host::ScriptHost;
use crate::trust::{TrustStore, sha256_hex, trust_path};

/// Instruction budget for embedded scripts (hook every 1000).
pub const EMBEDDED_INSTRUCTION_LIMIT: u32 = 1_000_000;
/// Memory budget for embedded scripts.
pub const EMBEDDED_MEMORY_LIMIT: usize = 8 * 1024 * 1024;
/// Maximum bytes in one user `init.lua` or plugin entry point.
pub const MAX_USER_SCRIPT_BYTES: u64 = 1024 * 1024;
/// Custom-part path for a workbook-embedded script.
pub const EMBEDDED_PART: &str = "xl/omacell/scripts/main.lua";
const JSON_ARRAY_MARKER: &str = "__omacell_json_array";

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

    /// Build policy from the retained, already-layered configuration.
    #[must_use]
    pub fn from_loaded(loaded: &omacell_conf::LoadedConfig) -> Self {
        Self::from_config(
            loaded.config.scripting.enabled,
            &loaded.config.scripting.embedded_scripts,
            &loaded.config.scripting.trusted_dirs,
        )
    }

    /// Apply a possibly stricter reload.
    pub fn tighten(&mut self, other: &Self) {
        self.enabled &= other.enabled;
        self.embedded = self.embedded.stricter(other.embedded);
        self.trusted_dirs
            .retain(|dir| other.trusted_dirs.contains(dir));
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
    instruction_counter: Option<Arc<AtomicU32>>,
    script_depth: Arc<AtomicU32>,
    function_depth: Arc<AtomicU32>,
    script_digest: Arc<Mutex<Sha256>>,
    ai_tasks: Arc<Mutex<BTreeMap<String, AiTaskSpec>>>,
}

impl Runtime {
    /// Construct a VM for `profile`.
    pub fn new(profile: Profile, host: Box<dyn ScriptHost>) -> Result<Self, CoreError> {
        let (lua, instruction_counter) = match profile {
            Profile::User => (Lua::new(), None),
            Profile::Embedded => {
                let (lua, counter) = embedded_lua()?;
                (lua, Some(counter))
            }
        };
        let host = Arc::new(Mutex::new(host));
        let lua = Arc::new(Mutex::new(lua));
        let function_depth = Arc::new(AtomicU32::new(0));
        let script_depth = Arc::new(AtomicU32::new(0));
        let script_digest = Arc::new(Mutex::new(Sha256::new()));
        let ai_tasks = Arc::new(Mutex::new(BTreeMap::new()));
        install_api(
            &lock_mutex(&lua),
            &lua,
            &host,
            profile,
            &function_depth,
            &script_depth,
            &ai_tasks,
        )?;
        Ok(Self {
            lua,
            host,
            profile,
            instruction_counter,
            script_depth,
            function_depth,
            script_digest,
            ai_tasks,
        })
    }

    /// Profile in force.
    #[must_use]
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Snapshot validated user-profile AI task registrations.
    #[must_use]
    pub fn ai_tasks(&self) -> Vec<AiTaskSpec> {
        lock_mutex(&self.ai_tasks).values().cloned().collect()
    }

    /// Snapshot operational user-profile AI request/response hooks, if any.
    #[must_use]
    pub fn ai_hooks(&self) -> Option<Arc<dyn AiHooks>> {
        if self.profile != Profile::User || !has_ai_hooks(&lock_mutex(&self.lua)) {
            return None;
        }
        Some(Arc::new(LuaAiHooks {
            lua: Arc::clone(&self.lua),
            script_depth: Arc::clone(&self.script_depth),
            function_depth: Arc::clone(&self.function_depth),
            script_digest: Arc::clone(&self.script_digest),
        }))
    }

    /// Run a host command (e.g. `file.save` after a script).
    pub fn execute_cmd(&self, id: &str, args: Json) -> Result<Json, CoreError> {
        let _execution = FunctionEvaluation::enter(&self.script_depth);
        self.reset_instruction_budget();
        self.sync_host();
        dispatch_before_command(&lock_mutex(&self.lua), id)?;
        let (result, events) = {
            let mut host = lock_mutex(&self.host);
            let result = host.execute(id, args)?;
            let events = host.take_events();
            (result, events)
        };
        dispatch_command_events(&lock_mutex(&self.lua), &events)?;
        Ok(result)
    }

    /// Execute a chunk. Errors include file:line when Lua reports it.
    pub fn exec(&self, source: &str, name: &str) -> Result<(), CoreError> {
        let _execution = FunctionEvaluation::enter(&self.script_depth);
        self.reset_instruction_budget();
        self.sync_host();
        let lua = lock_mutex(&self.lua);
        lua.load(source)
            .set_name(name)
            .exec()
            .map_err(|e| lua_error(e, name))?;
        let mut digest = lock_mutex(&self.script_digest);
        digest.update(name.as_bytes());
        digest.update([0]);
        digest.update(source.as_bytes());
        digest.update([0]);
        Ok(())
    }

    /// Fire a named hook (`on_open`, …). Missing hooks are no-ops.
    pub fn emit_hook(&self, name: &str) -> Result<(), CoreError> {
        let _execution = FunctionEvaluation::enter(&self.script_depth);
        self.reset_instruction_budget();
        self.sync_host();
        let lua = lock_mutex(&self.lua);
        dispatch_hook(&lua, name)
    }

    /// Fire pre-command hooks before an interactive host queues a save.
    pub fn before_command(&self, id: &str) -> Result<(), CoreError> {
        let _execution = FunctionEvaluation::enter(&self.script_depth);
        self.reset_instruction_budget();
        self.sync_host();
        dispatch_before_command(&lock_mutex(&self.lua), id)
    }

    /// Dispatch committed bus events drained by a retained frontend runtime.
    pub fn emit_events(&self, events: &[omacell_core::event::Event]) -> Result<(), CoreError> {
        let _execution = FunctionEvaluation::enter(&self.script_depth);
        self.reset_instruction_budget();
        self.sync_host();
        dispatch_command_events(&lock_mutex(&self.lua), events)
    }

    fn sync_host(&self) {
        lock_mutex(&self.host).refresh();
    }

    fn reset_instruction_budget(&self) {
        if let Some(counter) = &self.instruction_counter {
            counter.store(0, Ordering::Relaxed);
        }
    }
}

pub(crate) fn embedded_lua() -> Result<(Lua, Arc<AtomicU32>), CoreError> {
    let lua = Lua::new();
    lua.set_memory_limit(EMBEDDED_MEMORY_LIMIT)
        .map_err(|e| CoreError::new("lua.sandbox", e.to_string()))?;
    let counter = Arc::new(AtomicU32::new(0));
    let hook_counter = Arc::clone(&counter);
    let limit = EMBEDDED_INSTRUCTION_LIMIT / 1000;
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1000),
        move |_lua, _debug| {
            let n = hook_counter.fetch_add(1, Ordering::Relaxed);
            if n >= limit {
                return Err(mlua::Error::RuntimeError(
                    "instruction limit exceeded".into(),
                ));
            }
            Ok(VmState::Continue)
        },
    );
    strip_embedded(&lua)?;
    Ok((lua, counter))
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
        // Lua's hook is per-thread, and hook errors can be caught by pcall.
        // Removing these prevents coroutine and protected-call bypasses of the
        // hard instruction budget.
        "coroutine",
        "pcall",
        "xpcall",
    ] {
        globals
            .set(name, LuaValue::Nil)
            .map_err(|e| CoreError::new("lua.sandbox", e.to_string()))?;
    }
    Ok(())
}

fn install_api(
    lua: &Lua,
    runtime_lua: &Arc<Mutex<Lua>>,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    profile: Profile,
    function_depth: &Arc<AtomicU32>,
    script_depth: &Arc<AtomicU32>,
    ai_tasks: &Arc<Mutex<BTreeMap<String, AiTaskSpec>>>,
) -> Result<(), CoreError> {
    install_print(lua, host, function_depth)?;
    let omacell = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let host_cmd = Arc::clone(host);
    let cmd_depth = Arc::clone(function_depth);
    let cmd = lua
        .create_function(move |lua, (id, args): (String, LuaValue)| {
            host_api_available(&cmd_depth)?;
            if profile == Profile::Embedded && !lock_mutex(&host_cmd).embedded_command_allowed(&id)
            {
                return Err(mlua::Error::external(CoreError::new(
                    "lua.sandbox",
                    format!("command {id} is not available to embedded scripts"),
                )));
            }
            let json = lua_to_json(&args).map_err(mlua::Error::external)?;
            dispatch_before_command(lua, &id).map_err(mlua::Error::external)?;
            let (result, events) = {
                let mut host = lock_mutex(&host_cmd);
                let result = host.execute(&id, json).map_err(mlua::Error::external)?;
                let events = host.take_events();
                (result, events)
            };
            dispatch_command_events(lua, &events).map_err(mlua::Error::external)?;
            json_to_lua(lua, &result)
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("cmd", cmd)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;

    let registration = FunctionRegistration {
        runtime_lua: Arc::clone(runtime_lua),
        host: Arc::clone(host),
        function_depth: Arc::clone(function_depth),
        script_depth: Arc::clone(script_depth),
    };
    let register = lua
        .create_function(
            move |lua, (name, spec, func): (String, Table, mlua::Function)| {
                host_api_available(&registration.function_depth)?;
                register_lua_fn(lua, &registration, name, spec, func).map_err(mlua::Error::external)
            },
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("fn", register)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;

    install_ui(lua, &omacell, host, profile, function_depth)?;
    install_events(lua, &omacell, profile)?;
    install_keymap(lua, &omacell, host, profile, function_depth)?;
    install_ai(lua, &omacell, host, profile, function_depth, ai_tasks)?;
    install_book(lua, &omacell, host, function_depth)?;

    lua.globals()
        .set("omacell", omacell)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    Ok(())
}

fn install_print(
    lua: &Lua,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    function_depth: &Arc<AtomicU32>,
) -> Result<(), CoreError> {
    let h = Arc::clone(host);
    let depth = Arc::clone(function_depth);
    let print = lua
        .create_function(move |lua, values: Variadic<LuaValue>| {
            host_api_available(&depth)?;
            let tostring: mlua::Function = lua.globals().get("tostring")?;
            let mut rendered = Vec::with_capacity(values.len());
            for value in values {
                rendered.push(tostring.call::<String>(value)?);
            }
            lock_mutex(&h).status(&rendered.join("\t"));
            Ok(())
        })
        .map_err(|error| CoreError::new("lua.api", error.to_string()))?;
    lua.globals()
        .set("print", print)
        .map_err(|error| CoreError::new("lua.api", error.to_string()))
}

fn install_ui(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    profile: Profile,
    function_depth: &Arc<AtomicU32>,
) -> Result<(), CoreError> {
    let ui = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    let depth = Arc::clone(function_depth);
    ui.set(
        "status",
        lua.create_function(move |_, msg: String| {
            host_api_available(&depth)?;
            lock_mutex(&h).status(&msg);
            Ok(())
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
    )
    .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    let depth = Arc::clone(function_depth);
    ui.set(
        "notify",
        lua.create_function(move |_, msg: String| {
            host_api_available(&depth)?;
            lock_mutex(&h).notify(&msg);
            Ok(())
        })
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
    )
    .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    if profile == Profile::User {
        let h = Arc::clone(host);
        let depth = Arc::clone(function_depth);
        ui.set(
            "prompt",
            lua.create_function(move |_, msg: String| {
                host_api_available(&depth)?;
                lock_mutex(&h).prompt(&msg).map_err(mlua::Error::external)
            })
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    }
    omacell
        .set("ui", ui)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

fn install_events(lua: &Lua, omacell: &Table, profile: Profile) -> Result<(), CoreError> {
    let hooks = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    for name in [
        "on_open",
        "on_change",
        "on_before_save",
        "on_recalc",
        "on_theme_change",
        "on_ai_request",
        "on_ai_response",
    ] {
        if profile == Profile::Embedded && matches!(name, "on_ai_request" | "on_ai_response") {
            continue;
        }
        hooks
            .set(
                name,
                lua.create_table()
                    .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
            )
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
        let key = name.to_string();
        let setter = lua
            .create_function(move |lua, func: mlua::Function| {
                let g = lua.globals();
                let omacell: Table = g.get("omacell")?;
                let hooks: Table = omacell.get("_hooks")?;
                let registered: Table = hooks.get(key.as_str())?;
                registered.set(registered.raw_len() + 1, func)?;
                Ok(())
            })
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
        omacell
            .set(name, setter)
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    }
    omacell
        .set("_hooks", hooks)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

fn dispatch_before_command(lua: &Lua, id: &str) -> Result<(), CoreError> {
    if matches!(id, "file.save" | "file.saveas") {
        dispatch_hook(lua, "on_before_save")?;
    }
    Ok(())
}

fn dispatch_command_events(
    lua: &Lua,
    events: &[omacell_core::event::Event],
) -> Result<(), CoreError> {
    for event in events {
        // The file adapter publishes its frozen BeforeSave event with the
        // committed effect, after I/O. Lua receives the hook from
        // dispatch_before_command instead so mutations land in the save.
        if matches!(event, omacell_core::event::Event::BeforeSave { .. }) {
            continue;
        }
        if let Some(name) = crate::host::hook_name(event) {
            dispatch_hook(lua, name)?;
        }
    }
    Ok(())
}

fn dispatch_hook(lua: &Lua, name: &str) -> Result<(), CoreError> {
    let globals = lua.globals();
    let omacell: Table = globals
        .get("omacell")
        .map_err(|error| CoreError::new("lua.api", error.to_string()))?;
    let hooks: Table = omacell
        .get("_hooks")
        .map_err(|error| CoreError::new("lua.api", error.to_string()))?;
    let registered: Table = hooks
        .get(name)
        .map_err(|error| CoreError::new("lua.api", error.to_string()))?;
    for hook in registered.sequence_values::<mlua::Function>() {
        hook.map_err(|error| CoreError::new("lua.api", error.to_string()))?
            .call::<()>(())
            .map_err(|error| lua_error(error, name))?;
    }
    Ok(())
}

fn has_ai_hooks(lua: &Lua) -> bool {
    let Ok(omacell) = lua.globals().get::<Table>("omacell") else {
        return false;
    };
    let Ok(hooks) = omacell.get::<Table>("_hooks") else {
        return false;
    };
    ["on_ai_request", "on_ai_response"].into_iter().any(|name| {
        hooks
            .get::<Table>(name)
            .is_ok_and(|registered| registered.raw_len() > 0)
    })
}

struct LuaAiHooks {
    lua: Arc<Mutex<Lua>>,
    script_depth: Arc<AtomicU32>,
    function_depth: Arc<AtomicU32>,
    script_digest: Arc<Mutex<Sha256>>,
}

impl LuaAiHooks {
    fn transform(&self, name: &str, value: Json) -> Result<Json, AiError> {
        let lua = match self.lua.try_lock() {
            Ok(lua) => lua,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => {
                return Err(AiError::new(
                    omacell_ai::error::codes::PAYLOAD,
                    format!("Lua {name} hook cannot re-enter a running script"),
                ));
            }
        };
        let _script = FunctionEvaluation::enter(&self.script_depth);
        let _host_api_guard = FunctionEvaluation::enter(&self.function_depth);
        let omacell: Table = lua
            .globals()
            .get("omacell")
            .map_err(|error| ai_hook_error(name, error))?;
        let hooks: Table = omacell
            .get("_hooks")
            .map_err(|error| ai_hook_error(name, error))?;
        let registered: Table = hooks
            .get(name)
            .map_err(|error| ai_hook_error(name, error))?;
        let mut current = value;
        for hook in registered.sequence_values::<mlua::Function>() {
            let hook = hook.map_err(|error| ai_hook_error(name, error))?;
            let input = json_to_lua(&lua, &current).map_err(|error| ai_hook_error(name, error))?;
            let fallback = input.clone();
            let output = hook
                .call::<LuaValue>(input)
                .map_err(|error| ai_hook_error(name, error))?;
            let output = if matches!(output, LuaValue::Nil) {
                fallback
            } else {
                output
            };
            current = lua_to_json(&output).map_err(|error| ai_hook_error(name, error))?;
        }
        Ok(current)
    }
}

impl AiHooks for LuaAiHooks {
    fn cache_version(&self) -> String {
        let digest = lock_mutex(&self.script_digest).clone().finalize();
        format!("lua:{}", sha256_hex(&digest))
    }

    fn on_request(&self, request: AiHookRequest) -> Result<AiHookRequest, AiError> {
        let value =
            serde_json::to_value(request).map_err(|error| ai_hook_error("on_ai_request", error))?;
        serde_json::from_value(self.transform("on_ai_request", value)?)
            .map_err(|error| ai_hook_error("on_ai_request", error))
    }

    fn on_response(&self, response: AiHookResponse) -> Result<AiHookResponse, AiError> {
        let value = serde_json::to_value(response)
            .map_err(|error| ai_hook_error("on_ai_response", error))?;
        serde_json::from_value(self.transform("on_ai_response", value)?)
            .map_err(|error| ai_hook_error("on_ai_response", error))
    }
}

fn ai_hook_error(name: &str, error: impl std::fmt::Display) -> AiError {
    AiError::new(
        omacell_ai::error::codes::PAYLOAD,
        format!("Lua {name} hook failed: {error}"),
    )
}

fn install_keymap(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    profile: Profile,
    function_depth: &Arc<AtomicU32>,
) -> Result<(), CoreError> {
    if profile == Profile::Embedded {
        return Ok(());
    }
    let keymap = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let h = Arc::clone(host);
    let depth = Arc::clone(function_depth);
    keymap
        .set(
            "set",
            lua.create_function(move |_, (mode, keys, cmd): (String, String, String)| {
                host_api_available(&depth)?;
                lock_mutex(&h)
                    .try_keymap_set(&mode, &keys, &cmd)
                    .map_err(mlua::Error::external)
            })
            .map_err(|e| CoreError::new("lua.api", e.to_string()))?,
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("keymap", keymap)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

fn install_ai(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    profile: Profile,
    function_depth: &Arc<AtomicU32>,
    tasks: &Arc<Mutex<BTreeMap<String, AiTaskSpec>>>,
) -> Result<(), CoreError> {
    if profile == Profile::Embedded {
        return Ok(());
    }
    let ai = lua
        .create_table()
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let task_depth = Arc::clone(function_depth);
    let task_registrations = Arc::clone(tasks);
    let task = lua
        .create_function(
            move |_, (name, spec): (String, LuaValue)| -> mlua::Result<()> {
                host_api_available(&task_depth)?;
                let (task, arity) = parse_ai_spec(name, &spec).map_err(mlua::Error::external)?;
                if arity.0.is_some() || arity.1.is_some() {
                    return Err(mlua::Error::external(CoreError::new(
                        "lua.ai",
                        "omacell.ai.task does not accept min or max",
                    )));
                }
                lock_mutex(&task_registrations).insert(task.name.to_ascii_lowercase(), task);
                Ok(())
            },
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let host_fn = Arc::clone(host);
    let depth = Arc::clone(function_depth);
    let function_tasks = Arc::clone(tasks);
    let func = lua
        .create_function(
            move |_, (name, spec): (String, LuaValue)| -> mlua::Result<()> {
                host_api_available(&depth)?;
                if !valid_custom_function_name(&name) {
                    return Err(mlua::Error::external(CoreError::new(
                        "lua.ai",
                        "AI functions must use a valid namespace (MY.NAME)",
                    )));
                }
                let (task, (min, max)) =
                    parse_ai_spec(name.clone(), &spec).map_err(mlua::Error::external)?;
                let min_args = min.unwrap_or(1);
                let max_args = max.unwrap_or(8);
                if min_args > max_args {
                    return Err(mlua::Error::external(CoreError::new(
                        "lua.ai",
                        "AI function min argument count exceeds max",
                    )));
                }
                let def = DynamicFn {
                    name,
                    min_args,
                    max_args,
                    volatile: false,
                    array_lift: ArrayLift::None,
                    body: Arc::new(AiFnStub),
                };
                lock_mutex(&host_fn)
                    .register_function(def)
                    .map_err(mlua::Error::external)?;
                lock_mutex(&function_tasks).insert(task.name.to_ascii_lowercase(), task);
                Ok(())
            },
        )
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    ai.set("task", task)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    ai.set("fn", func)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    omacell
        .set("ai", ai)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LuaAiSpec {
    prompt: String,
    #[serde(default)]
    schema: Option<Json>,
    #[serde(default)]
    tools: Vec<ToolSpec>,
    #[serde(default)]
    min: Option<u8>,
    #[serde(default)]
    max: Option<u8>,
}

type AiArity = (Option<u8>, Option<u8>);

fn parse_ai_spec(name: String, value: &LuaValue) -> Result<(AiTaskSpec, AiArity), CoreError> {
    let json = lua_to_json(value)?;
    let spec: LuaAiSpec = serde_json::from_value(json)
        .map_err(|error| CoreError::new("lua.ai", format!("invalid AI task spec: {error}")))?;
    let task = AiTaskSpec {
        name,
        prompt: spec.prompt,
        schema: spec.schema,
        tools: spec.tools,
    };
    task.validate().map_err(CoreError::from)?;
    Ok((task, (spec.min, spec.max)))
}

struct AiFnStub;

impl DynamicFnBody for AiFnStub {
    fn async_node(&self) -> bool {
        true
    }

    fn eval(&self, _args: &[ArgVal]) -> RuntimeValue {
        RuntimeValue::error(ErrorKind::Na)
    }
}

fn install_book(
    lua: &Lua,
    omacell: &Table,
    host: &Arc<Mutex<Box<dyn ScriptHost>>>,
    function_depth: &Arc<AtomicU32>,
) -> Result<(), CoreError> {
    let h = Arc::clone(host);
    let depth = Arc::clone(function_depth);
    let getter = lua
        .create_function(move |lua, (): ()| {
            host_api_available(&depth)?;
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
                function_depth: Arc::clone(&depth),
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
    function_depth: Arc<AtomicU32>,
}

struct LuaSheet {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    name: String,
    function_depth: Arc<AtomicU32>,
}

struct LuaCell {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    sheet: String,
    a1: String,
    function_depth: Arc<AtomicU32>,
}

struct LuaRange {
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    sheet: String,
    a1: String,
    function_depth: Arc<AtomicU32>,
}

impl mlua::UserData for LuaBook {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("sheet", |_, this, name: Option<String>| {
            host_api_available(&this.function_depth)?;
            let requested = name.unwrap_or_else(|| this.active.clone());
            let host = lock_mutex(&this.host);
            let id = host
                .workbook()
                .resolve_sheet_name(&requested)
                .map_err(mlua::Error::external)?;
            let name = host
                .workbook()
                .sheet(id)
                .map(|sheet| sheet.name.clone())
                .ok_or_else(|| mlua::Error::external("resolved worksheet is missing"))?;
            drop(host);
            Ok(LuaSheet {
                host: Arc::clone(&this.host),
                name,
                function_depth: Arc::clone(&this.function_depth),
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
                function_depth: Arc::clone(&this.function_depth),
            })
        });
        methods.add_method("range", |_, this, a1: String| {
            Ok(LuaRange {
                host: Arc::clone(&this.host),
                sheet: this.name.clone(),
                a1,
                function_depth: Arc::clone(&this.function_depth),
            })
        });
        methods.add_method("name", |_, this, (): ()| Ok(this.name.clone()));
    }
}

impl mlua::UserData for LuaCell {
    fn add_fields<F: mlua::UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("value", |lua, this| {
            host_api_available(&this.function_depth)?;
            let host = lock_mutex(&this.host);
            cell_lua_value(lua, host.workbook(), &this.sheet, &this.a1)
        });
        fields.add_field_method_get("input", |_, this| {
            host_api_available(&this.function_depth)?;
            let host = lock_mutex(&this.host);
            Ok(cell_input(host.workbook(), &this.sheet, &this.a1))
        });
        fields.add_field_method_get("formula", |_, this| {
            host_api_available(&this.function_depth)?;
            let host = lock_mutex(&this.host);
            Ok(cell_formula(host.workbook(), &this.sheet, &this.a1))
        });
        fields.add_field_method_get("style", |lua, this| {
            host_api_available(&this.function_depth)?;
            let host = lock_mutex(&this.host);
            let style = cell_style(host.workbook(), &this.sheet, &this.a1);
            let json = serde_json::to_value(style).map_err(mlua::Error::external)?;
            json_to_lua(lua, &json)
        });
    }
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set", |lua, this, input: String| {
            host_api_available(&this.function_depth)?;
            let mut host = lock_mutex(&this.host);
            let r#ref = qualify(&this.sheet, &this.a1);
            host.execute(
                "cell.set",
                serde_json::json!({"ref": r#ref, "input": input}),
            )
            .map_err(mlua::Error::external)?;
            let events = host.take_events();
            drop(host);
            dispatch_command_events(lua, &events).map_err(mlua::Error::external)?;
            Ok(())
        });
        methods.add_method("set_style", |lua, this, patch: LuaValue| {
            host_api_available(&this.function_depth)?;
            let mut args = match lua_to_json(&patch).map_err(mlua::Error::external)? {
                Json::Object(args) => args,
                _ => {
                    return Err(mlua::Error::external(CoreError::new(
                        "lua.args",
                        "cell:set_style expects a table with string keys",
                    )));
                }
            };
            args.insert("range".into(), Json::String(qualify(&this.sheet, &this.a1)));
            let mut host = lock_mutex(&this.host);
            host.execute("style.set", Json::Object(args))
                .map_err(mlua::Error::external)?;
            let events = host.take_events();
            drop(host);
            dispatch_command_events(lua, &events).map_err(mlua::Error::external)?;
            Ok(())
        });
    }
}

impl mlua::UserData for LuaRange {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("cells", |lua, this, (): ()| {
            host_api_available(&this.function_depth)?;
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
                        function_depth: Arc::clone(&this.function_depth),
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
        format!("{}!{a1}", quote_sheet_name(sheet))
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

fn cell_formula(wb: &omacell_core::workbook::Workbook, sheet: &str, a1: &str) -> Option<String> {
    let parsed = parse_a1(&qualify(sheet, a1)).ok()?;
    let kind = wb.resolve_parsed(parsed).ok()?;
    let (sheet_id, row, col) = match kind {
        RefKind::Cell(cell) => (cell.sheet.unwrap_or(wb.active_sheet()), cell.row, cell.col),
        RefKind::Range(range) => (
            range.start.sheet.unwrap_or(wb.active_sheet()),
            range.start.row,
            range.start.col,
        ),
    };
    let slot = wb.get(sheet_id, row, col).ok().flatten()?;
    let formula = slot.formula?;
    wb.intern().formulas.get(formula).map(str::to_string)
}

fn cell_style(
    wb: &omacell_core::workbook::Workbook,
    sheet: &str,
    a1: &str,
) -> omacell_core::style::Style {
    let Some(parsed) = parse_a1(&qualify(sheet, a1)).ok() else {
        return omacell_core::style::Style::default();
    };
    let Some(kind) = wb.resolve_parsed(parsed).ok() else {
        return omacell_core::style::Style::default();
    };
    let (sheet_id, row, col) = match kind {
        RefKind::Cell(cell) => (cell.sheet.unwrap_or(wb.active_sheet()), cell.row, cell.col),
        RefKind::Range(range) => (
            range.start.sheet.unwrap_or(wb.active_sheet()),
            range.start.row,
            range.start.col,
        ),
    };
    wb.get(sheet_id, row, col)
        .ok()
        .flatten()
        .and_then(|slot| wb.intern().styles.get(slot.style))
        .cloned()
        .unwrap_or_default()
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
    stored_to_lua(lua, wb, slot.value)
}

fn stored_to_lua(
    lua: &Lua,
    wb: &omacell_core::workbook::Workbook,
    value: Value,
) -> mlua::Result<LuaValue> {
    match value {
        Value::Empty => Ok(LuaValue::Nil),
        Value::Number(n) => Ok(LuaValue::Number(n)),
        Value::Bool(b) => Ok(LuaValue::Boolean(b)),
        Value::Text(id) => {
            let t = wb.intern().strings.get(id).unwrap_or("");
            Ok(LuaValue::String(lua.create_string(t)?))
        }
        Value::Error(e) => Ok(LuaValue::String(lua.create_string(e.as_str())?)),
        Value::Array(id) => {
            let payload = wb
                .intern()
                .arrays
                .get(id)
                .ok_or_else(|| mlua::Error::external("missing workbook array payload"))?;
            let rows = lua.create_table_with_capacity(payload.shape.rows as usize, 0)?;
            for row in 0..payload.shape.rows as usize {
                let columns = lua.create_table_with_capacity(payload.shape.cols as usize, 0)?;
                for col in 0..payload.shape.cols as usize {
                    let index = row * payload.shape.cols as usize + col;
                    let value = payload.values.get(index).copied().unwrap_or(Value::Empty);
                    columns.set(col + 1, stored_to_lua(lua, wb, value)?)?;
                }
                rows.set(row + 1, columns)?;
            }
            Ok(LuaValue::Table(rows))
        }
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
            if r.is_3d() {
                return Err(CoreError::new(
                    "lua.range",
                    "range:cells does not support 3-D worksheet ranges",
                ));
            }
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
    let rows = u64::from(r1 - r0) + 1;
    let cols = u64::from(c1 - c0) + 1;
    let area = rows.saturating_mul(cols);
    if area > omacell_bus::MAX_RANGE_CELLS {
        return Err(CoreError::new(
            "lua.range",
            format!(
                "range has {area} cells; maximum is {}",
                omacell_bus::MAX_RANGE_CELLS
            ),
        ));
    }
    let sheet_name = wb
        .sheet(sheet_id)
        .map(|sheet| quote_sheet_name(&sheet.name))
        .ok_or_else(|| CoreError::new("lua.range", "resolved worksheet is missing"))?;
    let mut out = Vec::with_capacity(area as usize);
    for r in r0..=r1 {
        for c in c0..=c1 {
            let col = omacell_core::addr::col_to_letters(c).unwrap_or_else(|_| "?".into());
            out.push(format!("{sheet_name}!{col}{}", r + 1));
        }
    }
    Ok(out)
}

struct LuaBody {
    lua: Lua,
    func: mlua::Function,
    function_depth: Arc<AtomicU32>,
}

struct FunctionRegistration {
    runtime_lua: Arc<Mutex<Lua>>,
    host: Arc<Mutex<Box<dyn ScriptHost>>>,
    function_depth: Arc<AtomicU32>,
    script_depth: Arc<AtomicU32>,
}

struct IsolatedLuaBody {
    lua: Lua,
    func: mlua::Function,
}

struct HybridLuaBody {
    primary: LuaBody,
    fallback: IsolatedLuaBody,
    runtime_lua: Arc<Mutex<Lua>>,
    script_depth: Arc<AtomicU32>,
}

impl DynamicFnBody for LuaBody {
    fn eval(&self, args: &[ArgVal]) -> RuntimeValue {
        let _evaluation = FunctionEvaluation::enter(&self.function_depth);
        let mut values = Vec::new();
        for arg in args {
            let value = if arg.omitted {
                LuaValue::Nil
            } else {
                match runtime_to_lua(&self.lua, &arg.value) {
                    Ok(value) => value,
                    Err(_) => {
                        return RuntimeValue::error(omacell_core::error::ErrorKind::Value);
                    }
                }
            };
            values.push(value);
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

impl DynamicFnBody for IsolatedLuaBody {
    fn eval(&self, args: &[ArgVal]) -> RuntimeValue {
        let mut values = Vec::new();
        for arg in args {
            let value = if arg.omitted {
                LuaValue::Nil
            } else {
                match runtime_to_lua(&self.lua, &arg.value) {
                    Ok(value) => value,
                    Err(_) => return RuntimeValue::error(ErrorKind::Value),
                }
            };
            values.push(value);
        }
        match self
            .func
            .call::<LuaValue>(mlua::MultiValue::from_vec(values))
        {
            Ok(value) => lua_to_runtime(&value),
            Err(_) => RuntimeValue::error(ErrorKind::Value),
        }
    }
}

impl DynamicFnBody for HybridLuaBody {
    fn eval(&self, args: &[ArgVal]) -> RuntimeValue {
        if self.script_depth.load(Ordering::SeqCst) == 0 {
            match self.runtime_lua.try_lock() {
                Ok(_guard) => return self.primary.eval(args),
                Err(TryLockError::Poisoned(poisoned)) => {
                    let _guard = poisoned.into_inner();
                    return self.primary.eval(args);
                }
                Err(TryLockError::WouldBlock) => {}
            }
        }
        self.fallback.eval(args)
    }
}

fn runtime_to_lua(lua: &Lua, value: &RuntimeValue) -> mlua::Result<LuaValue> {
    match value {
        RuntimeValue::Scalar(scalar) => scalar_to_lua(lua, scalar),
        RuntimeValue::Array(array) => {
            let rows = lua.create_table_with_capacity(array.rows as usize, 0)?;
            for row in 0..array.rows as usize {
                let columns = lua.create_table_with_capacity(array.cols as usize, 0)?;
                for col in 0..array.cols as usize {
                    let index = row * array.cols as usize + col;
                    let value = array.values.get(index).unwrap_or(&Scalar::Empty);
                    columns.set(col + 1, scalar_to_lua(lua, value)?)?;
                }
                rows.set(row + 1, columns)?;
            }
            Ok(LuaValue::Table(rows))
        }
        RuntimeValue::Lambda(_) | RuntimeValue::Ref(_) => Ok(LuaValue::Nil),
    }
}

fn scalar_to_lua(lua: &Lua, value: &Scalar) -> mlua::Result<LuaValue> {
    match value {
        Scalar::Empty => Ok(LuaValue::Nil),
        Scalar::Number(number) => Ok(LuaValue::Number(*number)),
        Scalar::Bool(value) => Ok(LuaValue::Boolean(*value)),
        Scalar::Text(value) => Ok(LuaValue::String(lua.create_string(value.as_ref())?)),
        Scalar::Error(error) => Ok(LuaValue::String(lua.create_string(error.as_str())?)),
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
        LuaValue::Table(table) => lua_table_to_runtime(table)
            .unwrap_or_else(|| RuntimeValue::error(omacell_core::error::ErrorKind::Value)),
        _ => RuntimeValue::error(omacell_core::error::ErrorKind::Value),
    }
}

fn lua_table_to_runtime(table: &Table) -> Option<RuntimeValue> {
    let len = table.raw_len();
    if len == 0 || table.clone().pairs::<LuaValue, LuaValue>().count() != len {
        return None;
    }
    let first: LuaValue = table.raw_get(1).ok()?;
    if matches!(first, LuaValue::Table(_)) {
        let mut values = Vec::new();
        let mut columns = None;
        for row_index in 1..=len {
            let row: Table = table.raw_get(row_index).ok()?;
            let width = row.raw_len();
            if width == 0 || row.clone().pairs::<LuaValue, LuaValue>().count() != width {
                return None;
            }
            match columns {
                Some(expected) if expected != width => return None,
                None => columns = Some(width),
                _ => {}
            }
            for col_index in 1..=width {
                values.push(lua_to_scalar(&row.raw_get(col_index).ok()?)?);
            }
        }
        return Some(RuntimeValue::array(
            u32::try_from(len).ok()?,
            u32::try_from(columns?).ok()?,
            values,
        ));
    }
    let mut values = Vec::with_capacity(len);
    for index in 1..=len {
        values.push(lua_to_scalar(&table.raw_get(index).ok()?)?);
    }
    Some(RuntimeValue::array(1, u32::try_from(len).ok()?, values))
}

fn lua_to_scalar(value: &LuaValue) -> Option<Scalar> {
    match value {
        LuaValue::Nil => Some(Scalar::Empty),
        LuaValue::Boolean(value) => Some(Scalar::Bool(*value)),
        LuaValue::Integer(value) => Some(Scalar::Number(*value as f64)),
        LuaValue::Number(value) if value.is_finite() => Some(Scalar::Number(*value)),
        LuaValue::String(value) => value
            .to_str()
            .ok()
            .map(|value| Scalar::Text(Arc::<str>::from(value.as_ref()))),
        _ => None,
    }
}

fn register_lua_fn(
    lua: &Lua,
    registration: &FunctionRegistration,
    name: String,
    spec: Table,
    func: mlua::Function,
) -> Result<(), CoreError> {
    if !valid_custom_function_name(&name) {
        return Err(CoreError::new(
            "lua.fn",
            "custom functions must use a valid namespace (USER.NAME)",
        )
        .with_hint("register omacell.fn(\"USER.DOUBLE\", spec, fn)"));
    }
    for pair in spec.clone().pairs::<LuaValue, LuaValue>() {
        let (key, _) = pair.map_err(|error| CoreError::new("lua.fn", error.to_string()))?;
        let LuaValue::String(key) = key else {
            return Err(CoreError::new(
                "lua.fn",
                "custom function spec keys must be strings",
            ));
        };
        let key = key
            .to_str()
            .map_err(|error| CoreError::new("lua.fn", error.to_string()))?;
        if !matches!(key.as_ref(), "min" | "max" | "volatile" | "array_lift") {
            return Err(CoreError::new(
                "lua.fn",
                format!("unknown custom function spec field {key}"),
            ));
        }
    }
    let min_args = spec
        .get::<Option<u8>>("min")
        .map_err(|error| CoreError::new("lua.fn", error.to_string()))?
        .unwrap_or(0);
    let max_args = spec
        .get::<Option<u8>>("max")
        .map_err(|error| CoreError::new("lua.fn", error.to_string()))?
        .unwrap_or(min_args.max(1));
    if min_args > max_args {
        return Err(CoreError::new(
            "lua.fn",
            "custom function min argument count exceeds max",
        ));
    }
    let volatile = spec
        .get::<Option<bool>>("volatile")
        .map_err(|error| CoreError::new("lua.fn", error.to_string()))?
        .unwrap_or(false);
    let lift = spec
        .get::<Option<String>>("array_lift")
        .map_err(|error| CoreError::new("lua.fn", error.to_string()))?
        .unwrap_or_else(|| "none".into());
    let array_lift = match lift.as_str() {
        "none" => ArrayLift::None,
        "all" => ArrayLift::All,
        _ => {
            return Err(CoreError::new(
                "lua.fn",
                "array_lift must be 'none' or 'all'",
            ));
        }
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
    let isolated = lock_mutex(&registration.host).isolate_functions();
    let body: Arc<dyn DynamicFnBody> = if isolated {
        if func.info().what != "Lua" {
            return Err(CoreError::new(
                "lua.fn",
                "interactive custom functions must be Lua callbacks",
            ));
        }
        let evaluator = Lua::new();
        let callback = evaluator
            .load(func.dump(true))
            .set_name(name.as_str())
            .into_function()
            .map_err(|error| CoreError::new("lua.fn", error.to_string()))?;
        Arc::new(HybridLuaBody {
            primary: LuaBody {
                lua: lua.clone(),
                func: func.clone(),
                function_depth: Arc::clone(&registration.function_depth),
            },
            fallback: IsolatedLuaBody {
                lua: evaluator,
                func: callback,
            },
            runtime_lua: Arc::clone(&registration.runtime_lua),
            script_depth: Arc::clone(&registration.script_depth),
        })
    } else {
        Arc::new(LuaBody {
            lua: lua.clone(),
            func: func.clone(),
            function_depth: Arc::clone(&registration.function_depth),
        })
    };
    fns.set(name.as_str(), func)
        .map_err(|e| CoreError::new("lua.api", e.to_string()))?;
    let def = DynamicFn {
        name: name.clone(),
        min_args,
        max_args,
        volatile,
        array_lift,
        body,
    };
    lock_mutex(&registration.host).register_function(def)
}

struct FunctionEvaluation<'a>(&'a AtomicU32);

impl<'a> FunctionEvaluation<'a> {
    fn enter(depth: &'a AtomicU32) -> Self {
        depth.fetch_add(1, Ordering::Relaxed);
        Self(depth)
    }
}

impl Drop for FunctionEvaluation<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

fn host_api_available(function_depth: &AtomicU32) -> mlua::Result<()> {
    if function_depth.load(Ordering::Relaxed) == 0 {
        Ok(())
    } else {
        Err(mlua::Error::external(CoreError::new(
            "lua.fn",
            "Omacell host APIs cannot be called from a worksheet function",
        )))
    }
}

fn valid_custom_function_name(name: &str) -> bool {
    let parts = name.split('.').collect::<Vec<_>>();
    parts.len() >= 2 && parts.into_iter().all(valid_name_part)
}

fn valid_name_part(part: &str) -> bool {
    let mut chars = part.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(crate) fn lua_to_json(value: &LuaValue) -> Result<Json, CoreError> {
    lua_to_json_inner(value, 0)
}

fn lua_to_json_inner(value: &LuaValue, depth: u8) -> Result<Json, CoreError> {
    if depth >= 32 {
        return Err(CoreError::new(
            "lua.args",
            "Lua command arguments exceed 32 nested tables",
        ));
    }
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
            let marked_array = t
                .metatable()
                .and_then(|metatable| metatable.raw_get::<bool>(JSON_ARRAY_MARKER).ok())
                .unwrap_or(false);
            let mut object = serde_json::Map::new();
            let mut sequence = Vec::<(usize, Json)>::new();
            t.for_each(|key: LuaValue, value: LuaValue| {
                let json = lua_to_json_inner(&value, depth + 1).map_err(mlua::Error::external)?;
                match key {
                    LuaValue::String(key) if !marked_array && sequence.is_empty() => {
                        let key = key.to_str().map_err(mlua::Error::external)?.to_string();
                        object.insert(key, json);
                    }
                    LuaValue::Integer(index) if object.is_empty() && index > 0 => {
                        sequence.push((index as usize, json));
                    }
                    _ => {
                        return Err(mlua::Error::external(CoreError::new(
                            "lua.args",
                            "tables must use either string keys or contiguous 1-based integer keys",
                        )));
                    }
                }
                Ok(())
            })
            .map_err(|e| CoreError::new("lua.args", e.to_string()))?;
            if sequence.is_empty() {
                return Ok(if marked_array {
                    Json::Array(Vec::new())
                } else {
                    Json::Object(object)
                });
            }
            sequence.sort_by_key(|(index, _)| *index);
            if sequence
                .iter()
                .enumerate()
                .any(|(offset, (index, _))| *index != offset + 1)
            {
                return Err(CoreError::new(
                    "lua.args",
                    "array tables must use contiguous 1-based integer keys",
                ));
            }
            Ok(Json::Array(
                sequence.into_iter().map(|(_, value)| value).collect(),
            ))
        }
        _ => Err(CoreError::new("lua.args", "unsupported Lua value")),
    }
}

pub(crate) fn json_to_lua(lua: &Lua, value: &Json) -> mlua::Result<LuaValue> {
    match value {
        Json::Null => Ok(LuaValue::Nil),
        Json::Bool(value) => Ok(LuaValue::Boolean(*value)),
        Json::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Ok(LuaValue::Integer(integer))
            } else if let Some(number) = value.as_f64() {
                Ok(LuaValue::Number(number))
            } else {
                Err(mlua::Error::external(CoreError::new(
                    "lua.result",
                    "JSON number is not representable in Lua",
                )))
            }
        }
        Json::String(value) => Ok(LuaValue::String(lua.create_string(value)?)),
        Json::Array(values) => {
            let table = lua.create_table_with_capacity(values.len(), 0)?;
            let metatable = lua.create_table_with_capacity(0, 2)?;
            metatable.raw_set(JSON_ARRAY_MARKER, true)?;
            metatable.raw_set("__metatable", false)?;
            table.set_metatable(Some(metatable));
            for (index, value) in values.iter().enumerate() {
                table.set(index + 1, json_to_lua(lua, value)?)?;
            }
            Ok(LuaValue::Table(table))
        }
        Json::Object(values) => {
            let table = lua.create_table_with_capacity(0, values.len())?;
            for (key, value) in values {
                table.set(key.as_str(), json_to_lua(lua, value)?)?;
            }
            Ok(LuaValue::Table(table))
        }
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
    // Hash the exact byte slice the caller parsed. Re-reading `book` here
    // creates a check/use race where a different file can be swapped in after
    // parsing but before trust is checked.
    let hash = sha256_hex(bytes);
    if !store.contains_hash(&hash) {
        return Err(CoreError::new(
            "lua.untrusted",
            format!("workbook {} is not in the trust store", book.display()),
        )
        .with_hint("omacell trust add <file>"));
    }
    Ok(())
}

/// Load `init.lua` followed by sorted `plugins/*/init.lua` entry points.
///
/// Every resolved file must remain beneath one of the canonical
/// `policy.trusted_dirs`. Callers invoke this only for startup or an explicit
/// source action; filesystem notifications must never call it directly.
pub fn load_user_scripts(
    runtime: &Runtime,
    config_dir: &Path,
    policy: &ScriptPolicy,
) -> Result<Vec<std::path::PathBuf>, CoreError> {
    if !policy.enabled {
        return Err(CoreError::new("lua.disabled", "scripting is disabled"));
    }
    let mut candidates = vec![config_dir.join("init.lua")];
    let plugins = config_dir.join("plugins");
    if plugins.is_dir() {
        let mut entries = std::fs::read_dir(&plugins)
            .map_err(|error| CoreError::new("lua.io", error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CoreError::new("lua.io", error.to_string()))?;
        entries.sort_by_key(std::fs::DirEntry::file_name);
        if entries.len() > 1024 {
            return Err(CoreError::new(
                "lua.limit",
                "more than 1024 plugin directories",
            ));
        }
        candidates.extend(
            entries
                .into_iter()
                .map(|entry| entry.path().join("init.lua")),
        );
    }

    let mut loaded = Vec::new();
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let canonical = std::fs::canonicalize(&candidate)
            .map_err(|error| CoreError::new("lua.io", error.to_string()))?;
        let file = std::fs::File::open(&canonical)
            .map_err(|error| CoreError::new("lua.io", error.to_string()))?;
        let metadata = file
            .metadata()
            .map_err(|error| CoreError::new("lua.io", error.to_string()))?;
        if !metadata.is_file() {
            return Err(CoreError::new(
                "lua.trust",
                format!("{} is not a regular file", candidate.display()),
            ));
        }
        if !policy
            .trusted_dirs
            .iter()
            .any(|root| canonical.starts_with(root))
        {
            return Err(CoreError::new(
                "lua.trust",
                format!("{} is outside scripting.trusted_dirs", canonical.display()),
            ));
        }
        if metadata.len() > MAX_USER_SCRIPT_BYTES {
            return Err(CoreError::new(
                "lua.limit",
                format!("{} exceeds 1 MiB", canonical.display()),
            ));
        }
        let mut source = String::new();
        file.take(MAX_USER_SCRIPT_BYTES + 1)
            .read_to_string(&mut source)
            .map_err(|error| CoreError::new("lua.io", error.to_string()))?;
        if source.len() as u64 > MAX_USER_SCRIPT_BYTES {
            return Err(CoreError::new(
                "lua.limit",
                format!("{} exceeds 1 MiB", canonical.display()),
            ));
        }
        runtime.exec(&source, &canonical.display().to_string())?;
        loaded.push(canonical);
    }
    Ok(loaded)
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

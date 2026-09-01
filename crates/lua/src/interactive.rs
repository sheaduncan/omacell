//! Retained GUI/TUI runtime over the single-writer command task runner.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use omacell_bus::TaskRunnerHandle;
use omacell_conf::LoadedConfig;
use omacell_core::command::Origin;
use omacell_core::error::CoreError;
use omacell_core::eval::DynamicFn;
use omacell_core::event::Event;
use omacell_core::workbook::Workbook;
use serde_json::Value;

use crate::{Profile, Runtime, ScriptHost, ScriptPolicy, load_user_scripts};

const MAX_UI_MESSAGES: usize = 256;

/// UI callbacks required by a retained user-profile runtime.
pub trait InteractiveUi: Send + Sync {
    /// Prompt synchronously, or return `lua.prompt` when the frontend cannot.
    fn prompt(&self, message: &str) -> Result<String, CoreError> {
        Err(CoreError::new(
            "lua.prompt",
            format!("interactive prompt is unavailable: {message}"),
        ))
    }

    /// Validate and install one Lua keymap overlay.
    fn keymap_set(&self, mode: &str, keys: &str, cmd: &str) -> Result<(), CoreError>;

    /// Remove overlays installed by the previous sourced runtime.
    fn clear_keymap(&self);

    /// Deliver a user-script notification in addition to the status message.
    fn notify(&self, _message: &str) {}
}

/// User-profile Lua VM retained by an interactive frontend.
pub struct InteractiveRuntime {
    runtime: Option<Runtime>,
    handle: TaskRunnerHandle,
    ui: Arc<dyn InteractiveUi>,
    config_dir: PathBuf,
    policy: ScriptPolicy,
    messages: Arc<Mutex<VecDeque<String>>>,
    functions: Arc<Mutex<BTreeMap<String, DynamicFn>>>,
    functions_dirty: Arc<AtomicBool>,
}

impl InteractiveRuntime {
    /// Build the VM and source trusted `init.lua` / plugin entry points once.
    ///
    /// Script errors are retained as status messages so a bad user config does
    /// not prevent the GUI or TUI from opening.
    pub fn new(
        handle: TaskRunnerHandle,
        ui: Arc<dyn InteractiveUi>,
        config_dir: PathBuf,
        loaded: &LoadedConfig,
    ) -> Result<Self, CoreError> {
        let mut scripts = Self {
            runtime: None,
            handle,
            ui,
            config_dir,
            policy: ScriptPolicy::from_loaded(loaded),
            messages: Arc::new(Mutex::new(VecDeque::new())),
            functions: Arc::new(Mutex::new(BTreeMap::new())),
            functions_dirty: Arc::new(AtomicBool::new(false)),
        };
        if scripts.policy.enabled
            && let Err(error) = scripts.source()
        {
            scripts.push_message(format!("{}: {}", error.code, error.message));
        }
        Ok(scripts)
    }

    /// Run the save hook before `file.save` or `file.saveas` is queued.
    pub fn before_command(&self, id: &str) -> Result<(), CoreError> {
        if let Some(runtime) = &self.runtime {
            let result = runtime.before_command(id);
            self.refresh_functions_if_dirty()?;
            result?;
        }
        Ok(())
    }

    /// Drain committed bus events and dispatch their Lua hooks.
    pub fn poll_events(&self) -> Result<(), CoreError> {
        let events = self.handle.drain_bus_events();
        if let Some(runtime) = &self.runtime {
            let result = runtime.emit_events(&events);
            self.refresh_functions_if_dirty()?;
            result?;
        }
        Ok(())
    }

    /// Emit `on_open` for a workbook loaded before the task runner existed.
    pub fn emit_open(&self) -> Result<(), CoreError> {
        if let Some(runtime) = &self.runtime {
            let result = runtime.emit_hook("on_open");
            self.refresh_functions_if_dirty()?;
            result?;
        }
        Ok(())
    }

    /// Explicitly rebuild and source the user-profile VM.
    pub fn source(&mut self) -> Result<Vec<PathBuf>, CoreError> {
        if !self.policy.enabled {
            return Err(CoreError::new("lua.disabled", "scripting is disabled"));
        }
        if let Err(error) = self.poll_events() {
            self.push_message(format!("{}: {}", error.code, error.message));
        }
        let previous = lock_functions(&self.functions).clone();
        let functions = Arc::new(Mutex::new(BTreeMap::new()));
        let functions_dirty = Arc::new(AtomicBool::new(false));
        let host = RunnerHost {
            workbook: self.handle.snapshot().workbook.clone(),
            handle: self.handle.clone(),
            ui: Arc::clone(&self.ui),
            messages: Arc::clone(&self.messages),
            functions: Arc::clone(&functions),
            functions_dirty: Arc::clone(&functions_dirty),
        };
        let runtime = Runtime::new(Profile::User, Box::new(host))?;
        self.ui.clear_keymap();
        let loaded = match load_user_scripts(&runtime, &self.config_dir, &self.policy) {
            Ok(loaded) => loaded,
            Err(error) => {
                let mut replaced = function_names(&previous);
                replaced.extend(function_names(&lock_functions(&functions)));
                self.handle
                    .replace_functions(replaced, previous.into_values().collect())?;
                return Err(error);
            }
        };
        let current = lock_functions(&functions).clone();
        self.handle.replace_functions(
            function_names(&previous),
            current.values().cloned().collect(),
        )?;
        functions_dirty.store(false, Ordering::SeqCst);
        let event_result = runtime.emit_events(&self.handle.drain_bus_events());
        if functions_dirty.swap(false, Ordering::SeqCst) {
            self.handle.refresh_functions()?;
        }
        event_result?;
        self.runtime = Some(runtime);
        self.functions = functions;
        self.functions_dirty = functions_dirty;
        Ok(loaded)
    }

    /// Apply only stricter live scripting policy changes.
    pub fn tighten(&mut self, loaded: &LoadedConfig) -> Result<(), CoreError> {
        self.policy.tighten(&ScriptPolicy::from_loaded(loaded));
        if !self.policy.enabled || !trusted_config_dir(&self.config_dir, &self.policy) {
            let previous = function_names(&lock_functions(&self.functions));
            if !previous.is_empty() {
                self.handle.replace_functions(previous, Vec::new())?;
            }
            self.runtime = None;
            self.functions = Arc::new(Mutex::new(BTreeMap::new()));
            self.functions_dirty = Arc::new(AtomicBool::new(false));
            self.ui.clear_keymap();
        }
        Ok(())
    }

    /// Drain status/notification/error text produced since the prior call.
    pub fn take_messages(&self) -> Vec<String> {
        self.messages
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    fn push_message(&self, message: String) {
        push_message(&self.messages, message);
    }

    fn refresh_functions_if_dirty(&self) -> Result<(), CoreError> {
        if self.functions_dirty.swap(false, Ordering::SeqCst) {
            self.handle.refresh_functions()?;
        }
        Ok(())
    }
}

fn trusted_config_dir(config_dir: &Path, policy: &ScriptPolicy) -> bool {
    std::fs::canonicalize(config_dir).ok().is_some_and(|dir| {
        policy
            .trusted_dirs
            .iter()
            .any(|trusted| dir.starts_with(trusted))
    })
}

struct RunnerHost {
    workbook: Workbook,
    handle: TaskRunnerHandle,
    ui: Arc<dyn InteractiveUi>,
    messages: Arc<Mutex<VecDeque<String>>>,
    functions: Arc<Mutex<BTreeMap<String, DynamicFn>>>,
    functions_dirty: Arc<AtomicBool>,
}

impl ScriptHost for RunnerHost {
    fn execute(&mut self, id: &str, args: Value) -> Result<Value, CoreError> {
        let outcome = self.handle.submit_wait(Origin::Script, id, args);
        self.refresh();
        if !outcome.ok {
            return Err(outcome
                .error
                .unwrap_or_else(|| CoreError::new("lua.cmd", "command failed")));
        }
        Ok(outcome.result.unwrap_or(Value::Null))
    }

    fn refresh(&mut self) {
        self.workbook = self.handle.snapshot().workbook.clone();
    }

    fn workbook(&self) -> &Workbook {
        &self.workbook
    }

    fn register_function(&mut self, def: DynamicFn) -> Result<(), CoreError> {
        self.handle.register_function(def.clone())?;
        lock_functions(&self.functions).insert(def.name.to_ascii_uppercase(), def);
        self.functions_dirty.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn isolate_functions(&self) -> bool {
        true
    }

    fn take_events(&mut self) -> Vec<Event> {
        self.handle.drain_bus_events()
    }

    fn prompt(&mut self, message: &str) -> Result<String, CoreError> {
        self.ui.prompt(message)
    }

    fn status(&mut self, message: &str) {
        push_message(&self.messages, message.to_string());
    }

    fn notify(&mut self, message: &str) {
        push_message(&self.messages, message.to_string());
        self.ui.notify(message);
    }

    fn try_keymap_set(&mut self, mode: &str, keys: &str, cmd: &str) -> Result<(), CoreError> {
        if !self.handle.command_ids().contains(cmd) {
            return Err(CoreError::new(
                "lua.keymap",
                format!("unknown command {cmd}"),
            ));
        }
        self.ui.keymap_set(mode, keys, cmd)
    }
}

fn push_message(messages: &Mutex<VecDeque<String>>, message: String) {
    let mut messages = messages
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if messages.len() >= MAX_UI_MESSAGES {
        messages.pop_front();
    }
    messages.push_back(message);
}

fn lock_functions(
    functions: &Mutex<BTreeMap<String, DynamicFn>>,
) -> std::sync::MutexGuard<'_, BTreeMap<String, DynamicFn>> {
    functions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn function_names(functions: &BTreeMap<String, DynamicFn>) -> BTreeSet<String> {
    functions.keys().cloned().collect()
}

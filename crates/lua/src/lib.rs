//! Sandboxed Lua 5.4 scripting API for Omacell.
//!
//! WP-20: config-dir scripts run with the full standard library; workbook
//! embedded scripts are sandboxed and require an explicit trust grant.
#![deny(missing_docs)]

pub mod catalog;
pub mod commands;
pub mod host;
pub mod recorder;
pub mod runtime;
pub mod trust;

use omacell_bus::Bus;
use omacell_core::command::Origin;
use omacell_core::error::CoreError;
use omacell_core::eval::DynamicFn;
use omacell_core::workbook::Workbook;
use serde_json::Value;

pub use catalog::{API, ApiEntry, render_markdown};
pub use commands::{ScriptGate, register_script_commands};
pub use host::{ScriptHost, UiSink, hook_name};
pub use recorder::{Recorder, json_to_lua, replay_lua};
pub use runtime::{
    EMBEDDED_INSTRUCTION_LIMIT, EMBEDDED_MEMORY_LIMIT, EMBEDDED_PART, EmbeddedMode, Profile,
    Runtime, ScriptPolicy, allow_embedded, load_trust,
};
pub use trust::{TrustEntry, TrustStore, hash_path, sha256_hex, trust_path};

/// Command-bus host used by the CLI and tests.
pub struct BusHost {
    /// Live session.
    pub bus: Bus,
    /// Captured UI.
    pub ui: UiSink,
}

impl BusHost {
    /// Wrap a bus.
    #[must_use]
    pub fn new(bus: Bus) -> Self {
        Self {
            bus,
            ui: UiSink::default(),
        }
    }
}

impl ScriptHost for BusHost {
    fn execute(&mut self, id: &str, args: Value) -> Result<Value, CoreError> {
        let out = self.bus.execute(Origin::Script, id, args);
        if !out.ok {
            let err = out
                .error
                .unwrap_or_else(|| CoreError::new("lua.cmd", "command failed"));
            return Err(err);
        }
        Ok(out.result.unwrap_or(Value::Null))
    }

    fn workbook(&self) -> &Workbook {
        self.bus.workbook()
    }

    fn register_function(&mut self, def: DynamicFn) -> Result<(), CoreError> {
        self.bus.engine_mut().registry_mut().register_dynamic(def);
        Ok(())
    }

    fn prompt(&mut self, message: &str) -> Result<String, CoreError> {
        self.ui.take_prompt(message)
    }

    fn status(&mut self, message: &str) {
        self.ui.status.push(message.to_string());
    }

    fn notify(&mut self, message: &str) {
        self.ui.notify.push(message.to_string());
    }

    fn keymap_set(&mut self, mode: &str, keys: &str, cmd: &str) {
        self.ui
            .keys
            .push((mode.to_string(), keys.to_string(), cmd.to_string()));
    }
}

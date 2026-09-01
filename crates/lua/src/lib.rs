//! Sandboxed Lua 5.4 scripting API for Omacell.
//!
//! WP-20: config-dir scripts run with the full standard library; workbook
//! embedded scripts are sandboxed and require an explicit trust grant.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod catalog;
pub mod commands;
pub mod host;
pub mod interactive;
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
pub use commands::{ScriptGate, attach_recorder, register_script_commands};
pub use host::{ScriptHost, UiSink, hook_name};
pub use interactive::{InteractiveRuntime, InteractiveUi};
pub use recorder::{MAX_RECORDED_BYTES, MAX_RECORDED_STEPS, Recorder, json_to_lua, replay_lua};
pub use runtime::{
    EMBEDDED_INSTRUCTION_LIMIT, EMBEDDED_MEMORY_LIMIT, EMBEDDED_PART, EmbeddedMode,
    MAX_USER_SCRIPT_BYTES, Profile, Runtime, ScriptPolicy, allow_embedded, load_trust,
    load_user_scripts,
};
pub use trust::{TrustEntry, TrustStore, hash_path, sha256_hex, trust_path};

// Frozen capability set for workbook-provided code. New commands remain
// unavailable until they receive an explicit sandbox review here.
const EMBEDDED_COMMAND_ALLOWLIST: &[&str] = &[
    "audit.diagnose",
    "audit.run",
    "calc.mode",
    "cell.clear",
    "cell.set",
    "chart.fromselection",
    "condfmt.add",
    "edit.autosum",
    "edit.change",
    "edit.clear",
    "edit.clearcell",
    "edit.clearrow",
    "edit.collapse",
    "edit.comment",
    "edit.commentreply",
    "edit.commentresolve",
    "edit.copy",
    "edit.copyformulaabove",
    "edit.copyvalueabove",
    "edit.cut",
    "edit.delcells",
    "edit.delete",
    "edit.expand",
    "edit.filldown",
    "edit.fillleft",
    "edit.fillright",
    "edit.fillselection",
    "edit.fillup",
    "edit.findall",
    "edit.flashfill",
    "edit.group",
    "edit.hyperlink",
    "edit.insert",
    "edit.insertdate",
    "edit.inserttime",
    "edit.move",
    "edit.note",
    "edit.paste",
    "edit.pastespecial",
    "edit.replaceall",
    "edit.replacepreview",
    "edit.texttocolumns",
    "edit.ungroup",
    "edit.yank",
    "filter.clear",
    "filter.set",
    "filter.toggle",
    "filter.values",
    "format.autofitcols",
    "format.autofitrows",
    "format.bold",
    "format.bordernone",
    "format.borderoutline",
    "format.colwidth",
    "format.currency",
    "format.date",
    "format.general",
    "format.indent",
    "format.italic",
    "format.number",
    "format.numberstyle",
    "format.outdent",
    "format.panel",
    "format.percent",
    "format.protection",
    "format.rowheight",
    "format.scientific",
    "format.time",
    "format.underline",
    "formula.dependents",
    "formula.explain",
    "formula.precedents",
    "formula.trace",
    "name.define",
    "name.remove",
    "nav.address",
    "nav.gotospecial",
    "pivot.create",
    "pivot.remove",
    "range.clear",
    "range.consolidate",
    "range.merge",
    "range.mergeacross",
    "range.removeduplicates",
    "range.set",
    "range.sort",
    "range.unmerge",
    "sheet.add",
    "sheet.protect",
    "sheet.protectedrange",
    "sheet.remove",
    "sheet.rename",
    "sheet.reorder",
    "sheet.visibility",
    "sparkline.set",
    "stats.describe",
    "style.set",
    "table.convert",
    "table.create",
    "table.rename",
    "table.resize",
    "table.totals",
    "validation.set",
    "view.hidecols",
    "view.hiderows",
    "view.unhidecols",
    "view.unhiderows",
    "whatif.goalseek",
    "workbook.protect",
];

/// Command-bus host used by the CLI and tests.
pub struct BusHost {
    /// Live session.
    pub bus: Bus,
    /// Captured UI.
    pub ui: UiSink,
    event_subscriber: omacell_bus::SubscriberId,
}

impl BusHost {
    /// Wrap a bus.
    #[must_use]
    pub fn new(mut bus: Bus) -> Self {
        // One effect may legally carry MAX_EFFECT_RECORDS events plus the
        // synthetic recalc event. Drain synchronously after every command so
        // this remains bounded without dropping Lua hooks.
        let event_subscriber = bus.subscribe(omacell_bus::MAX_EFFECT_RECORDS + 1);
        Self {
            bus,
            ui: UiSink::default(),
            event_subscriber,
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

    fn embedded_command_allowed(&self, id: &str) -> bool {
        if !EMBEDDED_COMMAND_ALLOWLIST.contains(&id) {
            return false;
        }
        self.bus.registry().get_str(id).is_ok_and(|command| {
            command.exposure == omacell_bus::Exposure::Public
                && (command.kind == omacell_bus::CommandKind::Query || command.changeset_eligible)
        })
    }

    fn workbook(&self) -> &Workbook {
        self.bus.workbook()
    }

    fn register_function(&mut self, def: DynamicFn) -> Result<(), CoreError> {
        self.bus.engine_mut().registry_mut().register_dynamic(def);
        Ok(())
    }

    fn take_events(&mut self) -> Vec<omacell_core::event::Event> {
        self.bus.drain(self.event_subscriber)
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

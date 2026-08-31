//! Host callbacks from Lua into the command bus and UI.

use omacell_core::error::CoreError;
use omacell_core::eval::DynamicFn;
use omacell_core::event::Event;
use omacell_core::workbook::Workbook;
use serde_json::Value;

/// Side of the runtime that talks to the application.
pub trait ScriptHost: Send {
    /// Execute a command-bus command (`Origin::Script`).
    fn execute(&mut self, id: &str, args: Value) -> Result<Value, CoreError>;
    /// Borrow the live workbook.
    fn workbook(&self) -> &Workbook;
    /// Register a custom function on the live calc registry.
    fn register_function(&mut self, def: DynamicFn) -> Result<(), CoreError>;
    /// Prompt; tests inject a canned reply.
    fn prompt(&mut self, message: &str) -> Result<String, CoreError>;
    /// Status line.
    fn status(&mut self, message: &str);
    /// Notification.
    fn notify(&mut self, message: &str);
    /// Optional keymap overlay (`mode`, `keys`, `cmd`).
    fn keymap_set(&mut self, mode: &str, keys: &str, cmd: &str) {
        let _ = (mode, keys, cmd);
    }
}

/// In-memory UI sinks for tests and CLI.
#[derive(Clone, Debug, Default)]
pub struct UiSink {
    /// Status messages in order.
    pub status: Vec<String>,
    /// Notifications in order.
    pub notify: Vec<String>,
    /// Prompt answers to dequeue.
    pub prompts: Vec<String>,
    /// Keymap overlays.
    pub keys: Vec<(String, String, String)>,
}

impl UiSink {
    /// Next prompt answer or an error.
    pub fn take_prompt(&mut self, message: &str) -> Result<String, CoreError> {
        if self.prompts.is_empty() {
            return Err(CoreError::new(
                "lua.prompt",
                format!("no prompt reply queued for {message:?}"),
            ));
        }
        Ok(self.prompts.remove(0))
    }
}

/// Dispatch a frozen [`Event`] to Lua hook names.
#[must_use]
pub fn hook_name(event: &Event) -> Option<&'static str> {
    match event {
        Event::WorkbookOpened { .. } => Some("on_open"),
        Event::CellChanged { .. } => Some("on_change"),
        Event::BeforeSave { .. } => Some("on_before_save"),
        Event::RecalcDone { .. } => Some("on_recalc"),
        Event::ThemeChanged { .. } => Some("on_theme_change"),
        _ => None,
    }
}

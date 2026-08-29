//! UI-core error helpers.

use omacell_core::error::CoreError;

/// Keymap / session error.
pub fn keymap(message: impl Into<String>) -> CoreError {
    CoreError::new("ui.keymap", message).with_hint("check default/keys and Config.keys.file")
}

/// Edit-state error.
pub fn edit(message: impl Into<String>) -> CoreError {
    CoreError::new("ui.edit", message)
}

/// Session persistence error.
pub fn session(message: impl Into<String>) -> CoreError {
    CoreError::new("ui.session", message)
}

/// Clipboard size/encoding error.
pub fn clipboard(message: impl Into<String>) -> CoreError {
    CoreError::new("ui.clipboard", message)
}

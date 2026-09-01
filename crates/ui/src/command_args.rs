//! Shared selection and clipboard arguments for frontend command dispatch.

use serde_json::{Value, json};

use crate::UiSession;

/// Fill omitted command arguments from the active UI selection and clipboard.
///
/// GUI and TUI both call this immediately before schema prompting and bus
/// dispatch so keyboard, menu, palette, and context-menu paths behave alike.
#[must_use]
pub fn inject_selection_args(ui: &UiSession, cmd: &str, mut args: Value) -> Value {
    if matches!(cmd, "edit.copy" | "edit.cut" | "edit.yank")
        && args.get("range").is_none_or(Value::is_null)
    {
        args["range"] = json!(ui.selection().active().to_range().to_a1());
    }
    if matches!(cmd, "edit.paste" | "edit.pastespecial") {
        if args.get("range").is_none_or(Value::is_null) {
            args["range"] = json!(ui.selection().cursor.to_a1());
        }
        if args.get("payload").is_none_or(Value::is_null)
            && let Some(clipboard) = ui.clipboard()
            && let Ok(payload) = clipboard.internal_json()
            && payload.is_object()
        {
            args["payload"] = payload;
        }
    }
    if cmd == "edit.fillselection" {
        let selection = ui.selection();
        if args.get("src").is_none_or(Value::is_null) {
            args["src"] = json!(selection.cursor.to_a1());
        }
        if args.get("dest").is_none_or(Value::is_null) {
            args["dest"] = json!(selection.active().to_range().to_a1());
        }
    }
    if matches!(cmd, "chart.fromselection" | "name.createfrom")
        && args
            .get("range")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
    {
        args["range"] = json!(ui.selection().active().to_range().to_a1());
    }
    if cmd.starts_with("ai.formula.")
        && args
            .get("ref")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
    {
        args["ref"] = json!(ui.selection().cursor.to_a1());
    }
    args
}

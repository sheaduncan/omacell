//! Command-bus identifiers, descriptors, origins, and outcomes (spec F-10.4, A-6.1).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, codes};

/// Dotted command identifier, e.g. `range.sort`.
///
/// Segments are `[a-z][a-z0-9]*`, at least two of them.
///
/// ```
/// use omacell_core::command::CommandId;
/// let id = CommandId::new("cell.set").expect("id");
/// assert_eq!(id.as_str(), "cell.set");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommandId(String);

impl CommandId {
    /// Validate and wrap a dotted command id.
    pub fn new(id: impl Into<String>) -> Result<Self, CoreError> {
        let id = id.into();
        if !is_valid_command_id(&id) {
            return Err(
                CoreError::new(codes::COMMAND_ID, format!("invalid command id {id:?}"))
                    .with_hint("use dotted lowercase ids such as cell.set"),
            );
        }
        Ok(Self(id))
    }

    /// Id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_valid_command_id(s: &str) -> bool {
    let mut n = 0usize;
    for part in s.split('.') {
        if !is_valid_segment(part) {
            return false;
        }
        n += 1;
    }
    n >= 2
}

fn is_valid_segment(part: &str) -> bool {
    let mut chars = part.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

impl fmt::Display for CommandId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CommandId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<String> for CommandId {
    type Error = CoreError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<CommandId> for String {
    fn from(id: CommandId) -> Self {
        id.0
    }
}

/// Registry metadata for one command (WP-07 fills the registry).
///
/// ```
/// use omacell_core::command::{CommandDescriptor, CommandId};
/// let schema: schemars::Schema =
///     serde_json::from_value(serde_json::json!({"type": "object"})).unwrap();
/// let d = CommandDescriptor {
///     id: CommandId::new("cell.set").expect("id"),
///     doc: "Set a cell input".into(),
///     arg_schema: schema,
///     mutating: true,
/// };
/// assert!(d.mutating);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandDescriptor {
    /// Dotted command id.
    pub id: CommandId,
    /// One-line documentation for the palette / `omacell commands --json`.
    pub doc: String,
    /// JSON Schema for `args` (schemars 1.x `Schema`; RootSchema was removed).
    pub arg_schema: schemars::Schema,
    /// Whether executing the command mutates the workbook.
    pub mutating: bool,
}

/// Who issued a command or changeset (spec A-6.1, plus IPC).
///
/// ```
/// use omacell_core::command::Origin;
/// let o = Origin::User;
/// assert_eq!(o, Origin::User);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Interactive user (keyboard, mouse, command palette typed by a human).
    User,
    /// Lua script (config, plugin, or trusted embedded script).
    Script,
    /// In-app command-palette plan that has not been applied yet.
    PalettePlan,
    /// In-app agent.
    InAppAgent,
    /// External agent (MCP / `omacell agent`).
    ExternalAgent,
    /// Unix-socket IPC client.
    Ipc,
}

/// Command result in the IPC reply shape `{ok, result?, error?}` (spec F-10.6).
///
/// ```
/// use omacell_core::command::Outcome;
/// let out = Outcome::success(serde_json::json!({"changed": 1}));
/// assert!(out.ok);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// Whether the command succeeded.
    pub ok: bool,
    /// Success payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Failure payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CoreError>,
}

impl Outcome {
    /// Successful outcome.
    #[must_use]
    pub fn success(result: serde_json::Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Failed outcome.
    #[must_use]
    pub fn failure(error: CoreError) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

/// Marker: applying a changeset is one undo unit (spec A-6.2).
///
/// ```
/// use omacell_core::command::UndoUnit;
/// let _ = UndoUnit;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UndoUnit;

/// Identifier assigned by the undo log (WP-02) to one [`UndoUnit`].
///
/// ```
/// use omacell_core::command::UndoUnitId;
/// let id = UndoUnitId::new(1);
/// assert_eq!(id.index(), 1);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UndoUnitId(u64);

impl UndoUnitId {
    /// Wrap a log-assigned id.
    #[must_use]
    pub const fn new(index: u64) -> Self {
        Self(index)
    }

    /// Numeric id.
    #[must_use]
    pub const fn index(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_id_rejects_bad_shapes() {
        assert!(CommandId::new("cell.set").is_ok());
        assert!(CommandId::new("file.export").is_ok());
        assert!(CommandId::new("ai.card.refresh").is_ok());
        assert!(CommandId::new("cell.Set").is_err());
        assert!(CommandId::new("set").is_err());
        assert!(CommandId::new("").is_err());
        assert!(CommandId::new("Cell.set").is_err());
        assert!(CommandId::new("cell.").is_err());
        assert!(CommandId::new(".set").is_err());
    }
}

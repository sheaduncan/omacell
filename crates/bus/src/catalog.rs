//! Versioned `commands_json()` catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::registry::{CommandRegistry, Exposure};

/// Envelope schema version for [`commands_json`].
pub const SCHEMA: u32 = 1;

/// One public catalog entry.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CommandJson {
    /// Dotted command id.
    pub id: String,
    /// Palette / CLI documentation.
    pub doc: String,
    /// Whether the command mutates.
    pub mutating: bool,
    /// Whether the command may be a changeset forward.
    pub changeset_eligible: bool,
    /// Default keymap chords.
    pub default_keys: Vec<String>,
    /// JSON Schema for arguments.
    pub arg_schema: serde_json::Value,
}

/// Versioned catalog envelope.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CommandsEnvelope {
    /// Schema version (currently 1).
    pub schema: u32,
    /// Commands sorted by id.
    pub commands: Vec<CommandJson>,
}

/// Serialize the public catalog. Internal restore handlers are excluded.
pub fn commands_json(registry: &CommandRegistry) -> Result<String, serde_json::Error> {
    let mut commands = Vec::new();
    for (id, cmd) in registry.iter() {
        if cmd.exposure != Exposure::Public {
            continue;
        }
        let arg_schema = serde_json::to_value(&cmd.descriptor.arg_schema)?;
        commands.push(CommandJson {
            id: id.to_string(),
            doc: cmd.descriptor.doc.clone(),
            mutating: cmd.descriptor.mutating,
            changeset_eligible: cmd.changeset_eligible,
            default_keys: cmd.default_keys.clone(),
            arg_schema,
        });
    }
    commands.sort_by(|a, b| a.id.cmp(&b.id));
    let envelope = CommandsEnvelope {
        schema: SCHEMA,
        commands,
    };
    serde_json::to_string_pretty(&envelope)
}

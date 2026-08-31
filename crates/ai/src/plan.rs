//! Natural-language plan → validated command list.

use std::collections::BTreeSet;

use omacell_core::changeset::CommandCall;
use omacell_core::command::CommandId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// One planned command.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlannedCommand {
    /// Registry id.
    pub id: String,
    /// JSON arguments.
    #[serde(default)]
    pub args: Value,
}

/// Model plan envelope.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Ordered commands.
    pub commands: Vec<PlannedCommand>,
}

/// Defense-in-depth exclusions within the explicit command catalog.
pub const FORBIDDEN_PREFIXES: &[&str] = &[
    "trust.",
    "script.",
    "scripting.",
    "file.",
    "network.",
    "ai.agent",
    "config.",
];

/// Whether `id` is forbidden for model-originated mutations.
#[must_use]
pub fn forbidden(id: &str) -> bool {
    FORBIDDEN_PREFIXES.iter().any(|p| id.starts_with(p))
}

/// Parse and validate a model JSON plan against the command catalog.
pub fn parse_plan(value: &Value, catalog: &BTreeSet<String>) -> Result<Plan, AiError> {
    let plan: Plan = serde_json::from_value(value.clone())
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("plan JSON: {err}")))?;
    if plan.commands.len() > 200 {
        return Err(AiError::new(
            codes::PAYLOAD,
            "plan exceeds the 200-command limit",
        ));
    }
    for cmd in &plan.commands {
        if cmd.id.split('.').count() < 2 {
            return Err(AiError::new(
                codes::PAYLOAD,
                format!("plan command id {} is not dotted", cmd.id),
            ));
        }
        if forbidden(&cmd.id) {
            return Err(AiError::new(
                codes::PAYLOAD,
                format!("plan command {} is forbidden for models", cmd.id),
            )
            .with_hint("models cannot change trust, network, scripting, files, or AI policy"));
        }
        if !catalog.contains(&cmd.id) {
            return Err(AiError::new(
                codes::PAYLOAD,
                format!("unknown command {}", cmd.id),
            ));
        }
        if !cmd.args.is_null() && !cmd.args.is_object() {
            return Err(AiError::new(
                codes::PAYLOAD,
                "command args must be an object",
            ));
        }
    }
    Ok(plan)
}

/// Convert a plan to changeset forwards.
pub fn to_calls(plan: &Plan) -> Result<Vec<CommandCall>, AiError> {
    plan.commands
        .iter()
        .map(|c| {
            Ok(CommandCall {
                id: CommandId::new(&c.id).map_err(AiError::from)?,
                args: if c.args.is_null() {
                    serde_json::json!({})
                } else {
                    c.args.clone()
                },
            })
        })
        .collect()
}

/// JSON schema the model must return.
#[must_use]
pub fn plan_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["commands"],
        "additionalProperties": false,
        "properties": {
            "commands": {
                "type": "array",
                "maxItems": 200,
                "items": {
                    "type": "object",
                    "required": ["id"],
                    "additionalProperties": false,
                    "properties": {
                        "id": {"type": "string"},
                        "args": {"type": "object"}
                    }
                }
            }
        }
    })
}

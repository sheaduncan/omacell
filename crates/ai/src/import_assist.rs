//! Import-plan overlay. Never auto-applies.

use omacell_io::csv::{ImportPlan, PreviewRows, import_assist_request};
use serde_json::Value;

use crate::error::{AiError, codes};
use crate::policy::{PolicySnapshot, SendLevel};
use crate::redact::redact_json;

/// JSON schema for the bounded import-plan fields an assistant may propose.
///
/// The remaining [`ImportPlan`] fields retain their serde defaults. The user
/// still reviews the proposed plan before it is applied.
#[must_use]
pub fn import_plan_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["plan"],
        "additionalProperties": false,
        "properties": {
            "plan": {
                "type": "object",
                "required": ["delimiter", "has_header", "skip_rows", "decimal", "thousands"],
                "additionalProperties": false,
                "properties": {
                    "delimiter": {"type": "string", "minLength": 1, "maxLength": 1},
                    "has_header": {"type": "boolean"},
                    "skip_rows": {"type": "integer", "minimum": 0},
                    "decimal": {"type": "string", "minLength": 1, "maxLength": 1},
                    "thousands": {"type": ["string", "null"], "minLength": 1, "maxLength": 1}
                }
            }
        }
    })
}

/// Parse a plan overlay from the model.
pub fn parse_plan_overlay(value: &Value) -> Result<ImportPlan, AiError> {
    let plan = value.get("plan").cloned().unwrap_or_else(|| value.clone());
    serde_json::from_value(plan)
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("import plan: {err}")))
}

/// Build the import-assistant payload through the configured privacy boundary.
///
/// Schema-only policy retains headers, inferred kinds, and conversion markers
/// but strips sample values. Sample/full policy may include the bounded preview;
/// configured detectors redact it before provider hooks can observe the request.
pub fn import_request_payload(
    plan: ImportPlan,
    mut preview: PreviewRows,
    policy: &PolicySnapshot,
) -> Result<Value, AiError> {
    if policy.send == SendLevel::Schema {
        for cell in preview.rows.iter_mut().flatten() {
            cell.raw.clear();
            cell.would_become.clear();
        }
    }
    let mut payload = serde_json::to_value(import_assist_request(plan, preview))
        .map_err(|error| AiError::new(codes::PAYLOAD, error.to_string()))?;
    if policy.suggest_redaction {
        let _ = redact_json(&mut payload);
    }
    Ok(payload)
}

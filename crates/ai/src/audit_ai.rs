//! Model judgments layered on WP-19 findings.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// Extra AI finding.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiFinding {
    /// Stable id.
    pub id: String,
    /// Message.
    pub message: String,
    /// 0..=1.
    #[serde(default)]
    pub confidence: f64,
    /// Optional A1.
    #[serde(default)]
    pub cell_ref: Option<String>,
}

/// Parse findings.
pub fn parse_findings(value: &Value) -> Result<Vec<AiFinding>, AiError> {
    let rows = value
        .get("findings")
        .cloned()
        .unwrap_or_else(|| value.clone());
    let findings: Vec<AiFinding> = serde_json::from_value(rows)
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("ai audit: {err}")))?;
    if findings
        .iter()
        .any(|finding| !(0.0..=1.0).contains(&finding.confidence))
    {
        return Err(AiError::new(
            codes::PAYLOAD,
            "AI audit confidence must be between 0 and 1",
        ));
    }
    Ok(findings)
}

/// JSON schema for audit findings.
#[must_use]
pub fn findings_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["findings"],
        "additionalProperties": false,
        "properties": {
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "message"],
                    "additionalProperties": false,
                    "properties": {
                        "id": {"type": "string"},
                        "message": {"type": "string"},
                        "confidence": {"type": "number"},
                        "cell_ref": {"type": "string"}
                    }
                }
            }
        }
    })
}

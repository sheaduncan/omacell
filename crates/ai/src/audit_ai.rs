//! Model judgments layered on WP-19 findings.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AiError, codes};

/// Extra AI finding.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    serde_json::from_value(rows)
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("ai audit: {err}")))
}

/// JSON schema for audit findings.
#[must_use]
pub fn findings_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "required": ["findings"],
        "properties": {
            "findings": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "message"],
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

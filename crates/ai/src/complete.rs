//! Ghost-text formula-bar completion.

use serde_json::{Value, json};

use crate::error::{AiError, codes};

/// Schema for completion.
#[must_use]
pub fn complete_schema() -> Value {
    json!({"type":"object","required":["text"],"additionalProperties":false,"properties":{"text":{"type":"string"}}})
}

/// Extract ghost text.
pub fn parse_completion(value: &Value) -> Result<String, AiError> {
    value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AiError::new(codes::PAYLOAD, "completion response is missing text"))
}

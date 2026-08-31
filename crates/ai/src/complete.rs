//! Ghost-text formula-bar completion.

use serde_json::{Value, json};

/// Schema for completion.
#[must_use]
pub fn complete_schema() -> Value {
    json!({"type":"object","required":["text"],"properties":{"text":{"type":"string"}}})
}

/// Extract ghost text.
#[must_use]
pub fn parse_completion(value: &Value) -> String {
    value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

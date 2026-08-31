//! Import-plan overlay. Never auto-applies.

use omacell_io::csv::ImportPlan;
use serde_json::Value;

use crate::error::{AiError, codes};

/// Parse a plan overlay from the model.
pub fn parse_plan_overlay(value: &Value) -> Result<ImportPlan, AiError> {
    let plan = value.get("plan").cloned().unwrap_or_else(|| value.clone());
    serde_json::from_value(plan)
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("import plan: {err}")))
}

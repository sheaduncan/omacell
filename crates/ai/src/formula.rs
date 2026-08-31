//! Formula generate / explain / fix / refactor.

use omacell_core::eval::{RuntimeValue, eval_formula};
use omacell_core::formula::parse;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use serde_json::{Value, json};

use crate::error::{AiError, codes};

/// Model output for a generated formula.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormulaOut {
    /// Formula-bar text (`=SUM(A1:A10)`).
    pub formula: String,
}

/// JSON schema.
#[must_use]
pub fn formula_schema() -> Value {
    json!({"type":"object","required":["formula"],"additionalProperties":false,"properties":{"formula":{"type":"string"}}})
}

/// Parse model JSON and evaluate in a scratch cell. Rejects unparseable output.
pub fn parse_and_eval(
    value: &Value,
    wb: &Workbook,
    engine: &RecalcEngine,
    cell: CellCoord,
) -> Result<(String, RuntimeValue), AiError> {
    let out: FormulaOut = serde_json::from_value(value.clone())
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("formula JSON: {err}")))?;
    let src = if out.formula.starts_with('=') {
        out.formula.clone()
    } else {
        format!("={}", out.formula)
    };
    let parsed = parse(&src).map_err(|err| {
        AiError::new(codes::PAYLOAD, err.error.message).with_hint("generated formula must parse")
    })?;
    let (runtime, _flags) =
        eval_formula(wb, engine.registry(), engine.spill(), cell, &parsed.ast, 0);
    Ok((src, runtime))
}

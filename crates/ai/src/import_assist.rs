//! Import-plan overlay. Never auto-applies.

use omacell_io::csv::{ImportPlan, PreviewRows, import_assist_request};
use serde::Deserialize;
use serde_json::Value;

use crate::error::{AiError, codes};
use crate::policy::{PolicySnapshot, SendLevel};
use crate::redact::redact_json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportPlanResponse {
    plan: ImportPlanOverlay,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportPlanOverlay {
    delimiter: char,
    has_header: bool,
    skip_rows: u32,
    decimal: char,
    thousands: Option<char>,
}

/// JSON schema for the bounded import-plan fields an assistant may propose.
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

/// Decode the complete current plan supplied by the import preview.
pub fn parse_import_plan(value: &Value) -> Result<ImportPlan, AiError> {
    serde_json::from_value(value.clone())
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("import plan: {err}")))
}

/// Parse a bounded model overlay and merge it onto the current import plan.
pub fn parse_plan_overlay(current: &ImportPlan, value: &Value) -> Result<ImportPlan, AiError> {
    let response: ImportPlanResponse = serde_json::from_value(value.clone())
        .map_err(|err| AiError::new(codes::PAYLOAD, format!("import plan overlay: {err}")))?;
    let overlay = response.plan;
    let mut proposed = current.clone();
    proposed.delimiter = overlay.delimiter;
    proposed.has_header = overlay.has_header;
    proposed.skip_rows = overlay.skip_rows;
    proposed.decimal = overlay.decimal;
    proposed.thousands = overlay.thousands;
    proposed.validate().map_err(|err| {
        AiError::new(
            codes::PAYLOAD,
            format!("import plan overlay: {}", err.message),
        )
    })?;
    Ok(proposed)
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

#[cfg(test)]
mod tests {
    use omacell_core::date_system::DateSystem;
    use omacell_core::locale::LocaleId;
    use omacell_io::csv::{ColumnPlan, ColumnType, LineEnding, TextEncoding};
    use serde_json::json;

    use super::*;

    #[test]
    fn overlay_schema_is_closed_and_requires_every_bounded_field() {
        let schema = import_plan_schema();
        assert_eq!(
            schema["properties"]["plan"]["required"],
            json!([
                "delimiter",
                "has_header",
                "skip_rows",
                "decimal",
                "thousands"
            ])
        );
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"]["plan"]["additionalProperties"], false);
    }

    #[test]
    fn overlay_changes_only_the_five_bounded_fields() {
        let current = ImportPlan {
            delimiter: ',',
            quote: '\'',
            encoding: TextEncoding::Utf16Le,
            bom: true,
            has_header: false,
            skip_rows: 0,
            locale: LocaleId::DE_DE,
            decimal: '.',
            thousands: Some(','),
            line_ending: LineEnding::CrLf,
            date_system: DateSystem::Excel1904,
            columns: vec![ColumnPlan {
                name: Some("account".into()),
                ty: ColumnType::KeepAsText,
            }],
        };
        let proposed = parse_plan_overlay(
            &current,
            &json!({
                "plan": {
                    "delimiter": ";",
                    "has_header": true,
                    "skip_rows": 2,
                    "decimal": ",",
                    "thousands": "."
                }
            }),
        )
        .unwrap();

        assert_eq!(proposed.delimiter, ';');
        assert!(proposed.has_header);
        assert_eq!(proposed.skip_rows, 2);
        assert_eq!(proposed.decimal, ',');
        assert_eq!(proposed.thousands, Some('.'));
        assert_eq!(proposed.quote, current.quote);
        assert_eq!(proposed.encoding, current.encoding);
        assert_eq!(proposed.bom, current.bom);
        assert_eq!(proposed.locale, current.locale);
        assert_eq!(proposed.line_ending, current.line_ending);
        assert_eq!(proposed.date_system, current.date_system);
        assert_eq!(proposed.columns, current.columns);
    }

    #[test]
    fn overlay_rejects_fields_outside_the_bounded_contract() {
        let error = parse_plan_overlay(
            &ImportPlan::default(),
            &json!({
                "plan": {
                    "delimiter": ",",
                    "has_header": true,
                    "skip_rows": 0,
                    "decimal": ".",
                    "thousands": ",",
                    "encoding": "utf-16le"
                }
            }),
        )
        .unwrap_err();
        assert!(error.message.contains("unknown field"));
    }
}

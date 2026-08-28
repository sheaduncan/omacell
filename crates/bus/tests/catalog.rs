//! Catalog schema, sorting, and typed-argument rejection.

mod common;

use omacell_bus::args::{
    CalcModeArgs, CalcRecalcArgs, CellClearArgs, CellSetArgs, EmptyArgs, FormatNumberArgs,
    NameDefineArgs, NameRemoveArgs, RangeClearArgs, RangeSetArgs, SheetAddArgs, SheetRenameArgs,
    SheetVisibilityArgs, StyleSetArgs,
};
use omacell_bus::{SCHEMA, commands_json};
use omacell_core::command::Origin;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

fn validate_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if let Some(expected) = schema.get("const")
        && value != expected
    {
        return Err(format!("{path}: expected const {expected}, got {value}"));
    }
    if let Some(choices) = schema.get("enum").and_then(serde_json::Value::as_array)
        && !choices.contains(value)
    {
        return Err(format!("{path}: {value} is not in enum"));
    }
    match schema.get("type").and_then(serde_json::Value::as_str) {
        Some("object") => {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{path}: expected object"))?;
            let Some(properties) = schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
            else {
                return Ok(());
            };
            if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
                for key in required.iter().filter_map(serde_json::Value::as_str) {
                    if !object.contains_key(key) {
                        return Err(format!("{path}: missing {key}"));
                    }
                }
            }
            if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
                for key in object.keys() {
                    if !properties.contains_key(key) {
                        return Err(format!("{path}: unexpected property {key}"));
                    }
                }
            }
            for (key, child) in object {
                if let Some(child_schema) = properties.get(key) {
                    validate_schema(child, child_schema, &format!("{path}.{key}"))?;
                }
            }
        }
        Some("array") => {
            let array = value
                .as_array()
                .ok_or_else(|| format!("{path}: expected array"))?;
            if let Some(items) = schema.get("items") {
                for (index, item) in array.iter().enumerate() {
                    validate_schema(item, items, &format!("{path}[{index}]"))?;
                }
            }
        }
        Some("string") => {
            let string = value
                .as_str()
                .ok_or_else(|| format!("{path}: expected string"))?;
            if let Some(minimum) = schema.get("minLength").and_then(serde_json::Value::as_u64)
                && string.chars().count() < minimum as usize
            {
                return Err(format!("{path}: string is too short"));
            }
            if let Some(pattern) = schema.get("pattern").and_then(serde_json::Value::as_str) {
                let re = regex_is_command_id(pattern);
                if re && !is_command_id(string) {
                    return Err(format!(
                        "{path}: {string} does not match command id pattern"
                    ));
                }
            }
        }
        Some("integer") => {
            let number = value
                .as_i64()
                .ok_or_else(|| format!("{path}: expected integer"))?;
            if let Some(c) = schema.get("const").and_then(serde_json::Value::as_i64)
                && number != c
            {
                return Err(format!("{path}: expected {c}"));
            }
        }
        Some("boolean") if !value.is_boolean() => {
            return Err(format!("{path}: expected boolean"));
        }
        Some("boolean") => {}
        _ => {}
    }
    Ok(())
}

fn regex_is_command_id(pattern: &str) -> bool {
    pattern.contains("[a-z]")
}

fn is_command_id(s: &str) -> bool {
    omacell_core::command::CommandId::new(s).is_ok()
}

#[test]
fn commands_json_is_sorted_stable_and_matches_schema() {
    let bus = common::bus();
    let json = bus.commands_json().unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schema"], SCHEMA);
    let ids: Vec<String> = value["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    assert!(!ids.contains(&"cell.restore".to_string()));
    assert!(!ids.contains(&"sheet.remove".to_string()));
    for required in [
        "cell.set",
        "cell.clear",
        "range.set",
        "range.clear",
        "sheet.add",
        "sheet.rename",
        "sheet.visibility",
        "name.define",
        "name.remove",
        "format.number",
        "style.set",
        "calc.recalc",
        "calc.mode",
        "edit.undo",
        "edit.redo",
    ] {
        assert!(ids.contains(&required.to_string()), "missing {required}");
    }
    let schema_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas/commands.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(schema_path).unwrap()).unwrap();
    validate_schema(&value, &schema, "$").unwrap();
    let again = commands_json(bus.registry()).unwrap();
    assert_eq!(json, again);
}

#[test]
fn public_commands_have_typed_args_schema_doc_and_classification() {
    let bus = common::bus();
    let value: serde_json::Value = serde_json::from_str(&bus.commands_json().unwrap()).unwrap();
    for cmd in value["commands"].as_array().unwrap() {
        assert!(!cmd["doc"].as_str().unwrap().is_empty());
        assert!(cmd["arg_schema"].is_object());
        assert!(cmd["mutating"].is_boolean());
        assert!(cmd["changeset_eligible"].is_boolean());
        assert!(cmd["default_keys"].is_array());
    }
}

#[test]
fn unknown_fields_are_rejected() {
    let mut bus = common::bus();
    let err = common::exec_err(
        &mut bus,
        "cell.set",
        json!({"ref": "A1", "input": "1", "extra": true}),
    );
    assert_eq!(err.code, omacell_bus::codes::COMMAND_ARGS);
}

#[test]
fn unknown_command_is_rejected() {
    let mut bus = common::bus();
    let out = bus.execute(Origin::User, "nope.nope", json!({}));
    assert!(!out.ok);
    assert_eq!(out.error.unwrap().code, omacell_bus::codes::COMMAND_UNKNOWN);
}

#[test]
fn public_arg_types_round_trip() {
    let names = [
        "cell.set",
        "cell.clear",
        "range.set",
        "range.clear",
        "sheet.add",
        "sheet.rename",
        "sheet.visibility",
        "name.define",
        "name.remove",
        "format.number",
        "style.set",
        "calc.recalc",
        "calc.mode",
        "edit.undo",
        "edit.redo",
    ];
    let bus = common::bus();
    let catalog: serde_json::Value = serde_json::from_str(&bus.commands_json().unwrap()).unwrap();
    for name in names {
        assert!(
            catalog["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|command| command["id"] == name && command["arg_schema"].is_object()),
            "missing typed schema for {name}"
        );
    }

    assert_round_trip::<CellSetArgs>(json!({"ref": "Sheet1!B2", "input": "=A1+1"}));
    assert_round_trip::<CellClearArgs>(json!({"ref": "A1"}));
    assert_round_trip::<RangeSetArgs>(json!({"range": "A1:B2", "input": "1"}));
    assert_round_trip::<RangeClearArgs>(json!({"range": "A1:B2"}));
    assert_round_trip::<SheetAddArgs>(json!({"name": "Data"}));
    assert_round_trip::<SheetRenameArgs>(json!({"sheet": "Sheet1", "name": "Data"}));
    assert_round_trip::<SheetVisibilityArgs>(json!({"sheet": "Sheet1", "visibility": "hidden"}));
    assert_round_trip::<NameDefineArgs>(
        json!({"name": "Tax", "referent": {"type": "constant", "value": 0.2}}),
    );
    assert_round_trip::<NameRemoveArgs>(json!({"name": "Tax"}));
    assert_round_trip::<FormatNumberArgs>(json!({"range": "A1", "format": "0.00"}));
    assert_round_trip::<StyleSetArgs>(json!({"range": "A1", "bold": true}));
    assert_round_trip::<CalcRecalcArgs>(json!({"mode": "full"}));
    assert_round_trip::<CalcModeArgs>(json!({"mode": "manual"}));
    assert_round_trip::<EmptyArgs>(json!({}));
}

fn assert_round_trip<T>(sample: serde_json::Value)
where
    T: DeserializeOwned + Serialize,
{
    let parsed: T = serde_json::from_value(sample.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), sample);
}

//! IPC v1 schema, fixture, and decoder tests.

#![cfg(unix)]

use omacell_bus::ipc::{
    ControlOp, MAX_FRAME_BYTES, MAX_JSON_DEPTH, Mode, Request, check_json_depth, decode_request,
    decode_request_bytes,
};
use std::path::PathBuf;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ipc")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn schema(name: &str) -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas/ipc")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn validate(
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
    if let Some(options) = schema.get("oneOf").and_then(serde_json::Value::as_array) {
        let mut errors = Vec::new();
        for (i, option) in options.iter().enumerate() {
            match validate(value, option, &format!("{path}.oneOf[{i}]")) {
                Ok(()) => return Ok(()),
                Err(e) => errors.push(e),
            }
        }
        return Err(format!("{path}: no oneOf matched ({})", errors.join("; ")));
    }
    match schema.get("type").and_then(serde_json::Value::as_str) {
        Some("object") => {
            let object = value
                .as_object()
                .ok_or_else(|| format!("{path}: expected object"))?;
            let properties = schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
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
                    validate(child, child_schema, &format!("{path}.{key}"))?;
                }
            }
        }
        Some("array") => {
            let array = value
                .as_array()
                .ok_or_else(|| format!("{path}: expected array"))?;
            if let Some(items) = schema.get("items") {
                for (i, item) in array.iter().enumerate() {
                    validate(item, items, &format!("{path}[{i}]"))?;
                }
            }
        }
        Some("string") => {
            if !value.is_string() {
                return Err(format!("{path}: expected string"));
            }
        }
        Some("integer") => {
            if value.as_u64().is_none() && value.as_i64().is_none() {
                return Err(format!("{path}: expected integer"));
            }
        }
        Some("boolean") => {
            if !value.is_boolean() {
                return Err(format!("{path}: expected boolean"));
            }
        }
        Some(other) => return Err(format!("{path}: unsupported schema type {other}")),
        None => {}
    }
    Ok(())
}

#[test]
fn fixtures_validate_against_committed_schemas() {
    let cases = [
        ("command-propose.json", "request.schema.json"),
        ("command-query.json", "request.schema.json"),
        ("subscribe.json", "request.schema.json"),
        ("reply-ok.json", "reply.schema.json"),
        ("reply-error.json", "reply.schema.json"),
        ("event-cell.json", "event.schema.json"),
        ("overflow.json", "event.schema.json"),
        ("discovery.json", "discovery.schema.json"),
    ];
    for (file, sch) in cases {
        let value: serde_json::Value = serde_json::from_str(&fixture(file)).unwrap();
        validate(&value, &schema(sch), file).unwrap_or_else(|e| panic!("{file}: {e}"));
    }
}

#[test]
fn fixtures_round_trip_through_decoder() {
    let req = decode_request(&fixture("command-propose.json")).unwrap();
    match req {
        Request::Command { id, cmd, mode, .. } => {
            assert_eq!(id, 7);
            assert_eq!(cmd, "cell.set");
            assert_eq!(mode, Some(Mode::Propose));
        }
        other => panic!("{other:?}"),
    }
    let sub = decode_request(&fixture("subscribe.json")).unwrap();
    match sub {
        Request::Control { op, events, .. } => {
            assert!(matches!(op, ControlOp::Subscribe));
            assert_eq!(events, vec!["cell_changed", "recalc_done"]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn unknown_version_field_mode_and_cmd_op_are_rejected() {
    let err = decode_request(r#"{"v":2,"id":1,"op":"ping"}"#).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_VERSION);
    let err = decode_request(r#"{"v":1,"id":1,"op":"ping","nope":true}"#).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_PROTOCOL);
    let err = decode_request(r#"{"v":1,"id":1,"cmd":"cell.set","mode":"yeet"}"#).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_PROTOCOL);
    let err = decode_request(r#"{"v":1,"id":1,"cmd":"cell.set","op":"ping"}"#).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_PROTOCOL);
    let err = decode_request(r#"{"v":1,"id":1}"#).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_PROTOCOL);
    let err = decode_request(r#"{"v":1,"id":1,"cmd":"cell.restore"}"#).unwrap();
    match err {
        Request::Command { cmd, .. } => assert_eq!(cmd, "cell.restore"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn malformed_partial_nested_and_oversized_frames_fail_closed() {
    assert!(decode_request("{").is_err());
    assert!(decode_request_bytes(&[0xff, 0xfe]).is_err());
    let nested = format!(
        "{}{}",
        "{".repeat(MAX_JSON_DEPTH as usize + 2),
        "}".repeat(MAX_JSON_DEPTH as usize + 2)
    );
    let err = check_json_depth(&nested).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_LIMIT);
    let huge = "x".repeat(MAX_FRAME_BYTES + 8);
    let err = decode_request_bytes(huge.as_bytes()).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_FRAME);
}

#[test]
fn frame_buf_rejects_a_line_without_newline_past_the_cap() {
    let mut buf = omacell_bus::ipc::FrameBuf::new();
    let chunk = vec![b'a'; MAX_FRAME_BYTES + 1];
    let err = buf.push(&chunk).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_FRAME);
}

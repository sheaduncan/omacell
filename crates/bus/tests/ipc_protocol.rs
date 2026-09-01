//! IPC v1 schema, fixture, and decoder tests.

#![cfg(unix)]

use omacell_bus::ipc::{
    ControlOp, IpcLimits, MAX_FRAME_BYTES, MAX_JSON_DEPTH, Mode, Reply, Request, check_json_depth,
    decode_request, decode_request_bytes, decode_request_bytes_with_limits, encode_command,
    encode_line_with_limits,
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
    if let Some(boolean) = schema.as_bool() {
        return if boolean {
            Ok(())
        } else {
            Err(format!("{path}: false schema"))
        };
    }
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
        let mut matches = 0;
        for (i, option) in options.iter().enumerate() {
            match validate(value, option, &format!("{path}.oneOf[{i}]")) {
                Ok(()) => matches += 1,
                Err(e) => errors.push(e),
            }
        }
        if matches != 1 {
            return Err(format!(
                "{path}: expected one oneOf match, got {matches} ({})",
                errors.join("; ")
            ));
        }
    }
    if let Some(negated) = schema.get("not")
        && validate(value, negated, &format!("{path}.not")).is_ok()
    {
        return Err(format!("{path}: matched a forbidden schema"));
    }
    match schema.get("type").and_then(serde_json::Value::as_str) {
        Some("object") => {
            if !value.is_object() {
                return Err(format!("{path}: expected object"));
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
            if let Some(max) = schema.get("maxItems").and_then(serde_json::Value::as_u64)
                && array.len() as u64 > max
            {
                return Err(format!("{path}: array is longer than {max}"));
            }
        }
        Some("string") => {
            let string = value
                .as_str()
                .ok_or_else(|| format!("{path}: expected string"))?;
            if let Some(min) = schema.get("minLength").and_then(serde_json::Value::as_u64)
                && string.chars().count() < min as usize
            {
                return Err(format!("{path}: string is shorter than {min}"));
            }
        }
        Some("integer") => {
            if value.as_u64().is_none() && value.as_i64().is_none() {
                return Err(format!("{path}: expected integer"));
            }
            if let Some(minimum) = schema.get("minimum").and_then(serde_json::Value::as_i64)
                && value.as_i64().is_some_and(|number| number < minimum)
            {
                return Err(format!("{path}: integer is less than {minimum}"));
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
    if let Some(object) = value.as_object() {
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

    let null_reply = Reply::ok(9, serde_json::Value::Null);
    let encoded = serde_json::to_string(&null_reply).unwrap();
    assert_eq!(serde_json::from_str::<Reply>(&encoded).unwrap(), null_reply);
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
    let limits = IpcLimits::new(1_024).unwrap();
    let huge = "x".repeat(1_032);
    let err = decode_request_bytes_with_limits(huge.as_bytes(), limits).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_FRAME);
}

#[test]
fn frame_buf_rejects_a_line_without_newline_past_the_cap() {
    let limits = IpcLimits::new(1_024).unwrap();
    let mut buf = omacell_bus::ipc::FrameBuf::with_limits(limits);
    let chunk = vec![b'a'; 1_025];
    let err = buf.push(&chunk).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_FRAME);
}

#[test]
fn frame_cap_includes_the_trailing_newline() {
    let limits = IpcLimits::new(1_024).unwrap();
    let mut allowed = vec![b'a'; 1_023];
    allowed.push(b'\n');
    let mut buf = omacell_bus::ipc::FrameBuf::with_limits(limits);
    assert_eq!(buf.push(&allowed).unwrap()[0].len(), 1_023);

    let mut oversized = vec![b'a'; 1_024];
    oversized.push(b'\n');
    let err = buf.push(&oversized).unwrap_err();
    assert_eq!(err.code, omacell_bus::codes::IPC_FRAME);
}

#[test]
fn default_ceiling_accepts_a_request_larger_than_one_mib() {
    assert_eq!(MAX_FRAME_BYTES, 16_777_216);
    let input = "x".repeat(1_100_000);
    let line = encode_command(
        1,
        "cell.set",
        &serde_json::json!({"ref":"A1","input":input}),
        Some(Mode::Propose),
    )
    .unwrap();
    assert!(line.len() > 1_048_576);
    let request = decode_request_bytes(line.as_bytes()).unwrap();
    let Request::Command { args, .. } = request else {
        panic!("expected command request");
    };
    assert_eq!(args["input"].as_str().unwrap().len(), 1_100_000);
}

#[test]
fn configured_limit_is_enforced_for_decode_buffer_and_encode() {
    let limits = IpcLimits::new(256).unwrap();
    let oversized = serde_json::json!({"payload": "x".repeat(256)});
    let encoded = encode_line_with_limits(&oversized, limits).unwrap_err();
    assert_eq!(encoded.code, omacell_bus::codes::IPC_FRAME);
    assert!(
        encoded
            .hint
            .as_deref()
            .unwrap()
            .contains("split large ranges")
    );

    let bytes = serde_json::to_vec(&oversized).unwrap();
    let decoded = decode_request_bytes_with_limits(&bytes, limits).unwrap_err();
    assert_eq!(decoded.code, omacell_bus::codes::IPC_FRAME);
    let mut frames = omacell_bus::ipc::FrameBuf::with_limits(limits);
    let buffered = frames.push(&bytes).unwrap_err();
    assert_eq!(buffered.code, omacell_bus::codes::IPC_FRAME);

    let invalid = IpcLimits::new(MAX_FRAME_BYTES + 1).unwrap_err();
    assert_eq!(invalid.code, omacell_bus::codes::IPC_LIMIT);
}

#[test]
fn request_schema_matches_control_operation_fields() {
    let request = schema("request.schema.json");
    for invalid in [
        serde_json::json!({"v":1,"id":1,"op":"ping","events":[]}),
        serde_json::json!({"v":1,"id":1,"op":"subscribe","changeset":"cs-1"}),
        serde_json::json!({"v":1,"id":1,"op":"changeset.apply"}),
    ] {
        assert!(
            validate(&invalid, &request, "request").is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn decoder_rejects_fields_that_do_not_belong_to_the_control_op() {
    for invalid in [
        r#"{"v":1,"id":1,"op":"ping","events":[]}"#,
        r#"{"v":1,"id":1,"op":"subscribe","changeset":"cs-1"}"#,
        r#"{"v":1,"id":1,"op":"changeset.apply"}"#,
    ] {
        let err = decode_request(invalid).unwrap_err();
        assert_eq!(err.code, omacell_bus::codes::IPC_PROTOCOL, "{invalid}");
    }
}

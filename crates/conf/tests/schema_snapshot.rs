//! Snapshot of `docs/schemas/config.schema.json`.

use omacell_conf::schema::Config;
use schemars::schema_for;

#[test]
fn config_schema_matches_committed() {
    let schema = schema_for!(Config);
    let json = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas/config.schema.json");
    if !path.is_file() || std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &json).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        committed, json,
        "regenerate docs/schemas/config.schema.json"
    );
}

#[test]
fn schema_rejects_unknown_fields_and_documents_enum_values() {
    let schema = serde_json::to_value(schema_for!(Config)).unwrap();
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(
        schema["$defs"]["Calc"]["properties"]["mode"]["enum"],
        serde_json::json!(["automatic", "automatic_except_tables", "manual"])
    );
    assert_eq!(
        schema["$defs"]["Behavior"]["properties"]["date_system"]["enum"],
        serde_json::json!([1900, 1904])
    );
}

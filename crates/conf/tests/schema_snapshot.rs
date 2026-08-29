//! Snapshot of `docs/schemas/config.schema.json`.

use omacell_conf::schema::Config;
use schemars::schema_for;

#[test]
fn config_schema_matches_committed() {
    let schema = schema_for!(Config);
    let json = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas/config.schema.json");
    if !path.is_file() {
        std::fs::write(&path, &json).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        committed, json,
        "regenerate docs/schemas/config.schema.json"
    );
}

//! WP-19 audit and find commands.

mod common;

use serde_json::json;

fn audit_bus() -> omacell_bus::Bus {
    let mut bus = common::bus();
    omacell_bus::register_audit_commands(bus.registry_mut()).unwrap();
    bus
}

#[test]
fn audit_run_returns_schema_one() {
    let mut bus = audit_bus();
    let result = common::exec_ok(&mut bus, "audit.run", json!({}));
    assert_eq!(result["schema"], 1);
    assert!(result["findings"].is_array());
}

#[test]
fn edit_find_counts_matches() {
    let mut bus = audit_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "hello"}));
    let result = common::exec_ok(&mut bus, "edit.findall", json!({"query": "hello"}));
    assert_eq!(result["count"], 1);
}

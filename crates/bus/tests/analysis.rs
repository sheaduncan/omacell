//! Pivot, Goal Seek, and statistics command tests.

mod common;

use omacell_core::changeset::CommandCall;
use omacell_core::command::{CommandId, Origin};
use omacell_core::value::Value;
use serde_json::json;

fn analysis_bus() -> omacell_bus::Bus {
    let mut bus = common::bus();
    omacell_bus::register_edit_commands(bus.registry_mut()).unwrap();
    omacell_bus::register_analysis_commands(bus.registry_mut()).unwrap();
    bus
}

fn seed(bus: &mut omacell_bus::Bus) {
    common::exec_ok(bus, "cell.set", json!({"ref": "A1", "input": "Region"}));
    common::exec_ok(bus, "cell.set", json!({"ref": "B1", "input": "Amount"}));
    common::exec_ok(bus, "cell.set", json!({"ref": "A2", "input": "East"}));
    common::exec_ok(bus, "cell.set", json!({"ref": "B2", "input": "10"}));
    common::exec_ok(bus, "cell.set", json!({"ref": "A3", "input": "West"}));
    common::exec_ok(bus, "cell.set", json!({"ref": "B3", "input": "70"}));
}

#[test]
fn pivot_create_refresh_remove_and_changeset() {
    let mut bus = analysis_bus();
    seed(&mut bus);
    let created = common::exec_ok(
        &mut bus,
        "pivot.create",
        json!({
            "source": "A1:B3",
            "dest": "E1",
            "name": "Sales",
            "rows": ["Region"],
            "data": [{"source": "Amount", "agg": "sum"}]
        }),
    );
    assert_eq!(created["name"], "Sales");
    assert_eq!(bus.workbook().pivots().len(), 1);
    let east = common::cell_value(&bus, 1, 5);
    assert!(matches!(east, Some(Value::Number(n)) if (n - 10.0).abs() < 1e-9));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B2", "input": "15"}));
    common::exec_ok(&mut bus, "pivot.refresh", json!({"name": "Sales"}));
    let east = common::cell_value(&bus, 1, 5);
    assert!(matches!(east, Some(Value::Number(n)) if (n - 15.0).abs() < 1e-9));
    common::exec_ok(&mut bus, "pivot.remove", json!({"name": "Sales"}));
    assert!(bus.workbook().pivots().is_empty());

    let mut bus = analysis_bus();
    seed(&mut bus);
    let changeset = bus
        .propose(
            Origin::User,
            vec![CommandCall {
                id: CommandId::new("pivot.create").unwrap(),
                args: json!({
                    "source": "A1:B3",
                    "dest": "E1",
                    "rows": ["Region"],
                    "data": [{"source": "Amount", "agg": "sum"}]
                }),
            }],
        )
        .unwrap();
    bus.apply(Origin::User, &changeset.id).unwrap();
    assert_eq!(bus.workbook().pivots().len(), 1);
    bus.revert(Origin::User, &changeset.id).unwrap();
    assert!(bus.workbook().pivots().is_empty());
}

#[test]
fn pivot_output_edit_is_refused() {
    let mut bus = analysis_bus();
    seed(&mut bus);
    common::exec_ok(
        &mut bus,
        "pivot.create",
        json!({
            "source": "A1:B3",
            "dest": "E1",
            "rows": ["Region"],
            "data": [{"source": "Amount", "agg": "sum"}]
        }),
    );
    let err = common::exec_err(&mut bus, "cell.set", json!({"ref": "F2", "input": "99"}));
    assert_eq!(err.code, "pivot.readonly");
    let err = common::exec_err(&mut bus, "format.bold", json!({"range": "F2"}));
    assert_eq!(err.code, "pivot.readonly");
}

#[test]
fn goal_seek_sets_input_and_reports_non_convergence() {
    let mut bus = analysis_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "=A1*2"}));
    let result = common::exec_ok(
        &mut bus,
        "whatif.goalseek",
        json!({"target": "B1", "goal": 10.0, "input": "A1"}),
    );
    assert_eq!(result["converged"], true);
    let input = common::cell_value(&bus, 0, 0);
    assert!(matches!(input, Some(Value::Number(n)) if (n - 5.0).abs() < 1e-4));
    common::exec_ok(&mut bus, "edit.undo", json!({}));
    let input = common::cell_value(&bus, 0, 0);
    assert!(matches!(input, Some(Value::Number(n)) if (n - 1.0).abs() < 1e-9));
    assert!(
        bus.workbook()
            .get(bus.workbook().active_sheet(), 0, 1)
            .unwrap()
            .unwrap()
            .formula
            .is_some()
    );

    let mut bus = analysis_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "B1", "input": "=5"}));
    let result = common::exec_ok(
        &mut bus,
        "whatif.goalseek",
        json!({"target": "B1", "goal": 10.0, "input": "A1"}),
    );
    assert_eq!(result["converged"], false);
}

#[test]
fn pivot_create_validates_fields_during_dry_run() {
    let mut bus = analysis_bus();
    seed(&mut bus);
    let dry = bus
        .dry_run(
            Origin::User,
            "pivot.create",
            json!({
                "source": "A1:B3",
                "dest": "E1",
                "rows": ["Missing"],
                "data": [{"source": "Amount", "agg": "sum"}]
            }),
        )
        .unwrap();
    assert!(!dry.outcome.ok);
    assert_eq!(
        dry.outcome.error.as_ref().map(|error| error.code.as_str()),
        Some("pivot.field")
    );
}

#[test]
fn stats_describe_returns_summary() {
    let mut bus = analysis_bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A2", "input": "2"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A3", "input": "3"}));
    let result = common::exec_ok(&mut bus, "stats.describe", json!({"range": "A1:A3"}));
    assert_eq!(result["count"], 3);
    assert!((result["mean"].as_f64().unwrap() - 2.0).abs() < 1e-12);
}

#[test]
fn command_schema_rejects_unknown_agg() {
    let mut bus = analysis_bus();
    let outcome = bus.execute(
        Origin::User,
        "pivot.create",
        json!({
            "source": "A1:B2",
            "dest": "E1",
            "data": [{"source": "Amount", "agg": "median"}]
        }),
    );
    assert!(!outcome.ok);
    assert_eq!(
        outcome.error.as_ref().map(|error| error.code.as_str()),
        Some(omacell_bus::codes::COMMAND_ARGS)
    );
}

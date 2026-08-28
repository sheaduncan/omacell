//! Automatic vs manual recalculation at the command boundary.

mod common;

use omacell_core::value::Value;
use serde_json::json;

#[test]
fn automatic_recalc_runs_once_after_edit() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "B1", "input": "=A1+10"}),
    );
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(11.0)));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "5"}));
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(15.0)));
}

#[test]
fn manual_recalc_is_noop_until_calc_recalc() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "calc.mode", json!({"mode": "manual"}));
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "1"}));
    common::exec_ok(
        &mut bus,
        "cell.set",
        json!({"ref": "B1", "input": "=A1+10"}),
    );
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Empty));
    common::exec_ok(&mut bus, "calc.recalc", json!({}));
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(11.0)));
}

#[test]
fn batch_apply_recalcs_once() {
    let mut bus = common::bus();
    let cs = bus
        .propose(
            omacell_core::command::Origin::User,
            vec![
                omacell_core::changeset::CommandCall {
                    id: omacell_core::command::CommandId::new("cell.set").unwrap(),
                    args: json!({"ref": "A1", "input": "3"}),
                },
                omacell_core::changeset::CommandCall {
                    id: omacell_core::command::CommandId::new("cell.set").unwrap(),
                    args: json!({"ref": "B1", "input": "=A1*2"}),
                },
            ],
        )
        .unwrap();
    bus.apply(omacell_core::command::Origin::User, &cs.id)
        .unwrap();
    assert_eq!(common::cell_value(&bus, 0, 1), Some(Value::Number(6.0)));
}

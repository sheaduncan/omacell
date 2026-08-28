//! Event ordering and backpressure.

mod common;

use omacell_core::event::Event;
use serde_json::json;

#[test]
fn cell_changed_is_sorted_then_recalc_done() {
    let mut bus = common::bus();
    let sub = bus.subscribe(32);
    common::exec_ok(
        &mut bus,
        "range.set",
        json!({"range": "B1:C1", "values": [["1", "2"]]}),
    );
    let events = bus.drain(sub);
    let cells: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::CellChanged { col, .. } => Some(*col),
            _ => None,
        })
        .collect();
    assert_eq!(cells, vec![1, 2]);
    assert!(
        events.iter().any(|e| matches!(e, Event::RecalcDone { .. })),
        "{events:?}"
    );
}

#[test]
fn stalled_subscriber_cannot_block_or_grow() {
    let mut bus = common::bus();
    let sub = bus.subscribe(2);
    for i in 0..8 {
        common::exec_ok(
            &mut bus,
            "cell.set",
            json!({"ref": "A1", "input": i.to_string()}),
        );
    }
    assert!(bus.dropped(sub) > 0);
    let queued = bus.drain(sub);
    assert!(queued.len() <= 2);
}

#[test]
fn manual_mode_does_not_emit_recalc_on_edit() {
    let mut bus = common::bus();
    common::exec_ok(&mut bus, "calc.mode", json!({"mode": "manual"}));
    let sub = bus.subscribe(16);
    common::exec_ok(&mut bus, "cell.set", json!({"ref": "A1", "input": "=1+1"}));
    let events = bus.drain(sub);
    assert!(
        !events.iter().any(|e| matches!(e, Event::RecalcDone { .. })),
        "{events:?}"
    );
}

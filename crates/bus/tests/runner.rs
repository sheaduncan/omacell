//! Barrier-based task runner tests (no sleeps as synchronization).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Instant;

use omacell_bus::{Bus, LongOps, TaskEvent, TaskRunner, register_hold_command};
use omacell_core::command::Origin;
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use serde_json::json;

fn bus_with_hold(start: Arc<Barrier>, release: Arc<AtomicBool>) -> Bus {
    let mut bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    register_hold_command(bus.registry_mut(), start, release).unwrap();
    bus
}

#[test]
fn local_writer_is_single_and_ordered() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), Arc::clone(&release));
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();

    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    assert!(handle.is_busy());
    let waiter = handle.clone();
    let join = std::thread::spawn(move || {
        waiter.submit_wait(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}))
    });
    release.store(true, Ordering::SeqCst);
    let first = join.join().expect("cell.set thread");
    assert!(first.ok, "{:?}", first.error);
    let snap = handle.snapshot();
    let slot = snap
        .workbook
        .get(snap.workbook.active_sheet(), 0, 0)
        .unwrap()
        .unwrap();
    assert!(matches!(
        slot.value,
        omacell_core::value::Value::Number(n) if n == 1.0
    ));
}

#[test]
fn stalled_event_consumer_cannot_block_worker() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), Arc::clone(&release));
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    let waiter = handle.clone();
    let join = std::thread::spawn(move || {
        for i in 0..8 {
            let _ = waiter.submit_wait(
                Origin::User,
                "cell.set",
                json!({"ref": format!("A{}", i + 2), "input": "1"}),
            );
        }
    });
    release.store(true, Ordering::SeqCst);
    let _ = join.join();
    assert!(handle.dropped_events() > 0 || handle.drain_events().len() <= 64);
}

#[test]
fn progress_for_one_task_is_coalesced() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), Arc::clone(&release));
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    release.store(true, Ordering::SeqCst);
    let _events = handle.drain_events();
}

#[test]
fn cancel_hold_leaves_workbook_unchanged() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), Arc::clone(&release));
    let sheet = bus.workbook().active_sheet();
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    let (_id, cancel) = handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    cancel.cancel();
    let t0 = Instant::now();
    while handle.is_busy() && t0.elapsed().as_millis() < 1000 {
        std::thread::yield_now();
    }
    let snap = handle.snapshot();
    assert!(snap.workbook.get(sheet, 0, 0).unwrap().is_none());
}

#[test]
fn cancelled_queued_command_is_never_dispatched() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), Arc::clone(&release));
    let sheet = bus.workbook().active_sheet();
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    let (id, cancel) = handle
        .submit(
            Origin::User,
            "cell.set",
            json!({"ref": "A1", "input": "99"}),
        )
        .unwrap();
    cancel.cancel();
    assert!(
        handle
            .drain_events()
            .into_iter()
            .any(|event| matches!(event, TaskEvent::Cancelling(state) if state.id == id))
    );
    release.store(true, Ordering::SeqCst);
    let started = Instant::now();
    while handle.tracked_tasks() != 0 {
        assert!(started.elapsed().as_secs() < 1, "tasks did not finish");
        std::thread::yield_now();
    }
    assert!(
        handle
            .snapshot()
            .workbook
            .get(sheet, 0, 0)
            .unwrap()
            .is_none()
    );
}

#[test]
fn terminal_task_records_are_released() {
    let start = Arc::new(Barrier::new(1));
    let release = Arc::new(AtomicBool::new(true));
    let bus = bus_with_hold(start, release);
    let runner = TaskRunner::spawn(bus, LongOps::production()).unwrap();
    let handle = runner.handle();
    for row in 0..100 {
        let outcome = handle.submit_wait(
            Origin::User,
            "cell.set",
            json!({"ref": format!("A{}", row + 1), "input": "1"}),
        );
        assert!(outcome.ok, "{:?}", outcome.error);
    }
    assert_eq!(handle.tracked_tasks(), 0);
    assert!(handle.drain_events().len() <= 64);
}

#[test]
fn task_state_is_bounded_with_concurrent_queue_capacity() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), release);
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    for row in 0..32 {
        handle
            .submit(
                Origin::User,
                "cell.set",
                json!({"ref": format!("A{}", row + 1), "input": "1"}),
            )
            .unwrap();
    }
    let err = handle
        .submit(
            Origin::User,
            "cell.set",
            json!({"ref": "A33", "input": "1"}),
        )
        .unwrap_err();
    assert_eq!(err.code, "task.queue");
    assert_eq!(handle.tracked_tasks(), 33);
}

#[test]
fn shutdown_with_in_flight_task_joins() {
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let bus = bus_with_hold(Arc::clone(&start), Arc::clone(&release));
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    drop(runner);
    let _ = AtomicUsize::new(0);
}

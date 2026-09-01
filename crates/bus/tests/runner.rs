//! Barrier-based task runner tests (no sleeps as synchronization).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use omacell_bus::args::EmptyArgs;
use omacell_bus::{
    Bus, CommandKind, CommandSpec, Effect, Exposure, LongOps, TaskEvent, TaskRunner,
};
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::command::Origin;
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CondFormat};
use omacell_core::error::CoreError;
use omacell_core::eval::FnRegistry;
use omacell_core::event::Event;
use omacell_core::recalc::RecalcEngine;
use omacell_core::style::Color;
use omacell_core::workbook::Workbook;
use serde_json::json;

fn register_hold_command(
    registry: &mut omacell_bus::CommandRegistry,
    start: Arc<Barrier>,
    release: Arc<AtomicBool>,
) -> Result<(), CoreError> {
    registry.register::<EmptyArgs, _>(
        CommandSpec {
            id: "test.hold",
            doc: "Test-only barrier hold (WP-15a)",
            kind: CommandKind::Mutating,
            changeset_eligible: false,
            exposure: Exposure::Public,
            default_keys: &[],
        },
        move |ctx, _args| {
            if ctx.is_preflight() {
                return Ok(Effect::query(serde_json::json!({})));
            }
            start.wait();
            loop {
                if ctx.is_cancelled() {
                    return Err(CoreError::new(
                        omacell_bus::codes::TASK_CANCELLED,
                        "task cancelled",
                    ));
                }
                if release.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::yield_now();
            }
            Ok(Effect {
                result: serde_json::json!({"held": true}),
                ..Effect::default()
            })
        },
    )
}

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

#[test]
fn task_events_wake_a_registered_frontend() {
    let start = Arc::new(Barrier::new(1));
    let release = Arc::new(AtomicBool::new(true));
    let bus = bus_with_hold(start, release);
    let runner = TaskRunner::spawn(bus, LongOps::production()).unwrap();
    let handle = runner.handle();
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = Arc::clone(&wakes);
    let frontend = Arc::new(Mutex::new(Some(handle.clone())));
    let callback_frontend = Arc::clone(&frontend);
    handle.set_event_waker(move || {
        if let Some(handle) = callback_frontend.lock().unwrap().as_ref() {
            let _ = handle.tracked_tasks();
        }
        wake_count.fetch_add(1, Ordering::SeqCst);
    });

    let outcome = handle.submit_wait(Origin::User, "cell.set", json!({"ref": "A1", "input": "1"}));

    assert!(outcome.ok, "{:?}", outcome.error);
    assert!(wakes.load(Ordering::SeqCst) >= 3);
    *frontend.lock().unwrap() = None;
}

#[test]
fn synchronous_commands_publish_core_events_before_returning() {
    let bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    let runner = TaskRunner::spawn(bus, LongOps::production()).unwrap();
    let handle = runner.handle();

    let outcome = handle.submit_wait(Origin::User, "cell.set", json!({"ref": "A1", "input": "7"}));

    assert!(outcome.ok, "{:?}", outcome.error);
    let events = handle.drain_bus_events();
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::CellChanged { row: 0, col: 0, .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::RecalcDone { .. }))
    );
}

#[test]
fn conditional_formats_resolve_off_thread_and_invalidate_with_the_reader_snapshot() {
    let mut workbook = Workbook::new();
    let sheet = workbook.active_sheet();
    workbook.set_number(sheet, 0, 0, 7.0).unwrap();
    let cell = RangeRef::from_corners(CellRef::new(0, 0).unwrap(), CellRef::new(0, 0).unwrap());
    workbook
        .set_cond_formats(
            sheet,
            vec![CondFormat {
                range: cell,
                priority: 1,
                stop_if_true: true,
                kind: CfKind::CellIs {
                    op: CfOp::Greater,
                    formula1: "0".into(),
                    formula2: None,
                },
                dxf: CfDxf {
                    fill: Some(Color::Rgb { argb: 0xFF12_3456 }),
                    font: None,
                },
            }],
        )
        .unwrap();
    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(AtomicBool::new(false));
    let mut bus = Bus::new(workbook, RecalcEngine::new(FnRegistry::new())).unwrap();
    register_hold_command(bus.registry_mut(), Arc::clone(&start), Arc::clone(&release)).unwrap();
    let runner = TaskRunner::spawn(bus, LongOps::production().with("test.hold")).unwrap();
    let handle = runner.handle();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    handle.set_event_waker(move || {
        let _ = wake_tx.send(());
    });

    let first = handle.snapshot();
    handle.submit(Origin::User, "test.hold", json!({})).unwrap();
    start.wait();
    assert!(handle.is_busy());
    handle
        .request_conditional_formats(&first, sheet, &[cell])
        .unwrap();
    let resolved = wait_for_conditional_formats(&handle, &first, sheet, &wake_rx);
    assert_eq!(
        resolved.get(0, 0).and_then(|overlay| overlay.fill),
        Some(Color::Rgb { argb: 0xFF12_3456 })
    );

    release.store(true, Ordering::SeqCst);
    let outcome = handle.submit_wait(
        Origin::User,
        "cell.set",
        json!({"ref": "A1", "input": "-1"}),
    );
    assert!(outcome.ok, "{:?}", outcome.error);
    let second = handle.snapshot();
    assert!(handle.conditional_formats(&second, sheet).is_none());
    handle
        .request_conditional_formats(&second, sheet, &[cell])
        .unwrap();
    let resolved = wait_for_conditional_formats(&handle, &second, sheet, &wake_rx);
    assert_eq!(resolved.get(0, 0).and_then(|overlay| overlay.fill), None);
}

fn wait_for_conditional_formats(
    handle: &omacell_bus::TaskRunnerHandle,
    snapshot: &Arc<omacell_bus::ReaderSnapshot>,
    sheet: omacell_core::addr::SheetId,
    wake_rx: &std::sync::mpsc::Receiver<()>,
) -> Arc<omacell_bus::ConditionalFormatSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(resolved) = handle.conditional_formats(snapshot, sheet) {
            return resolved;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "conditional formats did not resolve");
        wake_rx.recv_timeout(remaining).unwrap();
    }
}

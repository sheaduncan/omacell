//! Progressive load, cancellation, and formula-injection guard.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use omacell_core::value::Value;
use omacell_io::csv::{LoadOptions, LoadProgress, load, sniff};
use omacell_io::error::codes;

#[test]
fn progress_events_fire() {
    let mut csv = String::new();
    for i in 0..25 {
        csv.push_str(&format!("{i},x\n"));
    }
    let sniff = sniff(csv.as_bytes()).unwrap();
    let rows = Arc::new(AtomicU64::new(0));
    let rows2 = Arc::clone(&rows);
    let done = Arc::new(AtomicBool::new(false));
    let done2 = Arc::clone(&done);
    let opts = LoadOptions {
        progress_every: 10,
        on_progress: Some(Arc::new(move |p: LoadProgress| {
            rows2.store(p.rows_loaded, Ordering::Relaxed);
            if p.done {
                done2.store(true, Ordering::Relaxed);
            }
        })),
        ..Default::default()
    };
    let (_wb, result) = load(csv.as_bytes(), &sniff.plan, opts).unwrap();
    assert_eq!(result.rows_written, 25);
    assert!(done.load(Ordering::Relaxed));
    assert_eq!(rows.load(Ordering::Relaxed), 25);
}

#[test]
fn cancel_stops_and_keeps_partial() {
    let mut csv = String::new();
    for i in 0..100 {
        csv.push_str(&format!("{i}\n"));
    }
    let sniff = sniff(csv.as_bytes()).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel2 = Arc::clone(&cancel);
    let opts = LoadOptions {
        progress_every: 1,
        cancel: Some(Arc::clone(&cancel)),
        on_progress: Some(Arc::new(move |p: LoadProgress| {
            if p.rows_loaded >= 5 {
                cancel2.store(true, Ordering::Relaxed);
            }
        })),
        ..Default::default()
    };
    let err = load(csv.as_bytes(), &sniff.plan, opts).unwrap_err();
    assert_eq!(err.code, codes::CSV_CANCELLED);
}

#[test]
fn leading_equals_stays_text() {
    let bytes = b"=1+1,2\n";
    let sniff = sniff(bytes).unwrap();
    let (wb, _) = load(bytes, &sniff.plan, Default::default()).unwrap();
    let slot = wb.get(wb.active_sheet(), 0, 0).unwrap().unwrap();
    assert!(slot.formula.is_none());
    match slot.value {
        Value::Text(id) => assert_eq!(wb.intern().strings.get(id), Some("=1+1")),
        other => panic!("{other:?}"),
    }
}

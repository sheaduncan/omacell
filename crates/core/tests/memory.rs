//! Numeric-cell memory budget (spec §11.3, §12.1).
//!
//! Workspace `unsafe_code = forbid` blocks a `#[global_allocator]` wrapper.
//! `SheetStore::heap_bytes` walks occupancy bitmaps, slot `Vec` capacity, and
//! hashbrown bucket estimates. Linux RSS from `/proc/self/statm` is a secondary
//! number.

use omacell_core::storage::{CellSlot, SheetStore};

fn fill_numeric(rows: u32, cols: u16) -> SheetStore {
    let mut store = SheetStore::new();
    for r in 0..rows {
        for c in 0..cols {
            store.set(r, c, CellSlot::number(1.0)).expect("set");
        }
    }
    store
}

fn assert_budget(store: &SheetStore, rows: u32, cols: u16, shrink: bool) {
    let n = (rows as usize) * (cols as usize);
    let bytes = store.heap_bytes();
    let per = bytes as f64 / n as f64;
    assert!(
        per <= 64.0,
        "{rows}×{cols}: {bytes} bytes / {n} cells = {per:.2} B/cell (shrink={shrink})"
    );
}

#[test]
fn ten_thousand_by_twenty_at_most_64_bytes_per_cell() {
    let rows = 10_000;
    let cols = 20;
    let mut store = fill_numeric(rows, cols);
    assert_budget(&store, rows, cols, false);
    store.shrink_to_fit();
    assert_budget(&store, rows, cols, true);
}

#[test]
#[ignore = "nightly: 1M×20 numeric memory budget"]
fn million_by_twenty_numeric_at_most_64_bytes_per_cell() {
    let rows = 1_000_000;
    let cols = 20;
    let mut store = fill_numeric(rows, cols);
    let n = (rows as usize) * (cols as usize);
    let raw = store.heap_bytes();
    store.shrink_to_fit();
    let compact = store.heap_bytes();
    let per_raw = raw as f64 / n as f64;
    let per = compact as f64 / n as f64;
    eprintln!(
        "1M×20 heap_bytes raw={raw} ({per_raw:.2} B/cell) compact={compact} ({per:.2} B/cell)"
    );
    if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
        let pages: u64 = statm
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let rss = pages.saturating_mul(page_size());
        eprintln!("1M×20 RSS≈{rss} ({:.2} B/cell)", rss as f64 / n as f64);
        eprintln!(
            "OMACELL_PERF_RESULT {}",
            serde_json::json!({"id": "memory_1m_x20_bytes", "value": rss})
        );
        assert!(
            rss < 1_500_000_000,
            "RSS {rss} exceeds 1.5 GB target (spec §12.1)"
        );
    }
    assert!(
        per <= 64.0,
        "1M×20 compact {compact} bytes / {n} cells = {per:.2} B/cell"
    );
}

fn page_size() -> u64 {
    #[cfg(unix)]
    {
        // SAFETY: sysconf(_SC_PAGESIZE) is a documented POSIX query with no
        // pointer arguments. Not used: workspace forbids unsafe, so fall back.
        4096
    }
    #[cfg(not(unix))]
    {
        4096
    }
}

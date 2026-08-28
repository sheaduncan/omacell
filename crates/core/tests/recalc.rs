//! Recalc engine: cycles, determinism, incremental dirty, async stale.

use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::{MockAsyncProvider, RecalcEngine, format_cell};
use omacell_core::value::Value;
use omacell_core::workbook::Workbook;
use std::sync::Arc;

fn display(wb: &Workbook, row: u32, col: u16) -> String {
    format_cell(wb, wb.active_sheet(), row, col)
}

#[test]
fn cycle_never_hangs_and_reports_set() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=A1+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    let r = eng.recalc_full(&mut wb);
    assert_eq!(r.circular, vec![CellCoord::new(s, 0, 0)]);
    assert_eq!(display(&wb, 0, 0), "0");
}

#[test]
fn incremental_dirties_dependents_only() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(s, 0, 0, 1.0).unwrap();
    wb.set_formula_text(s, 1, 0, "=A1+1").unwrap();
    wb.set_formula_text(s, 2, 0, "=A2+1").unwrap();
    wb.set_formula_text(s, 0, 1, "=1+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    let full = eng.recalc_full(&mut wb);
    assert!(full.cells_evaluated >= 3);
    wb.set_number(s, 0, 0, 5.0).unwrap();
    eng.notify_edit(&wb, CellCoord::new(s, 0, 0));
    let inc = eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 1, 0), "6");
    assert_eq!(display(&wb, 2, 0), "7");
    assert_eq!(display(&wb, 0, 1), "2");
    assert!(
        inc.cells_evaluated <= full.cells_evaluated,
        "incremental {} vs full {}",
        inc.cells_evaluated,
        full.cells_evaluated
    );
}

fn snapshot_values(wb: &Workbook) -> Vec<(u32, u16, String)> {
    let s = wb.active_sheet();
    let mut out = Vec::new();
    if let Ok(Some(ur)) = wb.used_range(s) {
        for row in ur.min_row..=ur.max_row {
            for col in ur.min_col..=ur.max_col {
                out.push((row, col, format_cell(wb, s, row, col)));
            }
        }
    }
    out
}

fn build_grid(n: usize) -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    wb.set_number(s, 0, 0, 1.0).unwrap();
    for i in 1..n {
        let row = (i as u32) / 16;
        let col = (i as u16) % 16;
        if row == 0 && col == 0 {
            continue;
        }
        // Mix of independent arithmetic and a shared precedent so 1- vs 8-thread
        // scheduling would diverge if results were racy.
        let src = if i % 3 == 0 { "=A1+1" } else { "=1+2*3" };
        wb.set_formula_text(s, row, col, src).unwrap();
    }
    wb
}

#[test]
fn determinism_2k_one_vs_eight_threads() {
    let n = 2_000;
    let mut a = build_grid(n);
    let mut b = build_grid(n);
    let mut e1 = RecalcEngine::new(FnRegistry::new());
    e1.set_threads(1);
    e1.recalc_full(&mut a);
    let mut e8 = RecalcEngine::new(FnRegistry::new());
    e8.set_threads(8);
    e8.recalc_full(&mut b);
    assert_eq!(snapshot_values(&a), snapshot_values(&b));
}

#[test]
#[ignore = "nightly / release: 200k-formula determinism (WP-04)"]
fn determinism_200k_one_vs_eight_threads() {
    let n = 200_000;
    let mut a = build_grid(n);
    let mut b = build_grid(n);
    let mut e1 = RecalcEngine::new(FnRegistry::new());
    e1.set_threads(1);
    e1.recalc_full(&mut a);
    let mut e8 = RecalcEngine::new(FnRegistry::new());
    e8.set_threads(8);
    e8.recalc_full(&mut b);
    assert_eq!(snapshot_values(&a), snapshot_values(&b));
}

#[test]
fn async_pending_then_ready() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    let mut reg = FnRegistry::new();
    reg.register(omacell_core::eval::FnDef {
        name: "AI",
        min_args: 1,
        max_args: 8,
        volatile: false,
        async_node: true,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: |_, _| omacell_core::eval::RuntimeValue::error(omacell_core::error::ErrorKind::Na),
    });
    wb.set_formula_text(s, 0, 0, "=AI(\"x\")").unwrap();
    wb.set_formula_text(s, 1, 0, "=A1").unwrap();
    let mut eng = RecalcEngine::new(reg);
    eng.set_async_provider(Arc::new(MockAsyncProvider::new(Value::Number(7.0))));
    let r1 = eng.recalc_full(&mut wb);
    assert!(!r1.pending_async.is_empty() || !r1.stale.is_empty());
    let r2 = eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "7");
    assert_eq!(display(&wb, 1, 0), "7");
    assert!(r2.pending_async.is_empty());
}

#[test]
fn whole_column_is_one_bucket() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_number(s, 0, 0, 1.0).unwrap();
    // SUM is not registered; the cell is `#NAME?` but the graph still records A:A.
    wb.set_formula_text(s, 0, 1, "=SUM(A:A)").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.rebuild(&wb);
    let precs = eng.graph().precedents(CellCoord::new(s, 0, 1));
    assert!(
        precs.iter().any(|p| matches!(
            p,
            omacell_core::graph::Precedent::Range {
                whole_col: true,
                ..
            }
        )),
        "expected whole-column bucket, got {precs:?}"
    );
}

#[test]
fn fn_registry_lookup_is_case_insensitive() {
    let mut r = FnRegistry::new();
    r.register(omacell_core::eval::FnDef {
        name: "SUM",
        min_args: 1,
        max_args: 255,
        volatile: false,
        async_node: false,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: |_, _| omacell_core::eval::RuntimeValue::error(omacell_core::error::ErrorKind::Value),
    });
    assert!(r.lookup("sum").is_some());
    assert!(r.lookup("Sum").is_some());
    assert!(r.lookup("NOPE").is_none());
}

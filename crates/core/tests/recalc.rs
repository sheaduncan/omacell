//! Recalc engine: cycles, determinism, incremental dirty, async stale.

use omacell_core::eval::FnRegistry;
use omacell_core::graph::CellCoord;
use omacell_core::recalc::{MockAsyncProvider, RecalcEngine, format_cell};
use omacell_core::storage::CellFlags;
use omacell_core::style::Style;
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
fn circular_set_excludes_downstream_cells() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=B1+1").unwrap();
    wb.set_formula_text(s, 0, 1, "=A1+1").unwrap();
    wb.set_formula_text(s, 0, 2, "=A1+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    let r = eng.recalc_full(&mut wb);
    assert_eq!(
        r.circular,
        vec![CellCoord::new(s, 0, 0), CellCoord::new(s, 0, 1)]
    );
    assert_eq!(display(&wb, 0, 2), "1");
}

#[test]
fn iterative_cycle_recalculates_its_dependents_after_convergence() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.settings_mut().iteration.enabled = true;
    wb.settings_mut().iteration.max_iterations = 100;
    wb.settings_mut().iteration.max_change = 0.0;
    wb.set_formula_text(s, 0, 0, "=(A1+2)/2").unwrap();
    wb.set_formula_text(s, 0, 1, "=A1+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    let r = eng.recalc_full(&mut wb);
    assert!(r.circular.is_empty());
    assert_eq!(display(&wb, 0, 0), "2");
    assert_eq!(display(&wb, 0, 1), "3");
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
    wb.set_formula_text(s, 0, 0, "=AI(\"x\")+1").unwrap();
    wb.set_formula_text(s, 1, 0, "=A1").unwrap();
    let mut eng = RecalcEngine::new(reg);
    eng.set_async_provider(Arc::new(MockAsyncProvider::new(Value::Number(7.0))));
    let r1 = eng.recalc_full(&mut wb);
    assert_eq!(r1.pending_async, vec![CellCoord::new(s, 0, 0)]);
    assert_eq!(
        r1.stale,
        vec![CellCoord::new(s, 0, 0), CellCoord::new(s, 1, 0)]
    );
    assert!(wb.get(s, 0, 0).unwrap().unwrap().flags.stale());
    assert!(wb.get(s, 1, 0).unwrap().unwrap().flags.stale());

    eng.notify_async_ready(CellCoord::new(s, 0, 0));
    let r2 = eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 0, 0), "8");
    assert_eq!(display(&wb, 1, 0), "8");
    assert!(r2.pending_async.is_empty());
    assert!(!wb.get(s, 0, 0).unwrap().unwrap().flags.stale());
    assert!(!wb.get(s, 1, 0).unwrap().unwrap().flags.stale());
}

#[test]
fn recalc_preserves_cell_protection_flags() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=1+1").unwrap();
    let mut slot = *wb.get(s, 0, 0).unwrap().unwrap();
    slot.flags = CellFlags::empty().with(CellFlags::HIDDEN, true);
    wb.set_slot(s, 0, 0, slot).unwrap();

    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    let flags = wb.get(s, 0, 0).unwrap().unwrap().flags;
    assert!(!flags.locked());
    assert!(flags.hidden());
}

#[test]
fn direct_reference_to_a_spill_ghost_sees_the_new_value() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "={1;2}").unwrap();
    wb.set_formula_text(s, 0, 1, "=A2+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 1), "3");
}

#[test]
fn full_rebuild_clears_ghosts_when_a_spill_shrinks() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "={1,2,3}").unwrap();
    wb.set_formula_text(s, 0, 3, "=C1+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 2), "3");
    assert_eq!(display(&wb, 0, 3), "4");

    wb.set_formula_text(s, 0, 0, "={4,5}").unwrap();
    eng.notify_edit(&wb, CellCoord::new(s, 0, 0));
    eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 0, 0), "4");
    assert_eq!(display(&wb, 0, 1), "5");
    assert_eq!(display(&wb, 0, 2), "");
    assert_eq!(display(&wb, 0, 3), "1");
}

#[test]
fn deleting_a_spill_origin_clears_its_ghosts() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "={1,2}").unwrap();
    wb.set_formula_text(s, 0, 2, "=B1+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 2), "3");
    wb.clear_cell(s, 0, 0).unwrap();
    eng.notify_edit(&wb, CellCoord::new(s, 0, 0));
    eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 0, 0), "");
    assert_eq!(display(&wb, 0, 1), "");
    assert_eq!(display(&wb, 0, 2), "1");
}

#[test]
fn editing_a_spill_ghost_redirties_and_blocks_its_origin() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "={1;2;3}").unwrap();
    wb.set_formula_text(s, 0, 1, "=A3+1").unwrap();
    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 1), "4");
    wb.set_number(s, 1, 0, 9.0).unwrap();
    eng.notify_edit(&wb, CellCoord::new(s, 1, 0));
    eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 0, 0), "#SPILL!");
    assert_eq!(display(&wb, 1, 0), "9");
    assert_eq!(display(&wb, 2, 0), "");
    assert_eq!(display(&wb, 0, 1), "1");
}

#[test]
fn clearing_a_spill_restores_ghost_style_and_protection() {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    let mut style = Style::default();
    style.font.bold = true;
    let style_id = wb.set_cell_style(s, 0, 1, style).unwrap();
    let mut ghost_slot = *wb.get(s, 0, 1).unwrap().unwrap();
    ghost_slot.flags = CellFlags::empty().with(CellFlags::HIDDEN, true);
    wb.set_slot(s, 0, 1, ghost_slot).unwrap();
    wb.set_formula_text(s, 0, 0, "={1,2}").unwrap();

    let mut eng = RecalcEngine::new(FnRegistry::new());
    eng.recalc_full(&mut wb);
    wb.clear_cell(s, 0, 0).unwrap();
    eng.notify_edit(&wb, CellCoord::new(s, 0, 0));
    eng.recalc_incremental(&mut wb);

    let restored = wb.get(s, 0, 1).unwrap().unwrap();
    assert_eq!(restored.value, Value::Empty);
    assert_eq!(restored.style, style_id);
    assert!(!restored.flags.locked());
    assert!(restored.flags.hidden());
    assert!(!restored.flags.spill());
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

#[test]
fn registry_volatile_metadata_redirties_the_formula_each_pass() {
    fn tick(
        ctx: &mut omacell_core::eval::EvalCtx<'_>,
        _args: &[omacell_core::eval::ArgVal],
    ) -> omacell_core::eval::RuntimeValue {
        omacell_core::eval::RuntimeValue::Scalar(omacell_core::coerce::Scalar::Number(f64::from(
            ctx.pass(),
        )))
    }

    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.set_formula_text(s, 0, 0, "=TICK()").unwrap();
    let mut registry = FnRegistry::new();
    registry.register(omacell_core::eval::FnDef {
        name: "TICK",
        min_args: 0,
        max_args: 0,
        volatile: true,
        async_node: false,
        array_lift: omacell_core::eval::ArrayLift::None,
        eval: tick,
    });
    let mut eng = RecalcEngine::new(registry);
    eng.recalc_full(&mut wb);
    assert_eq!(display(&wb, 0, 0), "1");
    eng.recalc_incremental(&mut wb);
    assert_eq!(display(&wb, 0, 0), "2");
}

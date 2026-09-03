//! Criterion benches for incremental and full recalculation (WP-04, §12.1).

use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::eval::{AstCache, FnRegistry};
use omacell_core::graph::{CellCoord, DepGraph};
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;

fn build_independent(n: usize) -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for i in 0..n {
        let row = (i as u32) / 20;
        let col = (i as u16) % 20;
        let _ = wb.set_formula_text(sheet, row, col, "=1+1");
    }
    wb
}

/// `A1` is a number; every other cell is `=A1+1` so one edit dirties `n-1` formulas.
fn build_star(n: usize) -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    let _ = wb.set_number(sheet, 0, 0, 1.0);
    for i in 1..n {
        let row = (i as u32) / 16;
        let col = (i as u16) % 16;
        if row == 0 && col == 0 {
            continue;
        }
        let _ = wb.set_formula_text(sheet, row, col, "=A1+1");
    }
    wb
}

fn build_disjoint_ranges(n: usize) -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for row in 0..=n as u32 {
        let _ = wb.set_number(sheet, row, 2, f64::from(row));
    }
    for row in 0..n as u32 {
        let excel_row = row + 1;
        let next_excel_row = excel_row + 1;
        let _ = wb.set_formula_text(sheet, row, 1, &format!("=C{excel_row}:C{next_excel_row}"));
    }
    wb
}

fn bench_recalc(c: &mut Criterion) {
    let mut group = c.benchmark_group("recalc");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(8));

    group.bench_function("full_10k_independent", |b| {
        let mut wb = build_independent(10_000);
        let mut engine = RecalcEngine::new(FnRegistry::new());
        engine.set_threads(8);
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.bench_function("incremental_100k_typical_one_cell", |b| {
        let mut wb = build_independent(100_000);
        let mut engine = RecalcEngine::new(FnRegistry::new());
        engine.set_threads(8);
        engine.recalc_full(&mut wb);
        let sheet = wb.active_sheet();
        b.iter(|| {
            let _ = wb.set_formula_text(sheet, 1, 1, "=1+1");
            engine.notify_edit(&wb, CellCoord::new(sheet, 1, 1));
            black_box(engine.recalc_incremental(&mut wb));
        });
    });

    group.bench_function("incremental_100k_one_edit", |b| {
        let mut wb = build_star(100_000);
        let mut engine = RecalcEngine::new(FnRegistry::new());
        engine.set_threads(8);
        engine.recalc_full(&mut wb);
        let sheet = wb.active_sheet();
        let mut n = 2.0;
        b.iter(|| {
            n += 1.0;
            let _ = wb.set_number(sheet, 0, 0, n);
            engine.notify_edit(&wb, CellCoord::new(sheet, 0, 0));
            black_box(engine.recalc_incremental(&mut wb));
        });
    });

    group.bench_function("generations_5k_disjoint_ranges", |b| {
        let wb = build_disjoint_ranges(5_000);
        let mut graph = DepGraph::new();
        graph.rebuild(&wb, &mut AstCache::new());
        let cells = graph.formula_cells();
        b.iter(|| {
            black_box(graph.generations(&cells));
        });
    });

    group.bench_function("full_1m_independent_8t", |b| {
        let mut wb = build_independent(1_000_000);
        let mut engine = RecalcEngine::new(FnRegistry::new());
        engine.set_threads(8);
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_recalc);
criterion_main!(benches);

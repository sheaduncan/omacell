//! Whole-column `SUM`, `SUMIFS`, and `SUBTOTAL` baselines (WP-05a).

use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;

fn filled_column(n: u32) -> Workbook {
    let mut wb = Workbook::new();
    let sheet = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for i in 0..n {
        let _ = wb.set_number(sheet, i, 0, 1.0);
        let _ = wb.set_number(sheet, i, 1, if i % 2 == 0 { 1.0 } else { 0.0 });
    }
    wb
}

fn registry() -> FnRegistry {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    registry
}

fn bench_aggregates(c: &mut Criterion) {
    let mut group = c.benchmark_group("fn_aggregates");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("whole_column_sum", |b| {
        let mut wb = filled_column(10_000);
        let sheet = wb.active_sheet();
        let _ = wb.set_formula_text(sheet, 0, 2, "=SUM(A:A)");
        let mut engine = RecalcEngine::new(registry());
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.bench_function("whole_column_sumifs", |b| {
        let mut wb = filled_column(10_000);
        let sheet = wb.active_sheet();
        let _ = wb.set_formula_text(sheet, 0, 2, "=SUMIFS(A:A,B:B,1)");
        let mut engine = RecalcEngine::new(registry());
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.bench_function("whole_column_subtotal", |b| {
        let mut wb = filled_column(10_000);
        let sheet = wb.active_sheet();
        let _ = wb.set_formula_text(sheet, 0, 2, "=SUBTOTAL(9,A:A)");
        let mut engine = RecalcEngine::new(registry());
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_aggregates);
criterion_main!(benches);

//! 1M-row lookup/array and representative solver baselines (WP-05c).

use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;

fn registry() -> FnRegistry {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    registry
}

fn fill_column(wb: &mut Workbook, col: u16, n: u32, f: impl Fn(u32) -> f64) {
    let sheet = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for i in 0..n {
        let _ = wb.set_number(sheet, i, col, f(i));
    }
}

fn bench_formula(
    c: &mut Criterion,
    id: &str,
    n: u32,
    formula: &str,
    extra: impl Fn(&mut Workbook),
) {
    let mut group = c.benchmark_group("wp05c");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(8));
    group.bench_function(id, |b| {
        let mut wb = Workbook::new();
        fill_column(&mut wb, 0, n, |i| f64::from(i + 1));
        extra(&mut wb);
        let sheet = wb.active_sheet();
        // Place the formula on column Z so spills do not collide with inputs.
        let _ = wb.set_formula_text(sheet, 0, 25, formula);
        let mut engine = RecalcEngine::new(registry());
        engine.set_threads(8);
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });
    group.finish();
}

fn run_benches(c: &mut Criterion) {
    bench_formula(
        c,
        "xlookup_1m",
        1_000_000,
        "=XLOOKUP(1000000,A1:A1000000,A1:A1000000)",
        |_| {},
    );
    bench_formula(
        c,
        "xmatch_1m",
        1_000_000,
        "=XMATCH(1000000,A1:A1000000)",
        |_| {},
    );
    bench_formula(
        c,
        "filter_1m",
        1_000_000,
        "=FILTER(A1:A1000000,A1:A1000000>999000)",
        |_| {},
    );
    bench_formula(c, "sort_1m", 1_000_000, "=SORT(A1:A1000000)", |_| {});
    bench_formula(
        c,
        "unique_1m",
        1_000_000,
        "=ROWS(UNIQUE(A1:A1000000))",
        |_| {},
    );
    bench_formula(
        c,
        "map_10k",
        10_000,
        "=MAP(A1:A10000,LAMBDA(x,x*2))",
        |_| {},
    );
    bench_formula(c, "irr_solver", 8, "=IRR({-100,10,20,30,40,50,60})", |_| {});
    bench_formula(c, "rate_solver", 1, "=RATE(12,-100,1000)", |_| {});
}

criterion_group!(benches, run_benches);
criterion_main!(benches);

//! TEXTSPLIT, regex, and 100k-row text/date scan baselines (WP-05b).

use std::time::Duration;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;
use omacell_fn::register_all;

fn registry() -> FnRegistry {
    let mut registry = FnRegistry::new();
    register_all(&mut registry);
    registry
}

fn bench_text_date(c: &mut Criterion) {
    let mut group = c.benchmark_group("text_date");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("textsplit_line", |b| {
        let mut registry = registry();
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.undo_log_mut().set_enabled(false);
        let line = format!(
            r#"=TEXTSPLIT("{}", ",")"#,
            (0..64)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let _ = wb.set_formula_text(sheet, 0, 0, &line);
        let mut engine = RecalcEngine::new(std::mem::take(&mut registry));
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.bench_function("regex_1k", |b| {
        let mut registry = registry();
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.undo_log_mut().set_enabled(false);
        for i in 0..1_000u32 {
            let _ = wb.set_formula_text(sheet, i, 0, r#"=REGEXTEST("abc123xyz", "[0-9]+")"#);
        }
        let mut engine = RecalcEngine::new(std::mem::take(&mut registry));
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.bench_function("len_scan_100k", |b| {
        let mut registry = registry();
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.undo_log_mut().set_enabled(false);
        let _ = wb.set_text(sheet, 0, 1, "hello world");
        for i in 0..100_000u32 {
            let row = i / 16;
            let col = ((i % 16) + 2) as u16;
            let _ = wb.set_formula_text(sheet, row, col, "=LEN($B$1)");
        }
        let mut engine = RecalcEngine::new(std::mem::take(&mut registry));
        engine.set_threads(8);
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.bench_function("year_scan_100k", |b| {
        let mut registry = registry();
        let mut wb = Workbook::new();
        let sheet = wb.active_sheet();
        wb.undo_log_mut().set_enabled(false);
        let _ = wb.set_number(sheet, 0, 1, 45292.0);
        for i in 0..100_000u32 {
            let row = i / 16;
            let col = ((i % 16) + 2) as u16;
            let _ = wb.set_formula_text(sheet, row, col, "=YEAR($B$1)");
        }
        let mut engine = RecalcEngine::new(std::mem::take(&mut registry));
        engine.set_threads(8);
        b.iter(|| {
            black_box(engine.recalc_full(&mut wb));
        });
    });

    group.finish();
}

criterion_group!(benches, bench_text_date);
criterion_main!(benches);

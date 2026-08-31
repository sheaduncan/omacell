//! Criterion benchmark for WP-24 columnar pivot aggregation.

use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::addr::{CellRef, RangeRef, SheetId};
use omacell_core::pivot::{
    CacheValue, PivotAgg, PivotDataField, PivotTable, materialize_from_cache,
};
use omacell_core::workbook::DateSystem;

fn fixture() -> (PivotTable, Vec<String>, Vec<Vec<CacheValue>>) {
    let headers = vec!["Region".into(), "Product".into(), "Amount".into()];
    let rows = (0..100_000u32)
        .map(|index| {
            vec![
                CacheValue::Text(format!("Region{}", index % 100)),
                CacheValue::Text(format!("Product{}", index % 10)),
                CacheValue::Number(f64::from(index % 1_000)),
            ]
        })
        .collect();
    let source = RangeRef::from_corners(
        CellRef::new(0, 0).expect("valid benchmark cell"),
        CellRef::new(100_000, 2).expect("valid benchmark cell"),
    );
    let mut pivot = PivotTable::new("Benchmark", SheetId::new(0), source, SheetId::new(0), 0, 5);
    pivot.rows = vec!["Region".into()];
    pivot.cols = vec!["Product".into()];
    pivot.data = vec![PivotDataField::new("Amount", PivotAgg::Sum)];
    (pivot, headers, rows)
}

fn bench_pivot(c: &mut Criterion) {
    let mut group = c.benchmark_group("pivot");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    let (pivot, headers, rows) = fixture();
    group.bench_function("materialize_100k_rows", |b| {
        b.iter(|| {
            black_box(
                materialize_from_cache(DateSystem::Excel1900, &pivot, &headers, &rows)
                    .expect("benchmark pivot materializes"),
            );
        });
    });
    group.finish();
}

criterion_group!(benches, bench_pivot);
criterion_main!(benches);

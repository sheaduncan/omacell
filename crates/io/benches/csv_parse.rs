//! UTF-8 CSV parse throughput (WP-08, spec §12.1).

use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use omacell_io::csv::{ImportPlan, load, preview};

fn numeric_csv(rows: usize, cols: usize, value: &str) -> Vec<u8> {
    let mut out = String::with_capacity(rows * cols * 8);
    for r in 0..rows {
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            out.push_str(value);
        }
        if r + 1 < rows {
            out.push('\n');
        }
    }
    out.into_bytes()
}

fn parse_records(c: &mut Criterion) {
    let data = numeric_csv(80_000, 20, "123.45");
    let mut group = c.benchmark_group("csv_parse");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("utf8_parse_only", |b| {
        b.iter(|| {
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(black_box(data.as_slice()));
            let mut fields = 0u64;
            for record in rdr.records() {
                fields += record.expect("record").len() as u64;
            }
            black_box(fields);
        });
    });
    group.bench_function("utf8_load_80k_x_20", |b| {
        let plan = ImportPlan::default();
        b.iter(|| {
            let (workbook, result) = load(
                black_box(data.as_slice()),
                black_box(&plan),
                Default::default(),
            )
            .expect("load");
            black_box((workbook, result));
        });
    });
    group.finish();

    // Exactly 100,000,000 bytes: 20 four-digit values, 19 separators, and a
    // newline per row. This makes the product gate an actual 100 MB load
    // rather than a projection from a smaller sample.
    let mut product_data = numeric_csv(1_000_000, 20, "1234");
    product_data.push(b'\n');
    assert_eq!(product_data.len(), 100_000_000);
    let mut product = c.benchmark_group("csv_product");
    product.sample_size(10);
    product.warm_up_time(Duration::from_millis(200));
    product.measurement_time(Duration::from_secs(8));
    product.bench_function("first_paint_100mb", |b| {
        let plan = ImportPlan::default();
        b.iter(|| {
            black_box(preview(black_box(&product_data), black_box(&plan), 100).expect("preview"));
        });
    });
    product.bench_function("full_load_100mb", |b| {
        let plan = ImportPlan::default();
        b.iter(|| {
            let (workbook, result) = load(
                black_box(product_data.as_slice()),
                black_box(&plan),
                Default::default(),
            )
            .expect("load");
            black_box((workbook, result));
        });
    });
    product.finish();
}

criterion_group!(benches, parse_records);
criterion_main!(benches);

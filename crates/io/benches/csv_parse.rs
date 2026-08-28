//! UTF-8 CSV parse throughput (WP-08, spec §12.1).

use std::time::Duration;

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use omacell_io::csv::{ImportPlan, sniff};

fn numeric_csv(rows: usize, cols: usize) -> Vec<u8> {
    let mut out = String::with_capacity(rows * cols * 8);
    for r in 0..rows {
        for c in 0..cols {
            if c > 0 {
                out.push(',');
            }
            out.push_str("123.45");
        }
        if r + 1 < rows {
            out.push('\n');
        }
    }
    out.into_bytes()
}

fn parse_records(c: &mut Criterion) {
    let data = numeric_csv(80_000, 20);
    let mut group = c.benchmark_group("csv_parse");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("utf8_80k_x_20", |b| {
        b.iter(|| {
            let sniff = sniff(black_box(&data)).expect("sniff");
            black_box(sniff.sample_rows.len())
        });
    });
    group.bench_function("utf8_parse_only", |b| {
        let plan = ImportPlan::default();
        b.iter(|| {
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(black_box(data.as_slice()));
            let mut n = 0u64;
            for rec in rdr.records() {
                n += rec.expect("rec").len() as u64;
            }
            black_box(n);
            black_box(&plan);
        });
    });
    group.finish();
}

criterion_group!(benches, parse_records);
criterion_main!(benches);

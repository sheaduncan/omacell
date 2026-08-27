//! Criterion bench for formula parse throughput (WP-03 gate ≥ 100k formulas/s).

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use omacell_core::formula::parse;

fn sample_formulas() -> Vec<&'static str> {
    vec![
        "=A1+B1",
        "=SUM(A1:A10)",
        "=IF(A1>0,A1,-A1)",
        "=INDEX(A1:C9,,2)",
        "=$A$1+A1+$A1+A$1",
        "=Sheet1!A1:B2",
        "=Table1[Col]",
        "=A1#",
        "={1,2;3,4}",
        "=LET(x,1,x+1)",
        "=Revenue*TaxRate",
        "=(A1,C3)",
        "=A1:B2 B2:C3",
        "=INDIRECT(\"A1\")",
        "=NOW()+TODAY()",
    ]
}

fn bench_parse(c: &mut Criterion) {
    let formulas = sample_formulas();
    let mut group = c.benchmark_group("parse_formula");
    group.throughput(Throughput::Elements(formulas.len() as u64));
    group.bench_function("mixed_15", |b| {
        b.iter(|| {
            for f in &formulas {
                let _ = parse(black_box(f));
            }
        });
    });
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);

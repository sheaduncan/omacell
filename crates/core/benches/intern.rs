//! Rich-string and array interner lookup scaling (WP-02).

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use omacell_core::intern::{ArrayInterner, ArrayPayload, RichTextRun, StringInterner};
use omacell_core::style::Font;
use omacell_core::value::{Array2D, Value};

const ENTRY_COUNTS: [usize; 2] = [1_000, 100_000];

fn rich_run() -> RichTextRun {
    RichTextRun {
        start: 0,
        len: 4,
        font: Font {
            bold: true,
            ..Font::default()
        },
    }
}

fn intern_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("intern_lookup");
    for entry_count in ENTRY_COUNTS {
        let run = rich_run();
        let mut strings = StringInterner::default();
        for index in 0..entry_count {
            strings.intern_rich(&format!("rich-{index}"), vec![run.clone()]);
        }
        let rich_target = format!("rich-{}", entry_count - 1);
        group.bench_with_input(
            BenchmarkId::new("rich_existing", entry_count),
            &entry_count,
            |b, _| {
                b.iter(|| {
                    let id = strings.intern_rich(
                        std::hint::black_box(rich_target.as_str()),
                        vec![run.clone()],
                    );
                    strings.release(id);
                });
            },
        );

        let shape = Array2D::new(1, 2).expect("valid benchmark shape");
        let mut arrays = ArrayInterner::default();
        for index in 0..entry_count {
            let payload =
                ArrayPayload::new(shape, vec![Value::Number(index as f64), Value::Bool(true)])
                    .expect("valid benchmark payload");
            arrays.intern(payload);
        }
        let array_target = ArrayPayload::new(
            shape,
            vec![Value::Number((entry_count - 1) as f64), Value::Bool(true)],
        )
        .expect("valid benchmark payload");
        group.bench_with_input(
            BenchmarkId::new("array_existing", entry_count),
            &entry_count,
            |b, _| {
                b.iter(|| {
                    let id = arrays.intern(std::hint::black_box(array_target.clone()));
                    arrays.release(id);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, intern_lookup);
criterion_main!(benches);

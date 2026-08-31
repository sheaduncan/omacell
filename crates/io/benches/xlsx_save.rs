//! Save a synthetic workbook (spec §12.1 50 MB save < 5 s).

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::workbook::Workbook;
use omacell_io::xlsx::save_workbook_bytes;

fn filled_workbook() -> Workbook {
    let mut wb = Workbook::new();
    wb.undo_log_mut().set_enabled(false);
    let id = wb.active_sheet();
    // Same grid as `xlsx_open`: 86k × 20 numeric cells (~50 MiB of sheet XML).
    for r in 0..86_000u32 {
        for c in 0..20u16 {
            let _ = wb.set_number(id, r, c, f64::from(r + u32::from(c)));
        }
    }
    wb
}

fn bench_save(c: &mut Criterion) {
    let wb = filled_workbook();
    let mut group = c.benchmark_group("xlsx_save");
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(200));
    group.measurement_time(Duration::from_secs(5));
    group.bench_function("numeric_86k_x_20", |b| {
        b.iter(|| {
            let bytes = save_workbook_bytes(&wb).expect("save");
            std::hint::black_box(bytes.len());
        });
    });
    group.finish();
}

criterion_group!(benches, bench_save);
criterion_main!(benches);

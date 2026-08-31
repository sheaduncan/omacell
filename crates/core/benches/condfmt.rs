//! Criterion bench for conditional-format overlay (WP-18).

use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::addr::{CellRef, RangeRef};
use omacell_core::condfmt::{CfDxf, CfKind, CfOp, CondFormat, resolve_overlay};
use omacell_core::style::Color;
use omacell_core::workbook::Workbook;

fn range(r0: u32, c0: u16, r1: u32, c1: u16) -> RangeRef {
    RangeRef::from_corners(CellRef::new(r0, c0).unwrap(), CellRef::new(r1, c1).unwrap())
}

fn build() -> Workbook {
    let mut wb = Workbook::new();
    let s = wb.active_sheet();
    wb.undo_log_mut().set_enabled(false);
    for i in 0..100_000u32 {
        let row = i / 20;
        let col = (i % 20) as u16;
        let _ = wb.set_number(s, row, col, f64::from(i));
    }
    let mut rules = Vec::new();
    for p in 1..=20u32 {
        rules.push(CondFormat {
            range: range(0, 0, 4999, 19),
            priority: p,
            stop_if_true: false,
            kind: CfKind::CellIs {
                op: CfOp::Greater,
                formula1: format!("{}", p * 1000),
                formula2: None,
            },
            dxf: CfDxf {
                fill: Some(Color::Rgb {
                    argb: 0xFF00_0000 | p,
                }),
                font: None,
            },
        });
    }
    let _ = wb.set_cond_formats(s, rules);
    wb
}

fn bench_condfmt(c: &mut Criterion) {
    let mut group = c.benchmark_group("condfmt");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
    let mut wb = build();
    let s = wb.active_sheet();
    group.bench_function("overlay_100k_20_rules_after_edit", |b| {
        b.iter(|| {
            let _ = wb.set_number(s, 0, 0, 42.0);
            black_box(resolve_overlay(&wb, s, range(0, 0, 4999, 19)).unwrap());
        });
    });
    group.finish();
}

criterion_group!(benches, bench_condfmt);
criterion_main!(benches);

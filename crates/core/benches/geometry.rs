//! Geometry pixel↔index scaling (WP-02).

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use omacell_core::geometry::AxisGeometry;

fn pixel_to_index_scales(c: &mut Criterion) {
    let mut group = c.benchmark_group("geometry_pixel_to_index");
    for n in [1_000u32, 10_000, 100_000, 1_000_000] {
        let mut axis = AxisGeometry::rows();
        for i in 0..n {
            if i % 7 == 0 {
                let _ = axis.set_size(i, 12);
            }
            if i % 13 == 0 {
                let _ = axis.set_hidden(i, true);
            }
        }
        let px = axis.index_to_pixel(n / 2);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| axis.pixel_to_index(std::hint::black_box(px)));
        });
    }
    group.finish();
}

fn index_to_pixel_scales(c: &mut Criterion) {
    let mut group = c.benchmark_group("geometry_index_to_pixel");
    for n in [1_000u32, 10_000, 100_000, 1_000_000] {
        let mut axis = AxisGeometry::rows();
        for i in 0..n {
            if i % 5 == 0 {
                let _ = axis.set_size(i, 8);
            }
        }
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| axis.index_to_pixel(std::hint::black_box(n / 2)));
        });
    }
    group.finish();
}

criterion_group!(benches, pixel_to_index_scales, index_to_pixel_scales);
criterion_main!(benches);

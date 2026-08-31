//! Occupied-cell iteration (WP-02).

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_core::storage::{CellSlot, SheetStore};

fn fill(rows: u32, cols: u16) -> SheetStore {
    let mut s = SheetStore::new();
    for r in 0..rows {
        for c in 0..cols {
            s.set(r, c, CellSlot::number(1.0)).expect("set");
        }
    }
    s
}

fn iter_occupied(c: &mut Criterion) {
    let store = fill(4_000, 20);
    c.bench_function("iter_80k_numeric", |b| {
        b.iter(|| {
            let mut n = 0u64;
            for (_r, _c, slot) in store.iter() {
                n = n.wrapping_add(slot.value.is_error() as u64);
            }
            std::hint::black_box(n)
        });
    });
}

criterion_group!(benches, iter_occupied);
criterion_main!(benches);

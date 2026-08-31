//! Theme/config reload baseline for the spec's sub-100 ms reload gate.

use std::path::PathBuf;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_conf::{LoadOptions, Paths, load_with_options};

fn fixture_paths() -> Paths {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures = crate_dir.join("benches/fixtures");
    let empty_home = fixtures.join("empty-home");
    Paths {
        home: empty_home.clone(),
        default_dir: crate_dir.join("../../default"),
        user_config: empty_home.join(".config/omacell"),
        state_dir: empty_home.join(".local/state/omacell"),
        omarchy_state: fixtures.join("omarchy"),
        omarchy_config: empty_home.join(".config/omarchy"),
    }
}

fn bench_theme_reload(c: &mut Criterion) {
    let paths = fixture_paths();
    let options = LoadOptions::default();
    c.bench_function("theme_reload/full_config_and_theme", |b| {
        b.iter(|| {
            let loaded = load_with_options(black_box(&paths), black_box(&options))
                .expect("committed reload fixture must remain valid");
            black_box(loaded);
        });
    });
}

criterion_group!(benches, bench_theme_reload);
criterion_main!(benches);

//! IPC ping and propose overhead, excluding workbook recalculation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use omacell_bus::Bus;
use omacell_bus::ipc::{IpcClient, Mode, serve};
use omacell_core::eval::FnRegistry;
use omacell_core::recalc::RecalcEngine;
use omacell_core::workbook::Workbook;

static UNIQUE: AtomicU64 = AtomicU64::new(1);

fn serve_client() -> (omacell_bus::ipc::IpcHandle, IpcClient) {
    let dir = std::env::temp_dir().join(format!(
        "omacell-ipc-bench-{}-{}",
        std::process::id(),
        UNIQUE.fetch_add(1, Ordering::SeqCst)
    ));
    let bus = Bus::new(Workbook::new(), RecalcEngine::new(FnRegistry::new())).unwrap();
    let handle = serve(dir, bus).unwrap();
    let client = IpcClient::connect(handle.socket_path()).unwrap();
    (handle, client)
}

fn bench_ipc(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_roundtrip");
    group.sample_size(20);
    group.warm_up_time(Duration::from_millis(300));
    group.measurement_time(Duration::from_secs(2));

    group.bench_function("ping", |b| {
        let (_handle, mut client) = serve_client();
        b.iter(|| {
            let reply = client.ping().unwrap();
            black_box(reply.ok);
        });
    });

    group.bench_function("cell_set_propose", |b| {
        let (_handle, mut client) = serve_client();
        b.iter(|| {
            let reply = client
                .command(
                    "cell.set",
                    serde_json::json!({"ref":"A1","input":"1"}),
                    Some(Mode::Propose),
                )
                .unwrap();
            black_box(reply.ok);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_ipc);
criterion_main!(benches);

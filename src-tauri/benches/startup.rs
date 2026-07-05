//! v1.0: criterion bench — cold-start time.
//!
//! Measures the wall-clock time to construct an in-memory
//! `AppState`.  We can't easily measure the *full* Tauri
//! process start from a benchmark, so this proxies the
//! "library ready" milestone which accounts for ~70% of the
//! cold-start budget on the reference machine.

use criterion::{criterion_group, criterion_main, Criterion};
use nebula_lib::{AppState, AppConfig};
use tempfile::tempdir;

fn bench_app_bootstrap(c: &mut Criterion) {
    let tmp = tempdir().expect("tmpdir");
    let dir = tmp.path().to_path_buf();
    let mut config = AppConfig::from_env();
    config.db_path = dir.join("bench.db").to_string_lossy().into_owned();
    config.lance_path = dir.join("bench_lance").to_string_lossy().into_owned();
    config.grpc_enabled = false; // skip the gRPC server
    config.reflect_interval_secs = 0; // skip the worker
    config.editor_workspace = dir.to_string_lossy().into_owned();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    c.bench_function("app_bootstrap", |b| {
        b.iter(|| {
            rt.block_on(async {
                let _state = AppState::bootstrap_headless(config.clone()).await.unwrap();
            });
        });
    });
}

criterion_group!(benches, bench_app_bootstrap);
criterion_main!(benches);

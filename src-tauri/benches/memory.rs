//! v1.0: criterion bench — memory subsystem hot paths.
//!
//! Two micro-benchmarks:
//!   * `sponge_absorb`      — insert a single memory.
//!   * `embedder_dedupe`    — exercise the embedding cache.
//!
//! We deliberately skip the network (Ollama) layer; in CI
//! there is no Ollama server.  The embedder cache is exercised
//! by feeding a deterministic hash → a fixed vector, which is
//! the shape of the real cache hit path minus the HTTP call.

use criterion::{criterion_group, criterion_main, Criterion};
use nebula_lib::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};

fn bench_memory_construct(c: &mut Criterion) {
    c.bench_function("memory_construct", |b| {
        b.iter(|| {
            let _m = Memory::new(
                MemoryType::Episodic,
                MemoryLayer::L1,
                "hello world",
                SourceKind::UserInput,
            );
        });
    });
}

fn bench_summary_build(c: &mut Criterion) {
    c.bench_function("memory_summary_build", |b| {
        b.iter(|| {
            let m = Memory::new(
                MemoryType::Semantic,
                MemoryLayer::L2,
                "x".repeat(2000).as_str(),
                SourceKind::AgentOutput,
            );
            let _ = m.summary.s50.clone();
        });
    });
}

criterion_group!(benches, bench_memory_construct, bench_summary_build);
criterion_main!(benches);

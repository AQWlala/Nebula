//! Lightweight in-process metrics.
//!
//! v0.2 deliberately avoids pulling in the full Prometheus client
//! stack. The [`Metrics`] struct is a collection of `AtomicU64`
//! counters accessible via the [`global`] singleton. The Tauri
//! command `metrics()` snapshots the counters into a
//! [`MetricsSnapshot`] for transport to the front-end.
//!
//! All counters are monotonic: increments are positive, resets are
//! not exposed. Snapshotting reads with `Ordering::Relaxed` is
//! safe because the counters are independent.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Atomic counter bag. Field naming uses snake_case so the JSON
/// snapshot is directly consumable by the front-end.
pub struct Metrics {
    /// Number of embedder cache hits since process start.
    pub embedding_cache_hits: AtomicU64,
    /// Number of embedder cache misses since process start.
    pub embedding_cache_misses: AtomicU64,
    /// Number of `memory_store` calls that returned `Inserted`.
    pub memory_stores_total: AtomicU64,
    /// Number of `memory_search` calls.
    pub memory_searches_total: AtomicU64,
    /// Number of memory rows absorbed by the black-hole engine.
    pub blackhole_compressions_total: AtomicU64,
    /// Number of L5 reflections produced by the reflection engine.
    pub reflections_generated_total: AtomicU64,
    /// Number of `swarm_execute` invocations.
    pub swarm_executions_total: AtomicU64,
    /// Number of `chat` invocations.
    pub chat_total: AtomicU64,
}

impl Metrics {
    /// Builds a zeroed metrics bag. The result is *not* a singleton
    /// — for the process-wide counters use [`global`].
    pub const fn new() -> Self {
        Self {
            embedding_cache_hits: AtomicU64::new(0),
            embedding_cache_misses: AtomicU64::new(0),
            memory_stores_total: AtomicU64::new(0),
            memory_searches_total: AtomicU64::new(0),
            blackhole_compressions_total: AtomicU64::new(0),
            reflections_generated_total: AtomicU64::new(0),
            swarm_executions_total: AtomicU64::new(0),
            chat_total: AtomicU64::new(0),
        }
    }

    /// Records an embedding cache hit.
    pub fn record_embedding_hit(&self) {
        self.embedding_cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    /// Records an embedding cache miss.
    pub fn record_embedding_miss(&self) {
        self.embedding_cache_misses.fetch_add(1, Ordering::Relaxed);
    }
    /// Records a successful memory store.
    pub fn record_store(&self) {
        self.memory_stores_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Records a memory search.
    pub fn record_search(&self) {
        self.memory_searches_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Records `n` rows compressed by the black-hole engine.
    pub fn record_blackhole(&self, n: u64) {
        self.blackhole_compressions_total
            .fetch_add(n, Ordering::Relaxed);
    }
    /// Records a single reflection.
    pub fn record_reflection(&self) {
        self.reflections_generated_total
            .fetch_add(1, Ordering::Relaxed);
    }
    /// Records a swarm execution.
    pub fn record_swarm(&self) {
        self.swarm_executions_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Records a chat turn.
    pub fn record_chat(&self) {
        self.chat_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Atomically snapshots all counters into a transport-friendly
    /// struct.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            embedding_cache_hits: self.embedding_cache_hits.load(Ordering::Relaxed),
            embedding_cache_misses: self.embedding_cache_misses.load(Ordering::Relaxed),
            memory_stores_total: self.memory_stores_total.load(Ordering::Relaxed),
            memory_searches_total: self.memory_searches_total.load(Ordering::Relaxed),
            blackhole_compressions_total: self.blackhole_compressions_total.load(Ordering::Relaxed),
            reflections_generated_total: self.reflections_generated_total.load(Ordering::Relaxed),
            swarm_executions_total: self.swarm_executions_total.load(Ordering::Relaxed),
            chat_total: self.chat_total.load(Ordering::Relaxed),
        }
    }
}

/// Plain-data snapshot used for transport to the front-end.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetricsSnapshot {
    /// Embedder cache hits at snapshot time.
    pub embedding_cache_hits: u64,
    /// Embedder cache misses at snapshot time.
    pub embedding_cache_misses: u64,
    /// Successful memory stores at snapshot time.
    pub memory_stores_total: u64,
    /// Memory searches at snapshot time.
    pub memory_searches_total: u64,
    /// Black-hole compression rows at snapshot time.
    pub blackhole_compressions_total: u64,
    /// Reflections generated at snapshot time.
    pub reflections_generated_total: u64,
    /// Swarm executions at snapshot time.
    pub swarm_executions_total: u64,
    /// Chat turns at snapshot time.
    pub chat_total: u64,
}

impl MetricsSnapshot {
    /// Embedder cache hit ratio in `[0.0, 1.0]`. Returns `0.0` when
    /// no lookups have been performed.
    pub fn embedding_cache_hit_ratio(&self) -> f32 {
        let total = self.embedding_cache_hits + self.embedding_cache_misses;
        if total == 0 {
            0.0
        } else {
            self.embedding_cache_hits as f32 / total as f32
        }
    }
}

/// Process-wide metrics singleton. Initialised lazily on first access.
pub fn global() -> &'static Metrics {
    static GLOBAL: OnceLock<Metrics> = OnceLock::new();
    GLOBAL.get_or_init(Metrics::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_snapshot_is_zero() {
        // We cannot use `global()` here because tests share state; use
        // a local bag instead.
        let m = Metrics::new();
        let s = m.snapshot();
        assert_eq!(s.embedding_cache_hits, 0);
        assert_eq!(s.embedding_cache_misses, 0);
        assert_eq!(s.memory_stores_total, 0);
        assert_eq!(s.memory_searches_total, 0);
        assert_eq!(s.blackhole_compressions_total, 0);
        assert_eq!(s.reflections_generated_total, 0);
        assert_eq!(s.swarm_executions_total, 0);
        assert_eq!(s.chat_total, 0);
    }

    #[test]
    fn recorders_increment_correctly() {
        let m = Metrics::new();
        m.record_embedding_hit();
        m.record_embedding_hit();
        m.record_embedding_miss();
        m.record_store();
        m.record_search();
        m.record_blackhole(3);
        m.record_reflection();
        m.record_swarm();
        m.record_chat();
        let s = m.snapshot();
        assert_eq!(s.embedding_cache_hits, 2);
        assert_eq!(s.embedding_cache_misses, 1);
        assert_eq!(s.memory_stores_total, 1);
        assert_eq!(s.memory_searches_total, 1);
        assert_eq!(s.blackhole_compressions_total, 3);
        assert_eq!(s.reflections_generated_total, 1);
        assert_eq!(s.swarm_executions_total, 1);
        assert_eq!(s.chat_total, 1);
    }

    #[test]
    fn hit_ratio_handles_zero_total() {
        let s = MetricsSnapshot::default();
        assert_eq!(s.embedding_cache_hit_ratio(), 0.0);
    }

    #[test]
    fn hit_ratio_computes_correctly() {
        let s = MetricsSnapshot {
            embedding_cache_hits: 3,
            embedding_cache_misses: 1,
            ..Default::default()
        };
        assert!((s.embedding_cache_hit_ratio() - 0.75).abs() < 1e-6);
    }

    #[test]
    fn global_singleton_is_stable() {
        let a = global();
        let b = global();
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn snapshot_serialises_to_json() {
        let s = MetricsSnapshot {
            memory_stores_total: 5,
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"memory_stores_total\":5"));
    }
}

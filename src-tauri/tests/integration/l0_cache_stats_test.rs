//! T-S1-A-01: L0Cache 命中率统计集成测试。
//!
//! 对应 ROADMAP_v2.1.md §4.4 Stage 1 测试策略要求的
//! `tests/integration/l0_cache_stats_test.rs`。
//!
//! 覆盖目标：
//! * `get()`/`put()` 路径累加 `hot_hits`/`hot_misses`（单元测试已覆盖单线程）
//! * **并发访问计数准确**（本文件重点，EXPERT_REVIEW_v2.1.md §7.6 要求）
//! * 跨多次操作后 stats 累计无丢失

use std::sync::Arc;
use std::thread;

use nebula_lib::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use nebula_lib::memory::L0Cache;

fn make_mem(id: &str, content: &str) -> Memory {
    let mut m = Memory::new(
        MemoryType::Semantic,
        MemoryLayer::L1,
        content.to_string(),
        SourceKind::UserInput,
    );
    m.id = id.to_string();
    m
}

/// 验证单线程下 stats 在多次操作后累计正确。
#[test]
fn stats_accumulate_single_thread() {
    let cache = L0Cache::new();
    cache.insert(make_mem("a", "alpha content"));
    cache.insert(make_mem("b", "beta content"));

    // 5 次命中
    for _ in 0..3 {
        let _ = cache.lookup_hot("a");
    }
    for _ in 0..2 {
        let _ = cache.lookup_hot("b");
    }
    // 4 次未命中
    for i in 0..4 {
        let _ = cache.lookup_hot(&format!("missing-{i}"));
    }

    let s = cache.stats();
    assert_eq!(s.hot_hits, 5, "hot_hits should be 5 (3+2)");
    assert_eq!(s.hot_misses, 4, "hot_misses should be 4");
}

/// 验证多线程并发 lookup 下原子计数器无丢失。
///
/// 8 线程 × 1000 次 lookup（500 命中 + 500 未命中）= 4000 hits + 4000 misses。
/// AtomicU64 + Relaxed ordering 保证最终一致性，不丢计数。
#[test]
fn stats_concurrent_lookup_no_loss() {
    let cache = Arc::new(L0Cache::new());
    cache.insert(make_mem("shared-hit", "shared content"));

    let threads = 8;
    let lookups_per_thread = 1000;
    let hits_per_thread = 500;
    let misses_per_thread = lookups_per_thread - hits_per_thread;

    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let c = cache.clone();
            thread::spawn(move || {
                for i in 0..lookups_per_thread {
                    if i % 2 == 0 {
                        // 命中
                        let _ = c.lookup_hot("shared-hit");
                    } else {
                        // 未命中
                        let _ = c.lookup_hot(&format!("miss-{i}"));
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().expect("worker thread panicked");
    }

    let s = cache.stats();
    let expected_hits = (threads * hits_per_thread) as u64;
    let expected_misses = (threads * misses_per_thread) as u64;
    assert_eq!(
        s.hot_hits, expected_hits,
        "hot_hits should be {expected_hits} (concurrent), got {}",
        s.hot_hits
    );
    assert_eq!(
        s.hot_misses, expected_misses,
        "hot_misses should be {expected_misses} (concurrent), got {}",
        s.hot_misses
    );
}

/// 验证 stats 在 insert 后 lookup 命中时 hot_hits 真实反映，而非硬编码 0。
/// 这是 EXPERT_REVIEW §2.4.3 指出的"L0Cache 命中率硬编码 0"回归保护。
#[test]
fn stats_not_hardcoded_zero() {
    let cache = L0Cache::new();
    let initial = cache.stats();
    assert_eq!(initial.hot_hits, 0, "initial hot_hits must be 0");
    assert_eq!(initial.hot_misses, 0, "initial hot_misses must be 0");

    cache.insert(make_mem("x", "content x"));
    let _ = cache.lookup_hot("x"); // 命中
    let _ = cache.lookup_hot("y"); // 未命中

    let after = cache.stats();
    assert_ne!(after.hot_hits, 0, "hot_hits must not stay at hardcoded 0");
    assert_ne!(
        after.hot_misses, 0,
        "hot_misses must not stay at hardcoded 0"
    );
    assert_eq!(after.hot_hits, 1);
    assert_eq!(after.hot_misses, 1);
}

/// 验证 LRU 淘汰后再次 lookup 被淘汰的 key 会产生未命中。
#[test]
fn stats_reflect_lru_eviction() {
    // 容量 2：插入 3 条后，最旧的被淘汰。
    let cache = L0Cache::with_capacity(2, 8000, 64);
    cache.insert(make_mem("k1", "one"));
    cache.insert(make_mem("k2", "two"));
    cache.insert(make_mem("k3", "three")); // 淘汰 k1

    // k1 已被淘汰 → 未命中
    assert!(cache.lookup_hot("k1").is_none());
    // k2, k3 仍在 → 命中
    assert!(cache.lookup_hot("k2").is_some());
    assert!(cache.lookup_hot("k3").is_some());

    let s = cache.stats();
    assert_eq!(s.hot_hits, 2, "k2 and k3 should hit");
    assert_eq!(s.hot_misses, 1, "evicted k1 should miss");
}

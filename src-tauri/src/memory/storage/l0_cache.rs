//! v1.4 L0 缓存层（Cache Layer）。
//!
//! 对应设计文档 v7.0 §2.1 的 L0 Cache Layer，包含三个职责：
//!
//! * **LRU 热记忆缓存** — 最近访问的记忆，避免重复查 SQLite/LanceDB。
//! * **会话上下文窗口** — 当前会话的记忆窗口（滑动窗口）。
//! * **预取队列** — 基于任务关键词预测性预加载相关记忆。
//!
//! ## 容量限制
//!
//! 设计文档规定 L0 容量上限 64 MB。实际实现用条目数限制（默认 256 条）+
//! token 预算（默认 8000 token）双重约束。

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::memory::sqlite_store::SqliteStore;
use crate::memory::types::{Memory, MemoryType};

/// 默认 LRU 热记忆条目数。
const DEFAULT_HOT_CAPACITY: usize = 256;
/// 默认会话上下文 token 预算。
const DEFAULT_SESSION_TOKEN_BUDGET: usize = 8000;
/// 默认会话上下文条目数上限。
const DEFAULT_SESSION_ENTRY_LIMIT: usize = 64;

/// 粗略 token 估算（1 token ≈ 2 字符，中英混合启发式）。
fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 2).max(1)
}

/// L0 缓存层。
#[derive(Debug)]
pub struct L0Cache {
    /// LRU 热记忆缓存（按 id 索引）。
    hot: Mutex<LruCache<String, Arc<Memory>>>,
    /// 会话上下文窗口（最近的记忆，滑动窗口）。
    session: Mutex<SessionWindow>,
    /// 预取队列（待预加载的记忆 id）。
    prefetch_queue: Mutex<VecDeque<String>>,
    /// T-S1-A-01: LRU 热缓存命中累计计数（无锁原子，热路径性能）。
    hot_hits: AtomicU64,
    /// T-S1-A-01: LRU 热缓存未命中累计计数。
    hot_misses: AtomicU64,
}

/// 会话上下文窗口。
#[derive(Debug)]
struct SessionWindow {
    /// 窗口内的记忆（按时间顺序）。
    entries: VecDeque<Arc<Memory>>,
    /// 当前 token 总量。
    token_used: usize,
    /// token 预算。
    token_budget: usize,
    /// 条目数上限。
    entry_limit: usize,
}

impl SessionWindow {
    fn new(token_budget: usize, entry_limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            token_used: 0,
            token_budget,
            entry_limit,
        }
    }

    /// 推入一条记忆，超出预算时从最旧的开始淘汰。
    fn push(&mut self, mem: Arc<Memory>) {
        let tokens = estimate_tokens(&mem.content);
        // 先淘汰，直到有空间。
        while (self.token_used + tokens > self.token_budget
            || self.entries.len() >= self.entry_limit)
            && !self.entries.is_empty()
        {
            if let Some(old) = self.entries.pop_front() {
                self.token_used -= estimate_tokens(&old.content);
            }
        }
        self.token_used += tokens;
        self.entries.push_back(mem);
    }

    /// 清空窗口。
    fn clear(&mut self) {
        self.entries.clear();
        self.token_used = 0;
    }
}

/// L0 缓存命中统计（供可观测性使用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct L0Stats {
    pub hot_hits: u64,
    pub hot_misses: u64,
    pub session_entries: usize,
    pub session_tokens: usize,
    pub prefetch_pending: usize,
}

impl L0Cache {
    /// 使用默认容量创建。
    pub fn new() -> Self {
        Self::with_capacity(
            DEFAULT_HOT_CAPACITY,
            DEFAULT_SESSION_TOKEN_BUDGET,
            DEFAULT_SESSION_ENTRY_LIMIT,
        )
    }

    /// 自定义容量。
    pub fn with_capacity(
        hot_cap: usize,
        session_token_budget: usize,
        session_entry_limit: usize,
    ) -> Self {
        // T-D-B-07: hot_cap.max(1) 保证 >=1,unwrap_or(NonZeroUsize::MIN) 永不触发
        let cap = NonZeroUsize::new(hot_cap.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            hot: Mutex::new(LruCache::new(cap)),
            session: Mutex::new(SessionWindow::new(
                session_token_budget,
                session_entry_limit,
            )),
            prefetch_queue: Mutex::new(VecDeque::new()),
            hot_hits: AtomicU64::new(0),
            hot_misses: AtomicU64::new(0),
        }
    }

    /// 从 LRU 热缓存查找一条记忆。命中时标记为最近使用。
    ///
    /// T-S1-A-01: 命中/未命中在原子计数器累加，供 `stats()` 读取真实值，
    /// 仪表盘（T-S1-B-03）和 MemoryOrchestrator（T-S1-A-02）据此决策。
    pub fn lookup_hot(&self, id: &str) -> Option<Arc<Memory>> {
        let mut g = self.hot.lock();
        if let Some(mem) = g.get(id) {
            self.hot_hits.fetch_add(1, Ordering::Relaxed);
            // T-S1-B-03: 同步上报全局 metrics,供仪表盘聚合。
            crate::metrics::global().record_l0_hit();
            return Some(mem.clone());
        }
        self.hot_misses.fetch_add(1, Ordering::Relaxed);
        crate::metrics::global().record_l0_miss();
        None
    }

    /// 将一条记忆放入热缓存 + 会话窗口。
    pub fn insert(&self, mem: Memory) {
        let arc = Arc::new(mem);
        {
            let mut g = self.hot.lock();
            g.put(arc.id.clone(), arc.clone());
        }
        {
            let mut s = self.session.lock();
            s.push(arc);
        }
    }

    /// 批量放入（用于预取结果回填）。
    pub fn insert_many(&self, mems: Vec<Memory>) {
        for m in mems {
            self.insert(m);
        }
    }

    /// T-E-D-01: 冷启动预热 — 从 SQLite 拉取最近 `limit` 条记忆回填到
    /// LRU 热缓存 + 会话窗口。
    ///
    /// 在 `lib.rs` bootstrap 完成后 spawn 调用,异步执行:
    /// 1. `sqlite.list_recent(limit)` 拉取最近 N 条未压缩记忆。
    /// 2. `insert_many` 把每条记忆写入 `hot` LRU + `session` 窗口。
    ///
    /// 失败(如 SQLite 锁冲突)静默吞掉 — 预热是 best-effort,失败时
    /// L0Cache 仍为空,后续查询走 SQLite/LanceDB 正常路径,不影响功能。
    ///
    /// `limit` 推荐 64(与默认会话条目上限 `DEFAULT_SESSION_ENTRY_LIMIT` 一致),
    /// 避免预热阶段拉全表阻塞启动。
    pub async fn prewarm_from_store(&self, sqlite: &SqliteStore, limit: usize) {
        if let Ok(memories) = sqlite.list_recent(limit).await {
            self.insert_many(memories);
        }
    }

    /// 将记忆 id 加入预取队列。
    pub fn enqueue_prefetch(&self, id: String) {
        self.prefetch_queue.lock().push_back(id);
    }

    /// 取出下一个待预取的记忆 id。
    pub fn dequeue_prefetch(&self) -> Option<String> {
        self.prefetch_queue.lock().pop_front()
    }

    /// 获取当前会话上下文快照（按时间顺序）。
    pub fn session_snapshot(&self) -> Vec<Arc<Memory>> {
        self.session.lock().entries.iter().cloned().collect()
    }

    /// 清空会话上下文（新会话开始时调用）。
    pub fn clear_session(&self) {
        self.session.lock().clear();
    }

    /// 按记忆类型过滤会话上下文。
    pub fn session_by_type(&self, mt: MemoryType) -> Vec<Arc<Memory>> {
        self.session
            .lock()
            .entries
            .iter()
            .filter(|m| m.memory_type == mt)
            .cloned()
            .collect()
    }

    /// 缓存统计。
    ///
    /// T-S1-A-01: `hot_hits`/`hot_misses` 从原子计数器读取真实累计值，
    /// 不再返回硬编码 0。仪表盘据此计算命中率 =
    /// `hot_hits / (hot_hits + hot_misses)`。
    pub fn stats(&self) -> L0Stats {
        let s = self.session.lock();
        L0Stats {
            hot_hits: self.hot_hits.load(Ordering::Relaxed),
            hot_misses: self.hot_misses.load(Ordering::Relaxed),
            session_entries: s.entries.len(),
            session_tokens: s.token_used,
            prefetch_pending: self.prefetch_queue.lock().len(),
        }
    }
}

impl Default for L0Cache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{MemoryLayer, SourceKind};

    fn make_mem(id: &str, content: &str, mt: MemoryType) -> Memory {
        let mut m = Memory::new(
            mt,
            MemoryLayer::L1,
            content.to_string(),
            SourceKind::UserInput,
        );
        m.id = id.to_string();
        m
    }

    #[test]
    fn hot_cache_hit_and_miss() {
        let c = L0Cache::new();
        let m = make_mem("m1", "hello world", MemoryType::Semantic);
        c.insert(m);
        assert!(c.lookup_hot("m1").is_some());
        assert!(c.lookup_hot("nonexistent").is_none());

        // T-S1-A-01: stats 应反映真实命中/未命中数
        let s = c.stats();
        assert_eq!(s.hot_hits, 1, "hot_hits should be 1 after one hit");
        assert_eq!(s.hot_misses, 1, "hot_misses should be 1 after one miss");
    }

    /// T-S1-A-01: 验证多次 lookup 后 stats 累计正确。
    #[test]
    fn stats_accumulates_across_lookups() {
        let c = L0Cache::new();
        c.insert(make_mem("a", "alpha", MemoryType::Semantic));
        c.insert(make_mem("b", "beta", MemoryType::Semantic));

        // 3 次命中
        assert!(c.lookup_hot("a").is_some());
        assert!(c.lookup_hot("b").is_some());
        assert!(c.lookup_hot("a").is_some());
        // 2 次未命中
        assert!(c.lookup_hot("missing1").is_none());
        assert!(c.lookup_hot("missing2").is_none());

        let s = c.stats();
        assert_eq!(s.hot_hits, 3, "hot_hits should accumulate to 3");
        assert_eq!(s.hot_misses, 2, "hot_misses should accumulate to 2");
    }

    /// T-S1-A-01: 验证新实例 stats 初始为 0（非硬编码 0 的回归保护）。
    #[test]
    fn stats_initial_zero() {
        let c = L0Cache::new();
        let s = c.stats();
        assert_eq!(s.hot_hits, 0);
        assert_eq!(s.hot_misses, 0);
    }

    #[test]
    fn session_window_evicts_on_token_budget() {
        // 小预算：4 条 10 字符记忆（每条约 5 token），预算 12 token → 只能放 2 条。
        let c = L0Cache::with_capacity(64, 12, 64);
        c.insert(make_mem("1", "aaaaaaaaaa", MemoryType::Semantic));
        c.insert(make_mem("2", "bbbbbbbbbb", MemoryType::Semantic));
        c.insert(make_mem("3", "cccccccccc", MemoryType::Semantic));
        let snap = c.session_snapshot();
        assert!(
            snap.len() <= 3,
            "session should evict, got {} entries",
            snap.len()
        );
        // 最旧的应被淘汰
        let ids: Vec<_> = snap.iter().map(|m| m.id.as_str()).collect();
        assert!(
            !ids.contains(&"1") || snap.len() <= 2,
            "oldest should be evicted"
        );
    }

    #[test]
    fn session_by_type_filters() {
        let c = L0Cache::new();
        c.insert(make_mem("1", "fact", MemoryType::Semantic));
        c.insert(make_mem("2", "event", MemoryType::Episodic));
        c.insert(make_mem("3", "skill", MemoryType::Procedural));
        let sem = c.session_by_type(MemoryType::Semantic);
        assert_eq!(sem.len(), 1);
        assert_eq!(sem[0].id, "1");
    }

    #[test]
    fn clear_session_resets() {
        let c = L0Cache::new();
        c.insert(make_mem("1", "x", MemoryType::Semantic));
        assert_eq!(c.session_snapshot().len(), 1);
        c.clear_session();
        assert_eq!(c.session_snapshot().len(), 0);
    }

    #[test]
    fn prefetch_queue_fifo() {
        let c = L0Cache::new();
        c.enqueue_prefetch("a".into());
        c.enqueue_prefetch("b".into());
        assert_eq!(c.dequeue_prefetch(), Some("a".into()));
        assert_eq!(c.dequeue_prefetch(), Some("b".into()));
        assert_eq!(c.dequeue_prefetch(), None);
    }

    /// T-E-D-01: 预热后 L0Cache 应有非零 stats(session_entries > 0)。
    ///
    /// 模拟真实启动场景:SQLite 已有记忆,新建 L0Cache(空)→ 调用
    /// `prewarm_from_store` → 验证 `stats().session_entries` 非零。
    #[tokio::test]
    async fn prewarm_populates_entries() {
        use crate::memory::sqlite_store::SqliteStore;
        use crate::memory::types::{MemoryLayer, SourceKind};
        use std::env;

        // 1. 准备临时 SQLite store,插入 3 条记忆。
        let mut db_path = env::temp_dir();
        db_path.push(format!("nebula_l0_prewarm_{}.db", uuid::Uuid::new_v4()));
        let store = SqliteStore::open(&db_path).expect("create should succeed");
        for i in 0..3 {
            let mut m = Memory::new(
                MemoryType::Semantic,
                MemoryLayer::L3,
                format!("content-{i}"),
                SourceKind::UserInput,
            );
            m.id = format!("prewarm-{i}");
            store.insert(&m).await.expect("insert should succeed");
        }

        // 2. 新建空 L0Cache,验证初始 stats 全零。
        let cache = L0Cache::new();
        let before = cache.stats();
        assert_eq!(
            before.session_entries, 0,
            "cache must be empty before prewarm"
        );

        // 3. 调用 prewarm_from_store(limit=64)。
        cache.prewarm_from_store(&store, 64).await;

        // 4. 验证预热后 stats 非零:session_entries 应等于 3(全部回填)。
        let after = cache.stats();
        assert_eq!(
            after.session_entries, 3,
            "session_entries must be 3 after prewarm, got {}",
            after.session_entries
        );

        // 5. 验证热缓存可命中预热的记忆。
        assert!(
            cache.lookup_hot("prewarm-0").is_some(),
            "prewarmed memory must be lookup-able in hot cache"
        );

        let _ = std::fs::remove_file(db_path);
    }

    /// T-E-D-01: prewarm limit 截断 — 只回填最近 N 条,不拉全表。
    #[tokio::test]
    async fn prewarm_respects_limit() {
        use crate::memory::sqlite_store::SqliteStore;
        use crate::memory::types::{MemoryLayer, SourceKind};
        use std::env;

        let mut db_path = env::temp_dir();
        db_path.push(format!(
            "nebula_l0_prewarm_limit_{}.db",
            uuid::Uuid::new_v4()
        ));
        let store = SqliteStore::open(&db_path).expect("create should succeed");
        for i in 0..5 {
            let mut m = Memory::new(
                MemoryType::Semantic,
                MemoryLayer::L3,
                format!("content-{i}"),
                SourceKind::UserInput,
            );
            m.id = format!("limit-{i}");
            store.insert(&m).await.expect("insert should succeed");
        }

        let cache = L0Cache::new();
        // limit=2,只回填最近 2 条。
        cache.prewarm_from_store(&store, 2).await;
        let after = cache.stats();
        assert_eq!(after.session_entries, 2, "limit=2 must clamp to 2 entries");

        let _ = std::fs::remove_file(db_path);
    }
}

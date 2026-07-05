//! T-E-A-01: L0.5 Semantic Cache 层。
//!
//! 在 L0 精确缓存（hash key 完全匹配）和 LLM 调用之间插入一道"语义
//! 近邻"短路：用 Embedder 把 query 向量化，在 LanceDB 中查找最近邻，
//! 若 cosine 相似度 ≥ `SIMILARITY_THRESHOLD` (0.92) 且未超过 TTL
//! （默认 1h），直接返回缓存的响应文本，跳过本次 LLM 调用。
//!
//! ## 设计要点
//!
//! * **复用现有基础设施**：直接持有 `LanceStore` + `Embedder` 句柄，
//!   零新增依赖。
//! * **响应文本与时间戳的存储**：`LanceStore` schema 只承载 `(id,
//!   vector)`，因此把响应正文 + 插入时刻保存在进程内的
//!   `Mutex<HashMap<String, CacheEntry>>` 中，键为 query 的稳定哈希。
//!   LanceDB 只负责"近邻检索"这一件事。
//! * **TTL 过期**：查询时检查 `CacheEntry.inserted_at`，过期则当作
//!   miss（不主动删除，避免在热路径上做 IO；下次 `store` 同 id 时
//!   自然覆盖）。
//! * **失败降级**：任何 embed / lance 查询错误都记录 `debug!` 并
//!   返回 `None`，绝不阻断主调用链——LLM 仍然能正常响应。
//! * **指标**：每次命中/未命中都通过 `metrics::global()` 累加
//!   `semantic_cache_hits` / `semantic_cache_misses` 两个原子计数器。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use tracing::debug;

use crate::memory::embedder::Embedder;
use crate::memory::sqlite_store::SqliteStore;
// T-E-S-42: SemanticCache 面向 VectorStore trait 编程,可接受任意后端。
use crate::memory::vector_store::VectorStore;

/// 默认相似度阈值：cosine ≥ 0.92 视为"语义相同"。
pub const DEFAULT_SIMILARITY_THRESHOLD: f32 = 0.92;

/// 默认 TTL：1 小时。
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// P1 修复:entries map 的 LRU 容量上限。
///
/// 原实现用 `HashMap` 无界增长,长运行进程会累积大量过期条目(虽然 TTL
/// 检查会让它们表现为 miss,但条目本身不会被回收)。改用 `LruCache`
/// 后,超过容量时自动淘汰最久未访问的条目,内存占用有上界。
///
/// 1024 是合理的桌面应用上限:每条 CacheEntry ≈ response 字符串 + 32
/// 字节元数据,1024 条约几 MB,远低于 LanceDB 的内存占用。
pub const ENTRIES_LRU_CAPACITY: usize = 1024;

/// 单条缓存项：响应正文 + 插入时刻。
#[derive(Debug, Clone)]
struct CacheEntry {
    response: String,
    inserted_at: Instant,
}

/// T-E-A-01: L0.5 语义缓存。
///
/// 持有 LanceDB 句柄 + Embedder + 进程内响应映射表。`check` /
/// `store` 全部 async，且任何底层错误都降级为"未命中"。
///
/// T-E-D-01: 新增 `sqlite` 字段(可选)用于持久化响应正文。重启后
/// entries map 清空,LanceDB 命中但本地映射缺失时,`check()` 回退查
/// SQLite;`store()` 同步写入 SQLite。`prewarm_from_store` 在启动后
/// 重建最近 256 条 entries。
pub struct SemanticCache {
    lance: Arc<dyn VectorStore>,
    embedder: Arc<Embedder>,
    /// id → (response, inserted_at)。LanceDB 只存向量，响应正文
    /// 在这里以 in-process LRU cache 保存。容量由 [`ENTRIES_LRU_CAPACITY`]
    /// 限制(默认 1024),超过时自动淘汰最久未访问的条目。
    entries: Mutex<LruCache<String, CacheEntry>>,
    similarity_threshold: f32,
    ttl: Duration,
    /// T-E-D-01: SQLite 持久化后端(可选)。`None` 时退化为纯内存模式
    /// (与 T-E-A-01 行为一致)。`Some(sqlite)` 时 `store()` 写入 SQLite,
    /// `check()` 在 entries map miss 时回退查 SQLite。
    sqlite: Option<Arc<SqliteStore>>,
}

impl SemanticCache {
    /// 创建一个语义缓存。`similarity_threshold` 与 `ttl` 用默认值
    /// 时请使用 [`SemanticCache::default_config`]。
    pub fn new(
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
        similarity_threshold: f32,
        ttl: Duration,
    ) -> Self {
        let cap = NonZeroUsize::new(ENTRIES_LRU_CAPACITY)
            .unwrap_or(NonZeroUsize::new(1).unwrap());
        Self {
            lance,
            embedder,
            entries: Mutex::new(LruCache::new(cap)),
            similarity_threshold,
            ttl,
            sqlite: None,
        }
    }

    /// 用默认阈值 (0.92) 与默认 TTL (1h) 创建。
    pub fn default_config(lance: Arc<dyn VectorStore>, embedder: Arc<Embedder>) -> Self {
        Self::new(lance, embedder, DEFAULT_SIMILARITY_THRESHOLD, DEFAULT_TTL)
    }

    /// T-E-D-01: 注入 SQLite 持久化后端(builder 模式)。
    ///
    /// 启用后:
    /// - `store()` 在写入 entries map 同时,异步写入 SQLite
    ///   `semantic_cache_entries` 表(失败静默吞掉,不影响主调用)。
    /// - `check()` 在 LanceDB 命中但 entries map miss 时,回退查 SQLite。
    /// - `prewarm_from_store()` 从 SQLite 拉取最近 256 条重建 entries map。
    ///
    /// 不持有 SqliteStore 的旧调用路径(如 `prefetch.rs` 测试)行为不变:
    /// `sqlite` 字段为 `None` 时退化为纯内存模式。
    pub fn with_sqlite(mut self, sqlite: Arc<SqliteStore>) -> Self {
        self.sqlite = Some(sqlite);
        self
    }

    /// 查询语义缓存。
    ///
    /// 流程：embed(query) → LanceDB.search(k=1) → 检查 cosine ≥ 阈值
    /// → 检查 TTL → 返回响应。任何步骤失败都返回 `None`，调用方
    /// 走原 LLM 路径。
    pub async fn check(&self, query: &str) -> Option<String> {
        let query_vec = match self.embedder.embed(query).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    target: "nebula.llm.semantic_cache",
                    error = ?e, "embed failed; treating as miss"
                );
                crate::metrics::global().record_semantic_cache_miss();
                return None;
            }
        };

        let hits = match self.lance.search(&query_vec, 1).await {
            Ok(h) if !h.is_empty() => h,
            Ok(_) => {
                crate::metrics::global().record_semantic_cache_miss();
                return None;
            }
            Err(e) => {
                debug!(
                    target: "nebula.llm.semantic_cache",
                    error = ?e, "lance search failed; treating as miss"
                );
                crate::metrics::global().record_semantic_cache_miss();
                return None;
            }
        };

        let (id, score) = &hits[0];
        if *score < self.similarity_threshold {
            debug!(
                target: "nebula.llm.semantic_cache",
                score, threshold = self.similarity_threshold,
                "below threshold; miss"
            );
            crate::metrics::global().record_semantic_cache_miss();
            return None;
        }

        // 命中候选 id，再校验 TTL 与本地响应映射。
        // LruCache::get 需要 &mut self(会 bump LRU 顺序),所以 guard 需 mut。
        let hit = {
            let mut g = self.entries.lock();
            g.get(id).cloned()
        };

        match hit {
            Some(entry) => {
                if entry.inserted_at.elapsed() >= self.ttl {
                    debug!(
                        target: "nebula.llm.semantic_cache",
                        age_secs = entry.inserted_at.elapsed().as_secs(),
                        ttl_secs = self.ttl.as_secs(),
                        "entry expired; miss"
                    );
                    crate::metrics::global().record_semantic_cache_miss();
                    return None;
                }
                debug!(
                    target: "nebula.llm.semantic_cache",
                    score, "semantic cache hit"
                );
                crate::metrics::global().record_semantic_cache_hit();
                Some(entry.response)
            }
            None => {
                // T-E-D-01: LanceDB 命中但本地映射缺失(可能进程重启过)。
                // 回退查 SQLite,若命中则回填 entries map,后续命中走内存路径。
                if let Some(sqlite) = &self.sqlite {
                    match sqlite.query_semantic_cache_entry(id).await {
                        Ok(Some(response)) => {
                            debug!(
                                target: "nebula.llm.semantic_cache",
                                score, "semantic cache hit (sqlite fallback)"
                            );
                            // 回填 entries map(用 Instant::now 重置 TTL 计时,
                            // 因为进程重启后无法恢复原始 inserted_at)。
                            self.entries.lock().put(
                                id.clone(),
                                CacheEntry {
                                    response: response.clone(),
                                    inserted_at: Instant::now(),
                                },
                            );
                            crate::metrics::global().record_semantic_cache_hit();
                            return Some(response);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            debug!(
                                target: "nebula.llm.semantic_cache",
                                error = ?e, "sqlite fallback query failed; miss"
                            );
                        }
                    }
                }
                crate::metrics::global().record_semantic_cache_miss();
                None
            }
        }
    }

    /// 把 (query, response) 写入缓存。
    ///
    /// 流程：embed(query) → 计算稳定 id → LanceStore.upsert(id, vec)
    /// → 本地 map 写入 (response, now)。任何步骤失败都 `debug!`
    /// 记录后吞掉错误（缓存写入失败不影响主调用）。
    ///
    /// T-E-D-01: 若 `sqlite` 字段存在,同步写入 SQLite
    /// `semantic_cache_entries` 表(失败静默吞掉,不影响主调用)。
    /// 这样进程重启后 `prewarm_from_store` 可重建 entries map,
    /// `check()` 也可在 entries map miss 时回退查 SQLite。
    pub async fn store(&self, query: &str, response: &str) {
        let id = stable_id(query);
        let query_vec = match self.embedder.embed(query).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    target: "nebula.llm.semantic_cache",
                    error = ?e, "embed failed during store; skipping"
                );
                return;
            }
        };

        if let Err(e) = self.lance.upsert(&id, &query_vec).await {
            debug!(
                target: "nebula.llm.semantic_cache",
                error = ?e, "lance upsert failed during store; skipping"
            );
            return;
        }

        self.entries.lock().put(
            id.clone(),
            CacheEntry {
                response: response.to_string(),
                inserted_at: Instant::now(),
            },
        );

        // T-E-D-01: 持久化到 SQLite(若启用)。失败静默吞掉 — 缓存写入
        // 失败不应阻塞主调用链。下次 `store` 同 id 时 INSERT OR REPLACE 覆盖。
        if let Some(sqlite) = &self.sqlite {
            if let Err(e) = sqlite.insert_semantic_cache_entry(&id, response).await {
                debug!(
                    target: "nebula.llm.semantic_cache",
                    error = ?e, "sqlite insert_semantic_cache_entry failed; skipping"
                );
            }
        }
    }

    /// T-E-D-01: 从 SQLite 预热 entries map(重启后调用)。
    ///
    /// 流程:
    /// 1. `sqlite.list_recent_semantic_cache_entries(256)` 拉取最近 256 条
    ///    `(query_hash, response)` 元组(按 inserted_at DESC 排序)。
    /// 2. 把每条回填到 `entries` map,`inserted_at` 设为 `Instant::now()`
    ///    (进程重启后无法恢复原始 Instant,TTL 从预热时刻重新计起)。
    ///
    /// 失败(SQLite 锁冲突 / 表不存在)静默吞掉 — 预热是 best-effort,
    /// 失败时 entries map 仍为空,后续 `check()` 走 LanceDB + SQLite fallback。
    ///
    /// `limit` 默认 256(LRU 容量上限)。调用方在 lib.rs setup 回调中 spawn:
    /// ```ignore
    /// let cache_clone = semantic_cache.clone();
    /// let sqlite_clone = sqlite.clone();
    /// tokio::spawn(async move {
    ///     cache_clone.prewarm_from_store(&sqlite_clone, 256).await;
    /// });
    /// ```
    pub async fn prewarm_from_store(&self, sqlite: &SqliteStore, limit: usize) {
        match sqlite.list_recent_semantic_cache_entries(limit).await {
            Ok(rows) => {
                let mut g = self.entries.lock();
                for (hash, response) in rows {
                    g.put(
                        hash,
                        CacheEntry {
                            response,
                            // 进程重启后无法恢复原始 Instant,TTL 从预热时刻重新计起。
                            // 这意味着预热后的 1h 内不会因 TTL 过期,与"刚写入"语义一致。
                            inserted_at: Instant::now(),
                        },
                    );
                }
                debug!(
                    target: "nebula.llm.semantic_cache",
                    count = g.len(),
                    "prewarmed semantic cache entries from sqlite"
                );
            }
            Err(e) => {
                debug!(
                    target: "nebula.llm.semantic_cache",
                    error = ?e, "prewarm_from_store failed; entries map stays empty"
                );
            }
        }
    }

    /// 清空本地响应映射（不主动删 LanceDB 行，因为下次同 query
    /// 的 `store` 会自然覆盖）。供测试与 `clear_cache` 使用。
    pub fn clear(&self) {
        self.entries.lock().clear();
    }

    /// 返回当前缓存条目数（仅供测试/诊断）。
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    /// 当前相似度阈值。
    pub fn similarity_threshold(&self) -> f32 {
        self.similarity_threshold
    }

    /// 当前 TTL。
    pub fn ttl(&self) -> Duration {
        self.ttl
    }
}

/// 计算 query 的稳定 id（hex 格式 u64 哈希）。
fn stable_id(query: &str) -> String {
    let mut h = DefaultHasher::new();
    query.hash(&mut h);
    format!("sem:{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_deterministic() {
        let a = stable_id("hello world");
        let b = stable_id("hello world");
        assert_eq!(a, b);
        assert!(a.starts_with("sem:"));
    }

    #[test]
    fn stable_id_changes_with_input() {
        let a = stable_id("hello");
        let b = stable_id("world");
        assert_ne!(a, b);
    }

    #[test]
    fn default_threshold_and_ttl_match_constants() {
        assert_eq!(DEFAULT_SIMILARITY_THRESHOLD, 0.92);
        assert_eq!(DEFAULT_TTL, Duration::from_secs(3600));
    }

    // -----------------------------------------------------------------------
    // T-E-D-01: SemanticCache 持久化 + 预热测试。
    //
    // 测试策略:
    // 1. 用真实 SqliteStore(临时文件)+ 真实 LanceStore(临时目录)+ 真实
    //    Embedder(用 seed_cache_for_test 跳过网络调用)。
    // 2. 模拟"重启"场景:第一次 store 写入 → drop SemanticCache → 新建
    //    SemanticCache(空 entries map)→ prewarm_from_store → 验证恢复。
    // -----------------------------------------------------------------------

    use crate::llm::ollama::OllamaClient;
    use crate::memory::embedder::Embedder;
    use crate::memory::lance_store::LanceStore;
    use crate::memory::sqlite_store::SqliteStore;
    use std::env;

    fn temp_db_path() -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!(
            "nebula_semantic_cache_test_{}.db",
            uuid::Uuid::new_v4()
        ));
        p
    }

    fn temp_lance_path() -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!(
            "nebula_semantic_cache_lance_{}",
            uuid::Uuid::new_v4()
        ));
        p
    }

    fn cleanup(db: std::path::PathBuf, lance: std::path::PathBuf) {
        let _ = std::fs::remove_file(db);
        let _ = std::fs::remove_dir_all(lance);
    }

    /// 构造测试 SemanticCache(注入 sqlite)。dim=4 与向量维度对齐。
    async fn make_cache(
        db_path: &std::path::Path,
        lance_path: &std::path::Path,
        dim: usize,
    ) -> (
        SemanticCache,
        Arc<SqliteStore>,
        Arc<LanceStore>,
        Arc<Embedder>,
    ) {
        let sqlite = Arc::new(SqliteStore::open(db_path).unwrap());
        let lance = Arc::new(LanceStore::open(lance_path, dim).await.unwrap());
        let embedder = Arc::new(Embedder::new(
            OllamaClient::new("http://127.0.0.1:1"),
            "test-model",
            dim,
        ));
        let cache = SemanticCache::default_config(lance.clone(), embedder.clone())
            .with_sqlite(sqlite.clone());
        (cache, sqlite, lance, embedder)
    }

    /// T-E-D-01: 重启后 prewarm_from_store 应恢复 entries map。
    ///
    /// 模拟场景:
    /// 1. 直接往 sqlite 写入 3 条 semantic_cache_entries(模拟上次进程写入)。
    /// 2. 新建一个 SemanticCache(entries map 为空)。
    /// 3. 调用 prewarm_from_store(sqlite, 256)。
    /// 4. 验证 cache.len() == 3,且每条可查到。
    #[tokio::test]
    async fn prewarm_restores_entries() {
        let db_path = temp_db_path();
        let lance_path = temp_lance_path();
        let (cache, sqlite, _, _) = make_cache(&db_path, &lance_path, 4).await;

        // 1. 往 sqlite 写入 3 条 entries(模拟上次进程的 store() 写入)。
        for i in 0..3 {
            sqlite
                .insert_semantic_cache_entry(&format!("sem:hash-{i}"), &format!("response-{i}"))
                .await
                .unwrap();
        }

        // 2. 验证 cache 初始为空。
        assert_eq!(cache.len(), 0, "cache must be empty before prewarm");

        // 3. 预热。
        cache.prewarm_from_store(&sqlite, 256).await;

        // 4. 验证恢复。
        assert_eq!(cache.len(), 3, "cache must have 3 entries after prewarm");

        cleanup(db_path, lance_path);
    }

    /// T-E-D-01: prewarm limit 截断 — 只拉最近 N 条,不拉全表。
    #[tokio::test]
    async fn prewarm_respects_limit() {
        let db_path = temp_db_path();
        let lance_path = temp_lance_path();
        let (cache, sqlite, _, _) = make_cache(&db_path, &lance_path, 4).await;

        for i in 0..5 {
            sqlite
                .insert_semantic_cache_entry(&format!("sem:limit-{i}"), &format!("resp-{i}"))
                .await
                .unwrap();
        }

        cache.prewarm_from_store(&sqlite, 2).await;
        assert_eq!(cache.len(), 2, "limit=2 must clamp to 2 entries");

        cleanup(db_path, lance_path);
    }

    /// T-E-D-01: store() 应同时写入 entries map + SQLite(若启用 sqlite)。
    ///
    /// 验证:
    /// 1. 调用 store(query, response) 后,cache.len() == 1(entries map 写入)。
    /// 2. sqlite.query_semantic_cache_entry(stable_id(query)) 返回 Some(response)。
    #[tokio::test]
    async fn store_writes_to_sqlite_when_enabled() {
        let db_path = temp_db_path();
        let lance_path = temp_lance_path();
        let (cache, sqlite, _, embedder) = make_cache(&db_path, &lance_path, 4).await;

        // 预填 embedder 缓存,跳过 Ollama 网络调用(127.0.0.1:1 强制失败)。
        embedder.seed_cache_for_test("hello cache", vec![0.1, 0.2, 0.3, 0.4]);

        // store() 应写入 entries map + sqlite。
        cache.store("hello cache", "cached response").await;

        // 1. entries map 写入。
        assert_eq!(cache.len(), 1, "entries map must have 1 entry after store");

        // 2. sqlite 写入。
        let id = stable_id("hello cache");
        let sqlite_resp = sqlite
            .query_semantic_cache_entry(&id)
            .await
            .unwrap()
            .expect("sqlite must have entry after store");
        assert_eq!(sqlite_resp, "cached response");

        cleanup(db_path, lance_path);
    }

    /// T-E-D-01: 模拟"重启" — entries map 清空后,check() 应回退查 SQLite。
    ///
    /// 场景:
    /// 1. store(query, response) 写入 entries map + lance + sqlite。
    /// 2. cache.clear() 模拟进程重启(entries map 清空,lance 仍在)。
    /// 3. check(query) 应走 LanceDB 命中 → entries map miss → sqlite fallback → 命中。
    #[tokio::test]
    async fn check_falls_back_to_sqlite_after_clear() {
        let db_path = temp_db_path();
        let lance_path = temp_lance_path();
        let (cache, sqlite, _, embedder) = make_cache(&db_path, &lance_path, 4).await;

        // 1. 预填 embedder 缓存 + store(query, response)。
        let query = "fallback test";
        let response = "fallback response";
        // embed(query) 与 embed(同 query) 返回相同向量,LanceDB 自匹配。
        embedder.seed_cache_for_test(query, vec![1.0, 0.0, 0.0, 0.0]);
        cache.store(query, response).await;

        // 验证 sqlite 已写入。
        let id = stable_id(query);
        assert!(
            sqlite
                .query_semantic_cache_entry(&id)
                .await
                .unwrap()
                .is_some(),
            "sqlite must have entry after store"
        );

        // 2. 模拟重启:清空 entries map(lance 仍在)。
        cache.clear();
        assert_eq!(cache.len(), 0, "entries map must be empty after clear");

        // 3. check() 应走 LanceDB → entries map miss → sqlite fallback → 命中。
        let got = cache.check(query).await;
        assert_eq!(
            got,
            Some(response.to_string()),
            "check() must fall back to sqlite and return response"
        );

        // 4. 验证 entries map 被回填(下次命中走内存路径)。
        assert_eq!(cache.len(), 1, "entries map must be backfilled after check");

        cleanup(db_path, lance_path);
    }

    /// T-E-D-01: 未启用 sqlite 时,行为与 T-E-A-01 完全一致。
    ///
    /// 验证:无 sqlite 的 SemanticCache store() 后 entries map 有 1 条,
    /// 但 sqlite 表中无对应记录(避免不必要的 IO)。
    #[tokio::test]
    async fn store_skips_sqlite_when_not_configured() {
        let db_path = temp_db_path();
        let lance_path = temp_lance_path();
        let lance = Arc::new(LanceStore::open(&lance_path, 4).await.unwrap());
        let embedder = Arc::new(Embedder::new(
            OllamaClient::new("http://127.0.0.1:1"),
            "test-model",
            4,
        ));
        // 不调用 with_sqlite — sqlite 字段为 None。
        let cache = SemanticCache::default_config(lance, embedder.clone());
        let sqlite = Arc::new(SqliteStore::open(&db_path).unwrap());

        embedder.seed_cache_for_test("no sqlite", vec![0.5, 0.5, 0.5, 0.5]);
        cache.store("no sqlite", "resp").await;
        assert_eq!(cache.len(), 1, "entries map must have 1 entry");

        // sqlite 表中应无对应记录(因为未启用持久化)。
        let id = stable_id("no sqlite");
        let got = sqlite.query_semantic_cache_entry(&id).await.unwrap();
        assert!(got.is_none(), "sqlite must not have entry when not configured");

        cleanup(db_path, lance_path);
    }
}

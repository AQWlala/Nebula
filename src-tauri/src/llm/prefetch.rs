//! T-E-A-11: 智能预取引擎。
//!
//! 用户打开文件时,预取该文件相关的历史对话缓存到 SemanticCache,
//! 后续 LLM 调用命中缓存跳过推理。三路检索融合:路径 LIKE + BM25
//! 文件名 + 向量检索,过滤 chat channel memory,turn_id 配对。
//!
//! ## 设计要点
//!
//! * **零新依赖**:复用 SqliteStore + Bm25Searcher + LanceStore +
//!   Embedder + SemanticCache,Cargo.toml 不变。
//! * **三路检索融合**:路径 LIKE(参数化绑定)+ BM25 文件名 +
//!   向量检索,过滤 metadata.channel LIKE 'chat.%'。
//! * **turn_id 配对**:user memory 与 assistant memory 通过
//!   metadata.turn_id 精确配对;turn_id 缺失时按 created_at ±30s
//!   就近匹配下一条 chat.assistant memory。
//! * **5 分钟去重**:`parking_lot::Mutex<HashMap<PathBuf, Instant>>`,
//!   PathBuf::canonicalize() 标准化路径。
//! * **K=10 上限**:max_pairs_per_file 截断,防 LanceDB 写放大。
//! * **降级**:embed 失败/路径不存在/无历史 全部降级为 debug 日志,
//!   不 panic。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::llm::semantic_cache::SemanticCache;
use crate::memory::bm25::Bm25Searcher;
use crate::memory::embedder::Embedder;
// T-E-S-42: PrefetchEngine 面向 VectorStore trait 编程,可接受任意后端。
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::types::Memory;
use crate::memory::vector_store::VectorStore;

/// 默认去重窗口:5 分钟。
pub const DEFAULT_DEDUP_WINDOW: Duration = Duration::from_secs(300);

/// 默认每文件最大配对数:10(防 LanceDB 写放大)。
pub const DEFAULT_MAX_PAIRS_PER_FILE: usize = 10;

/// 默认 BM25 检索 top-k。
pub const DEFAULT_TOP_K_BM25: usize = 20;

/// 默认向量检索 top-k。
pub const DEFAULT_TOP_K_VECTOR: usize = 20;

/// turn_id 缺失时,按 created_at 就近匹配的时间窗口(秒)。
const CREATED_AT_WINDOW_SECS: i64 = 30;

/// 预取配置。
#[derive(Debug, Clone)]
pub struct PrefetchConfig {
    /// 去重窗口,默认 5 分钟。
    pub dedup_window: Duration,
    /// 每文件最大配对数,默认 10。
    pub max_pairs_per_file: usize,
    /// BM25 检索 top-k,默认 20。
    pub top_k_bm25: usize,
    /// 向量检索 top-k,默认 20。
    pub top_k_vector: usize,
    /// 是否启用,默认 true。
    pub enabled: bool,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self {
            dedup_window: DEFAULT_DEDUP_WINDOW,
            max_pairs_per_file: DEFAULT_MAX_PAIRS_PER_FILE,
            top_k_bm25: DEFAULT_TOP_K_BM25,
            top_k_vector: DEFAULT_TOP_K_VECTOR,
            enabled: true,
        }
    }
}

/// 预取统计。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefetchStats {
    pub path: String,
    pub pairs_prefetched: usize,
    pub bm25_hits: usize,
    pub vector_hits: usize,
    pub path_hits: usize,
    pub skipped_dedup: bool,
    pub elapsed_ms: u64,
}

/// 一对配对好的 chat turn(user query + assistant response)。
#[derive(Debug, Clone)]
pub struct ChatTurnPair {
    pub user_query: String,
    pub assistant_response: String,
    pub user_id: String,
    pub assistant_id: String,
    pub turn_id: Option<String>,
    pub created_at: i64,
}

/// 路径 LIKE 检索返回的原始行。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct RawRow {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) metadata: serde_json::Value,
}

/// T-E-A-11: 智能预取引擎。
///
/// 持有 sqlite/lance/embedder/semantic_cache/bm25 句柄,以及进程内
/// 去重表 `recent`。`prefetch` 方法是主入口,执行三路检索 + 配对 +
/// 写入 SemanticCache 全流程。
pub struct PrefetchEngine {
    sqlite: Arc<SqliteStore>,
    lance: Arc<dyn VectorStore>,
    embedder: Arc<Embedder>,
    semantic_cache: Arc<SemanticCache>,
    bm25: Bm25Searcher,
    /// 去重表:canonicalize 后的路径 → 上次预取时刻。
    /// `parking_lot::Mutex` 非重入,持锁期间不调用其他用同一锁的方法。
    recent: Mutex<HashMap<PathBuf, Instant>>,
    config: PrefetchConfig,
}

impl PrefetchEngine {
    /// 创建预取引擎。`bm25` 从 sqlite 连接构造。
    pub fn new(
        sqlite: Arc<SqliteStore>,
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
        semantic_cache: Arc<SemanticCache>,
        config: PrefetchConfig,
    ) -> Self {
        let bm25 = Bm25Searcher::new(&sqlite);
        Self {
            sqlite,
            lance,
            embedder,
            semantic_cache,
            bm25,
            recent: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// 用默认配置构造。
    pub fn with_default_config(
        sqlite: Arc<SqliteStore>,
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
        semantic_cache: Arc<SemanticCache>,
    ) -> Self {
        Self::new(
            sqlite,
            lance,
            embedder,
            semantic_cache,
            PrefetchConfig::default(),
        )
    }

    /// 当前配置(只读引用)。
    pub fn config(&self) -> &PrefetchConfig {
        &self.config
    }

    /// 标准化路径。canonicalize 失败时回退到原路径。
    /// 处理 Windows 路径大小写/分隔符差异。
    pub fn canonicalize(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    /// 检查路径是否在去重窗口内被预取过。
    /// 使用 canonicalize 标准化路径后查询 `recent` 表。
    pub fn is_recent(&self, path: &Path) -> bool {
        let canon = Self::canonicalize(path);
        let now = Instant::now();
        // 块作用域释放锁,避免持锁期间调用其他方法。
        let g = self.recent.lock();
        if let Some(t) = g.get(&canon) {
            now.duration_since(*t) < self.config.dedup_window
        } else {
            false
        }
    }

    /// 记录预取时间戳(用于去重)。
    fn record_prefetch(&self, path: &Path) {
        let canon = Self::canonicalize(path);
        // 块作用域释放锁。
        let mut g = self.recent.lock();
        g.insert(canon, Instant::now());
    }

    /// 路径 LIKE 检索:`SELECT id,content,metadata FROM memories
    /// WHERE content LIKE ? OR metadata LIKE ?`。
    /// 参数化绑定 `%{path}%`,防 SQL 注入。
    pub(crate) async fn search_by_path_substring(&self, path: &str) -> Vec<RawRow> {
        let pattern = format!("%{path}%");
        let conn = self.sqlite.raw_connection();
        let pattern_for_task = pattern.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = conn.lock();
            let mut stmt = conn.prepare(
                "SELECT id, content, metadata FROM memories \
                 WHERE content LIKE ?1 OR metadata LIKE ?2",
            )?;
            let rows = stmt
                .query_map(params![pattern_for_task, pattern_for_task], |row| {
                    let id: String = row.get(0)?;
                    let content: String = row.get(1)?;
                    let metadata_s: String = row.get(2)?;
                    let metadata: serde_json::Value =
                        serde_json::from_str(&metadata_s).unwrap_or(serde_json::Value::Null);
                    Ok(RawRow {
                        id,
                        content,
                        metadata,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok::<_, anyhow::Error>(rows)
        })
        .await;
        match result {
            Ok(Ok(rows)) => rows,
            Ok(Err(e)) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "path LIKE search failed; degrading to empty"
                );
                Vec::new()
            }
            Err(e) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "spawn_blocking join error for path LIKE search"
                );
                Vec::new()
            }
        }
    }

    /// BM25 文件名检索。返回 (memory_id, score) 对,按相关性降序。
    pub async fn search_by_bm25(&self, file_name: &str) -> Vec<(String, f64)> {
        match self.bm25.search(file_name, self.config.top_k_bm25).await {
            Ok(hits) => hits,
            Err(e) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "bm25 search failed; degrading to empty"
                );
                Vec::new()
            }
        }
    }

    /// 向量检索:embed(file_name) → lance.search(vec, k) → 过滤
    /// metadata.channel LIKE 'chat.%'。
    /// embed 失败/lance 失败均降级为空 Vec,不 panic。
    pub async fn search_by_vector(&self, file_name: &str) -> Vec<String> {
        let vec = match self.embedder.embed(file_name).await {
            Ok(v) => v,
            Err(e) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "embed failed; skipping vector search"
                );
                return Vec::new();
            }
        };
        let hits = match self.lance.search(&vec, self.config.top_k_vector).await {
            Ok(h) => h,
            Err(e) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "lance search failed; degrading to empty"
                );
                return Vec::new();
            }
        };
        if hits.is_empty() {
            return Vec::new();
        }
        let ids: Vec<String> = hits.into_iter().map(|(id, _)| id).collect();
        let memories = match self.sqlite.get_many(&ids).await {
            Ok(m) => m,
            Err(e) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "get_many failed during vector search filter"
                );
                return Vec::new();
            }
        };
        // 过滤 metadata.channel LIKE 'chat.%'
        let filtered = filter_chat_channel(memories);
        filtered.into_iter().map(|m| m.id).collect()
    }

    /// 主入口:为指定文件路径预取相关历史对话。
    ///
    /// 流程:
    /// 1. 检查 enabled / 去重窗口
    /// 2. 提取文件名
    /// 3. 三路并行检索(路径 LIKE + BM25 + 向量)
    /// 4. 合并候选 id,批量获取完整 memory
    /// 5. 过滤 chat channel memory
    /// 6. turn_id / created_at 配对
    /// 7. 截断到 max_pairs_per_file
    /// 8. 写入 SemanticCache
    /// 9. 记录去重时间戳
    pub async fn prefetch(&self, path: &str) -> PrefetchStats {
        let start = Instant::now();
        let path_str = path.to_string();
        let path_buf = PathBuf::from(path);

        // disabled 时 no-op
        if !self.config.enabled {
            return PrefetchStats {
                path: path_str,
                pairs_prefetched: 0,
                bm25_hits: 0,
                vector_hits: 0,
                path_hits: 0,
                skipped_dedup: false,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        // 去重检查:5 分钟内不重复预取
        if self.is_recent(&path_buf) {
            return PrefetchStats {
                path: path_str,
                pairs_prefetched: 0,
                bm25_hits: 0,
                vector_hits: 0,
                path_hits: 0,
                skipped_dedup: true,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        // 提取文件名(用于 BM25 + 向量检索)
        let file_name = path_buf
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());

        // 三路并行检索
        let (path_rows, bm25_hits, vector_ids) = tokio::join!(
            self.search_by_path_substring(path),
            self.search_by_bm25(&file_name),
            self.search_by_vector(&file_name),
        );

        let path_hits = path_rows.len();
        let bm25_hits_count = bm25_hits.len();
        let vector_hits_count = vector_ids.len();

        // 合并所有候选 memory id
        let mut candidate_ids: HashSet<String> = HashSet::new();
        for row in &path_rows {
            candidate_ids.insert(row.id.clone());
        }
        for (id, _) in &bm25_hits {
            candidate_ids.insert(id.clone());
        }
        for id in &vector_ids {
            candidate_ids.insert(id.clone());
        }

        if candidate_ids.is_empty() {
            // 无历史:记录去重时间戳,返回 0
            self.record_prefetch(&path_buf);
            return PrefetchStats {
                path: path_str,
                pairs_prefetched: 0,
                bm25_hits: bm25_hits_count,
                vector_hits: vector_hits_count,
                path_hits,
                skipped_dedup: false,
                elapsed_ms: start.elapsed().as_millis() as u64,
            };
        }

        // 批量获取完整 memory 记录
        let ids: Vec<String> = candidate_ids.into_iter().collect();
        let memories = match self.sqlite.get_many(&ids).await {
            Ok(m) => m,
            Err(e) => {
                debug!(
                    target: "nebula.prefetch",
                    error = ?e, "get_many failed; degrading to 0 pairs"
                );
                self.record_prefetch(&path_buf);
                return PrefetchStats {
                    path: path_str,
                    pairs_prefetched: 0,
                    bm25_hits: bm25_hits_count,
                    vector_hits: vector_hits_count,
                    path_hits,
                    skipped_dedup: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                };
            }
        };

        // 过滤 chat channel memory
        let chat_memories = filter_chat_channel(memories);

        // turn_id / created_at 配对
        let pairs = pair_turns(chat_memories, CREATED_AT_WINDOW_SECS);

        // 截断到 max_pairs_per_file 上限
        let pairs = self.truncate_pairs(pairs);

        // 写入 SemanticCache
        let pairs_written = self.fill_semantic_cache(&pairs).await;

        // 记录去重时间戳
        self.record_prefetch(&path_buf);

        PrefetchStats {
            path: path_str,
            pairs_prefetched: pairs_written,
            bm25_hits: bm25_hits_count,
            vector_hits: vector_hits_count,
            path_hits,
            skipped_dedup: false,
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// 截断到 max_pairs_per_file 上限。
    fn truncate_pairs(&self, mut pairs: Vec<ChatTurnPair>) -> Vec<ChatTurnPair> {
        if pairs.len() > self.config.max_pairs_per_file {
            pairs.truncate(self.config.max_pairs_per_file);
        }
        pairs
    }

    /// 把配对写入 SemanticCache。返回写入的对数。
    /// 复用 SemanticCache::store(query, response),不新增 API。
    /// SemanticCache 内部对 embed/upsert 失败已降级,此处无需 try/catch。
    pub async fn fill_semantic_cache(&self, pairs: &[ChatTurnPair]) -> usize {
        let mut count = 0;
        for pair in pairs {
            self.semantic_cache
                .store(&pair.user_query, &pair.assistant_response)
                .await;
            count += 1;
        }
        count
    }
}

/// 过滤出 metadata.channel 以 "chat." 开头的 memory。
/// 向量检索后用此函数过滤非 chat channel 的记忆。
pub fn filter_chat_channel(memories: Vec<Memory>) -> Vec<Memory> {
    memories
        .into_iter()
        .filter(|m| {
            m.metadata
                .get("channel")
                .and_then(|c| c.as_str())
                .map(|c| c.starts_with("chat."))
                .unwrap_or(false)
        })
        .collect()
}

/// 配对 user memory 与 assistant memory。
///
/// 1. 优先按 metadata.turn_id 精确配对(双向匹配:user.turn_id ==
///    assistant.turn_id)。
/// 2. turn_id 缺失或未匹配时,按 created_at 就近匹配下一条
///    chat.assistant memory(created_at 差值 ≤ window_secs 且
///    assistant.created_at ≥ user.created_at)。
///
/// 每个 assistant 最多配对一次,避免重复写入 SemanticCache。
pub fn pair_turns(memories: Vec<Memory>, window_secs: i64) -> Vec<ChatTurnPair> {
    // 分离 user 与 assistant
    let mut user_mems: Vec<Memory> = Vec::new();
    let mut asst_mems: Vec<Memory> = Vec::new();
    for m in memories {
        let channel = m
            .metadata
            .get("channel")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        match channel {
            "chat.user" => user_mems.push(m),
            "chat.assistant" => asst_mems.push(m),
            _ => {}
        }
    }

    // assistant 按 created_at 升序排序,便于"就近匹配下一条"
    asst_mems.sort_by_key(|m| m.created_at);

    // 标记已被配对的 assistant id
    let mut paired_asst: HashSet<String> = HashSet::new();
    let mut pairs: Vec<ChatTurnPair> = Vec::new();

    // 第一轮:按 turn_id 精确配对
    for user in &user_mems {
        let user_turn_id = user.metadata.get("turn_id").and_then(|t| t.as_str());
        if let Some(tid) = user_turn_id {
            // 找到 assistant 中 turn_id 匹配且未被配对的
            if let Some(asst) = asst_mems.iter().find(|a| {
                !paired_asst.contains(&a.id)
                    && a.metadata.get("turn_id").and_then(|t| t.as_str()) == Some(tid)
            }) {
                pairs.push(ChatTurnPair {
                    user_query: user.content.clone(),
                    assistant_response: asst.content.clone(),
                    user_id: user.id.clone(),
                    assistant_id: asst.id.clone(),
                    turn_id: Some(tid.to_string()),
                    created_at: user.created_at,
                });
                paired_asst.insert(asst.id.clone());
            }
        }
    }

    // 第二轮:对未配对的 user,按 created_at 就近匹配下一条 assistant
    for user in &user_mems {
        // 已在第一轮配对的跳过
        if pairs.iter().any(|p| p.user_id == user.id) {
            continue;
        }

        // 找到 created_at >= user.created_at 且差值 ≤ window_secs 的
        // 最近 assistant(下一条回复)
        let mut best: Option<&Memory> = None;
        let mut best_diff: i64 = i64::MAX;
        for asst in &asst_mems {
            if paired_asst.contains(&asst.id) {
                continue;
            }
            let diff = asst.created_at - user.created_at;
            if diff >= 0 && diff <= window_secs && diff < best_diff {
                best_diff = diff;
                best = Some(asst);
            }
        }
        if let Some(asst) = best {
            pairs.push(ChatTurnPair {
                user_query: user.content.clone(),
                assistant_response: asst.content.clone(),
                user_id: user.id.clone(),
                assistant_id: asst.id.clone(),
                turn_id: None,
                created_at: user.created_at,
            });
            paired_asst.insert(asst.id.clone());
        }
    }

    // 按 user.created_at 升序排序,保持对话时间顺序
    pairs.sort_by_key(|p| p.created_at);
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    // T-E-S-42: 测试中直接构造 LanceStore 作为 VectorStore trait 对象。
    use crate::llm::ollama::OllamaClient;
    use crate::memory::lance_store::LanceStore;
    use crate::memory::types::{MemoryLayer, MemoryType, MultiGranularity, SourceKind};
    use std::env;

    fn temp_db_path() -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("nebula_prefetch_test_{}.db", uuid::Uuid::new_v4()));
        p
    }

    fn temp_lance_path() -> std::path::PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("nebula_prefetch_lance_{}", uuid::Uuid::new_v4()));
        p
    }

    /// 构造一个 chat memory(id/content/channel/turn_id/created_at 可控)。
    fn make_chat_memory(
        id: &str,
        content: &str,
        channel: &str,
        turn_id: Option<&str>,
        created_at: i64,
    ) -> Memory {
        let source = if channel == "chat.user" {
            SourceKind::UserInput
        } else {
            SourceKind::AgentOutput
        };
        let mut m = Memory::new(MemoryType::Episodic, MemoryLayer::L1, content, source);
        m.id = id.to_string();
        m.created_at = created_at;
        m.last_access = created_at;
        let metadata = match turn_id {
            Some(tid) => serde_json::json!({ "channel": channel, "turn_id": tid }),
            None => serde_json::json!({ "channel": channel }),
        };
        m.metadata = metadata;
        m.summary = MultiGranularity::new(content, content, content, content);
        m
    }

    /// 构造一个非 chat channel 的 memory(用于验证过滤)。
    fn make_other_memory(id: &str, content: &str, channel: &str) -> Memory {
        let mut m = Memory::new(
            MemoryType::Semantic,
            MemoryLayer::L3,
            content,
            SourceKind::UserInput,
        );
        m.id = id.to_string();
        m.metadata = serde_json::json!({ "channel": channel });
        m.summary = MultiGranularity::new(content, content, content, content);
        m
    }

    /// 构造测试引擎:dim=4,默认配置。返回 (engine, db_path, lance_path)。
    async fn make_engine(dim: usize) -> (PrefetchEngine, std::path::PathBuf, std::path::PathBuf) {
        make_engine_with_config(dim, PrefetchConfig::default()).await
    }

    /// 构造测试引擎:dim=4,自定义配置。
    async fn make_engine_with_config(
        dim: usize,
        config: PrefetchConfig,
    ) -> (PrefetchEngine, std::path::PathBuf, std::path::PathBuf) {
        let db_path = temp_db_path();
        let lance_path = temp_lance_path();
        let sqlite = Arc::new(SqliteStore::open(&db_path).expect("create should succeed"));
        let lance = Arc::new(
            LanceStore::open(&lance_path, dim)
                .await
                .expect("create should succeed"),
        );
        let embedder = Arc::new(Embedder::new(
            OllamaClient::new("http://127.0.0.1:1"),
            "test-model",
            dim,
        ));
        let semantic_cache = Arc::new(SemanticCache::default_config(
            lance.clone(),
            embedder.clone(),
        ));
        let engine = PrefetchEngine::new(sqlite, lance, embedder, semantic_cache, config);
        (engine, db_path, lance_path)
    }

    fn cleanup(db: std::path::PathBuf, lance: std::path::PathBuf) {
        let _ = std::fs::remove_file(db);
        let _ = std::fs::remove_dir_all(lance);
    }

    // =====================================================================
    // 单测 1:PrefetchConfig 默认值
    // =====================================================================
    #[test]
    fn test_prefetch_config_default_values() {
        let cfg = PrefetchConfig::default();
        assert_eq!(cfg.dedup_window, DEFAULT_DEDUP_WINDOW);
        assert_eq!(cfg.dedup_window.as_secs(), 300);
        assert_eq!(cfg.max_pairs_per_file, DEFAULT_MAX_PAIRS_PER_FILE);
        assert_eq!(cfg.max_pairs_per_file, 10);
        assert_eq!(cfg.top_k_bm25, DEFAULT_TOP_K_BM25);
        assert_eq!(cfg.top_k_bm25, 20);
        assert_eq!(cfg.top_k_vector, DEFAULT_TOP_K_VECTOR);
        assert_eq!(cfg.top_k_vector, 20);
        assert!(cfg.enabled);
    }

    // =====================================================================
    // 单测 2:is_recent 5 分钟窗口
    // =====================================================================
    #[tokio::test]
    async fn test_is_recent_within_5min_window() {
        let (engine, db, lance) = make_engine(4).await;
        let path = std::path::PathBuf::from("/nonexistent/test.txt");

        // 初始:未预取过 → not recent
        assert!(!engine.is_recent(&path));

        // 记录预取 → recent(在 5 分钟窗口内)
        engine.record_prefetch(&path);
        assert!(engine.is_recent(&path));

        // 模拟 6 分钟前的时间戳 → not recent(超出 5 分钟窗口)
        let old = Instant::now()
            .checked_sub(Duration::from_secs(360))
            .expect("system uptime should exceed 6 minutes");
        {
            let mut g = engine.recent.lock();
            let canon = PrefetchEngine::canonicalize(&path);
            g.insert(canon, old);
        }
        assert!(!engine.is_recent(&path));

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 3:canonicalize 路径标准化
    // =====================================================================
    #[test]
    fn test_canonicalize_normalizes_path() {
        // 对存在的路径,canonicalize 返回绝对规范路径
        let temp = env::temp_dir();
        let canon = PrefetchEngine::canonicalize(&temp);
        assert!(
            canon.is_absolute(),
            "canonicalized temp dir should be absolute"
        );

        // 对不存在的路径,canonicalize 回退到原路径(不 panic)
        let fake = std::path::PathBuf::from("/definitely/not/exist/path.txt");
        let canon = PrefetchEngine::canonicalize(&fake);
        assert_eq!(canon, fake);

        // 同一不存在路径多次调用结果一致(可用于去重 key)
        let canon2 = PrefetchEngine::canonicalize(&fake);
        assert_eq!(canon, canon2);
    }

    // =====================================================================
    // 单测 4:search_by_path_substring SQL 参数化(注入安全)
    // =====================================================================
    #[tokio::test]
    async fn test_search_by_path_substring_parameterized() {
        let (engine, db, lance) = make_engine(4).await;

        // 插入一条包含路径的 memory
        let m = make_chat_memory(
            "path-test-1",
            "讨论 /tmp/foo/bar.txt 的实现",
            "chat.user",
            None,
            1000,
        );
        engine
            .sqlite
            .insert(&m)
            .await
            .expect("insert should succeed");

        // 正常路径检索:应返回 1 行
        let rows = engine.search_by_path_substring("/tmp/foo/bar.txt").await;
        assert_eq!(rows.len(), 1, "should match the path substring");
        assert_eq!(rows[0].id, "path-test-1");

        // SQL 注入尝试:参数化绑定应将其视为字面量,不执行注入
        let injection = "'; DROP TABLE memories; --";
        let rows = engine.search_by_path_substring(injection).await;
        assert!(
            rows.is_empty(),
            "SQL injection string should not match any row"
        );

        // 验证 memories 表未被 DROP(仍可查询)
        let count = engine.sqlite.count().await.expect("task should complete");
        assert_eq!(count, 1, "memories table should still have 1 row");

        // 另一种注入尝试:OR 1=1
        let injection2 = "' OR '1'='1";
        let rows = engine.search_by_path_substring(injection2).await;
        assert!(rows.is_empty(), "OR 1=1 injection should not match any row");

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 5:BM25 文件名检索排序
    // =====================================================================
    #[tokio::test]
    async fn test_bm25_search_returns_relevant_sorted() {
        let (engine, db, lance) = make_engine(4).await;

        // 插入三条 memory,文件名 "main.rs" 出现在两条中
        let m1 = make_chat_memory(
            "bm25-1",
            "main.rs 的入口函数怎么写",
            "chat.user",
            None,
            1000,
        );
        let m2 = make_chat_memory(
            "bm25-2",
            "完全无关的内容 about something else",
            "chat.user",
            None,
            1001,
        );
        let m3 = make_chat_memory(
            "bm25-3",
            "main.rs 里的 main.rs 引用",
            "chat.user",
            None,
            1002,
        );
        engine
            .sqlite
            .insert(&m1)
            .await
            .expect("insert should succeed");
        engine
            .sqlite
            .insert(&m2)
            .await
            .expect("insert should succeed");
        engine
            .sqlite
            .insert(&m3)
            .await
            .expect("insert should succeed");

        let hits = engine.search_by_bm25("main.rs").await;
        let ids: Vec<&str> = hits.iter().map(|(id, _)| id.as_str()).collect();

        // m1 和 m3 包含 "main.rs",m2 不包含
        assert!(ids.contains(&"bm25-1"), "bm25-1 should match: {ids:?}");
        assert!(ids.contains(&"bm25-3"), "bm25-3 should match: {ids:?}");
        assert!(!ids.contains(&"bm25-2"), "bm25-2 should not match: {ids:?}");

        // 分数降序(更高 = 更好)
        for w in hits.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "scores should be descending: {} vs {}",
                w[0].1,
                w[1].1
            );
        }

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 6:向量检索过滤 chat channel
    // =====================================================================
    #[tokio::test]
    async fn test_vector_search_filters_chat_channel() {
        let dim = 4;
        let (engine, db, lance) = make_engine(dim).await;

        // 预填充 embedder 缓存:file_name → 固定向量
        let file_name = "test_vector_file.txt";
        let vec = vec![1.0, 0.0, 0.0, 0.0];
        engine.embedder.seed_cache_for_test(file_name, vec.clone());

        // 插入三条 memory 到 sqlite + lance(同一向量)
        let m_user = make_chat_memory("vec-user", "用户问题", "chat.user", None, 1000);
        let m_asst = make_chat_memory("vec-asst", "助手回答", "chat.assistant", None, 1001);
        let m_other = make_other_memory("vec-other", "系统记忆", "system");
        engine
            .sqlite
            .insert(&m_user)
            .await
            .expect("insert should succeed");
        engine
            .sqlite
            .insert(&m_asst)
            .await
            .expect("insert should succeed");
        engine
            .sqlite
            .insert(&m_other)
            .await
            .expect("insert should succeed");
        engine
            .lance
            .upsert("vec-user", &vec)
            .await
            .expect("task should complete");
        engine
            .lance
            .upsert("vec-asst", &vec)
            .await
            .expect("task should complete");
        engine
            .lance
            .upsert("vec-other", &vec)
            .await
            .expect("task should complete");

        // 向量检索:应返回 2 条(chat.user + chat.assistant),过滤掉 system
        let ids = engine.search_by_vector(file_name).await;
        assert_eq!(ids.len(), 2, "should return 2 chat.* memories, got {ids:?}");
        assert!(ids.contains(&"vec-user".to_string()));
        assert!(ids.contains(&"vec-asst".to_string()));
        assert!(!ids.contains(&"vec-other".to_string()));

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 7:turn_id 精确配对
    // =====================================================================
    #[test]
    fn test_pair_by_turn_id_exact() {
        let user1 = make_chat_memory("u1", "问题1", "chat.user", Some("turn-aaa"), 1000);
        let asst1 = make_chat_memory("a1", "回答1", "chat.assistant", Some("turn-aaa"), 1001);
        let user2 = make_chat_memory("u2", "问题2", "chat.user", Some("turn-bbb"), 2000);
        let asst2 = make_chat_memory("a2", "回答2", "chat.assistant", Some("turn-bbb"), 2001);

        let memories = vec![user1, asst1, user2, asst2];
        let pairs = pair_turns(memories, CREATED_AT_WINDOW_SECS);

        assert_eq!(pairs.len(), 2, "should pair 2 turns by turn_id");
        // 第一对
        assert_eq!(pairs[0].user_query, "问题1");
        assert_eq!(pairs[0].assistant_response, "回答1");
        assert_eq!(pairs[0].turn_id, Some("turn-aaa".to_string()));
        // 第二对
        assert_eq!(pairs[1].user_query, "问题2");
        assert_eq!(pairs[1].assistant_response, "回答2");
        assert_eq!(pairs[1].turn_id, Some("turn-bbb".to_string()));
    }

    // =====================================================================
    // 单测 8:turn_id 缺失按 created_at 就近匹配
    // =====================================================================
    #[test]
    fn test_pair_by_created_at_when_turn_id_missing() {
        let user1 = make_chat_memory("u1", "问题1", "chat.user", None, 1000);
        let asst1 = make_chat_memory("a1", "回答1", "chat.assistant", None, 1005);
        let user2 = make_chat_memory("u2", "问题2", "chat.user", None, 2000);
        // asst2 在 25 秒后(≤30s 窗口内)
        let asst2 = make_chat_memory("a2", "回答2", "chat.assistant", None, 2025);
        // asst3 在 60 秒后(超出 30s 窗口),不应配对
        let user3 = make_chat_memory("u3", "问题3", "chat.user", None, 3000);
        let asst3 = make_chat_memory("a3", "回答3", "chat.assistant", None, 3060);

        let memories = vec![user1, asst1, user2, asst2, user3, asst3];
        let pairs = pair_turns(memories, CREATED_AT_WINDOW_SECS);

        // user1↔asst1(5s 差),user2↔asst2(25s 差),user3 无配对(60s 超 30s 窗口)
        assert_eq!(pairs.len(), 2, "should pair 2 turns by created_at");
        assert_eq!(pairs[0].user_query, "问题1");
        assert_eq!(pairs[0].assistant_response, "回答1");
        assert!(pairs[0].turn_id.is_none());
        assert_eq!(pairs[1].user_query, "问题2");
        assert_eq!(pairs[1].assistant_response, "回答2");
        assert!(pairs[1].turn_id.is_none());
    }

    // =====================================================================
    // 单测 9:fill_semantic_cache 写入后 len 增加
    // =====================================================================
    #[tokio::test]
    async fn test_fill_semantic_cache_increases_len() {
        let dim = 4;
        let (engine, db, lance) = make_engine(dim).await;

        // 预填充 embedder 缓存:每个 user_query 都返回固定向量
        let vec = vec![1.0, 0.0, 0.0, 0.0];
        engine.embedder.seed_cache_for_test("query-A", vec.clone());
        engine.embedder.seed_cache_for_test("query-B", vec.clone());

        let pairs = vec![
            ChatTurnPair {
                user_query: "query-A".to_string(),
                assistant_response: "response-A".to_string(),
                user_id: "u1".to_string(),
                assistant_id: "a1".to_string(),
                turn_id: None,
                created_at: 1000,
            },
            ChatTurnPair {
                user_query: "query-B".to_string(),
                assistant_response: "response-B".to_string(),
                user_id: "u2".to_string(),
                assistant_id: "a2".to_string(),
                turn_id: None,
                created_at: 2000,
            },
        ];

        let before = engine.semantic_cache.len();
        assert_eq!(before, 0, "cache should be empty initially");

        let written = engine.fill_semantic_cache(&pairs).await;
        assert_eq!(written, 2);

        let after = engine.semantic_cache.len();
        assert_eq!(
            after, 2,
            "cache len should increase by 2 after filling 2 pairs"
        );

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 10:fill_semantic_cache_respects_max_pairs_limit
    // =====================================================================
    #[tokio::test]
    async fn test_fill_semantic_cache_respects_max_pairs_limit() {
        let dim = 4;
        let config = PrefetchConfig {
            max_pairs_per_file: 3,
            ..Default::default()
        };
        let (engine, db, lance) = make_engine_with_config(dim, config).await;

        // 预填充 embedder 缓存:5 个不同 query
        let vec = vec![1.0, 0.0, 0.0, 0.0];
        for i in 0..5 {
            engine
                .embedder
                .seed_cache_for_test(&format!("query-{i}"), vec.clone());
        }

        // 构造 5 对(超过 max=3)
        let memories: Vec<Memory> = (0..5)
            .map(|i| {
                let u = make_chat_memory(
                    &format!("u{i}"),
                    &format!("query-{i}"),
                    "chat.user",
                    None,
                    1000 + i as i64,
                );
                let _a = make_chat_memory(
                    &format!("a{i}"),
                    &format!("response-{i}"),
                    "chat.assistant",
                    None,
                    1001 + i as i64,
                );
                // 返回 user 即可,pair_turns 会配对
                u
            })
            .collect();
        let mut all = memories;
        for i in 0..5 {
            all.push(make_chat_memory(
                &format!("a{i}"),
                &format!("response-{i}"),
                "chat.assistant",
                None,
                1001 + i as i64,
            ));
        }
        let pairs = pair_turns(all, CREATED_AT_WINDOW_SECS);
        assert_eq!(pairs.len(), 5, "should pair 5 turns before truncation");

        // 截断到 max=3
        let truncated = engine.truncate_pairs(pairs);
        assert_eq!(
            truncated.len(),
            3,
            "should truncate to max_pairs_per_file=3"
        );

        // 预填充剩余 3 个 query 的 embedder 缓存
        for p in &truncated {
            engine
                .embedder
                .seed_cache_for_test(&p.user_query, vec.clone());
        }

        let written = engine.fill_semantic_cache(&truncated).await;
        assert_eq!(written, 3);
        assert_eq!(
            engine.semantic_cache.len(),
            3,
            "cache should have 3 entries"
        );

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 11:prefetch 无历史返回 0
    // =====================================================================
    #[tokio::test]
    async fn test_prefetch_no_history_returns_zero() {
        let (engine, db, lance) = make_engine(4).await;

        // 空数据库,无任何历史
        let stats = engine.prefetch("/nonexistent/empty-file.txt").await;

        assert_eq!(stats.pairs_prefetched, 0, "should prefetch 0 pairs");
        assert_eq!(stats.path_hits, 0);
        assert_eq!(stats.bm25_hits, 0);
        // 向量检索因 embed 失败降级为 0
        assert_eq!(stats.vector_hits, 0);
        assert!(!stats.skipped_dedup);
        assert_eq!(stats.path, "/nonexistent/empty-file.txt");

        cleanup(db, lance);
    }

    // =====================================================================
    // 单测 12:prefetch disabled 时 noop
    // =====================================================================
    #[tokio::test]
    async fn test_prefetch_disabled_is_noop() {
        let config = PrefetchConfig {
            enabled: false,
            ..Default::default()
        };
        let (engine, db, lance) = make_engine_with_config(4, config).await;

        // 即使数据库有历史,disabled 时也应返回 0
        let m = make_chat_memory("u1", "问题 /tmp/foo.txt", "chat.user", None, 1000);
        engine
            .sqlite
            .insert(&m)
            .await
            .expect("insert should succeed");

        let stats = engine.prefetch("/tmp/foo.txt").await;

        assert_eq!(stats.pairs_prefetched, 0, "disabled should be noop");
        assert!(!stats.skipped_dedup, "disabled should not check dedup");

        cleanup(db, lance);
    }

    // =====================================================================
    // 补充单测:filter_chat_channel 过滤逻辑
    // =====================================================================
    #[test]
    fn test_filter_chat_channel() {
        let memories = vec![
            make_chat_memory("u1", "q", "chat.user", None, 1000),
            make_chat_memory("a1", "r", "chat.assistant", None, 1001),
            make_other_memory("s1", "other", "system"),
            make_other_memory("s2", "other2", "file.watcher"),
        ];
        let filtered = filter_chat_channel(memories);
        assert_eq!(filtered.len(), 2, "should keep only chat.* channels");
        let ids: Vec<&str> = filtered.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"u1"));
        assert!(ids.contains(&"a1"));
    }

    // =====================================================================
    // 补充单测:去重 5 分钟内重复预取被跳过
    // =====================================================================
    #[tokio::test]
    async fn test_prefetch_dedup_skips_recent() {
        let (engine, db, lance) = make_engine(4).await;

        // 第一次预取(无历史,返回 0 但记录时间戳)
        let stats1 = engine.prefetch("/nonexistent/dedup-test.txt").await;
        assert!(!stats1.skipped_dedup, "first call should not skip");
        assert_eq!(stats1.pairs_prefetched, 0);

        // 第二次预取(5 分钟内,应跳过)
        let stats2 = engine.prefetch("/nonexistent/dedup-test.txt").await;
        assert!(
            stats2.skipped_dedup,
            "second call within window should skip"
        );
        assert_eq!(stats2.pairs_prefetched, 0);

        cleanup(db, lance);
    }
}

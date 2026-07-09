//! T-D-B-14: Sidecar Memory 服务 — 单二进制多角色方案。
//!
//! 延续 T-S4-B-01 / T-S4-B-02 / T-S6-A-01a 的单二进制多角色方案
//! (`nebula-sidecar --kind=memory`),为记忆存储 + 向量搜索提供
//! 独立进程隔离。
//!
//! 本模块定义 Memory sidecar 的服务处理器 [`MemoryServiceHandler`],
//! 它包装 [`SqliteStore`](crate::memory::SqliteStore) 持久化后端,
//! 并可选注入 [`VectorStore`](crate::memory::VectorStore) +
//! [`Embedder`](crate::memory::Embedder) 启用向量检索。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  gRPC (tonic)
//!    ▼
//! nebula-sidecar --kind=memory  (监听 127.0.0.1:50051)
//!    │  MemoryServiceHandler
//!    ▼
//! SqliteStore (结构化记忆) + VectorStore (向量检索) + Embedder
//!    │
//!    ▼
//! SQLite + LanceDB
//! ```
//!
//! ## RPC 映射
//!
//! | gRPC RPC          | Handler 方法        | 后端                     |
//! |-------------------|---------------------|--------------------------|
//! | `Store`           | `store_memory`      | SqliteStore::insert      |
//! | `Get`             | `get_memory`        | SqliteStore::get         |
//! | `Search`          | `search_memory`     | Embedder + VectorStore   |
//! | `ListRecent`      | `list_recent`       | SqliteStore::list_recent |
//! | `HealthCheck`     | `health_check`      | (always ok)              |
//!
//! ## 依赖
//!
//! * `SqliteStore` 必须注入(必备后端)。
//! * `VectorStore` + `Embedder` 可选注入(未注入时 `search_memory`
//!   返回 `Err`,日志 warn,其他 RPC 不受影响)。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tracing::{info, instrument, warn};

use crate::memory::embedder::Embedder;
use crate::memory::sqlite_store::SqliteStore;
use crate::memory::types::{Memory, MemoryLayer, MemoryType, SourceKind};
use crate::memory::vector_store::VectorStore;

/// Memory sidecar 服务处理器。
///
/// 包装 [`SqliteStore`] 提供结构化记忆存储;可选注入 `VectorStore` +
/// `Embedder` 启用向量检索。在进程内模式下也可直接使用(无需 gRPC)。
pub struct MemoryServiceHandler {
    /// 持久化后端(必备)。
    sqlite: Arc<SqliteStore>,
    /// 向量检索后端(可选,未注入时 `search_memory` 返回 Err)。
    vector_store: Option<Arc<dyn VectorStore>>,
    /// 嵌入器(可选,与 `vector_store` 配套使用)。
    embedder: Option<Arc<Embedder>>,
}

impl MemoryServiceHandler {
    /// 创建新的 Memory 服务处理器(仅 SQLite 后端,无向量检索)。
    ///
    /// 通常在 sidecar 进程启动时构造。如需启用 `search_memory`,
    /// 通过 [`with_vector_store`](Self::with_vector_store) 注入。
    pub fn new(sqlite: Arc<SqliteStore>) -> Self {
        info!(
            target: "nebula.sidecar.memory",
            "MemoryServiceHandler initialized (sqlite-only)"
        );
        Self {
            sqlite,
            vector_store: None,
            embedder: None,
        }
    }

    /// 注入向量检索后端 + 嵌入器,builder 风格。
    ///
    /// 启用后 `search_memory` 可用。两者必须同时注入(向量检索需要
    /// 先用 embedder 把 query 转为向量,再交给 vector_store 检索)。
    pub fn with_vector_store(
        mut self,
        vector_store: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
    ) -> Self {
        info!(
            target: "nebula.sidecar.memory",
            "MemoryServiceHandler: vector search enabled"
        );
        self.vector_store = Some(vector_store);
        self.embedder = Some(embedder);
        self
    }

    /// 访问底层 SqliteStore(供 IPC 客户端在进程内模式下直接调用)。
    pub fn sqlite(&self) -> &Arc<SqliteStore> {
        &self.sqlite
    }

    /// 是否已启用向量检索。
    pub fn has_vector_search(&self) -> bool {
        self.vector_store.is_some() && self.embedder.is_some()
    }

    /// RPC: HealthCheck — 始终返回 Ok(若 handler 存在则服务可用)。
    pub async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// RPC: Store — 存储一条记忆。
    ///
    /// 根据传入的原始字段构造 [`Memory`] 并写入 SQLite。
    /// 返回新分配的记忆 id (UUIDv4)。
    #[instrument(skip(self, content), fields(type = %memory_type.as_str(), layer = %layer.as_str()))]
    pub async fn store_memory(
        &self,
        content: String,
        memory_type: MemoryType,
        layer: MemoryLayer,
        source: SourceKind,
    ) -> Result<String> {
        let memory = Memory::new(memory_type, layer, content, source);
        let id = memory.id.clone();
        self.sqlite.insert(&memory).await?;
        Ok(id)
    }

    /// RPC: Get — 按 id 查询单条记忆。
    #[instrument(skip(self), fields(id = %id))]
    pub async fn get_memory(&self, id: &str) -> Result<Option<Memory>> {
        self.sqlite.get(id).await
    }

    /// RPC: Search — 向量检索 top-k 相关记忆。
    ///
    /// 返回 `(memory_id, score)` 列表,按相似度降序。
    ///
    /// 未注入 `VectorStore` + `Embedder` 时返回 `Err`(日志 warn)。
    #[instrument(skip(self, query), fields(k = k))]
    pub async fn search_memory(&self, query: String, k: u32) -> Result<Vec<(String, f32)>> {
        let embedder = self.embedder.as_ref().ok_or_else(|| {
            warn!(
                target: "nebula.sidecar.memory",
                "search_memory called but embedder not configured"
            );
            anyhow!("vector search not configured: embedder missing")
        })?;
        let vector_store = self.vector_store.as_ref().ok_or_else(|| {
            warn!(
                target: "nebula.sidecar.memory",
                "search_memory called but vector_store not configured"
            );
            anyhow!("vector search not configured: vector_store missing")
        })?;

        let query_emb = embedder.embed(&query).await?;
        vector_store.search(&query_emb, k as usize).await
    }

    /// RPC: ListRecent — 列出最近写入的记忆(按 created_at 倒序)。
    #[instrument(skip(self), fields(limit = limit))]
    pub async fn list_recent(&self, limit: usize) -> Result<Vec<Memory>> {
        self.sqlite.list_recent(limit).await
    }
}

impl std::fmt::Debug for MemoryServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryServiceHandler")
            .field("sqlite", &"Arc<SqliteStore>")
            .field("has_vector_search", &self.has_vector_search())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handler() -> MemoryServiceHandler {
        let tmp = std::env::temp_dir().join(format!(
            "nebula_memory_sidecar_test_{}_{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("test op should succeed")
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&tmp);
        let sqlite = Arc::new(SqliteStore::open(&tmp).expect("open temp db"));
        MemoryServiceHandler::new(sqlite)
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let h = make_handler();
        assert!(h.health_check().await.expect("task should complete"));
    }

    #[tokio::test]
    async fn store_and_get_memory() {
        let h = make_handler();
        let id = h
            .store_memory(
                "hello world".to_string(),
                MemoryType::Semantic,
                MemoryLayer::L3,
                SourceKind::UserInput,
            )
            .await
            .expect("store should succeed");
        assert!(!id.is_empty(), "id should be non-empty");

        let mem = h
            .get_memory(&id)
            .await
            .expect("get should succeed")
            .expect("memory should exist");
        assert_eq!(mem.id, id);
        assert_eq!(mem.content, "hello world");
        assert_eq!(mem.memory_type, MemoryType::Semantic);
        assert_eq!(mem.layer, MemoryLayer::L3);
        assert_eq!(mem.source, SourceKind::UserInput);
    }

    #[tokio::test]
    async fn get_memory_returns_none_for_missing() {
        let h = make_handler();
        let result = h
            .get_memory("nonexistent-id-12345")
            .await
            .expect("get should not error");
        assert!(result.is_none(), "missing id should return None");
    }

    #[tokio::test]
    async fn list_recent_returns_inserted() {
        let h = make_handler();
        for i in 0..3 {
            h.store_memory(
                format!("content-{}", i),
                MemoryType::Episodic,
                MemoryLayer::L1,
                SourceKind::System,
            )
            .await
            .expect("store should succeed");
        }
        let recent = h.list_recent(10).await.expect("list_recent should succeed");
        assert_eq!(recent.len(), 3, "should list all 3 inserted memories");
    }

    #[tokio::test]
    async fn list_recent_respects_limit() {
        let h = make_handler();
        for i in 0..5 {
            h.store_memory(
                format!("item-{}", i),
                MemoryType::Semantic,
                MemoryLayer::L3,
                SourceKind::External,
            )
            .await
            .expect("store should succeed");
        }
        let recent = h.list_recent(2).await.expect("list_recent should succeed");
        assert_eq!(recent.len(), 2, "should respect limit");
    }

    #[tokio::test]
    async fn search_memory_fails_without_vector_store() {
        let h = make_handler();
        let result = h.search_memory("query".to_string(), 5).await;
        assert!(
            result.is_err(),
            "search should fail without vector store configured"
        );
    }

    #[test]
    fn sqlite_accessor_works() {
        let h = make_handler();
        let _ = h.sqlite();
    }

    #[test]
    fn has_vector_search_default_false() {
        let h = make_handler();
        assert!(!h.has_vector_search(), "default should be no vector search");
    }

    #[tokio::test]
    async fn store_multiple_types_and_layers() {
        let h = make_handler();
        let cases = [
            (MemoryType::Semantic, MemoryLayer::L3),
            (MemoryType::Episodic, MemoryLayer::L1),
            (MemoryType::Procedural, MemoryLayer::L4),
            (MemoryType::Emotional, MemoryLayer::L2),
            (MemoryType::Metacognitive, MemoryLayer::L5),
        ];
        for (mt, ml) in &cases {
            let id = h
                .store_memory("c".to_string(), *mt, *ml, SourceKind::AgentOutput)
                .await
                .expect("store should succeed");
            let mem = h
                .get_memory(&id)
                .await
                .expect("get should succeed")
                .expect("memory should exist");
            assert_eq!(mem.memory_type, *mt);
            assert_eq!(mem.layer, *ml);
        }
        let all = h.list_recent(100).await.expect("list should succeed");
        assert_eq!(all.len(), cases.len());
    }
}

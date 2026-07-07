//! T-E-S-42: VectorStore trait 抽象 — 支持切换 LanceDB/Qdrant/ChromaDB。
//!
//! 设计文档:`.trae/specs/wire-stage8-p2-vector-store-trait/spec.md`
//!
//! 在 LanceStore / QdrantStore / ChromaDbStore 三个具体后端之上抽出
//! 统一的 [`VectorStore`] trait,让 11 个调用方(`memory/llm/skills/
//! swarm/grpc/commands`)面向 trait 编程。运行时通过 [`create_vector_store`]
//! 工厂按 [`VectorStoreBackend`] 选择底层实现,无行为变更。
//!
//! ## 设计约束(对应 spec §设计约束)
//!
//! 1. **零新依赖**:LanceDB 已有;Qdrant/Chroma 用 reqwest REST(已有),
//!    不引入 `qdrant-client` / `chromadb` crate。
//! 2. **feature gate**:Cargo.toml 加 `qdrant = []` / `chroma = []` 空
//!    feature(默认关闭)。开启时编译对应 REST 骨架,关闭时 stub 返回 Err。
//! 3. **trait 方法对齐现有 LanceStore API**:保证 11 调用方零行为变更。
//! 4. **batch_upsert 默认实现**:逐条 upsert;LanceStore 覆盖走原生批写。
//! 5. **search_with_filter 默认 Err**:前向能力,当前无 metadata 列。
//! 6. **serde 向后兼容**:[`VectorStoreBackend`] 用 `#[serde(rename_all =
//!    "lowercase")]`,与 [`crate::memory::embedder::EmbedderKind`] 对齐。
//!
//! ## 模块结构
//!
//! - [`mod@lance`] — `impl VectorStore for LanceStore` 桥接(两套 cfg)。
//! - [`mod@qdrant`] — QdrantStore stub + `#[cfg(feature = "qdrant")]` REST 骨架。
//! - [`mod@chroma`] — ChromaDbStore stub + `#[cfg(feature = "chroma")]` REST 骨架。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::memory::lance_store::LanceStore;

pub mod chroma;
pub mod lance;
pub mod qdrant;

/// 向量存储抽象 — 支持 LanceDB / Qdrant / ChromaDB 后端。
///
/// 方法签名严格对齐现有 [`LanceStore`] API,保证 11 个调用方零行为变更。
/// `batch_upsert` / `search_with_filter` 提供默认实现,具体后端可覆盖。
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// 返回后端存储路径(LanceDB 目录 / Qdrant URL / Chroma URL)。
    fn path(&self) -> &str;

    /// 返回向量维度。
    fn dim(&self) -> usize;

    /// 插入或更新单条向量。`vector.len()` 必须等于 `dim()`。
    async fn upsert(&self, id: &str, vector: &[f32]) -> Result<()>;

    /// 批量插入。默认逐条 `upsert`,具体后端可覆盖走原生批写。
    ///
    /// `ids` 与 `vectors` 长度必须相等;每条 `vectors[i]` 的维度必须等于
    /// `dim()`。任一条失败立即返回错误(已写入的条目不回滚,与 LanceDB
    /// 原生批写语义一致)。
    async fn batch_upsert(&self, ids: &[String], vectors: &[Vec<f32>]) -> Result<()> {
        anyhow::ensure!(
            ids.len() == vectors.len(),
            "ids/vectors length mismatch: {} vs {}",
            ids.len(),
            vectors.len()
        );
        for (id, vec) in ids.iter().zip(vectors.iter()) {
            self.upsert(id, vec).await?;
        }
        Ok(())
    }

    /// 删除单条向量。返回 `true` 表示原存在并已删除,`false` 表示原本不存在。
    async fn delete(&self, id: &str) -> Result<bool>;

    /// Top-k 余弦相似度搜索。返回 `(id, score)` 对,按 `score` 降序。
    async fn search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>>;

    /// 带过滤条件的 Top-k 搜索。默认返回 `Err("not supported")`,
    /// 当前 LanceDB schema 无 metadata 列。前向能力:未来 Qdrant/Chroma
    /// 可覆盖以支持 metadata 过滤。
    async fn search_with_filter(
        &self,
        _query: &[f32],
        _k: usize,
        _filter: &str,
    ) -> Result<Vec<(String, f32)>> {
        Err(anyhow!("search_with_filter not supported by this backend"))
    }

    /// 当前存储的向量数量。
    async fn len(&self) -> usize;

    /// 健康检查 — 供 `doctor::check_lancedb` 等诊断调用。
    ///
    /// Lance:验证 LanceDB 目录可访问或 fallback mirror 可读。
    /// Qdrant/Chroma:GET `/` 或 `/health` 端点返回 2xx。
    async fn health_check(&self) -> Result<()>;
}

/// 向量存储后端类型。serde 序列化为 `"lance"` / `"qdrant"` / `"chroma"`,
/// 与 [`crate::memory::embedder::EmbedderKind`] 的 lowercase 风格对齐。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VectorStoreBackend {
    /// LanceDB 嵌入式向量库(默认,无外部服务依赖)。
    Lance,
    /// Qdrant 远程向量库(REST API,需 `qdrant` feature)。
    Qdrant,
    /// ChromaDB 远程向量库(REST API,需 `chroma` feature)。
    Chroma,
}

impl Default for VectorStoreBackend {
    fn default() -> Self {
        Self::Lance
    }
}

/// 创建向量存储 — 工厂函数,参照 [`crate::memory::embedder::create_embedder`]。
///
/// 根据 `backend` 选择 LanceDB / Qdrant / ChromaDB 后端,返回
/// `Arc<dyn VectorStore>` trait 对象。调用方(`AppState::bootstrap_storage`)
/// 持有 trait 对象,无需关心底层实现。
///
/// ## 参数
///
/// - `backend` — 后端类型。
/// - `path` — LanceDB 目录路径(Lance 后端使用)。
/// - `dim` — 向量维度。
/// - `remote_url` — Qdrant/Chroma 的 REST API 根 URL(Qdrant/Chroma 后端
///   使用;Lance 后端忽略此参数)。
///
/// ## 错误
///
/// - Lance:`LanceStore::open` 失败(目录权限 / IO 错误)。
/// - Qdrant 未启用 `qdrant` feature:`Err("qdrant feature not enabled")`。
/// - Chroma 未启用 `chroma` feature:`Err("chroma feature not enabled")`。
pub async fn create_vector_store(
    backend: VectorStoreBackend,
    path: &str,
    dim: usize,
    remote_url: Option<&str>,
) -> Result<Arc<dyn VectorStore>> {
    match backend {
        VectorStoreBackend::Lance => {
            let store = LanceStore::open(path, dim).await?;
            Ok(Arc::new(store) as Arc<dyn VectorStore>)
        }
        VectorStoreBackend::Qdrant => {
            #[cfg(feature = "qdrant")]
            {
                let url = remote_url.unwrap_or("http://127.0.0.1:6333");
                let store = qdrant::QdrantStore::new(url, dim)?;
                Ok(Arc::new(store) as Arc<dyn VectorStore>)
            }
            #[cfg(not(feature = "qdrant"))]
            {
                let _ = (dim, remote_url);
                Err(anyhow!(
                    "qdrant feature not enabled; rebuild with --features qdrant"
                ))
            }
        }
        VectorStoreBackend::Chroma => {
            #[cfg(feature = "chroma")]
            {
                let url = remote_url.unwrap_or("http://127.0.0.1:8000");
                let store = chroma::ChromaDbStore::new(url, dim)?;
                Ok(Arc::new(store) as Arc<dyn VectorStore>)
            }
            #[cfg(not(feature = "chroma"))]
            {
                let _ = (dim, remote_url);
                Err(anyhow!(
                    "chroma feature not enabled; rebuild with --features chroma"
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- VectorStoreBackend serde lowercase ----

    /// serde 序列化为 lowercase,与 EmbedderKind 风格对齐。
    #[test]
    fn backend_serde_lowercase() {
        let lance = serde_json::to_string(&VectorStoreBackend::Lance).unwrap();
        let qdrant = serde_json::to_string(&VectorStoreBackend::Qdrant).unwrap();
        let chroma = serde_json::to_string(&VectorStoreBackend::Chroma).unwrap();
        assert_eq!(lance, "\"lance\"");
        assert_eq!(qdrant, "\"qdrant\"");
        assert_eq!(chroma, "\"chroma\"");
    }

    /// serde 反序列化接受 lowercase。
    #[test]
    fn backend_serde_deserialize_lowercase() {
        let lance: VectorStoreBackend = serde_json::from_str("\"lance\"").unwrap();
        let qdrant: VectorStoreBackend = serde_json::from_str("\"qdrant\"").unwrap();
        let chroma: VectorStoreBackend = serde_json::from_str("\"chroma\"").unwrap();
        assert_eq!(lance, VectorStoreBackend::Lance);
        assert_eq!(qdrant, VectorStoreBackend::Qdrant);
        assert_eq!(chroma, VectorStoreBackend::Chroma);
    }

    /// Default 实现:未指定时回退 Lance。
    #[test]
    fn backend_default_is_lance() {
        assert_eq!(VectorStoreBackend::default(), VectorStoreBackend::Lance);
    }

    // ---- create_vector_store 工厂 ----

    /// Lance 后端默认可用(无 feature gate),工厂返回 trait 对象。
    #[tokio::test]
    async fn factory_lance_default() {
        let path =
            std::env::temp_dir().join(format!("nebula_vs_factory_lance_{}", uuid::Uuid::new_v4()));
        let store = create_vector_store(VectorStoreBackend::Lance, path.to_str().unwrap(), 4, None)
            .await
            .expect("Lance backend should build without feature flag");
        assert_eq!(store.dim(), 4);
        assert_eq!(store.path(), path.to_str().unwrap());
        assert_eq!(store.len().await, 0);
        let _ = std::fs::remove_dir_all(path);
    }

    /// Qdrant 后端在未启用 feature 时返回 Err。
    #[tokio::test]
    async fn factory_qdrant_without_feature_errors() {
        let err = create_vector_store(
            VectorStoreBackend::Qdrant,
            "ignored",
            4,
            Some("http://127.0.0.1:6333"),
        )
        .await
        .is_err();
        #[cfg(not(feature = "qdrant"))]
        {
            assert!(err, "Qdrant without feature flag must Err");
        }
        #[cfg(feature = "qdrant")]
        {
            // 启用 feature 时构造应成功(不实际连接,只是建客户端)。
            let _ = err;
        }
    }

    /// Chroma 后端在未启用 feature 时返回 Err。
    #[tokio::test]
    async fn factory_chroma_without_feature_errors() {
        let err = create_vector_store(
            VectorStoreBackend::Chroma,
            "ignored",
            4,
            Some("http://127.0.0.1:8000"),
        )
        .await
        .is_err();
        #[cfg(not(feature = "chroma"))]
        {
            assert!(err, "Chroma without feature flag must Err");
        }
        #[cfg(feature = "chroma")]
        {
            let _ = err;
        }
    }

    // ---- batch_upsert 默认实现 ----

    /// batch_upsert 默认逐条 upsert,语义等价。
    #[tokio::test]
    async fn batch_upsert_default_impl_equivalent_to_loop() {
        let path = std::env::temp_dir().join(format!("nebula_vs_batch_{}", uuid::Uuid::new_v4()));
        let store = create_vector_store(VectorStoreBackend::Lance, path.to_str().unwrap(), 4, None)
            .await
            .unwrap();
        let ids = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let vecs = vec![
            vec![1.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 0.0],
        ];
        store.batch_upsert(&ids, &vecs).await.expect("batch upsert");
        assert_eq!(store.len().await, 3);
        let _ = std::fs::remove_dir_all(path);
    }

    /// batch_upsert 长度不匹配返回 Err。
    #[tokio::test]
    async fn batch_upsert_length_mismatch_errors() {
        let path =
            std::env::temp_dir().join(format!("nebula_vs_batch_mismatch_{}", uuid::Uuid::new_v4()));
        let store = create_vector_store(VectorStoreBackend::Lance, path.to_str().unwrap(), 4, None)
            .await
            .unwrap();
        let ids = vec!["a".to_string(), "b".to_string()];
        let vecs = vec![vec![1.0, 0.0, 0.0, 0.0]];
        let err = store.batch_upsert(&ids, &vecs).await;
        assert!(err.is_err(), "length mismatch must Err");
        let _ = std::fs::remove_dir_all(path);
    }

    // ---- search_with_filter 默认 Err ----

    /// Lance 后端的 search_with_filter 默认返回 Err(无 metadata 列)。
    #[tokio::test]
    async fn search_with_filter_default_errors() {
        let path = std::env::temp_dir().join(format!("nebula_vs_filter_{}", uuid::Uuid::new_v4()));
        let store = create_vector_store(VectorStoreBackend::Lance, path.to_str().unwrap(), 4, None)
            .await
            .unwrap();
        let err = store
            .search_with_filter(&[1.0, 0.0, 0.0, 0.0], 1, "id = 'a'")
            .await;
        assert!(err.is_err(), "search_with_filter must Err by default");
        let _ = std::fs::remove_dir_all(path);
    }
}

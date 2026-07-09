//! T-E-S-42: QdrantStore — Qdrant 远程向量库 REST 客户端骨架。
//!
//! ## 设计约束(对应 spec §设计约束)
//!
//! 1. **零新依赖**:不引入 `qdrant-client` crate,用已有的 `reqwest`
//!    走 Qdrant REST API(<https://api.qdrant.com/v1>)。
//! 2. **feature gate**:`#[cfg(feature = "qdrant")]` 编译真实 REST 骨架;
//!    未启用时 stub 所有方法返回 `Err`。
//! 3. **trait 方法对齐 VectorStore**:与 LanceStore 行为一致,但底层走
//!    HTTP REST 而非嵌入式 LanceDB。
//!
//! ## REST API 端点(P2 MVP 范围)
//!
//! - `PUT /collections/{collection}` — 创建集合(指定向量维度)。
//! - `PUT /collections/{collection}/points` — upsert 点(带 id + vector)。
//! - `POST /collections/{collection}/points/search` — top-k 搜索。
//! - `DELETE /collections/{collection}/points` — 按 id 删除。
//! - `GET /collections/{collection}` — 集合元信息(用于 len / health_check)。
//!
//! 当前 P2 MVP 仅实现 stub:启用 `qdrant` feature 时构造客户端,但所有
//! 方法返回 `Err("qdrant backend not yet implemented")`。后续 P3 任务
//! 填充 REST 调用。

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::memory::vector_store::VectorStore;
#[cfg(feature = "qdrant")]
use crate::security::SsrfGuard;

/// Qdrant 远程向量库客户端。
///
/// 字段:
/// - `url` — Qdrant REST API 根 URL(如 `http://127.0.0.1:6333`)。
/// - `collection` — 集合名(对应 LanceStore 的 `memories` 表)。
/// - `dim` — 向量维度。
///
/// 启用 `qdrant` feature 时构造客户端;未启用时 `QdrantStore::new` 返回
/// `Err`(`create_vector_store` 工厂已处理 cfg 分支,这里只是兜底)。
pub struct QdrantStore {
    /// Qdrant REST API 根 URL。
    url: String,
    /// 集合名(默认 `memories`,对齐 LanceStore 表名)。
    collection: String,
    /// 向量维度。
    dim: usize,
    /// reqwest 客户端(启用 feature 时构造)。
    #[cfg(feature = "qdrant")]
    client: reqwest::Client,
}

impl QdrantStore {
    /// 创建 QdrantStore。
    ///
    /// 启用 `qdrant` feature 时构造 reqwest 客户端;未启用时返回 Err
    /// (理论上不会执行到这里,因为 `create_vector_store` 工厂已 cfg 分流)。
    pub fn new(url: &str, dim: usize) -> Result<Self> {
        Self::with_collection(url, "memories", dim)
    }

    /// 创建 QdrantStore 并指定集合名。
    pub fn with_collection(url: &str, collection: &str, dim: usize) -> Result<Self> {
        #[cfg(feature = "qdrant")]
        {
            // M7b #94: SSRF 校验
            SsrfGuard::new()
                .validate_url(url)
                .map_err(|e| anyhow!("SSRF validation failed for Qdrant URL: {e}"))?;
            let client = reqwest::Client::builder()
                .build()
                .map_err(|e| anyhow!("failed to build reqwest client for qdrant: {e}"))?;
            Ok(Self {
                url: url.trim_end_matches('/').to_string(),
                collection: collection.to_string(),
                dim,
                client,
            })
        }
        #[cfg(not(feature = "qdrant"))]
        {
            // 消除 feature 关闭时参数未使用的告警。
            let _ = (url, collection, dim);
            Err(anyhow!(
                "qdrant feature not enabled; rebuild with --features qdrant"
            ))
        }
    }

    /// 返回 Qdrant REST API 根 URL。
    #[allow(dead_code)]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 返回集合名。
    #[allow(dead_code)]
    pub fn collection(&self) -> &str {
        &self.collection
    }
}

#[async_trait]
impl VectorStore for QdrantStore {
    fn path(&self) -> &str {
        // 对 Qdrant 而言,"path" 语义映射为 REST API 根 URL。
        &self.url
    }

    fn dim(&self) -> usize {
        self.dim
    }

    async fn upsert(&self, _id: &str, _vector: &[f32]) -> Result<()> {
        Err(anyhow!("qdrant backend not yet implemented (P2 MVP stub)"))
    }

    async fn delete(&self, _id: &str) -> Result<bool> {
        Err(anyhow!("qdrant backend not yet implemented (P2 MVP stub)"))
    }

    async fn search(&self, _query: &[f32], _k: usize) -> Result<Vec<(String, f32)>> {
        Err(anyhow!("qdrant backend not yet implemented (P2 MVP stub)"))
    }

    async fn len(&self) -> usize {
        // 未实现时返回 0(不报错,因为 trait 签名不带 Result)。
        // 调用方应通过 health_check 验证后端可用性。
        0
    }

    async fn health_check(&self) -> Result<()> {
        // 启用 feature 时,GET / 验证 Qdrant 服务可达。
        #[cfg(feature = "qdrant")]
        {
            let url = format!("{}/", self.url);
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| anyhow!("qdrant health_check request failed: {e}"))?;
            if !resp.status().is_success() {
                return Err(anyhow!(
                    "qdrant health_check failed: HTTP {}",
                    resp.status()
                ));
            }
            Ok(())
        }
        #[cfg(not(feature = "qdrant"))]
        {
            Err(anyhow!(
                "qdrant feature not enabled; rebuild with --features qdrant"
            ))
        }
    }
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ---- QdrantStore stub 返回 Err(feature 关闭) ----

    /// 未启用 feature 时,QdrantStore::new 返回 Err。
    #[test]
    fn qdrant_new_without_feature_errors() {
        #[cfg(not(feature = "qdrant"))]
        {
            let err = QdrantStore::new("http://127.0.0.1:6333", 4).is_err();
            assert!(err, "QdrantStore::new without feature must Err");
        }
        #[cfg(feature = "qdrant")]
        {
            // 启用 feature 时构造应成功(不实际连接)。
            let _ = QdrantStore::new("http://127.0.0.1:6333", 4);
        }
    }

    /// 启用 feature 时,所有 trait 方法返回 Err(P2 MVP stub)。
    #[tokio::test]
    async fn qdrant_upsert_returns_err_stub() {
        #[cfg(feature = "qdrant")]
        {
            let store =
                QdrantStore::new("http://127.0.0.1:6333", 4).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            let err = vs.upsert("a", &[1.0, 0.0, 0.0, 0.0]).await;
            assert!(err.is_err(), "P2 MVP stub upsert must Err");
        }
        // feature 关闭时由 qdrant_new_without_feature_errors 覆盖。
    }

    /// 启用 feature 时,search 返回 Err(P2 MVP stub)。
    #[tokio::test]
    async fn qdrant_search_returns_err_stub() {
        #[cfg(feature = "qdrant")]
        {
            let store =
                QdrantStore::new("http://127.0.0.1:6333", 4).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            let err = vs.search(&[1.0, 0.0, 0.0, 0.0], 1).await;
            assert!(err.is_err(), "P2 MVP stub search must Err");
        }
    }

    /// 启用 feature 时,delete 返回 Err(P2 MVP stub)。
    #[tokio::test]
    async fn qdrant_delete_returns_err_stub() {
        #[cfg(feature = "qdrant")]
        {
            let store =
                QdrantStore::new("http://127.0.0.1:6333", 4).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            let err = vs.delete("a").await;
            assert!(err.is_err(), "P2 MVP stub delete must Err");
        }
    }

    /// dim / path 不依赖网络,可静态返回。
    #[test]
    fn qdrant_dim_and_path_static() {
        #[cfg(feature = "qdrant")]
        {
            let store =
                QdrantStore::new("http://127.0.0.1:6333/", 8).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            assert_eq!(vs.dim(), 8);
            assert_eq!(vs.path(), "http://127.0.0.1:6333");
        }
    }
}

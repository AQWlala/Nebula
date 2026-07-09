//! T-E-S-42: ChromaDbStore — ChromaDB 远程向量库 REST 客户端骨架。
//!
//! ## 设计约束(对应 spec §设计约束)
//!
//! 1. **零新依赖**:不引入 `chromadb` crate,用已有的 `reqwest`
//!    走 ChromaDB REST API(<https://docs.trychroma.com/api>)。
//! 2. **feature gate**:`#[cfg(feature = "chroma")]` 编译真实 REST 骨架;
//!    未启用时 stub 所有方法返回 `Err`。
//! 3. **trait 方法对齐 VectorStore**:与 LanceStore 行为一致,但底层走
//!    HTTP REST 而非嵌入式 LanceDB。
//!
//! ## REST API 端点(P2 MVP 范围)
//!
//! - `POST /api/v1/collections` — 创建集合。
//! - `POST /api/v1/collections/{collection}/add` — 添加向量。
//! - `POST /api/v1/collections/{collection}/query` — top-k 搜索。
//! - `POST /api/v1/collections/{collection}/delete` — 删除向量。
//! - `GET /api/v1/collections/{collection}` — 集合元信息(用于 len / health_check)。
//!
//! 当前 P2 MVP 仅实现 stub:启用 `chroma` feature 时构造客户端,但所有
//! 方法返回 `Err("chroma backend not yet implemented")`。后续 P3 任务
//! 填充 REST 调用。

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::memory::vector_store::VectorStore;
#[cfg(feature = "chroma")]
use crate::security::SsrfGuard;

/// ChromaDB 远程向量库客户端。
///
/// 字段:
/// - `url` — ChromaDB REST API 根 URL(如 `http://127.0.0.1:8000`)。
/// - `collection` — 集合名(对应 LanceStore 的 `memories` 表)。
/// - `dim` — 向量维度。
///
/// 启用 `chroma` feature 时构造客户端;未启用时 `ChromaDbStore::new` 返回
/// `Err`(`create_vector_store` 工厂已处理 cfg 分支,这里只是兜底)。
pub struct ChromaDbStore {
    /// ChromaDB REST API 根 URL。
    url: String,
    /// 集合名(默认 `memories`,对齐 LanceStore 表名)。
    collection: String,
    /// 向量维度。
    dim: usize,
    /// reqwest 客户端(启用 feature 时构造)。
    #[cfg(feature = "chroma")]
    client: reqwest::Client,
}

impl ChromaDbStore {
    /// 创建 ChromaDbStore。
    ///
    /// 启用 `chroma` feature 时构造 reqwest 客户端;未启用时返回 Err
    /// (理论上不会执行到这里,因为 `create_vector_store` 工厂已 cfg 分流)。
    pub fn new(url: &str, dim: usize) -> Result<Self> {
        Self::with_collection(url, "memories", dim)
    }

    /// 创建 ChromaDbStore 并指定集合名。
    pub fn with_collection(url: &str, collection: &str, dim: usize) -> Result<Self> {
        #[cfg(feature = "chroma")]
        {
            // M7b #94: SSRF 校验
            SsrfGuard::new()
                .validate_url(url)
                .map_err(|e| anyhow!("SSRF validation failed for ChromaDB URL: {e}"))?;
            let client = reqwest::Client::builder()
                .build()
                .map_err(|e| anyhow!("failed to build reqwest client for chroma: {e}"))?;
            Ok(Self {
                url: url.trim_end_matches('/').to_string(),
                collection: collection.to_string(),
                dim,
                client,
            })
        }
        #[cfg(not(feature = "chroma"))]
        {
            // 消除 feature 关闭时参数未使用的告警。
            let _ = (url, collection, dim);
            Err(anyhow!(
                "chroma feature not enabled; rebuild with --features chroma"
            ))
        }
    }

    /// 返回 ChromaDB REST API 根 URL。
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
impl VectorStore for ChromaDbStore {
    fn path(&self) -> &str {
        // 对 ChromaDB 而言,"path" 语义映射为 REST API 根 URL。
        &self.url
    }

    fn dim(&self) -> usize {
        self.dim
    }

    async fn upsert(&self, _id: &str, _vector: &[f32]) -> Result<()> {
        Err(anyhow!("chroma backend not yet implemented (P2 MVP stub)"))
    }

    async fn delete(&self, _id: &str) -> Result<bool> {
        Err(anyhow!("chroma backend not yet implemented (P2 MVP stub)"))
    }

    async fn search(&self, _query: &[f32], _k: usize) -> Result<Vec<(String, f32)>> {
        Err(anyhow!("chroma backend not yet implemented (P2 MVP stub)"))
    }

    async fn len(&self) -> usize {
        // 未实现时返回 0(不报错,因为 trait 签名不带 Result)。
        // 调用方应通过 health_check 验证后端可用性。
        0
    }

    async fn health_check(&self) -> Result<()> {
        // 启用 feature 时,GET /api/v1/heartbeat 验证 ChromaDB 服务可达。
        #[cfg(feature = "chroma")]
        {
            let url = format!("{}/api/v1/heartbeat", self.url);
            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|e| anyhow!("chroma health_check request failed: {e}"))?;
            if !resp.status().is_success() {
                return Err(anyhow!(
                    "chroma health_check failed: HTTP {}",
                    resp.status()
                ));
            }
            Ok(())
        }
        #[cfg(not(feature = "chroma"))]
        {
            Err(anyhow!(
                "chroma feature not enabled; rebuild with --features chroma"
            ))
        }
    }
}

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ---- ChromaDbStore stub 返回 Err(feature 关闭) ----

    /// 未启用 feature 时,ChromaDbStore::new 返回 Err。
    #[test]
    fn chroma_new_without_feature_errors() {
        #[cfg(not(feature = "chroma"))]
        {
            let err = ChromaDbStore::new("http://127.0.0.1:8000", 4).is_err();
            assert!(err, "ChromaDbStore::new without feature must Err");
        }
        #[cfg(feature = "chroma")]
        {
            // 启用 feature 时构造应成功(不实际连接)。
            let _ = ChromaDbStore::new("http://127.0.0.1:8000", 4);
        }
    }

    /// 启用 feature 时,所有 trait 方法返回 Err(P2 MVP stub)。
    #[tokio::test]
    async fn chroma_upsert_returns_err_stub() {
        #[cfg(feature = "chroma")]
        {
            let store =
                ChromaDbStore::new("http://127.0.0.1:8000", 4).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            let err = vs.upsert("a", &[1.0, 0.0, 0.0, 0.0]).await;
            assert!(err.is_err(), "P2 MVP stub upsert must Err");
        }
        // feature 关闭时由 chroma_new_without_feature_errors 覆盖。
    }

    /// 启用 feature 时,search 返回 Err(P2 MVP stub)。
    #[tokio::test]
    async fn chroma_search_returns_err_stub() {
        #[cfg(feature = "chroma")]
        {
            let store =
                ChromaDbStore::new("http://127.0.0.1:8000", 4).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            let err = vs.search(&[1.0, 0.0, 0.0, 0.0], 1).await;
            assert!(err.is_err(), "P2 MVP stub search must Err");
        }
    }

    /// 启用 feature 时,delete 返回 Err(P2 MVP stub)。
    #[tokio::test]
    async fn chroma_delete_returns_err_stub() {
        #[cfg(feature = "chroma")]
        {
            let store =
                ChromaDbStore::new("http://127.0.0.1:8000", 4).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            let err = vs.delete("a").await;
            assert!(err.is_err(), "P2 MVP stub delete must Err");
        }
    }

    /// dim / path 不依赖网络,可静态返回。
    #[test]
    fn chroma_dim_and_path_static() {
        #[cfg(feature = "chroma")]
        {
            let store =
                ChromaDbStore::new("http://127.0.0.1:8000/", 8).expect("create should succeed");
            let vs: Arc<dyn VectorStore> = Arc::new(store);
            assert_eq!(vs.dim(), 8);
            assert_eq!(vs.path(), "http://127.0.0.1:8000");
        }
    }
}

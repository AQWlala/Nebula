//! Embedding client that wraps the local Ollama HTTP endpoint.
//!
//! BGE-small-zh-v1.5 produces 512-dimensional vectors and is small enough
//! to run on a laptop GPU. We keep a tiny in-process LRU cache (using
//! the `lru` crate) so that re-embedding the same chunk of text does
//! not hit the network twice.
//!
//! v0.2: cache hits and misses are also fed to the global metrics
//! counter so the front-end can plot the cache effectiveness.

use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::llm::ollama::OllamaClient;

/// Configuration payload for an embedding request (matches Ollama's
/// `/api/embeddings` endpoint).
#[derive(Debug, Clone, Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbedResponse {
    embedding: Vec<f32>,
}

/// Default capacity for the LRU embedding cache.
const CACHE_CAPACITY: usize = 512;

/// Embedding cache + Ollama client glue.
pub struct Embedder {
    client: OllamaClient,
    model: String,
    dim: usize,
    /// Bounded LRU cache: when it overflows the least-recently-used
    /// entry is evicted in O(1). The `parking_lot::Mutex` provides
    /// interior mutability (and `Send + Sync` since the cache itself
    /// is `Send`).
    cache: Arc<Mutex<LruCache<String, Vec<f32>>>>,
}

impl Embedder {
    /// Creates a new embedder. `dim` is the expected vector length.
    pub fn new(client: OllamaClient, model: impl Into<String>, dim: usize) -> Self {
        let cap = NonZeroUsize::new(CACHE_CAPACITY).unwrap_or(NonZeroUsize::new(1).expect("1 is non-zero"));
        Self {
            client,
            model: model.into(),
            dim,
            cache: Arc::new(Mutex::new(LruCache::new(cap))),
        }
    }

    /// Returns the configured vector dimensionality.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Computes the embedding of `text`. Cached on second call.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if let Some(v) = self.cache.lock().get(text).cloned() {
            crate::metrics::global().record_embedding_hit();
            return Ok(v);
        }

        crate::metrics::global().record_embedding_miss();
        let req = EmbedRequest {
            model: &self.model,
            prompt: text,
        };
        let url = format!("{}/api/embeddings", self.client.base_url());
        let resp: EmbedResponse = self
            .client
            .http()
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("sending embedding request to ollama")?
            .error_for_status()
            .context("ollama returned non-2xx for embedding")?
            .json()
            .await
            .context("parsing ollama embedding response")?;

        if resp.embedding.len() != self.dim {
            return Err(anyhow!(
                "embedding dim mismatch: expected {}, got {}",
                self.dim,
                resp.embedding.len()
            ));
        }
        debug!(target: "nebula.embedder", dim = self.dim, "embedded text");

        // LRU insertion — evicts the least-recently-used entry when the
        // cache is full.
        self.cache
            .lock()
            .put(text.to_string(), resp.embedding.clone());
        Ok(resp.embedding)
    }

    /// Computes embeddings for a batch of texts sequentially. Sequential
    /// (rather than concurrent) to avoid overloading a local Ollama
    /// server.
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }

    /// T-E-A-11: 测试辅助方法 — 直接向 LRU 缓存写入 (text → vec)。
    ///
    /// 仅在 `cfg(test)` 下编译,供 prefetch / semantic_cache 等模块的
    /// 单测跳过网络调用(OllamaClient 指向 127.0.0.1:1 强制失败)。
    /// 维度不做校验,由调用方保证与 `dim()` 一致。
    #[cfg(test)]
    pub fn seed_cache_for_test(&self, text: &str, vec: Vec<f32>) {
        self.cache.lock().put(text.to_string(), vec);
    }

    /// Cosine similarity between two equal-length vectors. Returns 0 if
    /// either is the zero vector.
    pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let (mut dot, mut na, mut nb) = (0.0_f64, 0.0_f64, 0.0_f64);
        for (x, y) in a.iter().zip(b.iter()) {
            let (x, y) = (*x as f64, *y as f64);
            dot += x * y;
            na += x * x;
            nb += y * y;
        }
        let denom = (na * nb).sqrt();
        if denom == 0.0 {
            0.0
        } else {
            (dot / denom) as f32
        }
    }
}

// ===========================================================================
// T-S6-B-01: 多模态嵌入抽象
// ===========================================================================

/// 嵌入模型抽象 — 支持文本和图片嵌入。
///
/// T-S6-B-01: 多模态嵌入抽象,允许不同模型(BGE 文本 / CLIP 图片)实现
/// 同一接口。下游(如 LanceStore、MemoryOrchestrator)可面向 trait 编程,
/// 在运行时切换底层模型而无需改动调用方代码。
#[async_trait]
pub trait EmbedderTrait: Send + Sync {
    /// 嵌入文本,返回向量。
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>>;

    /// 嵌入图片,返回向量。`image_data` 是图片的原始字节(如 PNG/JPEG)。
    ///
    /// 文本专用模型(如 BGE)应返回明确的 `Err`,而不是静默失败。
    async fn embed_image(&self, image_data: &[u8]) -> Result<Vec<f32>>;

    /// 返回向量维度。
    fn dim(&self) -> usize;

    /// 返回模型名称。
    fn model_name(&self) -> &str;
}

#[async_trait]
impl EmbedderTrait for Embedder {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        self.embed(text).await
    }

    async fn embed_image(&self, _image_data: &[u8]) -> Result<Vec<f32>> {
        // BGE 是纯文本嵌入模型,不支持图片向量化。
        // 返回明确错误而不是静默降级,避免把空向量写入向量库污染检索。
        Err(anyhow!(
            "BGE embedder does not support image embedding; use ClipEmbedder instead"
        ))
    }

    fn dim(&self) -> usize {
        // 直接读字段,避免与同名固有方法 `dim()` 形成递归。
        self.dim
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

/// 嵌入器类型 — 用于工厂方法选择底层实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbedderKind {
    /// BGE 文本嵌入模型(默认,512 维)。
    Bge,
    /// CLIP 多模态嵌入模型(T-S6-B-01,支持图片)。
    Clip,
}

/// 根据类型创建嵌入器,返回 trait 对象。
///
/// 调用方可把返回值存入 `Box<dyn EmbedderTrait>` 统一调度,无需关心
/// 底层是 BGE 还是 CLIP。
#[allow(dead_code)]
pub fn create_embedder(
    client: OllamaClient,
    kind: EmbedderKind,
    model: String,
    dim: usize,
) -> Box<dyn EmbedderTrait> {
    match kind {
        EmbedderKind::Bge => Box::new(Embedder::new(client, model, dim)),
        EmbedderKind::Clip => Box::new(super::clip_embedder::ClipEmbedder::new(client, model, dim)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((Embedder::cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(Embedder::cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_is_zero() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 1.0];
        assert_eq!(Embedder::cosine(&a, &b), 0.0);
    }

    #[test]
    fn cosine_mismatched_lengths_is_zero() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(Embedder::cosine(&a, &b), 0.0);
    }

    #[test]
    fn lru_cache_evicts_oldest() {
        // Test the LRU behaviour directly without touching the network.
        use lru::LruCache;
        use std::num::NonZeroUsize;
        let mut cache: LruCache<String, Vec<f32>> = LruCache::new(NonZeroUsize::new(2).expect("create should succeed"));
        cache.put("a".to_string(), vec![1.0]);
        cache.put("b".to_string(), vec![2.0]);
        // Touch "a" so it becomes most-recently used.
        let _ = cache.get(&"a".to_string());
        cache.put("c".to_string(), vec![3.0]);
        assert!(cache.get(&"a".to_string()).is_some());
        assert!(
            cache.get(&"b".to_string()).is_none(),
            "b should have been evicted"
        );
        assert!(cache.get(&"c".to_string()).is_some());
    }
}

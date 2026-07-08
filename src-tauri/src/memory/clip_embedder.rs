//! T-S6-B-01: CLIP 多模态嵌入 — 支持图片向量化和跨模态检索。
//!
//! CLIP (Contrastive Language-Image Pre-training) 模型可以同时嵌入
//! 文本和图片到同一个向量空间,实现"以图搜文"和"以文搜图"。
//!
//! 当前实现通过 Ollama 的多模态 API(如 `llava` / `clip` 模型)进行
//! 图片嵌入。Ollama 的 `/api/embeddings` 端点原生只接受 `prompt`
//! 文本字段;此处我们在请求体中附带 base64 编码的 `images` 字段
//! (与 `/api/chat` / `/api/generate` 的图片字段同名)。若部署的
//! Ollama 版本不支持图片嵌入,`embed_image` 会返回明确的错误,
//! 而不是静默降级。
//!
//! 维度:CLIP ViT-B/32 = 512 维,与 BGE-small-zh-v1.5 相同,
//! 可共存于同一 LanceDB 表中,实现跨模态统一检索。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use lru::LruCache;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::llm::ollama::OllamaClient;
use crate::memory::embedder::EmbedderTrait;

/// CLIP 嵌入请求(Ollama 兼容格式)。
///
/// `images` 为 base64 编码的图片字节列表,与 Ollama `/api/chat`、
/// `/api/generate` 的图片字段同名;`prompt` 在纯图片嵌入时可为空。
#[derive(Debug, Clone, Serialize)]
struct ClipEmbedRequest<'a> {
    model: &'a str,
    /// base64 编码的图片。文本嵌入时为空数组。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
    /// 文本提示。纯图片嵌入时可省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<&'a str>,
}

/// CLIP 嵌入响应(与 BGE `/api/embeddings` 响应同构)。
#[derive(Debug, Clone, Deserialize)]
struct ClipEmbedResponse {
    embedding: Vec<f32>,
}

/// 默认 LRU 缓存容量(图片嵌入计算成本高,缓存命中价值大)。
const CACHE_CAPACITY: usize = 256;

/// CLIP 多模态嵌入器 — 通过 Ollama 嵌入图片和文本。
///
/// - 图片嵌入:base64 编码 → POST `/api/embeddings`(带 `images` 字段)。
/// - 文本嵌入:走标准 `/api/embeddings`(仅 `prompt`)。
///
/// 两条路径各自维护独立的 LRU 缓存,避免重复计算。
pub struct ClipEmbedder {
    client: OllamaClient,
    model: String,
    dim: usize,
    /// 图片嵌入 LRU 缓存,key = 图片字节的 hash。
    image_cache: Arc<Mutex<LruCache<u64, Vec<f32>>>>,
    /// 文本嵌入 LRU 缓存,key = 原始文本。
    text_cache: Arc<Mutex<LruCache<String, Vec<f32>>>>,
}

impl ClipEmbedder {
    /// 创建一个新的 CLIP 嵌入器。`dim` 为期望的向量维度
    /// (CLIP ViT-B/32 通常为 512)。
    pub fn new(client: OllamaClient, model: impl Into<String>, dim: usize) -> Self {
        let cap = NonZeroUsize::new(CACHE_CAPACITY).unwrap_or(NonZeroUsize::new(1).expect("1 is non-zero"));
        Self {
            client,
            model: model.into(),
            dim,
            image_cache: Arc::new(Mutex::new(LruCache::new(cap))),
            text_cache: Arc::new(Mutex::new(LruCache::new(cap))),
        }
    }

    /// 返回配置的向量维度。
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// 返回模型名称。
    pub fn model_name(&self) -> &str {
        &self.model
    }

    /// 计算图片字节的 64 位 hash,用作图片缓存 key。
    fn hash_image(data: &[u8]) -> u64 {
        let mut h = DefaultHasher::new();
        data.hash(&mut h);
        h.finish()
    }
}

#[async_trait]
impl EmbedderTrait for ClipEmbedder {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        // 缓存命中:直接返回,并记录 metrics 命中。
        if let Some(v) = self.text_cache.lock().get(text).cloned() {
            crate::metrics::global().record_embedding_hit();
            return Ok(v);
        }

        crate::metrics::global().record_embedding_miss();

        let req = ClipEmbedRequest {
            model: &self.model,
            images: Vec::new(),
            prompt: Some(text),
        };
        let url = format!("{}/api/embeddings", self.client.base_url());
        let resp: ClipEmbedResponse = self
            .client
            .http()
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("sending CLIP text embedding request to ollama")?
            .error_for_status()
            .context("ollama returned non-2xx for CLIP text embedding")?
            .json()
            .await
            .context("parsing ollama CLIP text embedding response")?;

        if resp.embedding.len() != self.dim {
            return Err(anyhow!(
                "CLIP text embedding dim mismatch: expected {}, got {}",
                self.dim,
                resp.embedding.len()
            ));
        }
        debug!(target: "nebula.clip_embedder", dim = self.dim, "embedded text via CLIP");

        self.text_cache
            .lock()
            .put(text.to_string(), resp.embedding.clone());
        Ok(resp.embedding)
    }

    async fn embed_image(&self, image_data: &[u8]) -> Result<Vec<f32>> {
        if image_data.is_empty() {
            return Err(anyhow!("embed_image received empty image data"));
        }

        let key = Self::hash_image(image_data);

        // 缓存命中。
        if let Some(v) = self.image_cache.lock().get(&key).cloned() {
            crate::metrics::global().record_embedding_hit();
            return Ok(v);
        }

        crate::metrics::global().record_embedding_miss();

        // base64 编码图片字节。
        let b64 = B64.encode(image_data);
        let req = ClipEmbedRequest {
            model: &self.model,
            images: vec![b64],
            prompt: None,
        };
        let url = format!("{}/api/embeddings", self.client.base_url());
        let resp: ClipEmbedResponse = self
            .client
            .http()
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("sending CLIP image embedding request to ollama")?
            .error_for_status()
            .context(
                "ollama returned non-2xx for CLIP image embedding; \
                 the deployed Ollama version may not support image embedding via /api/embeddings",
            )?
            .json()
            .await
            .context("parsing ollama CLIP image embedding response")?;

        if resp.embedding.len() != self.dim {
            return Err(anyhow!(
                "CLIP image embedding dim mismatch: expected {}, got {}",
                self.dim,
                resp.embedding.len()
            ));
        }
        debug!(target: "nebula.clip_embedder", dim = self.dim, "embedded image via CLIP");

        self.image_cache.lock().put(key, resp.embedding.clone());
        Ok(resp.embedding)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_image_is_deterministic() {
        let a = ClipEmbedder::hash_image(b"hello");
        let b = ClipEmbedder::hash_image(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_image_differs_for_different_input() {
        let a = ClipEmbedder::hash_image(b"hello");
        let b = ClipEmbedder::hash_image(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn constructor_sets_fields() {
        let client = OllamaClient::new("http://127.0.0.1:11434");
        let emb = ClipEmbedder::new(client, "clip", 512);
        assert_eq!(emb.dim(), 512);
        assert_eq!(emb.model_name(), "clip");
    }

    #[test]
    fn trait_dim_and_model_name_via_box() {
        let client = OllamaClient::new("http://127.0.0.1:11434");
        let emb: Box<dyn EmbedderTrait> = Box::new(ClipEmbedder::new(client, "clip", 512));
        assert_eq!(emb.dim(), 512);
        assert_eq!(emb.model_name(), "clip");
    }
}

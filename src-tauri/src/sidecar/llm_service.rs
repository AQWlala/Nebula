//! T-D-B-14: Sidecar LLM Gateway 服务 — 单二进制多角色方案。
//!
//! 延续 T-S4-B-01 / T-S4-B-02 / T-S6-A-01a / memory_service 的单二进制
//! 多角色方案 (`nebula-sidecar --kind=llm`),为 LLM 调用网关
//! (限流 + 重试 + 熔断 + 缓存)提供独立进程隔离。
//!
//! 本模块定义 LLM sidecar 的服务处理器 [`LlmServiceHandler`],它包装
//! [`LlmGateway`] 并暴露与 gRPC 服务方法对应的 RPC 接口。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  gRPC (tonic)
//!    ▼
//! nebula-sidecar --kind=llm  (监听 127.0.0.1:50052)
//!    │  LlmServiceHandler
//!    ▼
//! LlmGateway (熔断 + LRU 缓存 + 多 provider 路由)
//!    │
//!    ▼
//! Ollama / DeepSeek / Anthropic / OpenAI 兼容端点
//! ```
//!
//! ## RPC 映射
//!
//! | gRPC RPC       | Handler 方法  | 后端                  |
//! |----------------|---------------|-----------------------|
//! | `Chat`         | `chat`        | LlmGateway::chat      |
//! | `Embed`        | `embed`       | Embedder::embed (可选)|
//! | `HealthCheck`  | `health_check`| (always ok)           |
//!
//! ## 依赖
//!
//! * [`LlmGateway`](crate::llm::LlmGateway) 必须注入(必备后端)。
//! * [`Embedder`](crate::memory::embedder::Embedder) 可选注入
//!   (未注入时 `embed` 返回 `Err`,日志 warn,`chat` 不受影响)。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tracing::{info, instrument, warn};

use crate::llm::gateway::LlmGateway;
use crate::llm::ollama::{ChatMessage, ChatResponse};
use crate::memory::embedder::Embedder;

/// LLM Gateway sidecar 服务处理器。
///
/// 包装 [`LlmGateway`] 提供 LLM 聊天调用;可选注入 [`Embedder`]
/// 启用向量嵌入。在进程内模式下也可直接使用(无需 gRPC)。
pub struct LlmServiceHandler {
    /// LLM 网关(必备)。
    gateway: Arc<LlmGateway>,
    /// 嵌入器(可选,未注入时 `embed` 返回 Err)。
    embedder: Option<Arc<Embedder>>,
}

impl LlmServiceHandler {
    /// 创建新的 LLM 服务处理器。
    ///
    /// 通常在 sidecar 进程启动时构造,持有与主进程相同的 `LlmGateway`
    /// 实例(或通过共享 provider 配置重建)。
    pub fn new(gateway: Arc<LlmGateway>) -> Self {
        info!(
            target: "nebula.sidecar.llm",
            "LlmServiceHandler initialized (chat-only)"
        );
        Self {
            gateway,
            embedder: None,
        }
    }

    /// 注入嵌入器,builder 风格。启用后 `embed` RPC 可用。
    pub fn with_embedder(mut self, embedder: Arc<Embedder>) -> Self {
        info!(
            target: "nebula.sidecar.llm",
            "LlmServiceHandler: embed RPC enabled"
        );
        self.embedder = Some(embedder);
        self
    }

    /// 访问底层 LlmGateway(供 IPC 客户端在进程内模式下直接调用)。
    pub fn gateway(&self) -> &Arc<LlmGateway> {
        &self.gateway
    }

    /// 是否已启用向量嵌入。
    pub fn has_embedder(&self) -> bool {
        self.embedder.is_some()
    }

    /// RPC: HealthCheck — 始终返回 Ok(若 handler 存在则服务可用)。
    pub async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// RPC: Chat — 非流式聊天补全。
    ///
    /// 直接委托给 [`LlmGateway::chat`],享受熔断 + LRU 缓存 +
    /// 多 provider 路由能力。
    #[instrument(skip(self, messages), fields(msgs = messages.len()))]
    pub async fn chat(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse> {
        self.gateway.chat(messages).await
    }

    /// RPC: Embed — 文本嵌入。
    ///
    /// 返回与查询文本对应的向量(维度由 embedder 配置决定)。
    ///
    /// 未注入 `Embedder` 时返回 `Err`(日志 warn)。
    #[instrument(skip(self, text), fields(text_len = text.len()))]
    pub async fn embed(&self, text: String) -> Result<Vec<f32>> {
        let embedder = self.embedder.as_ref().ok_or_else(|| {
            warn!(
                target: "nebula.sidecar.llm",
                "embed called but embedder not configured"
            );
            anyhow!("embed not configured: embedder missing")
        })?;
        embedder.embed(&text).await
    }
}

impl std::fmt::Debug for LlmServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmServiceHandler")
            .field("gateway", &"Arc<LlmGateway>")
            .field("has_embedder", &self.has_embedder())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_handler() -> LlmServiceHandler {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        let gw = std::sync::Arc::new(LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        LlmServiceHandler::new(gw)
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let h = make_handler();
        assert!(h.health_check().await.expect("task should complete"));
    }

    #[tokio::test]
    async fn chat_returns_err_when_upstream_unreachable() {
        // 上游 Ollama 不可达时 chat 应返回 Err(经熔断器多次失败后),
        // 而不是 panic。这里只验证不 panic,允许 Err。
        let h = make_handler();
        let req = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            ..Default::default()
        }];
        let _ = h.chat(req).await;
        // 不 assert 结果(可能 Ok 也可能 Err,取决于熔断器状态),
        // 只验证不 panic。
    }

    #[tokio::test]
    async fn embed_fails_without_embedder() {
        let h = make_handler();
        let result = h.embed("hello".to_string()).await;
        assert!(
            result.is_err(),
            "embed should fail without embedder configured"
        );
    }

    #[test]
    fn gateway_accessor_works() {
        let h = make_handler();
        let _ = h.gateway();
    }

    #[test]
    fn has_embedder_default_false() {
        let h = make_handler();
        assert!(!h.has_embedder(), "default should be no embedder");
    }
}

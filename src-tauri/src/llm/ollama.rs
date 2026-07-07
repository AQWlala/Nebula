//! Thin async HTTP wrapper around the local Ollama server.
//!
//! This module intentionally exposes only the three endpoints we use
//! (chat, generate, embeddings) so the rest of the code base does not
//! have to know about Ollama's request/response shapes.

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::security::SsrfGuard;

/// Role of a chat participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// T-E-S-02: OpenAI 兼容的 function calling — 工具调用请求(LLM 发起)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// 调用 ID(如 "call_abc123"),用于关联 tool 角色响应。
    pub id: String,
    /// 工具类型,OpenAI 固定为 "function"。
    #[serde(rename = "type")]
    pub ty: String,
    /// 函数调用详情。
    pub function: FunctionCall,
}

/// T-E-S-02: function calling 的函数调用详情。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    /// 函数名(对应 ToolSpec.function.name)。
    pub name: String,
    /// 参数 JSON 字符串(OpenAI 协议是字符串,非对象)。
    pub arguments: String,
}

/// T-E-S-02: 工具规格定义(发给 LLM 的 tools 参数项)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// 工具类型,OpenAI 固定为 "function"。
    #[serde(rename = "type")]
    pub ty: String,
    /// 函数规格。
    pub function: FunctionSpec,
}

/// T-E-S-02: 函数规格(名称 + 描述 + JSON Schema 参数)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    pub description: String,
    /// 参数的 JSON Schema(serde_json::Value)。
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// 构造函数规格,ty 自动设为 "function"。
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            ty: "function".to_string(),
            function: FunctionSpec {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// One chat message in an Ollama chat request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// T-E-S-02: LLM 发起的工具调用(assistant 角色消息可能含)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// T-E-S-02: tool 角色消息关联的 tool_call_id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// T-E-S-02: tool 角色消息的工具名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// T-E-S-28: 由 commands/chat.rs 注入的 turn_id(UUID v4),
    /// 用于关联 chat_annotations 表中的 good/bad 标注。
    /// `#[serde(default)]` 保证旧消息反序列化时为 None(向后兼容)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// T-E-C-02: 多模态图像支持 — base64 编码的图片数组。
    /// 对接 Ollama 多模态 API(如 qwen2.5-vl:3b),与
    /// `memory/clip_embedder.rs::ClipEmbedRequest.images` 同样的 wire format。
    /// 空 Vec 时 `skip_serializing_if` 省略字段(向后兼容旧消息)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            ..Default::default()
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            ..Default::default()
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            ..Default::default()
        }
    }
}

/// `/api/chat` request body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub stream: bool,
    /// T-E-S-02: 可用工具列表(OpenAI 兼容 function calling)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolSpec>>,
    /// T-E-S-02: 工具选择策略("auto"/"none"/{type:"function",function:{name:"..."}})。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
}

/// `/api/chat` response body (non-streaming).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChatResponse {
    pub model: String,
    pub message: ChatMessage,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub eval_count: Option<u64>,
    /// T-E-S-02: LLM 响应中的工具调用请求(OpenAI 兼容)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// T-E-B-17: 推理链(可选,仅推理模型如 DeepSeek-R1 返回)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_chain: Option<crate::llm::reasoning::ReasoningChain>,
}

/// T-E-S-51: `/api/generate` 的采样选项(对应 Ollama 的 `options` 字段)。
///
/// 仅暴露内联补全需要的两个 knob:`num_predict`(max tokens)与
/// `temperature`。其余选项(top_p / top_k / stop 等)按需再扩。
#[derive(Debug, Clone, Serialize)]
pub struct GenerateOptions {
    /// 最大生成 token 数(对应 Ollama `num_predict`)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
    /// 采样温度。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

impl GenerateOptions {
    /// T-E-S-51: 内联补全默认采样参数 — num_predict=20, temperature=0.2。
    pub fn inline_completion_defaults() -> Self {
        Self {
            num_predict: Some(20),
            temperature: Some(0.2),
        }
    }
}

/// `/api/generate` request body.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateRequest<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    pub stream: bool,
    /// T-E-S-51: 可选采样参数。`None` 时省略字段,Ollama 走默认采样。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<GenerateOptions>,
}

/// `/api/generate` response body (non-streaming).
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateResponse {
    pub model: String,
    pub response: String,
    #[serde(default)]
    pub done: bool,
}

/// A long-lived HTTP client + base URL pair.
#[derive(Clone)]
pub struct OllamaClient {
    base_url: String,
    http: Client,
    /// T-E-S-XX: chat 重试次数上限(不含首次尝试)。默认 3。
    max_retries: u32,
    /// T-E-S-XX: 重试间隔。默认 1s。
    retry_delay: Duration,
    /// M3 #49: 并发限流信号量 — 限制同时发往 Ollama 的请求数,
    /// 防止打爆本地推理服务。默认 2。
    ///
    /// 设计说明:
    /// - 在 OllamaClient 层加 Semaphore 保护**所有**调用路径
    ///   (chat / generate / embed / chat_with_retry)
    /// - UnifiedModelDispatcher 的 `local_semaphore` 是额外保护,
    ///   防止 dispatch_local 路径绕过限流;两层 Semaphore 取较小值
    ///   为实际并发上限(通常都是 2)
    /// - 其他直接调用 OllamaClient 的代码(embedder / generator /
    ///   ModelRouter 旧路径)通过此 Semaphore 限流
    max_concurrency: Arc<tokio::sync::Semaphore>,
    /// M3 #49: 配置的并发上限(Semaphore 不暴露总许可数,需单独跟踪)。
    max_concurrency_limit: usize,
}

/// M3 #49: OllamaClient 默认并发上限。
pub const DEFAULT_OLLAMA_MAX_CONCURRENCY: usize = 2;

impl OllamaClient {
    /// Creates a new client targeting `base_url` (e.g.
    /// `http://127.0.0.1:11434`).
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        // M7b #94: SSRF 校验 — Ollama 默认在 127.0.0.1:11434,需 allow_loopback。
        // 构造器返回 Self(非 Result),用 warn log 记录失败而非中断构造
        // (向后兼容:旧调用方不期望 new 失败)。
        if let Err(e) = SsrfGuard::new()
            .with_allow_loopback(true)
            .validate_url(&base_url)
        {
            warn!(target: "nebula.ollama", url = %base_url, "SSRF validation failed for Ollama URL: {e}");
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client should build");
        Self {
            base_url,
            http,
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
            max_concurrency: Arc::new(tokio::sync::Semaphore::new(DEFAULT_OLLAMA_MAX_CONCURRENCY)),
            max_concurrency_limit: DEFAULT_OLLAMA_MAX_CONCURRENCY,
        }
    }

    /// Creates a new client with a custom HTTP timeout.  Tests use
    /// this to avoid waiting 120 s on a dead port before the
    /// request fails.
    pub fn new_with_timeout(base_url: impl Into<String>, timeout: Duration) -> Self {
        let base_url = base_url.into();
        // M7b #94: SSRF 校验 — Ollama 默认在 127.0.0.1:11434,需 allow_loopback。
        // 构造器返回 Self(非 Result),用 warn log 记录失败而非中断构造
        // (向后兼容:旧调用方不期望 new 失败)。
        if let Err(e) = SsrfGuard::new()
            .with_allow_loopback(true)
            .validate_url(&base_url)
        {
            warn!(target: "nebula.ollama", url = %base_url, "SSRF validation failed for Ollama URL: {e}");
        }
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .expect("reqwest client should build");
        Self {
            base_url,
            http,
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
            max_concurrency: Arc::new(tokio::sync::Semaphore::new(DEFAULT_OLLAMA_MAX_CONCURRENCY)),
            max_concurrency_limit: DEFAULT_OLLAMA_MAX_CONCURRENCY,
        }
    }

    /// M3 #49: Builder-style configurator for the concurrency limit.
    ///
    /// Sets the maximum number of concurrent requests the client will
    /// issue to the Ollama server. Additional callers wait on the
    /// Semaphore until an in-flight request completes. Default is
    /// [`DEFAULT_OLLAMA_MAX_CONCURRENCY`] (2).
    ///
    /// Use a higher value (e.g. 4) on machines with multiple GPUs
    /// or when running batch classification; use 1 to serialize all
    /// Ollama traffic.
    pub fn with_max_concurrency(mut self, max: usize) -> Self {
        // 0 would deadlock; clamp to at least 1.
        let clamped = max.max(1);
        self.max_concurrency = Arc::new(tokio::sync::Semaphore::new(clamped));
        self.max_concurrency_limit = clamped;
        self
    }

    /// M3 #49: Returns the configured concurrency limit.
    pub fn max_concurrency(&self) -> usize {
        self.max_concurrency_limit
    }

    /// M3 #49: Returns the number of currently available permits
    /// (i.e. how many more requests can proceed without waiting).
    pub fn available_concurrency(&self) -> usize {
        self.max_concurrency.available_permits()
    }

    /// M3 #49: Acquire a concurrency permit before issuing the HTTP
    /// request. Returns an error if the semaphore is closed (impossible
    /// in practice because we hold an Arc to it).
    async fn acquire_permit(&self) -> anyhow::Result<tokio::sync::OwnedSemaphorePermit> {
        self.max_concurrency
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("ollama concurrency semaphore closed"))
    }

    /// T-E-S-XX: Builder-style configurator for retry behavior.
    /// `max_retries` is the number of retries after the first
    /// attempt (so total attempts = max_retries + 1). `retry_delay`
    /// is the wait between retries. Only [`chat_with_retry`] uses
    /// this config; the plain [`chat`] method is unaffected.
    pub fn with_retry_config(mut self, max_retries: u32, retry_delay: Duration) -> Self {
        self.max_retries = max_retries;
        self.retry_delay = retry_delay;
        self
    }

    /// Returns the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns a reference to the underlying HTTP client. Useful for
    /// callers (e.g. the embedder) that need a `reqwest::Client`.
    pub fn http(&self) -> &Client {
        &self.http
    }

    /// Issues a non-streaming chat completion.
    ///
    /// M3 #49: 通过 Semaphore 限流,防止并发请求打爆 Ollama。
    /// 默认上限 [`DEFAULT_OLLAMA_MAX_CONCURRENCY`] = 2,
    /// 可通过 [`with_max_concurrency`](Self::with_max_concurrency) 配置。
    pub async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<ChatResponse> {
        // M3 #49: 限流 — permit 在函数结束时自动 drop。
        let _permit = self.acquire_permit().await?;
        let url = format!("{}/api/chat", self.base_url);
        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: false,
            ..Default::default()
        };
        let resp: ChatResponse = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp)
    }

    /// T-E-S-XX: Like [`chat`](Self::chat) but retries on transient
    /// failures (network errors, timeouts, HTTP 5xx). HTTP 4xx are
    /// NOT retried — they represent client-side mistakes (bad model
    /// name, malformed request) that won't be fixed by retrying.
    ///
    /// Uses `max_retries` + `retry_delay` from
    /// [`with_retry_config`](Self::with_retry_config). Total attempts
    /// = `max_retries + 1`. The plain [`chat`](Self::chat) method is
    /// unchanged; callers choose whether to retry.
    pub async fn chat_with_retry(
        &self,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<ChatResponse> {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..=self.max_retries {
            match self.chat(model, messages).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if !is_retryable(&e) || attempt == self.max_retries {
                        return Err(e);
                    }
                    warn!(
                        target: "nebula.llm",
                        attempt,
                        max_retries = self.max_retries,
                        error = %e,
                        "ollama chat failed, retrying"
                    );
                    tokio::time::sleep(self.retry_delay).await;
                    last_err = Some(e);
                }
            }
        }
        // Unreachable: the loop returns on every iteration's branches.
        // The compiler can't prove it, so provide a fallback.
        Err(last_err.expect("retry loop must have produced an error"))
    }

    /// M5 #74: 流式 chat 完成（NDJSON）。
    ///
    /// 与 [`chat`](Self::chat) 的区别：
    /// - 发送 `"stream": true`，Ollama 返回 NDJSON（每行一个 JSON 对象）
    /// - 通过 [`StreamToken`] 逐块返回文本，调用方可实时渲染
    /// - **共享同一个 Semaphore**（M3 #49）：流式请求也受 `max_concurrency` 限制
    ///
    /// NDJSON 协议（每行一个 JSON）：
    /// ```json
    /// {"model":"...","message":{"role":"assistant","content":"Hel"},"done":false}
    /// {"model":"...","message":{"role":"assistant","content":"lo"},"done":false}
    /// {"model":"...","message":{"role":"assistant","content":""},"done":true,
    ///  "total_duration":...,"eval_count":...}
    /// ```
    ///
    /// 失败模式：
    /// - HTTP 错误 → 流的第一个元素是 `Err`
    /// - 网络中断 → 发出 `incomplete=true` 的尾部 token 后结束
    /// - 解析失败的单行 → 跳过（debug 日志），不影响其他行
    pub fn chat_stream(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, anyhow::Result<crate::llm::gateway::StreamToken>> {
        // 捕获 self 的字段，避免借用跨 await 点。
        let base_url = self.base_url.clone();
        let http = self.http.clone();
        let max_concurrency = self.max_concurrency.clone();

        let model = model.to_string();
        let stream = async_stream::stream! {
            // M3 #49: 限流 — permit 在 stream 结束时自动 drop。
            // 注意：acquire_owned 是 async，需在 stream! 块内 await。
            let _permit = match max_concurrency.acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    yield Err(anyhow::anyhow!("ollama concurrency semaphore closed"));
                    return;
                }
            };

            let url = format!("{}/api/chat", base_url);
            let req_body = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": true,
            });

            let resp = match http.post(&url).json(&req_body).send().await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(anyhow!("streaming chat request failed: {e}"));
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                yield Err(anyhow!("streaming chat HTTP {status}: {body}"));
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut incomplete = false;
            use futures::StreamExt;
            while let Some(chunk) = byte_stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        // 网络中断：发出 incomplete 尾部 token 后结束。
                        yield Ok(crate::llm::gateway::StreamToken {
                            text: String::new(),
                            done: false,
                            incomplete: true,
                        });
                        warn!(target: "nebula.llm", error = %e, "stream interrupted");
                        return;
                    }
                };

                for line in String::from_utf8_lossy(&bytes).lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<serde_json::Value>(line) {
                        Ok(v) => {
                            let done = v.get("done")
                                .and_then(|d| d.as_bool())
                                .unwrap_or(false);
                            let text = v.get("message")
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            if done {
                                yield Ok(crate::llm::gateway::StreamToken {
                                    text,
                                    done: true,
                                    incomplete: false,
                                });
                                return;
                            }
                            incomplete = !text.is_empty();
                            yield Ok(crate::llm::gateway::StreamToken {
                                text,
                                done: false,
                                incomplete: false,
                            });
                        }
                        Err(e) => {
                            debug!(target: "nebula.llm",
                                error = %e, line, "skipping unparseable stream line");
                        }
                    }
                }
            }

            // 流自然结束（未收到 done:true）。
            yield Ok(crate::llm::gateway::StreamToken {
                text: String::new(),
                done: true,
                incomplete,
            });
        };

        Box::pin(stream)
    }

    /// Issues a non-streaming generation.
    ///
    /// T-E-S-51: 保留原网络行为(不发送 `options` 字段, wire format
    /// 与历史一致), 仅因结构体新增 `options` 字段而补 `options: None`。
    /// 需要限 token / 调温度的调用方请走 [`generate_with_options`]。
    ///
    /// M3 #49: 通过 Semaphore 限流(与 chat 共享同一个 Semaphore)。
    pub async fn generate(&self, model: &str, prompt: &str) -> anyhow::Result<GenerateResponse> {
        // M3 #49: 限流
        let _permit = self.acquire_permit().await?;
        let url = format!("{}/api/generate", self.base_url);
        let req = GenerateRequest {
            model,
            prompt,
            stream: false,
            options: None,
        };
        let resp: GenerateResponse = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp)
    }

    /// T-E-S-51: Issues a non-streaming generation with explicit
    /// sampling `options` (num_predict / temperature).  Existing
    /// [`generate`](Self::generate) keeps its old wire format (no
    /// `options` field) for backward compatibility; callers that
    /// need to cap token count (e.g. inline completion) call this
    /// directly.
    ///
    /// M3 #49: 通过 Semaphore 限流(与 chat 共享同一个 Semaphore)。
    pub async fn generate_with_options(
        &self,
        model: &str,
        prompt: &str,
        options: GenerateOptions,
    ) -> anyhow::Result<GenerateResponse> {
        // M3 #49: 限流
        let _permit = self.acquire_permit().await?;
        let url = format!("{}/api/generate", self.base_url);
        let req = GenerateRequest {
            model,
            prompt,
            stream: false,
            options: Some(options),
        };
        let resp: GenerateResponse = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp)
    }

    /// Lightweight health check that pings `/api/tags` (lists installed
    /// models). Returns `true` if the server responds with 2xx.
    pub async fn ping(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self.http.get(&url).send().await {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    /// T-E-D-02: 后台预热 Ollama 模型(加载到显存),降低首响延迟。
    ///
    /// 1. GET `{base_url}/api/ps` 返回当前已加载模型列表。
    /// 2. 若目标模型已加载,debug log + return Ok(())。
    /// 3. 否则 POST `{base_url}/api/generate` with `{model, prompt:"",
    ///    stream:false, options:{num_predict:1}}` 触发加载。
    ///
    /// 本方法签名返回 `Result<()>`,错误通过 `?` 传播;调用方
    /// (lib.rs setup 回调)用 `warn!` 兜底使其非阻塞。
    pub async fn warmup_model(&self, model: &str) -> anyhow::Result<()> {
        let ps_url = format!("{}/api/ps", self.base_url);
        let resp = self.http.get(&ps_url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("GET /api/ps returned {}", resp.status());
        }
        let body: serde_json::Value = resp.json().await?;
        let loaded: Vec<String> = body
            .get("models")
            .and_then(|m| m.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if loaded.iter().any(|m| m == model) {
            debug!(target: "nebula.ollama", model, "model already loaded, skip warmup");
            return Ok(());
        }
        // 触发加载(空 prompt,num_predict=1 限制输出 1 token)。
        let gen_url = format!("{}/api/generate", self.base_url);
        let _ = self
            .http
            .post(&gen_url)
            .json(&serde_json::json!({
                "model": model,
                "prompt": "",
                "stream": false,
                "options": { "num_predict": 1 }
            }))
            .send()
            .await?;
        info!(target: "nebula.ollama", model, "model warmed up");
        Ok(())
    }
}

/// T-E-S-XX: Classify an `anyhow::Error` (carrying a `reqwest::Error`)
/// as retryable or not. Used by [`OllamaClient::chat_with_retry`].
///
/// Retryable:
/// - Connect / timeout errors (server down, network blip).
/// - HTTP 5xx responses (server error, possibly transient).
///
/// Not retryable:
/// - HTTP 4xx responses (client error — bad model name, malformed
///   request; retrying won't help).
/// - Other reqwest error kinds (body decode failure, redirect loop).
/// - Non-reqwest errors (e.g. serde JSON failures downstream).
fn is_retryable(err: &anyhow::Error) -> bool {
    let Some(req_err) = err.downcast_ref::<reqwest::Error>() else {
        return false;
    };
    if req_err.is_connect() || req_err.is_timeout() {
        return true;
    }
    match req_err.status() {
        Some(status) if status.is_server_error() => true,
        Some(status) if status.is_client_error() => false,
        // No status: not a status-code error. Fall through to false.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_constructors_set_role_correctly() {
        assert_eq!(ChatMessage::system("x").role, "system");
        assert_eq!(ChatMessage::user("x").role, "user");
        assert_eq!(ChatMessage::assistant("x").role, "assistant");
    }

    #[test]
    fn role_as_str() {
        assert_eq!(Role::System.as_str(), "system");
        assert_eq!(Role::User.as_str(), "user");
        assert_eq!(Role::Assistant.as_str(), "assistant");
    }

    #[test]
    fn chat_message_without_tool_calls_serializes_without_field() {
        let msg = ChatMessage::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        // 不含 "tool_calls" 字段
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn chat_response_with_tool_calls_deserializes() {
        let json = r#"{
            "model": "deepseek-chat",
            "message": {"role": "assistant", "content": ""},
            "done": true,
            "tool_calls": [
                {
                    "id": "call_abc123",
                    "type": "function",
                    "function": {"name": "shell", "arguments": "{\"command\":\"ls\"}"}
                }
            ]
        }"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert!(resp.tool_calls.is_some());
        let calls = resp.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc123");
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(calls[0].function.arguments, "{\"command\":\"ls\"}");
    }

    #[test]
    fn chat_request_with_tools_roundtrip() {
        let req = ChatRequest {
            model: "deepseek-chat".to_string(),
            messages: vec![ChatMessage::user("list files")],
            stream: false,
            tools: Some(vec![ToolSpec::function(
                "shell",
                "Execute shell command",
                serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            )]),
            tool_choice: Some("auto".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"tools\""));
        assert!(json.contains("\"tool_choice\":\"auto\""));
        let back: ChatRequest = serde_json::from_str(&json).unwrap();
        assert!(back.tools.is_some());
        assert_eq!(back.tools.unwrap().len(), 1);
    }

    // T-E-S-51: GenerateRequest/GenerateOptions 序列化测试。
    #[test]
    fn generate_request_without_options_omits_field() {
        let req = GenerateRequest {
            model: "qwen2.5-coder:0.5b",
            prompt: "hello",
            stream: false,
            options: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        // options 字段必须被 skip_serializing_if 省略 — wire format
        // 与未加字段前完全一致(向后兼容)。
        assert!(!json.contains("options"));
        assert!(!json.contains("num_predict"));
        assert!(!json.contains("temperature"));
    }

    #[test]
    fn generate_request_with_options_serializes_fields() {
        let req = GenerateRequest {
            model: "qwen2.5-coder:0.5b",
            prompt: "hello",
            stream: false,
            options: Some(GenerateOptions {
                num_predict: Some(20),
                temperature: Some(0.2),
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"options\""));
        assert!(json.contains("\"num_predict\":20"));
        assert!(json.contains("\"temperature\":0.2"));
    }

    #[test]
    fn generate_options_inline_completion_defaults() {
        let opts = GenerateOptions::inline_completion_defaults();
        assert_eq!(opts.num_predict, Some(20));
        assert_eq!(opts.temperature, Some(0.2));
    }

    // ---- T-E-D-02: warmup_model 测试 ----

    /// 手写 mock HTTP server:返回指定 body,记录请求路径到 shared counter。
    /// 用 std::net::TcpListener + std::thread::spawn 实现,避免依赖外部 crate。
    struct MockOllamaServer {
        base_url: String,
        generate_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl MockOllamaServer {
        /// `ps_body` 是 /api/ps 的固定响应;对 /api/generate 返回最小成功响应。
        /// `generate_count` 在每次收到 POST /api/generate 时自增。
        fn start(ps_body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");
            let generate_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let gen_counter = generate_count.clone();
            // 设置较短的超时,避免测试卡死。
            listener.set_nonblocking(false).expect("set blocking ok");
            std::thread::spawn(move || {
                // 最多处理 4 个连接后退出,避免线程泄漏。
                for _ in 0..4 {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let req_str = String::from_utf8_lossy(&buf);
                    if req_str.starts_with("GET /api/ps") {
                        let body = ps_body;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                    } else if req_str.starts_with("POST /api/generate") {
                        gen_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let body = r#"{"model":"x","response":"","done":true}"#;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                    } else {
                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
                        let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                    }
                }
            });
            Self {
                base_url,
                generate_count,
            }
        }

        fn generate_calls(&self) -> usize {
            self.generate_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    /// T-E-D-02: 模型已加载时(/api/ps 返回目标模型),warmup_model 不发 generate。
    #[tokio::test]
    async fn test_warmup_model_already_loaded() {
        // /api/ps 返回的模型列表中包含目标模型。
        let ps_body = r#"{"models":[{"name":"qwen2.5:3b"},{"name":"test-model"}]}"#;
        let server = MockOllamaServer::start(ps_body);
        let client = OllamaClient::new(server.base_url.clone());
        client.warmup_model("test-model").await.unwrap();
        // 不应触发 /api/generate。
        assert_eq!(server.generate_calls(), 0);
    }

    /// T-E-D-02: 模型未加载时(/api/ps 返回空),warmup_model 触发 POST /api/generate。
    #[tokio::test]
    async fn test_warmup_model_triggers_load() {
        // /api/ps 返回空模型列表。
        let ps_body = r#"{"models":[]}"#;
        let server = MockOllamaServer::start(ps_body);
        let client = OllamaClient::new(server.base_url.clone());
        client.warmup_model("test-model").await.unwrap();
        // 应触发一次 /api/generate。
        assert_eq!(server.generate_calls(), 1);
    }

    // ---- T-E-C-02: ChatMessage images 字段测试 ----

    /// T-E-C-02: ChatMessage 含 images 字段时序列化/反序列化 round-trip。
    #[test]
    fn test_chat_message_with_images() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "describe this image".to_string(),
            images: vec!["iVBORw0KGgoAAAANSUhEUg==".to_string()],
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).unwrap();
        // JSON 应包含 "images" 字段且为数组。
        assert!(json.contains("\"images\""));
        assert!(json.contains("iVBORw0KGgoAAAANSUhEUg=="));
        // 反序列化回 ChatMessage,字段应保持一致。
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.content, "describe this image");
        assert_eq!(back.images.len(), 1);
        assert_eq!(back.images[0], "iVBORw0KGgoAAAANSUhEUg==");
    }

    /// T-E-C-02: 空 images Vec 不应出现在序列化 JSON 中(向后兼容)。
    #[test]
    fn test_chat_message_skip_empty_images() {
        let msg = ChatMessage::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        // 空 images 不应出现在 JSON 中(skip_serializing_if = Vec::is_empty)。
        assert!(
            !json.contains("images"),
            "empty images must be omitted from JSON; got: {json}"
        );
        // 反序列化仍应得到空 Vec(serde default)。
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert!(back.images.is_empty());
    }

    /// T-E-C-02: 构造带 images 的 ChatRequest 并校验请求体。
    /// 与 describe_image 调用路径保持一致 — Ollama /api/chat 期望
    /// `{"model","messages":[{"role","content","images"}],"stream":false}`。
    #[test]
    fn test_describe_image_payload() {
        let msg = ChatMessage {
            role: "user".to_string(),
            content: "what do you see?".to_string(),
            images: vec!["BASE64PNGDATA".to_string()],
            ..Default::default()
        };
        let req = ChatRequest {
            model: "qwen2.5-vl:3b".to_string(),
            messages: vec![msg],
            stream: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&req).unwrap();
        // 请求体应包含 model / messages / stream / images 字段。
        assert!(json.contains("\"model\":\"qwen2.5-vl:3b\""));
        assert!(json.contains("\"messages\""));
        assert!(json.contains("\"stream\":false"));
        assert!(json.contains("\"images\""));
        assert!(json.contains("BASE64PNGDATA"));
        // 反序列化 round-trip 保持字段一致。
        let back: ChatRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, "qwen2.5-vl:3b");
        assert_eq!(back.messages.len(), 1);
        assert_eq!(back.messages[0].images.len(), 1);
        assert_eq!(back.messages[0].images[0], "BASE64PNGDATA");
        // tools / tool_choice 字段不应出现(默认 None 被 skip)。
        assert!(!json.contains("\"tools\""));
        assert!(!json.contains("\"tool_choice\""));
    }

    // ---- T-E-S-XX: chat_with_retry 测试 ----

    /// Mock HTTP server for /api/chat: always responds with a fixed
    /// HTTP `status` and counts requests. Mirrors the existing
    /// `MockOllamaServer` style (manual std::net::TcpListener +
    /// std::thread::spawn, no external mockito dep).
    struct MockChatServer {
        base_url: String,
        chat_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl MockChatServer {
        /// `always_status` is the HTTP status returned for every
        /// POST /api/chat. On 200, returns a minimal valid
        /// `ChatResponse` body so `chat()` can deserialize.
        fn start(always_status: u16) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");
            let chat_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let cc = chat_count.clone();
            listener.set_nonblocking(false).expect("set blocking ok");
            std::thread::spawn(move || {
                // Accept up to 8 connections so retries have room;
                // leftover accepts are simply never made (the thread
                // exits when the test process tears down).
                for _ in 0..8 {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let req_str = String::from_utf8_lossy(&buf);
                    if !req_str.starts_with("POST /api/chat") {
                        continue;
                    }
                    cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let status_text = match always_status {
                        200 => "200 OK",
                        400 => "400 Bad Request",
                        503 => "503 Service Unavailable",
                        _ => "500 Internal Server Error",
                    };
                    let body = if always_status == 200 {
                        r#"{"model":"x","message":{"role":"assistant","content":"hi"},"done":true}"#
                    } else {
                        ""
                    };
                    let resp = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        status_text,
                        body.len(),
                        body
                    );
                    let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                }
            });
            Self {
                base_url,
                chat_count,
            }
        }

        fn chat_calls(&self) -> usize {
            self.chat_count.load(std::sync::atomic::Ordering::SeqCst)
        }

        /// M3 #49: 启动一个 mock 服务器,每个请求处理前 sleep `delay`。
        /// 用于测试 Semaphore 限流:串行 vs 并行执行时间可区分。
        fn start_with_delay(always_status: u16, delay: Duration) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");
            let chat_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let cc = chat_count.clone();
            listener.set_nonblocking(false).expect("set blocking ok");
            std::thread::spawn(move || {
                // 接受足够多连接以容纳 3 并发 + retry 余量。
                for _ in 0..16 {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let req_str = String::from_utf8_lossy(&buf);
                    if !req_str.starts_with("POST /api/chat") {
                        continue;
                    }
                    cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // M3 #49: 模拟推理延迟。
                    std::thread::sleep(delay);
                    let status_text = match always_status {
                        200 => "200 OK",
                        400 => "400 Bad Request",
                        503 => "503 Service Unavailable",
                        _ => "500 Internal Server Error",
                    };
                    let body = if always_status == 200 {
                        r#"{"model":"x","message":{"role":"assistant","content":"hi"},"done":true}"#
                    } else {
                        ""
                    };
                    let resp = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        status_text,
                        body.len(),
                        body
                    );
                    let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                }
            });
            Self {
                base_url,
                chat_count,
            }
        }
    }

    /// Connect error (port unbound) is retryable: chat_with_retry
    /// returns Err and does not hang. We can't easily count attempts
    /// on a dead port from the client side, so we assert that:
    /// (1) the call returns Err, and (2) the error is classified as
    /// retryable by `is_retryable`. Combined with the timing bound
    /// (small retry_delay + max_retries keeps total wait bounded),
    /// this confirms the retry loop ran.
    #[tokio::test]
    async fn test_retry_on_network_error() {
        // Bind a port, then immediately release it, so the port is
        // unbound and TCP connect gets RST ("connection refused").
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let base_url = format!("http://127.0.0.1:{port}");

        let client = OllamaClient::new_with_timeout(base_url, Duration::from_millis(200))
            .with_retry_config(2, Duration::from_millis(10));
        let result = client
            .chat_with_retry("test-model", &[ChatMessage::user("hi")])
            .await;
        assert!(result.is_err(), "expected error on dead port");
        // Connect-refused is exactly the kind of error we retry.
        assert!(
            is_retryable(&result.unwrap_err()),
            "connect error should be classified retryable"
        );
    }

    /// 4xx (Bad Request) must NOT be retried: chat_with_retry makes
    /// exactly 1 HTTP request and propagates the error.
    #[tokio::test]
    async fn test_no_retry_on_4xx() {
        let server = MockChatServer::start(400);
        let client =
            OllamaClient::new_with_timeout(server.base_url.clone(), Duration::from_secs(2))
                .with_retry_config(3, Duration::from_millis(10));
        let result = client
            .chat_with_retry("test-model", &[ChatMessage::user("hi")])
            .await;
        assert!(result.is_err(), "400 should propagate as error");
        // Even with max_retries=3, only 1 attempt should be made.
        assert_eq!(
            server.chat_calls(),
            1,
            "4xx must not be retried (expected 1 attempt)"
        );
    }

    /// 5xx is retried up to max_retries times: chat_with_retry makes
    /// max_retries + 1 attempts (1 initial + max_retries retries)
    /// before giving up.
    #[tokio::test]
    async fn test_retry_exhausted() {
        let server = MockChatServer::start(503);
        let client =
            OllamaClient::new_with_timeout(server.base_url.clone(), Duration::from_secs(2))
                .with_retry_config(2, Duration::from_millis(10));
        let result = client
            .chat_with_retry("test-model", &[ChatMessage::user("hi")])
            .await;
        assert!(result.is_err(), "exhausted retries should yield Err");
        // max_retries=2 → total attempts = 3.
        assert_eq!(
            server.chat_calls(),
            3,
            "expected max_retries+1 = 3 attempts on persistent 503"
        );
    }

    // M3 #49: 默认并发上限为 DEFAULT_OLLAMA_MAX_CONCURRENCY (2)。
    #[test]
    fn test_default_max_concurrency() {
        let client = OllamaClient::new("http://127.0.0.1:1");
        assert_eq!(client.max_concurrency(), DEFAULT_OLLAMA_MAX_CONCURRENCY);
        assert_eq!(client.max_concurrency(), 2);
        // 默认无请求在飞 → available == max。
        assert_eq!(client.available_concurrency(), 2);
    }

    // M3 #49: with_max_concurrency 配置自定义上限,0 自动 clamp 到 1。
    #[test]
    fn test_with_max_concurrency_clamps_zero_to_one() {
        let client = OllamaClient::new("http://127.0.0.1:1").with_max_concurrency(0);
        assert_eq!(client.max_concurrency(), 1);
        assert_eq!(client.available_concurrency(), 1);
    }

    // M3 #49: with_max_concurrency 配置更高上限。
    #[test]
    fn test_with_max_concurrency_higher() {
        let client = OllamaClient::new("http://127.0.0.1:1").with_max_concurrency(4);
        assert_eq!(client.max_concurrency(), 4);
        assert_eq!(client.available_concurrency(), 4);
    }

    // M3 #49: Semaphore 限流实际生效。
    // 3 个并发请求 + max_concurrency=1 → 串行执行,
    // 总耗时 >= 3 × per_request_delay(60ms) = 180ms。
    #[tokio::test]
    async fn test_concurrency_limit_serializes_with_max_one() {
        use std::time::Instant;

        // 服务器每个请求延迟 60ms,让串行/并行执行时间可区分。
        let server = MockChatServer::start_with_delay(200, Duration::from_millis(60));
        let client =
            OllamaClient::new_with_timeout(server.base_url.clone(), Duration::from_secs(5))
                .with_max_concurrency(1);

        // 预创建消息 Vec 避免 temporary lifetime 问题。
        let m1: Vec<ChatMessage> = vec![ChatMessage::user("a")];
        let m2: Vec<ChatMessage> = vec![ChatMessage::user("b")];
        let m3: Vec<ChatMessage> = vec![ChatMessage::user("c")];

        let start = Instant::now();
        let (r1, r2, r3) = tokio::join!(
            client.chat("m", &m1),
            client.chat("m", &m2),
            client.chat("m", &m3),
        );
        let elapsed = start.elapsed();

        // 三个请求都应成功
        assert!(r1.is_ok() && r2.is_ok() && r3.is_ok());
        // max_concurrency=1 → 串行:3 × 60ms = 180ms 是下限。
        // 并行(无 Semaphore)则约 60ms。
        assert!(
            elapsed >= Duration::from_millis(180),
            "expected >= 180ms (serialized by Semaphore=1), got {:?}",
            elapsed
        );
        // 上限:留出网络/调度开销,500ms 足够。
        assert!(
            elapsed < Duration::from_millis(1000),
            "expected < 1000ms (3 × 60ms + overhead), got {:?}",
            elapsed
        );
    }

    // M3 #49: max_concurrency=2 时,3 个请求分 2 批:
    // 第 1 批 2 个并行(60ms),第 2 批 1 个(60ms) → 总 ~120ms。
    #[tokio::test]
    async fn test_concurrency_limit_allows_two_parallel() {
        use std::time::Instant;

        let server = MockChatServer::start_with_delay(200, Duration::from_millis(60));
        let client =
            OllamaClient::new_with_timeout(server.base_url.clone(), Duration::from_secs(5))
                .with_max_concurrency(2);

        let m1: Vec<ChatMessage> = vec![ChatMessage::user("a")];
        let m2: Vec<ChatMessage> = vec![ChatMessage::user("b")];
        let m3: Vec<ChatMessage> = vec![ChatMessage::user("c")];

        let start = Instant::now();
        let (r1, r2, r3) = tokio::join!(
            client.chat("m", &m1),
            client.chat("m", &m2),
            client.chat("m", &m3),
        );
        let elapsed = start.elapsed();

        assert!(r1.is_ok() && r2.is_ok() && r3.is_ok());
        // max=2: 第 1 批 2 个并行(60ms),第 3 个等 1 个空位后再 60ms → >= 120ms。
        assert!(
            elapsed >= Duration::from_millis(120),
            "expected >= 120ms (max=2: 2 batches), got {:?}",
            elapsed
        );
        // 上限:CI 环境(macOS/Windows runner)调度开销大,单线程 mock
        // 服务器实际串行处理 3 请求(3×60ms=180ms)+ TCP/tokio 开销。
        // 放宽到 3000ms 避免 flaky(原 360ms 在慢 CI 上偶发超限)。
        assert!(
            elapsed < Duration::from_millis(3000),
            "expected < 3000ms (max=2 with CI overhead), got {:?}",
            elapsed
        );
    }

    // M5 #74: 流式 mock 服务器 — 返回 NDJSON 多行响应。
    struct MockStreamServer {
        base_url: String,
    }

    impl MockStreamServer {
        /// 启动一个返回 NDJSON 流式响应的 mock 服务器。
        /// `lines` 是 NDJSON 每行的内容(已序列化 JSON 字符串)。
        /// 服务器将所有行合并为一个 chunk 返回(模拟流式但单次发送)。
        fn start(lines: Vec<String>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");
            listener.set_nonblocking(false).expect("set blocking ok");
            std::thread::spawn(move || {
                for _ in 0..4 {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    // 不校验路径,任何 POST 都返回 NDJSON。
                    let body = lines.join("\n");
                    // 用 Content-Length 而非 Transfer-Encoding: chunked,
                    // 避免 reqwest 期望 chunked 格式(hex size + data + 0\r\n\r\n)。
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\
                         Content-Length: {}\r\n\r\n{}",
                        body.len(),
                        body,
                    );
                    let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                }
            });
            Self { base_url }
        }

        /// 启动一个返回 HTTP 错误的 mock 服务器(用于测试错误流)。
        fn start_with_error(status: u16) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");
            listener.set_nonblocking(false).expect("set blocking ok");
            std::thread::spawn(move || {
                for _ in 0..4 {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let status_text = match status {
                        400 => "400 Bad Request",
                        500 => "500 Internal Server Error",
                        _ => "500 Internal Server Error",
                    };
                    let body = r#"{"error":"test error"}"#;
                    let resp = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\n\r\n{}",
                        status_text,
                        body.len(),
                        body,
                    );
                    let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                }
            });
            Self { base_url }
        }
    }

    /// M5 #74: chat_stream() 正确解析 NDJSON 流,产出多个 StreamToken。
    #[tokio::test]
    async fn test_chat_stream_parses_ndjson() {
        use futures::StreamExt;

        let lines = vec![
            r#"{"model":"x","message":{"role":"assistant","content":"Hel"},"done":false}"#
                .to_string(),
            r#"{"model":"x","message":{"role":"assistant","content":"lo"},"done":false}"#
                .to_string(),
            r#"{"model":"x","message":{"role":"assistant","content":""},"done":true}"#.to_string(),
        ];
        let server = MockStreamServer::start(lines);
        let client = OllamaClient::new(server.base_url.clone());

        let mut stream = client.chat_stream("x", vec![ChatMessage::user("hi")]);
        let mut tokens = Vec::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(token) => tokens.push(token),
                Err(e) => panic!("unexpected error: {e}"),
            }
        }

        // 应该收到 3 个 token(2 个文本 + 1 个 done)。
        assert_eq!(tokens.len(), 3, "expected 3 tokens, got {}", tokens.len());
        assert_eq!(tokens[0].text, "Hel");
        assert_eq!(tokens[0].done, false);
        assert_eq!(tokens[1].text, "lo");
        assert_eq!(tokens[1].done, false);
        assert_eq!(tokens[2].done, true, "last token should have done=true");
        // 拼接文本应等于完整输出。
        let full: String = tokens.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(full, "Hello");
    }

    /// M5 #74: chat_stream() 在 HTTP 错误时返回 Err 流。
    #[tokio::test]
    async fn test_chat_stream_error_on_http_400() {
        use futures::StreamExt;

        let server = MockStreamServer::start_with_error(400);
        let client = OllamaClient::new(server.base_url.clone());

        let mut stream = client.chat_stream("x", vec![ChatMessage::user("hi")]);
        let result = stream.next().await;
        assert!(result.is_some(), "stream should yield at least one item");
        match result.unwrap() {
            Ok(_) => panic!("expected Err on HTTP 400"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(
                    msg.contains("400") || msg.contains("Bad Request"),
                    "error should mention HTTP 400, got: {msg}"
                );
            }
        }
    }

    /// M5 #74: chat_stream() 在连接失败时返回 Err 流。
    #[tokio::test]
    async fn test_chat_stream_error_on_dead_port() {
        use futures::StreamExt;

        // Bind a port, then immediately release it.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let base_url = format!("http://127.0.0.1:{port}");

        let client = OllamaClient::new_with_timeout(base_url, Duration::from_millis(200));
        let mut stream = client.chat_stream("x", vec![ChatMessage::user("hi")]);
        let result = stream.next().await;
        assert!(result.is_some());
        match result.unwrap() {
            Ok(_) => panic!("expected Err on dead port"),
            Err(e) => {
                let msg = format!("{e}");
                assert!(
                    msg.contains("streaming chat request failed"),
                    "error should mention request failure, got: {msg}"
                );
            }
        }
    }

    /// M5 #74: chat_stream() 受 Semaphore 限流(共享 max_concurrency)。
    /// 与 chat() 共享同一个 Semaphore,所以并发上限相同。
    #[tokio::test]
    async fn test_chat_stream_shares_semaphore_with_chat() {
        // 仅验证 chat_stream 不会绕过 Semaphore 配置。
        // max_concurrency=1 时,流式请求也受限。
        let client = OllamaClient::new("http://127.0.0.1:1").with_max_concurrency(1);
        assert_eq!(client.max_concurrency(), 1);
        assert_eq!(client.available_concurrency(), 1);
        // 创建流(未 await,不消耗 permit)。
        let _stream = client.chat_stream("x", vec![ChatMessage::user("hi")]);
        // permit 尚未获取(在 stream! 块内 await 才获取)。
        assert_eq!(client.available_concurrency(), 1);
    }

    /// M5 #74: chat_stream() 跳过无法解析的 NDJSON 行,继续处理后续行。
    #[tokio::test]
    async fn test_chat_stream_skips_unparseable_lines() {
        use futures::StreamExt;

        let lines = vec![
            r#"{"model":"x","message":{"role":"assistant","content":"ok"},"done":false}"#
                .to_string(),
            // 故意放入无法解析的行(非 JSON)。
            "this is not json".to_string(),
            r#"{"model":"x","message":{"role":"assistant","content":"!"},"done":true}"#.to_string(),
        ];
        let server = MockStreamServer::start(lines);
        let client = OllamaClient::new(server.base_url.clone());

        let mut stream = client.chat_stream("x", vec![ChatMessage::user("hi")]);
        let mut tokens = Vec::new();
        while let Some(result) = stream.next().await {
            match result {
                Ok(token) => tokens.push(token),
                Err(e) => panic!("unexpected error: {e}"),
            }
        }

        // 跳过坏行后,应收到 2 个 token("ok" + done)。
        assert_eq!(
            tokens.len(),
            2,
            "expected 2 tokens (bad line skipped), got {}",
            tokens.len()
        );
        assert_eq!(tokens[0].text, "ok");
        assert_eq!(tokens[1].done, true);
    }
}

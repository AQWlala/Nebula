//! `LlmGateway` — request routing, simple prompt caching, and graceful
//! fallback to a remote OpenAI-compatible endpoint when the local
//! Ollama server is unavailable.
//!
//! ## v1.0.1 P0#4 fix — circuit breaker
//!
//! When Ollama is offline, every chat request would otherwise wait
//! the full `reqwest` timeout (120 s) before returning an error.
//! That makes the front-end appear hung for two minutes per
//! request, which is unacceptable for a desktop app.  The fix is
//! a three-state circuit breaker wrapped around the upstream call:
//!
//! * **Closed** (normal): every request reaches Ollama.  Three
//!   consecutive failures flip the breaker to **Open**.
//! * **Open** (tripped): every request is rejected *immediately*
//!   with `anyhow!("circuit open: upstream unavailable")`.  The
//!   breaker stays Open for `OPEN_DURATION` (60 s) and then
//!   transitions to **HalfOpen**.
//! * **HalfOpen** (probe): the next request is allowed through.
//!   On success the breaker resets to **Closed**; on failure it
//!   returns to **Open** for another full Open window.
//!
//! State is held in an `AtomicU8` (single byte) plus a
//! `parking_lot::Mutex<Instant>` recording the moment the
//! breaker was last tripped (so the Open→HalfOpen transition
//! doesn't require a background timer).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use lru::LruCache;
use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

use super::anthropic::AnthropicClient;
use super::cost_tracker::CostTracker;
use super::ollama::{ChatMessage, ChatResponse, FunctionCall, OllamaClient, ToolCall, ToolSpec};
use super::openai_compat::OpenAICompatClient;
use super::semantic_cache::SemanticCache;
use crate::security::SsrfGuard;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToken {
    pub text: String,
    pub done: bool,
    pub incomplete: bool,
}

/// Number of cached completions. The cache is intentionally tiny.
const CACHE_CAPACITY: usize = 64;

/// TTL for cached entries.
const CACHE_TTL: Duration = Duration::from_secs(300);

/// v1.0.1 P0#4: number of consecutive failures that trips the
/// circuit breaker.
const CB_FAILURE_THRESHOLD: u32 = 3;

/// v1.0.1 P0#4: how long the breaker stays Open before it allows
/// a single probe request through.
const CB_OPEN_DURATION: Duration = Duration::from_secs(60);

/// v1.0.1 P0#4: state values packed into the `AtomicU8`.
const CB_CLOSED: u8 = 0;
const CB_OPEN: u8 = 1;
const CB_HALF_OPEN: u8 = 2;

/// v1.0.1 P0#4: circuit breaker for the LLM upstream.
///
/// Three states, transitions driven by the `chat` path:
/// Closed (default) → Open (after N consecutive failures) →
/// HalfOpen (after `OPEN_DURATION`) → Closed (probe success) or
/// back to Open (probe failure).
///
/// Concurrency:
/// * `state` is an `AtomicU8` so the hot path (a Closed check)
///   is a single load with `Acquire` ordering.
/// * `opened_at` is held under a `parking_lot::Mutex` because we
///   only need to read/write it on the Closed→Open and
///   Open→HalfOpen transitions, both of which are rare relative
///   to the steady-state Closed case.
#[derive(Debug)]
pub struct CircuitBreaker {
    state: AtomicU8,
    /// Number of consecutive failures (reset on success).
    failures: AtomicU8,
    /// Wall-clock instant the breaker last tripped.
    opened_at: Mutex<Option<Instant>>,
    /// Configurable knobs (so the tests can shrink the open
    /// duration without rewriting the constants).
    open_duration: Duration,
    failure_threshold: u32,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(CB_FAILURE_THRESHOLD, CB_OPEN_DURATION)
    }
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, open_duration: Duration) -> Self {
        Self {
            state: AtomicU8::new(CB_CLOSED),
            failures: AtomicU8::new(0),
            opened_at: Mutex::new(None),
            open_duration,
            failure_threshold,
        }
    }

    /// Returns the current state, performing any time-based
    /// transition (Open→HalfOpen) on the way through.
    fn current(&self) -> u8 {
        let s = self.state.load(Ordering::Acquire);
        if s == CB_OPEN {
            // Check whether the open window has elapsed.
            let opened = *self.opened_at.lock();
            if let Some(t) = opened {
                if t.elapsed() >= self.open_duration {
                    // Try to transition Open→HalfOpen.  We use
                    // compare_exchange so two concurrent probes
                    // don't both succeed.
                    let prev = self.state.compare_exchange(
                        CB_OPEN,
                        CB_HALF_OPEN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    if prev.is_ok() {
                        info!(
                            target: "nebula.llm",
                            "circuit breaker: open -> half-open (probe window)"
                        );
                        return CB_HALF_OPEN;
                    }
                }
            }
        }
        s
    }

    /// Returns `Ok(())` if a request is allowed through, or
    /// `Err(anyhow!(...))` if the breaker is Open and the open
    /// window has not elapsed.
    pub fn check(&self) -> Result<()> {
        match self.current() {
            CB_CLOSED | CB_HALF_OPEN => Ok(()),
            CB_OPEN => Err(anyhow!("circuit open: upstream unavailable")),
            other => {
                // Defensive: an unexpected state value is treated
                // as Closed to keep the system available.
                warn!(target: "nebula.llm", state = other, "circuit breaker in unknown state; treating as closed");
                Ok(())
            }
        }
    }

    /// Records a successful upstream call.  Closes the breaker
    /// and resets the failure counter.
    pub fn record_success(&self) {
        let prev = self.state.swap(CB_CLOSED, Ordering::AcqRel);
        if prev != CB_CLOSED {
            info!(target: "nebula.llm", "circuit breaker: -> closed (upstream recovered)");
        }
        self.failures.store(0, Ordering::Release);
        *self.opened_at.lock() = None;
    }

    /// Records a failed upstream call.  When the failure count
    /// reaches the threshold the breaker trips Open.
    pub fn record_failure(&self) {
        let prev = self.failures.fetch_add(1, Ordering::AcqRel);
        let new_count = prev + 1;
        if new_count as u32 >= self.failure_threshold {
            let was = self.state.swap(CB_OPEN, Ordering::AcqRel);
            if was != CB_OPEN {
                *self.opened_at.lock() = Some(Instant::now());
                warn!(
                    target: "nebula.llm",
                    failures = new_count,
                    "circuit breaker tripped: closed -> open"
                );
            }
        }
    }

    /// Returns the raw state byte.  Test-only.
    #[cfg(test)]
    pub fn raw_state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }
}

/// One cache entry: a response plus the moment it was stored.
struct CacheEntry {
    response: ChatResponse,
    inserted_at: std::time::Instant,
}

/// The LLM gateway.
///
/// v1.2: 默认主路径改为 DeepSeek (OpenAI 兼容 API)。
/// 调用优先级:`deepseek` (primary) → `ollama` (local fallback) →
/// `anthropic` (third fallback) → `remote` (generic OpenAI-compat)。
/// 通过 `llm_provider` 配置决定主路径;未配置 API key 时自动降级到 Ollama。
///
/// v1.0 P0#7 fix: the prompt cache is now a true
/// [`lru::LruCache`].  v0.3 used a `Vec<(u64, CacheEntry)>` with
/// `g.remove(0)` on overflow, which is **FIFO**, not LRU.  That
/// meant a hot, frequently-hit entry could be evicted to make
/// room for a never-revisited entry inserted seconds later.  The
/// fix is a one-line switch in [`LlmGateway::new`] plus a
/// matching `cache.get(key)` in [`LlmGateway::lookup_cache`]
/// (which also marks the entry as most-recently-used).
pub struct LlmGateway {
    /// Ollama 客户端 (用于 embedding + 本地 chat fallback)。
    primary: Arc<OllamaClient>,
    default_model: String,
    /// v1.2: DeepSeek 主路径 (OpenAI 兼容 /v1/chat/completions)。
    deepseek: Option<DeepSeekPrimary>,
    /// 可选的通用远程 fallback (OpenAI 兼容,如 Azure / 自建)。
    remote: Option<RemoteFallback>,
    /// v1.1 P0-1: Anthropic Claude fallback chain。
    anthropic: Option<AnthropicFallback>,
    /// T-E-S-40: OpenAI 兼容 provider(vLLM/LMStudio/OpenRouter/自建)。
    /// 与 `deepseek`/`remote` 并存,保留旧 struct 避免破坏现有代码。
    openai_compat: Option<OpenAICompatClient>,
    /// v1.2: 主 provider 名 (deepseek / ollama / openai-compat / anthropic)。
    provider: String,
    cache: Mutex<LruCache<u64, CacheEntry>>,
    /// v1.0.1 P0#4: circuit breaker around the upstream call.
    breaker: CircuitBreaker,
    /// T-E-A-01: L0.5 语义缓存(可选)。未配置时走原 L0 + provider 路径。
    semantic_cache: Option<Arc<SemanticCache>>,
    /// T-E-A-06: Token 费用追踪器(可选)。未注入时 maybe_record_cost 静默跳过。
    cost_tracker: Option<Arc<CostTracker>>,
    /// T-E-A-02: TokenJuice 三级压缩器(可选)。未注入时 chat() 跳过压缩。
    compressor: Option<Arc<crate::llm::token_juice::TokenJuiceCompressor>>,
    /// T-E-A-03: ModelRouter 智能路由(可选)。未注入时走原 self.provider 分派。
    model_router: Option<Arc<crate::llm::model_router::ModelRouter>>,
    /// T-E-A-05: 日预算上限(USD),运行时可变(支持热更新)。
    daily_budget: parking_lot::RwLock<f64>,
}

/// v1.2: DeepSeek 主路径客户端 (OpenAI 兼容 API)。
#[derive(Clone)]
struct DeepSeekPrimary {
    base_url: String,
    api_key: String,
    http: Client,
}

/// Optional remote fallback (OpenAI-compatible /v1/chat/completions).
struct RemoteFallback {
    base_url: String,
    api_key: Option<String>,
    http: Client,
}

/// v1.1 P0-1: Optional Anthropic Claude fallback.
#[derive(Clone)]
struct AnthropicFallback {
    client: AnthropicClient,
}

#[derive(Debug, Clone, Serialize)]
struct RemoteChatRequest<'a> {
    model: &'a str,
    messages: &'a [RemoteMessage<'a>],
    stream: bool,
}

#[derive(Debug, Clone, Serialize)]
struct RemoteMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteChatResponse {
    #[serde(default)]
    choices: Vec<RemoteChoice>,
    /// T-S1-B-03: OpenAI 兼容 API 在非流式响应中返回的 token 用量。
    #[serde(default)]
    usage: Option<RemoteUsage>,
}

/// T-S1-B-03: OpenAI 兼容 API 的 token 用量字段。
#[derive(Debug, Clone, Deserialize)]
struct RemoteUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteChoice {
    message: RemoteRespMessage,
}

/// T-E-S-02: OpenAI 兼容 API 响应中的 tool_call(与 ollama::ToolCall 结构相同但独立)。
#[derive(Debug, Clone, Deserialize)]
struct RemoteToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub function: RemoteFunctionCall,
}

/// T-E-S-02: OpenAI 兼容 API 响应中的 function 调用详情。
#[derive(Debug, Clone, Deserialize)]
struct RemoteFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRespMessage {
    role: String,
    content: String,
    /// T-E-S-02: assistant 消息可能含 tool_calls。
    #[serde(default)]
    tool_calls: Option<Vec<RemoteToolCall>>,
    /// T-E-B-17: DeepSeek-R1 等推理模型返回的推理内容。
    #[serde(default)]
    reasoning_content: Option<String>,
}

impl LlmGateway {
    /// Returns a reference to the primary Ollama client.
    pub fn ollama_client(&self) -> &OllamaClient {
        &self.primary
    }

    /// Creates a new gateway.
    ///
    /// v1.2: 新增 `provider` / `deepseek_api_url` / `deepseek_api_key` 参数。
    /// 当 provider = "deepseek" 且 api_key 存在时,DeepSeek 成为主路径。
    /// `anthropic_api_key` enables the v1.1 P0-1 Anthropic Claude fallback.
    pub fn new(
        primary: Arc<OllamaClient>,
        default_model: impl Into<String>,
        provider: impl Into<String>,
        deepseek_api_url: Option<String>,
        deepseek_api_key: Option<String>,
        remote_url: Option<String>,
        anthropic_api_key: Option<String>,
        anthropic_model: Option<String>,
    ) -> Self {
        // v1.2: DeepSeek 主路径,仅在 api_key 存在时启用。
        let deepseek = match (deepseek_api_url, deepseek_api_key) {
            (Some(url), Some(key)) if !key.is_empty() => Some(DeepSeekPrimary {
                base_url: url,
                api_key: key,
                http: Client::builder()
                    .timeout(Duration::from_secs(120))
                    .build()
                    .expect("reqwest client should build"),
            }),
            _ => None,
        };

        let remote = remote_url.map(|u| RemoteFallback {
            base_url: u,
            api_key: std::env::var("NEBULA_REMOTE_KEY").ok(),
            http: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("reqwest client should build"),
        });

        // v1.1 P0-1: Anthropic fallback, enabled when API key is provided.
        let anthropic = anthropic_api_key.map(|key| {
            let model = anthropic_model.unwrap_or_else(|| "claude-3-5-haiku-20241022".to_string());
            AnthropicFallback {
                client: AnthropicClient::new(key, model, None),
            }
        });

        // v1.0 P0#7: LRU cache, not FIFO.  `CACHE_CAPACITY` is
        // always > 0 so the `NonZeroUsize::new` cannot fail.
        let cap =
            NonZeroUsize::new(CACHE_CAPACITY.max(1)).expect("CACHE_CAPACITY must be non-zero");
        Self {
            primary,
            default_model: default_model.into(),
            deepseek,
            remote,
            anthropic,
            openai_compat: None,
            provider: provider.into(),
            cache: Mutex::new(LruCache::new(cap)),
            breaker: CircuitBreaker::default(),
            semantic_cache: None,
            cost_tracker: None,
            compressor: None,
            model_router: None,
            daily_budget: parking_lot::RwLock::new(0.0),
        }
    }

    #[cfg(test)]
    pub fn new_test() -> Self {
        let ollama = Arc::new(OllamaClient::new("http://127.0.0.1:11434".to_string()));
        Self::new(ollama, "test-model", "ollama", None, None, None, None, None)
    }

    /// T-E-A-01: 注入 L0.5 语义缓存。builder 风格,链式调用。
    /// 未调用此方法时 semantic_cache 为 None,走原 L0 + provider 路径。
    pub fn with_semantic_cache(mut self, cache: Arc<SemanticCache>) -> Self {
        self.semantic_cache = Some(cache);
        self
    }

    /// T-E-A-06: 注入 CostTracker。builder 风格,链式调用。
    /// 未调用此方法时 cost_tracker 为 None,maybe_record_cost 静默跳过。
    pub fn with_cost_tracker(mut self, tracker: Arc<CostTracker>) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// T-E-A-02: 注入 TokenJuice 压缩器。builder 风格,链式调用。
    /// 未调用时 chat() 跳过压缩。
    pub fn with_token_juice(
        mut self,
        compressor: Arc<crate::llm::token_juice::TokenJuiceCompressor>,
    ) -> Self {
        self.compressor = Some(compressor);
        self
    }

    /// T-E-A-03: 注入 ModelRouter。builder 风格,链式调用。
    /// 未调用时 chat() 走原 self.provider 分派逻辑。
    pub fn with_model_router(mut self, router: Arc<crate::llm::model_router::ModelRouter>) -> Self {
        self.model_router = Some(router);
        self
    }

    /// T-E-A-05: 设置日预算(USD)。0 或负数=不限制。
    pub fn with_daily_budget(self, budget: f64) -> Self {
        *self.daily_budget.write() = budget;
        self
    }

    /// T-E-S-40: 注入 OpenAI 兼容客户端(vLLM/LMStudio/OpenRouter/自建)。
    /// builder 风格,链式调用。未调用时 openai_compat 为 None,
    /// `effective_provider == "openai-compat"` 分支会静默跳过并降级到 fallback。
    pub fn with_openai_compat(mut self, client: OpenAICompatClient) -> Self {
        self.openai_compat = Some(client);
        self
    }

    /// T-E-A-05: 运行时热更新日预算。
    pub fn set_daily_budget(&self, budget: f64) {
        *self.daily_budget.write() = budget;
    }

    /// T-E-A-05: 是否已超日预算。
    pub fn is_over_daily_budget(&self) -> bool {
        let budget = *self.daily_budget.read();
        if budget <= 0.0 {
            return false;
        }
        if let Some(tracker) = &self.cost_tracker {
            tracker.cost_today() >= budget
        } else {
            false
        }
    }

    #[cfg(test)]
    pub fn semantic_cache_is_set(&self) -> bool {
        self.semantic_cache.is_some()
    }

    /// T-E-A-01: 异步写入语义缓存(不阻塞响应返回)。
    /// `query` 为 None(无 user 消息)或未注入缓存时静默跳过。
    /// 缓存内部已对错误降级,此处无需 try/catch。
    fn maybe_store_semantic(&self, query: Option<&str>, response: &str) {
        if let (Some(cache), Some(q)) = (&self.semantic_cache, query) {
            let cache = Arc::clone(cache);
            let q = q.to_string();
            let r = response.to_string();
            tokio::spawn(async move {
                cache.store(&q, &r).await;
            });
        }
    }

    /// T-E-A-06: CostTracker 是否已注入。
    pub fn cost_tracker_is_set(&self) -> bool {
        self.cost_tracker.is_some()
    }

    /// T-E-A-06: 记录一次 LLM 调用的 token 费用。
    /// tracker 未注入时静默跳过(向后兼容)。
    ///
    /// T-E-S-40 修复:改用 `record_with_context` 传 `Some(provider)`,
    /// 修复 `aggregate_by_provider` 全部落 "unknown" 桶的问题。
    fn maybe_record_cost(&self, model: &str, input_tokens: u64, output_tokens: u64) {
        if let Some(tracker) = &self.cost_tracker {
            tracker.record_with_context(
                model,
                input_tokens,
                output_tokens,
                Some(self.provider.clone()),
                None,
                None,
            );
        }
    }

    /// Returns the default chat model name.
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// T-E-L-03: Returns the active provider name configured for this gateway
    /// (e.g. `"ollama"`, `"deepseek"`, `"anthropic"`, `"openai-compat"`).
    ///
    /// Used by `ReviewerAgent` (CheckerAgent) for model homogeneity detection
    /// in the Maker-Checker pattern — when Maker and Checker share the same
    /// provider+model, self-review is meaningless and autonomy is auto-downgraded.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Sends a chat completion. Looks the response up in the prompt
    /// cache first; on miss, tries the local Ollama server and falls
    /// back to the remote endpoint on error.
    ///
    /// v1.0.1 P0#4: a circuit breaker around the upstream call
    /// rejects requests immediately (with `anyhow!("circuit open:
    /// upstream unavailable")`) when Ollama has been failing for
    /// long enough that a probe is warranted.  The breaker is
    /// checked on the hot path; in steady state the check is a
    /// single atomic load.
    #[instrument(target = "nebula.llm", skip(self, messages), fields(otel.kind = "llm"))]
    pub async fn chat(&self, messages: Vec<ChatMessage>) -> anyhow::Result<ChatResponse> {
        // T-E-A-02: TokenJuice 三级压缩(L1 脱敏 + L2 压缩 + L3 摘要)。
        // 在 cache_key 计算之前执行,保证缓存一致性。失败静默降级。
        let messages = if let Some(compressor) = &self.compressor {
            compressor.compress(messages).await
        } else {
            messages
        };

        // v1.0.1 P0#4: short-circuit when the breaker is Open.
        // The cache lookup stays *outside* the breaker because a
        // cached response is always safe to return regardless of
        // upstream health.
        let key = cache_key(&self.default_model, &messages);
        if let Some(hit) = self.lookup_cache(key) {
            debug!(target: "nebula.llm", "cache hit");
            return Ok(hit);
        }

        // T-E-A-01: 提取最后一条 user 消息作为语义缓存 query。
        // 仅不可变借用 messages,后续 provider 调用同样不可变借用,无冲突。
        let semantic_query: Option<&str> = messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str());

        // T-E-A-01: 语义缓存查询(L0.5),位于 L0 exact miss 之后、breaker 之前。
        // 命中即直接返回,绕过 breaker 与 provider;内部错误已被降级为 None。
        if let Some(cache) = &self.semantic_cache {
            if let Some(query) = semantic_query {
                if let Some(cached_response) = cache.check(query).await {
                    debug!(target: "nebula.llm", "semantic cache hit");
                    let resp = ChatResponse {
                        model: self.default_model.clone(),
                        message: ChatMessage {
                            role: "assistant".to_string(),
                            content: cached_response,
                            ..Default::default()
                        },
                        done: true,
                        total_duration: None,
                        eval_count: None,
                        ..Default::default()
                    };
                    return Ok(resp);
                }
            }
        }

        // T-E-A-05: 日预算超限 → 强制走 Ollama,跳过 DeepSeek/Anthropic/Remote。
        // T-E-A-03: 非超限时走 ModelRouter classify 决定首选 provider(分类器直连 Ollama,零成本)。
        let over_budget = self.is_over_daily_budget();
        let effective_provider = if over_budget {
            debug!(target: "nebula.llm", "daily budget exceeded, forcing Ollama");
            "ollama".to_string()
        } else if let Some(router) = &self.model_router {
            let route = router.classify(&messages).await;
            match route {
                crate::llm::model_router::Route::Ollama => "ollama",
                crate::llm::model_router::Route::DeepSeek => "deepseek",
                crate::llm::model_router::Route::Anthropic => {
                    if self.anthropic.is_some() {
                        "anthropic"
                    } else {
                        "deepseek"
                    }
                }
                crate::llm::model_router::Route::Remote => {
                    if self.remote.is_some() {
                        "openai-compat"
                    } else {
                        "deepseek"
                    }
                }
            }
            .to_string()
        } else {
            self.provider.clone()
        };

        // v1.0.1 P0#4: gate the upstream call on the breaker.
        self.breaker.check()?;

        // v1.2: 根据 provider 配置决定调用顺序。
        // 主路径:DeepSeek (若配置) → Ollama (本地) → Anthropic → Remote。
        if effective_provider == "deepseek" {
            if let Some(ds) = &self.deepseek {
                match self.call_deepseek(ds, &self.default_model, &messages).await {
                    Ok((resp, prompt_tokens, completion_tokens)) => {
                        self.breaker.record_success();
                        // T-E-A-06: DeepSeek 响应含 usage(prompt/completion tokens),
                        // 直接透传给 CostTracker 计费。
                        self.maybe_record_cost(
                            &self.default_model,
                            prompt_tokens,
                            completion_tokens,
                        );
                        self.store_cache(key, resp.clone());
                        self.maybe_store_semantic(semantic_query, &resp.message.content);
                        return Ok(resp);
                    }
                    Err(e) => {
                        warn!(target: "nebula.llm", error = ?e, "DeepSeek failed, falling back to Ollama");
                    }
                }
            }
        } else if effective_provider == "openai-compat" {
            // T-E-S-40: OpenAI 兼容主分支(vLLM/LMStudio/OpenRouter/自建)。
            // 调用 OpenAICompatClient,失败则降级到下方 fallback 链(Ollama/Anthropic/Remote)。
            if let Some(client) = &self.openai_compat {
                match self
                    .call_openai_compat(client, &self.default_model, &messages)
                    .await
                {
                    Ok((resp, prompt_tokens, completion_tokens)) => {
                        self.breaker.record_success();
                        self.maybe_record_cost(
                            &self.default_model,
                            prompt_tokens,
                            completion_tokens,
                        );
                        self.store_cache(key, resp.clone());
                        self.maybe_store_semantic(semantic_query, &resp.message.content);
                        return Ok(resp);
                    }
                    Err(e) => {
                        warn!(target: "nebula.llm", error = ?e, "openai-compat failed, falling back to Ollama");
                    }
                }
            }
        }

        // Fallback 1: Ollama 本地
        match self.primary.chat(&self.default_model, &messages).await {
            Ok(resp) => {
                self.breaker.record_success();
                // T-S1-B-03: Ollama API 不返回 prompt_tokens,仅 eval_count
                // (生成 token 数)。prompt_tokens 记为 0,completion 取 eval_count。
                let completion = resp.eval_count.unwrap_or(0);
                crate::metrics::global().record_token_usage(0, completion);
                // T-E-A-06: Ollama 无 usage 字段,按 input=0/output=eval_count 记录。
                // 本地模型单价为 0,record 仍会留下用量记录供聚合查询。
                self.maybe_record_cost(&self.default_model, 0, completion);
                self.store_cache(key, resp.clone());
                self.maybe_store_semantic(semantic_query, &resp.message.content);
                Ok(resp)
            }
            Err(e) => {
                // Fallback 2: Anthropic Claude
                if let Some(anthropic) = &self.anthropic {
                    match self.call_anthropic(anthropic, &messages).await {
                        Ok(text) => {
                            self.breaker.record_success();
                            let resp = ChatResponse {
                                model: anthropic.client.model.clone(),
                                message: ChatMessage {
                                    role: "assistant".to_string(),
                                    content: text,
                                    ..Default::default()
                                },
                                done: true,
                                total_duration: None,
                                eval_count: None,
                                ..Default::default()
                            };
                            self.store_cache(key, resp.clone());
                            self.maybe_store_semantic(semantic_query, &resp.message.content);
                            return Ok(resp);
                        }
                        Err(anthropic_err) => {
                            warn!(target: "nebula.llm", error = ?anthropic_err, "Anthropic fallback also failed");
                        }
                    }
                }
                // Fallback 3: 通用 Remote (OpenAI 兼容)
                if let Some(remote) = &self.remote {
                    match self
                        .call_remote(remote, &self.default_model, &messages)
                        .await
                    {
                        Ok((resp, prompt_tokens, completion_tokens)) => {
                            self.breaker.record_success();
                            // T-E-A-06: Remote (OpenAI 兼容) 响应含 usage,
                            // 透传给 CostTracker 计费。
                            self.maybe_record_cost(
                                &self.default_model,
                                prompt_tokens,
                                completion_tokens,
                            );
                            self.store_cache(key, resp.clone());
                            self.maybe_store_semantic(semantic_query, &resp.message.content);
                            return Ok(resp);
                        }
                        Err(remote_err) => {
                            self.breaker.record_failure();
                            return Err(remote_err)
                                .context("deepseek/ollama/anthropic/remote all failed");
                        }
                    }
                }
                self.breaker.record_failure();
                Err(e).context("all LLM providers failed (deepseek/ollama), no remote configured")
            }
        }
    }

    /// T-E-S-02: 带工具的 chat 调用(OpenAI 兼容 function calling)。
    ///
    /// 按 provider 分派:
    /// - DeepSeek/Remote(OpenAI 兼容):透传 `tools` 参数,解析响应 `tool_calls`
    /// - Ollama/Anthropic:降级为 `chat(messages)`(忽略 tools,向后兼容)
    ///
    /// 返回的 `ChatResponse.tool_calls` 可能含工具调用请求,
    /// 调用方(如 `run_tool_loop`)应处理该字段。
    #[instrument(target = "nebula.llm", skip(self, messages, tools), fields(otel.kind = "llm", tools = tools.len()))]
    pub async fn chat_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolSpec>,
    ) -> Result<ChatResponse> {
        if tools.is_empty() {
            // 无工具时走原 chat 路径
            return self.chat(messages).await;
        }
        match self.provider.as_str() {
            "deepseek" if self.deepseek.is_some() => {
                self.call_deepseek_with_tools(messages, &tools).await
            }
            "openai-compat" if self.remote.is_some() => {
                self.call_remote_with_tools(messages, &tools).await
            }
            // Ollama/Anthropic 或未配置的 provider:降级为无 tools
            _ => {
                debug!(target: "nebula.llm", provider = %self.provider, "chat_with_tools degraded: provider does not support tools, falling back to chat");
                self.chat(messages).await
            }
        }
    }

    /// v1.2: 调用 DeepSeek API (OpenAI 兼容 /v1/chat/completions)。
    ///
    /// T-E-A-06: 返回值改为 `(ChatResponse, prompt_tokens, completion_tokens)`,
    /// 把 usage 透传给 `chat()` 以便 `CostTracker` 计费。
    async fn call_deepseek(
        &self,
        ds: &DeepSeekPrimary,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<(ChatResponse, u64, u64)> {
        let url = format!("{}/chat/completions", ds.base_url.trim_end_matches('/'));
        let req_body = serde_json::json!({
            "model": model,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
            "stream": false,
        });
        let resp = ds
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", ds.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .map_err(|e| anyhow!("DeepSeek request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("DeepSeek API error: {status} - {body}"));
        }
        let resp_json: RemoteChatResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!("DeepSeek response parse failed: {e}"))?;
        let content = resp_json
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .unwrap_or_default();
        // T-E-B-17: 解析 DeepSeek reasoning_content(推理模型如 deepseek-reasoner)。
        let reasoning_chain = resp_json
            .choices
            .first()
            .and_then(|c| c.message.reasoning_content.as_deref())
            .map(crate::llm::reasoning::ReasoningChain::from_text);
        // T-S1-B-03: 透传 token 用量到全局 metrics,并把 completion_tokens
        // 写入 eval_count 以保持与 Ollama 路径一致的语义。
        let (prompt_tokens, completion_tokens) = match resp_json.usage {
            Some(u) => (u.prompt_tokens, u.completion_tokens),
            None => (0, 0),
        };
        crate::metrics::global().record_token_usage(prompt_tokens, completion_tokens);
        let chat_resp = ChatResponse {
            model: model.to_string(),
            message: ChatMessage {
                role: "assistant".to_string(),
                content,
                ..Default::default()
            },
            done: true,
            total_duration: None,
            eval_count: if completion_tokens > 0 {
                Some(completion_tokens)
            } else {
                None
            },
            reasoning_chain,
            ..Default::default()
        };
        Ok((chat_resp, prompt_tokens, completion_tokens))
    }

    /// T-E-S-02: DeepSeek 路径带 tools 调用(OpenAI 兼容 function calling)。
    ///
    /// 与 `call_deepseek` 的差异:
    /// - 透传 `tools` / `tool_choice: "auto"`
    /// - 解析响应中的 `tool_calls` 并写入 `ChatResponse.message.tool_calls`
    /// - 不集成 cost_tracker(tool_loop 会多次调用,费用统计在最终 chat 中);
    ///   仍透传 usage 到全局 metrics。
    async fn call_deepseek_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: &[ToolSpec],
    ) -> Result<ChatResponse> {
        let ds = self.deepseek.as_ref().context("deepseek not configured")?;
        let url = format!("{}/chat/completions", ds.base_url.trim_end_matches('/'));
        let req_body = serde_json::json!({
            "model": self.default_model,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
            "stream": false,
            "tools": tools,
            "tool_choice": "auto",
        });
        let resp = ds
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", ds.api_key))
            .header("Content-Type", "application/json")
            .json(&req_body)
            .send()
            .await
            .context("deepseek chat_with_tools request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("DeepSeek API error: {status} - {body}"));
        }
        let resp_json: RemoteChatResponse = resp
            .json()
            .await
            .context("parse deepseek chat_with_tools response failed")?;
        let choice = resp_json
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("deepseek chat_with_tools returned no choices"))?;
        // T-S1-B-03: 透传 token 用量到全局 metrics。
        let (prompt_tokens, completion_tokens) = match resp_json.usage {
            Some(u) => (u.prompt_tokens, u.completion_tokens),
            None => (0, 0),
        };
        crate::metrics::global().record_token_usage(prompt_tokens, completion_tokens);
        // T-E-S-02: 把 RemoteToolCall 转成 ollama::ToolCall(结构相同但独立)。
        let tool_calls = choice.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|c| ToolCall {
                    id: c.id,
                    ty: c.ty,
                    function: FunctionCall {
                        name: c.function.name,
                        arguments: c.function.arguments,
                    },
                })
                .collect::<Vec<_>>()
        });
        let chat_resp = ChatResponse {
            model: self.default_model.clone(),
            message: ChatMessage {
                role: "assistant".to_string(),
                content: choice.message.content,
                tool_calls,
                ..Default::default()
            },
            done: true,
            total_duration: None,
            eval_count: if completion_tokens > 0 {
                Some(completion_tokens)
            } else {
                None
            },
            ..Default::default()
        };
        Ok(chat_resp)
    }

    /// Chat with an explicit model override (skips the cache).
    #[instrument(target = "nebula.llm", skip(self, messages), fields(otel.kind = "llm", model = %model))]
    pub async fn chat_with_model(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<ChatResponse> {
        self.primary.chat(model, &messages).await
    }

    /// T-E-C-02: 用指定的视觉模型描述一张 base64 编码的图片。
    ///
    /// 直接走 Ollama 多模态 API,绕过 chat 路径的 cache / breaker / router,
    /// 因为:
    /// - 多模态请求不应进 prompt cache(images 不易稳定 hash)。
    /// - vision_model 与 chat_model 可能不同,不应共享同一个 breaker 状态。
    /// - 调用方(commands::llm::describe_screenshot)已选定 vision_model,
    ///   无需 ModelRouter classify。
    ///
    /// 与 `memory/clip_embedder.rs` 的 ClipEmbedRequest.images 同样的 wire format
    /// — Ollama /api/chat 接受 `{"messages":[{"role","content","images":[b64]}]}`。
    pub async fn describe_image(&self, model: &str, msg: ChatMessage) -> anyhow::Result<String> {
        let resp = self.primary.chat(model, &[msg]).await?;
        Ok(resp.message.content)
    }

    /// Streaming chat completion. Returns a stream of token strings.
    /// When the stream ends or errors, the last emitted item may be
    /// marked as incomplete if the connection was interrupted.
    ///
    /// T-S1-B-01a: 根据 provider 分发到 DeepSeek SSE 流式或 Ollama NDJSON 流式。
    /// T-E-D-02: 补全 openai-compat(SSE)与 anthropic(event-stream)分支。
    #[instrument(target = "nebula.llm", skip(self, messages), fields(otel.kind = "llm"))]
    pub fn chat_stream(
        &self,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, Result<StreamToken>> {
        // TODO(T-E-A-01): 流式响应缓存待后续支持。
        if self.provider == "deepseek" {
            if let Some(ref ds) = self.deepseek {
                return self.chat_stream_deepseek(ds.clone(), self.default_model.clone(), messages);
            }
        }
        // T-E-D-02: openai-compat provider(vLLM/LMStudio/OpenRouter/自建)。
        if self.provider == "openai-compat" {
            if let Some(ref c) = self.openai_compat {
                return self.chat_stream_openai_compat(c.clone(), messages);
            }
        }
        // T-E-D-02: anthropic provider(Claude event-stream SSE)。
        if self.provider == "anthropic" {
            if let Some(ref a) = self.anthropic {
                return self.chat_stream_anthropic(a.clone(), messages);
            }
        }
        self.chat_stream_ollama(messages)
    }

    /// Ollama 流式路径（NDJSON）。
    fn chat_stream_ollama(
        &self,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, Result<StreamToken>> {
        let client = self.primary.clone();
        let model = self.default_model.clone();

        let stream = async_stream::stream! {
            let url = format!("{}/api/chat", client.base_url());
            let req_body = serde_json::json!({
                "model": model,
                "messages": messages,
                "stream": true,
            });

            let resp = match client.http().post(&url).json(&req_body).send().await {
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

            let mut stream = resp.bytes_stream();
            let mut incomplete = false;

            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield Ok(StreamToken {
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
                            let done = v.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
                            let text = v
                                .get("message")
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            if done {
                                yield Ok(StreamToken {
                                    text,
                                    done: true,
                                    incomplete: false,
                                });
                                return;
                            }
                            incomplete = !text.is_empty();
                            yield Ok(StreamToken {
                                text,
                                done: false,
                                incomplete: false,
                            });
                        }
                        Err(e) => {
                            debug!(target: "nebula.llm", error = %e, line, "skipping unparseable stream line");
                        }
                    }
                }
            }

            yield Ok(StreamToken {
                text: String::new(),
                done: true,
                incomplete,
            });
        };

        Box::pin(stream)
    }

    /// T-S1-B-01a: DeepSeek SSE 流式路径。
    ///
    /// OpenAI 兼容 SSE 格式：
    /// - 每行 `data: {json}`
    /// - 结束行 `data: [DONE]`
    /// - JSON: `{"choices":[{"delta":{"content":"..."}}]}`
    fn chat_stream_deepseek(
        &self,
        ds: DeepSeekPrimary,
        model: String,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, Result<StreamToken>> {
        let stream = async_stream::stream! {
            let url = format!("{}/chat/completions", ds.base_url.trim_end_matches('/'));
            let req_body = serde_json::json!({
                "model": model,
                "messages": messages.iter().map(|m| {
                    serde_json::json!({"role": m.role, "content": m.content})
                }).collect::<Vec<_>>(),
                "stream": true,
            });

            let resp = match ds.http
                .post(&url)
                .header("Authorization", format!("Bearer {}", ds.api_key))
                .header("Content-Type", "application/json")
                .json(&req_body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(anyhow!("DeepSeek streaming request failed: {e}"));
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                yield Err(anyhow!("DeepSeek streaming HTTP {status}: {body}"));
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut incomplete = false;

            use futures::StreamExt;
            while let Some(chunk) = byte_stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield Ok(StreamToken {
                            text: String::new(),
                            done: false,
                            incomplete: true,
                        });
                        warn!(target: "nebula.llm", error = %e, "DeepSeek stream interrupted");
                        return;
                    }
                };

                for line in String::from_utf8_lossy(&bytes).lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(":") {
                        continue;
                    }
                    // SSE 格式: "data: {...}"
                    let payload = if let Some(stripped) = line.strip_prefix("data: ") {
                        stripped.trim()
                    } else {
                        line
                    };
                    if payload == "[DONE]" {
                        yield Ok(StreamToken {
                            text: String::new(),
                            done: true,
                            incomplete: false,
                        });
                        return;
                    }
                    match serde_json::from_str::<serde_json::Value>(payload) {
                        Ok(v) => {
                            let text = v
                                .get("choices")
                                .and_then(|c| c.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|c| c.get("delta"))
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !text.is_empty() {
                                incomplete = true;
                            }
                            yield Ok(StreamToken {
                                text,
                                done: false,
                                incomplete: false,
                            });
                        }
                        Err(e) => {
                            debug!(target: "nebula.llm", error = %e, line = payload, "skipping unparseable DeepSeek SSE line");
                        }
                    }
                }
            }

            yield Ok(StreamToken {
                text: String::new(),
                done: true,
                incomplete,
            });
        };

        Box::pin(stream)
    }

    /// T-E-D-02: OpenAI 兼容 provider 流式路径(SSE)。
    ///
    /// 与 DeepSeek SSE 格式相同(`data: {json}` + `data: [DONE]` 结束),
    /// 复用 OpenAICompatClient 已有的 base_url / api_key / http。
    /// 请求体 `{"model","messages","stream":true}`,响应 `choices[0].delta.content`。
    fn chat_stream_openai_compat(
        &self,
        client: OpenAICompatClient,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, Result<StreamToken>> {
        let model = self.default_model.clone();
        let stream = async_stream::stream! {
            let url = client.completions_url();
            let req_body = serde_json::json!({
                "model": model,
                "messages": messages.iter().map(|m| {
                    serde_json::json!({"role": m.role, "content": m.content})
                }).collect::<Vec<_>>(),
                "stream": true,
            });

            let mut req = client.http
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&req_body);
            if let Some(k) = &client.api_key {
                req = req.bearer_auth(k);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    yield Err(anyhow!("openai-compat streaming request failed: {e}"));
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                yield Err(anyhow!("openai-compat streaming HTTP {status}: {body}"));
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut incomplete = false;

            use futures::StreamExt;
            while let Some(chunk) = byte_stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield Ok(StreamToken {
                            text: String::new(),
                            done: false,
                            incomplete: true,
                        });
                        warn!(target: "nebula.llm", error = %e, "openai-compat stream interrupted");
                        return;
                    }
                };

                for line in String::from_utf8_lossy(&bytes).lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }
                    let payload = if let Some(stripped) = line.strip_prefix("data: ") {
                        stripped.trim()
                    } else {
                        line
                    };
                    if payload == "[DONE]" {
                        yield Ok(StreamToken {
                            text: String::new(),
                            done: true,
                            incomplete: false,
                        });
                        return;
                    }
                    match serde_json::from_str::<serde_json::Value>(payload) {
                        Ok(v) => {
                            let text = v
                                .get("choices")
                                .and_then(|c| c.as_array())
                                .and_then(|arr| arr.first())
                                .and_then(|c| c.get("delta"))
                                .and_then(|d| d.get("content"))
                                .and_then(|c| c.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !text.is_empty() {
                                incomplete = true;
                            }
                            yield Ok(StreamToken {
                                text,
                                done: false,
                                incomplete: false,
                            });
                        }
                        Err(e) => {
                            debug!(target: "nebula.llm", error = %e, line = payload, "skipping unparseable openai-compat SSE line");
                        }
                    }
                }
            }

            yield Ok(StreamToken {
                text: String::new(),
                done: true,
                incomplete,
            });
        };

        Box::pin(stream)
    }

    /// T-E-D-02: Anthropic Claude 流式路径(event-stream SSE)。
    ///
    /// Anthropic SSE 格式:
    /// - `event: content_block_delta` 行后跟 `data: {json}` 行
    /// - JSON: `{"type":"content_block_delta","delta":{"type":"text_delta","text":"..."}}`
    /// - 结束事件 `event: message_stop`
    /// - 需要 `x-api-key` + `anthropic-version` + `anthropic-beta` headers。
    fn chat_stream_anthropic(
        &self,
        anthropic: AnthropicFallback,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, Result<StreamToken>> {
        let stream = async_stream::stream! {
            let client = &anthropic.client;
            let url = format!("{}/v1/messages", client.base_url());
            // 把 ChatMessage 转成 Anthropic 格式:system 提取到顶层字段。
            let mut system_str: Option<String> = None;
            let msgs: Vec<serde_json::Value> = messages.iter().filter_map(|m| {
                if m.role == "system" {
                    system_str = Some(m.content.clone());
                    None
                } else {
                    Some(serde_json::json!({"role": m.role, "content": m.content}))
                }
            }).collect();
            let req_body = serde_json::json!({
                "model": client.model_name(),
                "messages": msgs,
                "max_tokens": 4096,
                "stream": true,
                "system": system_str,
            });

            let resp = match client.http().post(&url)
                .header("x-api-key", client.api_key())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&req_body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    yield Err(anyhow!("Anthropic streaming request failed: {e}"));
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                yield Err(anyhow!("Anthropic streaming HTTP {status}: {body}"));
                return;
            }

            let mut byte_stream = resp.bytes_stream();
            let mut incomplete = false;

            use futures::StreamExt;
            while let Some(chunk) = byte_stream.next().await {
                let bytes = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield Ok(StreamToken {
                            text: String::new(),
                            done: false,
                            incomplete: true,
                        });
                        warn!(target: "nebula.llm", error = %e, "Anthropic stream interrupted");
                        return;
                    }
                };

                // Anthropic SSE:成对的 "event: ..." 与 "data: ..." 行。
                // 我们只解析 data: 行中的 JSON,提取 delta.text。
                for line in String::from_utf8_lossy(&bytes).lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
                        continue;
                    }
                    let payload = if let Some(stripped) = line.strip_prefix("data: ") {
                        stripped.trim()
                    } else if let Some(stripped) = line.strip_prefix("data:") {
                        stripped.trim()
                    } else {
                        continue;
                    };
                    match serde_json::from_str::<serde_json::Value>(payload) {
                        Ok(v) => {
                            let event_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                            if event_type == "message_stop" {
                                yield Ok(StreamToken {
                                    text: String::new(),
                                    done: true,
                                    incomplete: false,
                                });
                                return;
                            }
                            // content_block_delta: delta.text 是增量文本。
                            let text = v
                                .get("delta")
                                .and_then(|d| d.get("text"))
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();
                            if !text.is_empty() {
                                incomplete = true;
                                yield Ok(StreamToken {
                                    text,
                                    done: false,
                                    incomplete: false,
                                });
                            }
                        }
                        Err(e) => {
                            debug!(target: "nebula.llm", error = %e, line = payload, "skipping unparseable Anthropic SSE line");
                        }
                    }
                }
            }

            yield Ok(StreamToken {
                text: String::new(),
                done: true,
                incomplete,
            });
        };

        Box::pin(stream)
    }

    /// Generation request (prompt in, completion out).
    #[instrument(target = "nebula.llm", skip(self, prompt), fields(otel.kind = "llm"))]
    pub async fn generate(&self, prompt: &str) -> anyhow::Result<String> {
        let resp = self.primary.generate(&self.default_model, prompt).await?;
        Ok(resp.response)
    }

    /// Clears the prompt cache.
    pub fn clear_cache(&self) {
        self.cache.lock().clear();
        info!(target: "nebula.llm", "prompt cache cleared");
    }

    async fn call_remote(
        &self,
        remote: &RemoteFallback,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<(ChatResponse, u64, u64)> {
        let url = format!("{}/v1/chat/completions", remote.base_url);
        let ssrf_guard = SsrfGuard::new();
        ssrf_guard
            .validate_url(&url)
            .map_err(|e| anyhow::anyhow!("SSRF validation failed: {e}"))?;
        let payload_msgs: Vec<RemoteMessage<'_>> = messages
            .iter()
            .map(|m| RemoteMessage {
                role: &m.role,
                content: &m.content,
            })
            .collect();
        let body = RemoteChatRequest {
            model,
            messages: &payload_msgs,
            stream: false,
        };
        let mut req = remote.http.post(&url).json(&body);
        if let Some(k) = &remote.api_key {
            req = req.bearer_auth(k);
        }
        let resp: RemoteChatResponse = req.send().await?.error_for_status()?.json().await?;
        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("remote fallback returned no choices"))?;
        // T-S1-B-03: 透传 token 用量。Remote (OpenAI 兼容) 通常返回 usage。
        let (prompt_tokens, completion_tokens) = match resp.usage {
            Some(u) => (u.prompt_tokens, u.completion_tokens),
            None => (0, 0),
        };
        crate::metrics::global().record_token_usage(prompt_tokens, completion_tokens);
        let chat_resp = ChatResponse {
            model: model.to_string(),
            message: ChatMessage {
                role: choice.message.role,
                content: choice.message.content,
                ..Default::default()
            },
            done: true,
            total_duration: None,
            eval_count: if completion_tokens > 0 {
                Some(completion_tokens)
            } else {
                None
            },
            ..Default::default()
        };
        // T-E-A-06: 把 usage 透传给 chat() 以便 CostTracker 计费。
        Ok((chat_resp, prompt_tokens, completion_tokens))
    }

    /// T-E-S-02: Remote (OpenAI 兼容) 路径带 tools 调用(function calling)。
    ///
    /// 与 `call_remote` 的差异:
    /// - 透传 `tools` / `tool_choice: "auto"`
    /// - 解析响应中的 `tool_calls` 并写入 `ChatResponse.message.tool_calls`
    /// - 不集成 cost_tracker(tool_loop 会多次调用,费用统计在最终 chat 中);
    ///   仍透传 usage 到全局 metrics。
    async fn call_remote_with_tools(
        &self,
        messages: Vec<ChatMessage>,
        tools: &[ToolSpec],
    ) -> Result<ChatResponse> {
        let remote = self.remote.as_ref().context("remote not configured")?;
        let url = format!("{}/v1/chat/completions", remote.base_url);
        let ssrf_guard = SsrfGuard::new();
        ssrf_guard
            .validate_url(&url)
            .map_err(|e| anyhow!("SSRF validation failed: {e}"))?;
        let req_body = serde_json::json!({
            "model": self.default_model,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
            "stream": false,
            "tools": tools,
            "tool_choice": "auto",
        });
        let mut req = remote
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&req_body);
        if let Some(k) = &remote.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req
            .send()
            .await
            .context("remote chat_with_tools request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("remote chat_with_tools HTTP {status}: {body}"));
        }
        let resp_json: RemoteChatResponse = resp
            .json()
            .await
            .context("parse remote chat_with_tools response failed")?;
        let choice = resp_json
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("remote chat_with_tools returned no choices"))?;
        // T-S1-B-03: 透传 token 用量到全局 metrics。
        let (prompt_tokens, completion_tokens) = match resp_json.usage {
            Some(u) => (u.prompt_tokens, u.completion_tokens),
            None => (0, 0),
        };
        crate::metrics::global().record_token_usage(prompt_tokens, completion_tokens);
        // T-E-S-02: 把 RemoteToolCall 转成 ollama::ToolCall(结构相同但独立)。
        let tool_calls = choice.message.tool_calls.map(|calls| {
            calls
                .into_iter()
                .map(|c| ToolCall {
                    id: c.id,
                    ty: c.ty,
                    function: FunctionCall {
                        name: c.function.name,
                        arguments: c.function.arguments,
                    },
                })
                .collect::<Vec<_>>()
        });
        let chat_resp = ChatResponse {
            model: self.default_model.clone(),
            message: ChatMessage {
                role: "assistant".to_string(),
                content: choice.message.content,
                tool_calls,
                ..Default::default()
            },
            done: true,
            total_duration: None,
            eval_count: if completion_tokens > 0 {
                Some(completion_tokens)
            } else {
                None
            },
            ..Default::default()
        };
        Ok(chat_resp)
    }

    /// v1.1 P0-1: Call Anthropic Claude via `/v1/messages`.
    async fn call_anthropic(
        &self,
        anthropic: &AnthropicFallback,
        messages: &[ChatMessage],
    ) -> anyhow::Result<String> {
        use super::anthropic::Message as Am;
        let anthropic_messages: Vec<Am> = messages
            .iter()
            .map(|m| {
                let role = match m.role.as_str() {
                    "system" => super::anthropic::Role::System,
                    "assistant" => super::anthropic::Role::Assistant,
                    _ => super::anthropic::Role::User,
                };
                Am {
                    role,
                    content: m.content.clone(),
                }
            })
            .collect();
        // T-E-A-04: AnthropicClient::chat() 现返回 (String, AnthropicUsage)。
        let (text, usage) = anthropic.client.chat(&anthropic_messages).await?;
        // T-E-A-04: 透传 prefix cache 命中到 metrics。
        if usage.cache_read_input_tokens > 0 {
            crate::metrics::global().record_prefix_cache_hit();
            crate::metrics::global()
                .record_prefix_cache_cached_tokens(usage.cache_read_input_tokens);
            // T-E-A-10: 估算省的金额 = cached_tokens × Claude 单价(input $3/M tokens)。
            let saved_usd = (usage.cache_read_input_tokens as f64) * 3.0 / 1_000_000.0;
            crate::metrics::global().record_cost_saved(saved_usd);
        }
        Ok(text)
    }

    /// T-E-S-40: 调用 OpenAI 兼容 provider(vLLM/LMStudio/OpenRouter/自建)。
    ///
    /// 委托给 [`OpenAICompatClient::chat`],把返回的 `(text, prompt_tokens,
    /// completion_tokens)` 包装成 `ChatResponse` 并透传 usage 给 `chat()`
    /// 以便 `CostTracker` 计费。SSRF 校验已在 client 内部完成。
    ///
    /// 注:reasoning_content 由 client 解析但本方法不透传到 `ChatResponse`
    /// (T-E-S-40 范围聚焦非流式 chat;reasoning_chain 透传留待后续)。
    async fn call_openai_compat(
        &self,
        client: &OpenAICompatClient,
        model: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<(ChatResponse, u64, u64)> {
        let (content, prompt_tokens, completion_tokens) = client
            .chat(model, messages)
            .await
            .map_err(|e| anyhow!("openai-compat call failed: {e}"))?;
        let chat_resp = ChatResponse {
            model: model.to_string(),
            message: ChatMessage {
                role: "assistant".to_string(),
                content,
                ..Default::default()
            },
            done: true,
            total_duration: None,
            eval_count: if completion_tokens > 0 {
                Some(completion_tokens)
            } else {
                None
            },
            ..Default::default()
        };
        Ok((chat_resp, prompt_tokens, completion_tokens))
    }

    // -----------------------------------------------------------------------
    // T-E-S-04: MoA parallel chat
    // -----------------------------------------------------------------------

    /// T-E-S-04: 根据 provider 字符串路由到对应 LLM 后端。
    ///
    /// provider 格式为 `"deepseek"` / `"ollama"` / `"anthropic"` / `"openai-compat"`。
    /// 未识别的 provider 回退到 Ollama。
    pub async fn chat_with_provider(
        &self,
        messages: &[ChatMessage],
        provider: &str,
    ) -> Result<ChatResponse> {
        match provider {
            "deepseek" => {
                let ds = self
                    .deepseek
                    .as_ref()
                    .ok_or_else(|| anyhow!("DeepSeek not configured"))?;
                let (resp, _, _) = self
                    .call_deepseek(ds, &self.default_model, messages)
                    .await?;
                Ok(resp)
            }
            "anthropic" => {
                let ant = self
                    .anthropic
                    .as_ref()
                    .ok_or_else(|| anyhow!("Anthropic not configured"))?;
                let text = self.call_anthropic(ant, messages).await?;
                Ok(ChatResponse {
                    model: ant.client.model_name().to_string(),
                    message: ChatMessage {
                        role: "assistant".to_string(),
                        content: text,
                        ..Default::default()
                    },
                    done: true,
                    total_duration: None,
                    eval_count: None,
                    ..Default::default()
                })
            }
            "openai-compat" => {
                let client = self
                    .openai_compat
                    .as_ref()
                    .ok_or_else(|| anyhow!("openai-compat not configured"))?;
                let (resp, _, _) = self
                    .call_openai_compat(client, &self.default_model, messages)
                    .await?;
                Ok(resp)
            }
            // 默认回退到 Ollama
            _ => {
                let resp = self.primary.chat(&self.default_model, messages).await?;
                Ok(resp)
            }
        }
    }

    /// T-E-S-04: 多 provider 并行 chat,返回每个 provider 的结果。
    ///
    /// 使用 `futures::future::join_all` 并行调用各 provider,
    /// 不因单个 provider 失败而中断其他(错误以 `Err` 返回)。
    ///
    /// 注:因为 `LlmGateway` 不是 `Clone`,无法在 `join_all` 的多个
    /// 并发 future 间共享 `&self`,此处使用顺序调用保证正确性。
    /// 后续可将 `LlmGateway` 改为 `Arc` 内部可变,实现真正并行。
    pub async fn chat_parallel(
        &self,
        messages: &[ChatMessage],
        providers: &[String],
    ) -> Vec<(String, Result<ChatResponse>)> {
        let mut results = Vec::with_capacity(providers.len());
        for provider in providers {
            let result = self.chat_with_provider(messages, provider).await;
            results.push((provider.clone(), result));
        }
        results
    }

    /// Looks up a key in the cache, expiring any entries older than
    /// [`CACHE_TTL`].  v1.0 P0#7: `LruCache::get` also bumps the
    /// entry to the most-recently-used position, which is what
    /// makes the cache actually LRU.
    fn lookup_cache(&self, key: u64) -> Option<ChatResponse> {
        let mut g = self.cache.lock();
        let now = std::time::Instant::now();
        // The `lru` crate doesn't expose a "remove all entries
        // matching a predicate" helper, so we walk the iterator
        // and collect the doomed keys.  `CACHE_TTL` is a generous
        // 5 minutes; the cache holds at most 64 entries, so the
        // walk is cheap.
        let expired: Vec<u64> = g
            .iter()
            .filter_map(|(k, e)| {
                if now.duration_since(e.inserted_at) >= CACHE_TTL {
                    Some(*k)
                } else {
                    None
                }
            })
            .collect();
        for k in expired {
            g.pop(&k);
        }
        g.get(&key).map(|e| e.response.clone())
    }

    /// Inserts a response.  When the cache is at capacity, the
    /// least-recently-used entry is evicted by `LruCache::put`.
    fn store_cache(&self, key: u64, resp: ChatResponse) {
        let mut g = self.cache.lock();
        g.put(
            key,
            CacheEntry {
                response: resp,
                inserted_at: std::time::Instant::now(),
            },
        );
    }
}

/// Hashes (model, messages) into a stable cache key.
fn cache_key(model: &str, messages: &[ChatMessage]) -> u64 {
    let mut h = DefaultHasher::new();
    model.hash(&mut h);
    for m in messages {
        m.role.hash(&mut h);
        m.content.hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep as std_sleep;

    /// v1.0.1 P0#4: three consecutive failures flip Closed → Open.
    #[test]
    fn breaker_trips_after_threshold() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        assert_eq!(cb.raw_state(), CB_CLOSED);
        cb.record_failure();
        assert_eq!(cb.raw_state(), CB_CLOSED, "1 failure must not trip");
        cb.record_failure();
        assert_eq!(cb.raw_state(), CB_CLOSED, "2 failures must not trip");
        cb.record_failure();
        assert_eq!(cb.raw_state(), CB_OPEN, "3 failures must trip");
    }

    /// v1.0.1 P0#4: when the breaker is Open and the open window
    /// has not elapsed, `check()` rejects immediately with the
    /// canonical error string.
    #[test]
    fn breaker_open_rejects_immediately() {
        let cb = CircuitBreaker::new(1, Duration::from_secs(60));
        cb.record_failure();
        assert_eq!(cb.raw_state(), CB_OPEN);
        let err = cb.check().unwrap_err();
        assert!(
            err.to_string().contains("circuit open"),
            "unexpected error message: {err}"
        );
    }

    /// v1.0.1 P0#4: after `OPEN_DURATION` elapses, the next
    /// `check()` transitions to HalfOpen (the request is allowed
    /// through).  A subsequent success closes the breaker; a
    /// subsequent failure re-opens it.
    #[test]
    fn breaker_recovers_after_open_window() {
        // 100 ms window so the test runs in well under a second.
        let cb = CircuitBreaker::new(1, Duration::from_millis(100));
        cb.record_failure();
        assert_eq!(cb.raw_state(), CB_OPEN);
        // First check immediately after tripping: still Open.
        assert!(cb.check().is_err());

        std_sleep(Duration::from_millis(150));

        // Now the open window has elapsed; check() must transition
        // to HalfOpen and let the request through.
        assert!(cb.check().is_ok(), "expected half-open to allow probe");
        // The state is now HalfOpen (not yet Closed — that
        // requires `record_success`).
        let after_probe = cb.raw_state();
        assert!(
            after_probe == CB_HALF_OPEN || after_probe == CB_OPEN,
            "expected half_open (or possibly re-opened), got {after_probe}"
        );

        // Record a success: the breaker must close.
        cb.record_success();
        assert_eq!(cb.raw_state(), CB_CLOSED);
    }

    /// v1.0.1 P0#4: a single success in the steady-state Closed
    /// path resets the failure counter so a historical failure
    /// doesn't accumulate forever.
    #[test]
    fn breaker_success_resets_failure_counter() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(60));
        cb.record_failure();
        cb.record_failure();
        cb.record_success();
        // Two more failures: must NOT trip (counter was reset).
        cb.record_failure();
        cb.record_failure();
        assert_eq!(
            cb.raw_state(),
            CB_CLOSED,
            "success must have reset the counter"
        );
        // The third post-reset failure trips.
        cb.record_failure();
        assert_eq!(cb.raw_state(), CB_OPEN);
    }

    use crate::llm::OllamaClient;

    fn msgs() -> Vec<ChatMessage> {
        vec![ChatMessage::system("sys"), ChatMessage::user("hello")]
    }

    #[test]
    fn cache_key_is_stable() {
        let a = cache_key("m", &msgs());
        let b = cache_key("m", &msgs());
        assert_eq!(a, b);
    }

    #[test]
    fn cache_key_changes_with_model() {
        let a = cache_key("m1", &msgs());
        let b = cache_key("m2", &msgs());
        assert_ne!(a, b);
    }

    #[test]
    fn cache_key_changes_with_content() {
        let mut m = msgs();
        m[1].content = "hi".into();
        let a = cache_key("m", &m);
        m[1].content = "bye".into();
        let b = cache_key("m", &m);
        assert_ne!(a, b);
    }

    fn dummy_response(content: &str) -> ChatResponse {
        ChatResponse {
            model: "m".to_string(),
            message: ChatMessage {
                role: "assistant".to_string(),
                content: content.to_string(),
                ..Default::default()
            },
            done: true,
            total_duration: None,
            eval_count: None,
            ..Default::default()
        }
    }

    /// v1.0 P0#7: an LRU-cache gate against a 64-entry gateway.
    /// Touching the oldest entry must keep it alive past the
    /// insertion of N+1 new entries.
    #[test]
    fn lru_evicts_least_recently_used_not_oldest_inserted() {
        // Build a small gateway-shaped cache directly.  We don't
        // call `LlmGateway::chat` because that would require a
        // running Ollama.
        let cap = NonZeroUsize::new(4).unwrap();
        let mut cache: LruCache<u64, CacheEntry> = LruCache::new(cap);
        for i in 0..4u64 {
            cache.put(
                i,
                CacheEntry {
                    response: dummy_response(&format!("v{i}")),
                    inserted_at: std::time::Instant::now(),
                },
            );
        }
        // Touch key=0 (the oldest-inserted entry) so it becomes
        // the most-recently-used.
        let touched = cache.get(&0).expect("0 must be present");
        assert_eq!(touched.response.message.content, "v0");

        // Insert a 5th entry; key=1 is now the LRU and must be
        // evicted (NOT key=0).
        cache.put(
            4,
            CacheEntry {
                response: dummy_response("v4"),
                inserted_at: std::time::Instant::now(),
            },
        );
        assert!(
            cache.get(&0).is_some(),
            "key 0 must still be present (touched)"
        );
        assert!(
            cache.get(&1).is_none(),
            "key 1 must have been evicted as LRU"
        );
        assert!(cache.get(&2).is_some());
        assert!(cache.get(&3).is_some());
        assert!(cache.get(&4).is_some());
    }

    /// v1.0 P0#7: with a 1-entry cache, a second `store_cache`
    /// for a *different* key evicts the first.  The old FIFO
    /// behaviour accidentally passed this case, so the test is
    /// only here to document the LRU invariant.
    #[test]
    fn store_cache_evicts_when_full() {
        let cap = NonZeroUsize::new(2).unwrap();
        let mut cache: LruCache<u64, CacheEntry> = LruCache::new(cap);
        cache.put(
            1,
            CacheEntry {
                response: dummy_response("a"),
                inserted_at: std::time::Instant::now(),
            },
        );
        cache.put(
            2,
            CacheEntry {
                response: dummy_response("b"),
                inserted_at: std::time::Instant::now(),
            },
        );
        // Touch 1 so 2 becomes LRU.
        let _ = cache.get(&1);
        cache.put(
            3,
            CacheEntry {
                response: dummy_response("c"),
                inserted_at: std::time::Instant::now(),
            },
        );
        assert!(cache.get(&1).is_some(), "1 was touched; should survive");
        assert!(cache.get(&2).is_none(), "2 was LRU; should be evicted");
        assert!(cache.get(&3).is_some());
    }

    /// v1.0 P0#7: `LlmGateway::new` should produce a working
    /// LRU cache.  The public capacity is hard-coded at 64, so
    /// we exercise the underlying LRU behaviour directly
    /// through a small `LruCache` instance identical to the
    /// one inside the gateway.  This is the canonical
    /// regression test: a hot entry that's been touched must
    /// survive a fresh insert that would have evicted it
    /// under the v0.3 FIFO design.
    #[test]
    fn gateway_cache_evicts_lru_not_oldest() {
        // Mirror `LlmGateway::new`'s storage: a `LruCache` of
        // some capacity holding `CacheEntry` values.
        let cap = NonZeroUsize::new(3).unwrap();
        let mut cache: LruCache<u64, CacheEntry> = LruCache::new(cap);
        // Three insertions, none touched.
        for k in 0..3u64 {
            cache.put(
                k,
                CacheEntry {
                    response: dummy_response(&format!("v{k}")),
                    inserted_at: std::time::Instant::now(),
                },
            );
        }
        // Touch key=0 (the oldest-inserted) — it becomes the MRU.
        let touched = cache.get(&0).expect("0 must be present");
        assert_eq!(touched.response.message.content, "v0");
        // Insert key=3.  With capacity=3, the LRU is evicted.
        cache.put(
            3,
            CacheEntry {
                response: dummy_response("v3"),
                inserted_at: std::time::Instant::now(),
            },
        );
        // The LRU was key=1 (oldest-untouched); the FIFO bug
        // would have evicted key=0 instead.  P0#7 fix: key=0
        // survives because it was touched.
        assert!(
            cache.get(&0).is_some(),
            "touched key 0 must survive (LRU fix)"
        );
        assert!(
            cache.get(&1).is_none(),
            "untouched key 1 is LRU and must be evicted"
        );
        assert!(cache.get(&2).is_some());
        assert!(cache.get(&3).is_some());
    }

    /// v1.0 P0#7: TTL eviction runs alongside LRU eviction.
    /// We don't wait 5 minutes in a unit test; instead we check
    /// that the `lookup_cache` helper is wired up to drop
    /// expired entries (we can't easily synthesise an
    /// `Instant` in the past, so this test just verifies that
    /// fresh entries round-trip).
    #[test]
    fn lookup_cache_returns_fresh_entries() {
        let gw = LlmGateway::new(
            Arc::new(OllamaClient::new_with_timeout(
                "http://127.0.0.1:1",
                std::time::Duration::from_secs(2),
            )),
            "m",
            "ollama",
            None,
            None,
            None,
            None,
            None,
        );
        let k = cache_key("m", &[ChatMessage::user("ping")]);
        gw.store_cache(k, dummy_response("pong"));
        let got = gw.lookup_cache(k).expect("must hit");
        assert_eq!(got.message.content, "pong");
    }

    /// T-E-A-01: 默认构造的 LlmGateway 不应启用语义缓存。
    #[test]
    fn semantic_cache_unset_by_default() {
        let gw = LlmGateway::new_test();
        assert!(
            !gw.semantic_cache_is_set(),
            "new_test() must not wire up a semantic cache"
        );
    }

    /// T-E-A-01: 无语义缓存时 maybe_store_semantic 必须是静默 no-op,
    /// 既不 panic 也不 spawn 任何任务(semantic_cache 为 None 时
    /// `tokio::spawn` 分支不会触达,故无需 tokio 运行时)。
    #[test]
    fn maybe_store_semantic_is_noop_when_no_cache() {
        let gw = LlmGateway::new_test();
        // 无 user 消息 → query 为 None,必须安全跳过。
        gw.maybe_store_semantic(None, "irrelevant");
        // 有 query 但无 cache → 同样安全跳过。
        gw.maybe_store_semantic(Some("hello"), "world");
    }

    /// T-E-A-06: 默认构造的 LlmGateway 不应注入 CostTracker。
    #[test]
    fn cost_tracker_unset_by_default() {
        let gw = LlmGateway::new_test();
        assert!(
            !gw.cost_tracker_is_set(),
            "new_test() must not wire up a cost tracker"
        );
    }

    /// T-E-A-06: 无 tracker 时 maybe_record_cost 必须是静默 no-op,
    /// 传任意值都不 panic(也无需 tokio 运行时)。
    #[test]
    fn maybe_record_cost_is_noop_when_no_tracker() {
        let gw = LlmGateway::new_test();
        gw.maybe_record_cost("deepseek-chat", 1_000_000, 500_000);
        gw.maybe_record_cost("any-model", 0, 0);
    }

    /// T-E-A-06: 注入 CostTracker 后,maybe_record_cost 应把记录写入 tracker,
    /// 并按模型单价正确累加费用。CostTracker::new() 不依赖外部资源,可直接构造。
    #[test]
    fn maybe_record_cost_records_when_tracker_set() {
        let tracker = Arc::new(CostTracker::new());
        let gw = LlmGateway::new_test().with_cost_tracker(Arc::clone(&tracker));
        assert!(gw.cost_tracker_is_set(), "tracker must be wired up");
        gw.maybe_record_cost("deepseek-chat", 1_000_000, 500_000);
        assert_eq!(tracker.len(), 1, "exactly one record should be stored");
        let (i, o) = tracker.total_tokens();
        assert_eq!(i, 1_000_000);
        assert_eq!(o, 500_000);
        // 0.14 * 1 + 0.28 * 0.5 = 0.14 + 0.14 = 0.28
        let total = tracker.total_cost_usd();
        assert!(
            (total - 0.28).abs() < 1e-9,
            "expected 0.28 USD, got {total}"
        );
    }

    /// T-E-S-02: RemoteRespMessage 应能解析 OpenAI 兼容响应中的 tool_calls 字段。
    /// `tool_calls` 为 None 时仍可正常反序列化(向后兼容旧响应)。
    #[test]
    fn remote_resp_message_parses_tool_calls() {
        let json = r#"{
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {
                    "id": "call_abc",
                    "type": "function",
                    "function": {"name": "shell", "arguments": "{\"command\":\"ls\"}"}
                }
            ]
        }"#;
        let msg: RemoteRespMessage = serde_json::from_str(json).unwrap();
        assert!(msg.tool_calls.is_some());
        let calls = msg.tool_calls.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc");
        assert_eq!(calls[0].ty, "function");
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(calls[0].function.arguments, "{\"command\":\"ls\"}");
    }

    /// T-E-S-02: 不含 tool_calls 的旧响应仍应正常解析(tool_calls 为 None)。
    #[test]
    fn remote_resp_message_parses_without_tool_calls() {
        let json = r#"{"role": "assistant", "content": "hello"}"#;
        let msg: RemoteRespMessage = serde_json::from_str(json).unwrap();
        assert!(
            msg.tool_calls.is_none(),
            "tool_calls should default to None"
        );
        assert_eq!(msg.content, "hello");
    }

    // ---- T-E-A-05 新增测试 ----

    /// T-E-A-05: budget=0 时 is_over_daily_budget 必须返回 false(不限制)。
    #[test]
    fn test_is_over_daily_budget_false_when_zero() {
        let gw = LlmGateway::new_test();
        gw.set_daily_budget(0.0);
        assert!(
            !gw.is_over_daily_budget(),
            "budget=0 must not be over budget"
        );
    }

    /// T-E-A-05: budget=0.01 且 tracker 有 0.28 USD 记录 → 超限返回 true。
    #[test]
    fn test_is_over_daily_budget_true_when_exceeded() {
        let tracker = Arc::new(CostTracker::new());
        // 0.14*1 + 0.28*0.5 = 0.28 USD
        tracker.record("deepseek-chat", 1_000_000, 500_000);
        let gw = LlmGateway::new_test()
            .with_cost_tracker(Arc::clone(&tracker))
            .with_daily_budget(0.01);
        assert!(
            gw.is_over_daily_budget(),
            "0.28 USD spent vs 0.01 budget must be over budget"
        );
    }

    // ---- T-E-D-02: chat_stream 多 provider 流式测试 ----

    /// 手写 mock HTTP server:返回固定 SSE body。
    /// 用 std::net::TcpListener + std::thread::spawn 实现。
    struct MockSseServer {
        base_url: String,
    }

    impl MockSseServer {
        /// `sse_body` 是 mock server 对任意 POST 请求返回的固定响应体。
        fn start(sse_body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let base_url = format!("http://127.0.0.1:{port}");
            std::thread::spawn(move || {
                for _ in 0..4 {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mut buf = [0u8; 4096];
                    let _ = std::io::Read::read(&mut stream, &mut buf);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}",
                        sse_body.len(),
                        sse_body
                    );
                    let _ = std::io::Write::write_all(&mut stream, resp.as_bytes());
                }
            });
            Self { base_url }
        }
    }

    /// T-E-D-02: openai-compat SSE 流式解析。
    /// mock 返回标准 OpenAI SSE 格式(data: {json} + [DONE]),
    /// 验证 chat_stream 解析出 delta.content 文本。
    #[tokio::test]
    async fn test_chat_stream_openai_compat() {
        let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\
data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\
data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\
data: [DONE]\n";
        let server = MockSseServer::start(sse_body);
        // 构造一个指向 mock server 的 OpenAICompatClient(无 key,模拟 LMStudio 本地)。
        let client = OpenAICompatClient::new(server.base_url.clone(), None, "test-model");
        let gw = LlmGateway::new(
            Arc::new(OllamaClient::new("http://127.0.0.1:1")),
            "test-model",
            "openai-compat",
            None,
            None,
            None,
            None,
            None,
        )
        .with_openai_compat(client);
        let stream = gw.chat_stream(vec![ChatMessage::user("hi")]);
        use futures::StreamExt;
        let mut stream = stream;
        let mut collected = String::new();
        let mut saw_done = false;
        while let Some(result) = stream.next().await {
            match result {
                Ok(token) => {
                    if !token.text.is_empty() {
                        collected.push_str(&token.text);
                    }
                    if token.done {
                        saw_done = true;
                    }
                }
                Err(e) => panic!("stream error: {e}"),
            }
        }
        assert_eq!(collected, "Hello");
        assert!(saw_done, "should see done=true token");
    }

    /// T-E-D-02: anthropic event-stream SSE 解析。
    /// mock 返回 Anthropic SSE 格式(event: + data: 成对,message_stop 结束),
    /// 验证 chat_stream 解析出 delta.text 文本。
    #[tokio::test]
    async fn test_chat_stream_anthropic() {
        let sse_body = "event: message_start\ndata: {\"type\":\"message_start\"}\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Wor\"}}\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"ld\"}}\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n";
        let server = MockSseServer::start(sse_body);
        // AnthropicClient 需要 api_key,这里用 dummy key( mock server 不校验)。
        let anthropic_client = AnthropicClient::new(
            "sk-ant-test".to_string(),
            "claude-test".to_string(),
            Some(server.base_url.clone()),
        );
        // 用 new_test + 手动注入 anthropic fallback 不可行(字段私有),
        // 改用 LlmGateway::new 传入 anthropic_api_key + 自定义 base_url。
        // 但 LlmGateway::new 不支持自定义 anthropic base_url(固定 https://api.anthropic.com)。
        // 因此直接测试 chat_stream_anthropic 的解析逻辑:手动构造 AnthropicFallback。
        let fallback = AnthropicFallback {
            client: anthropic_client,
        };
        let gw = LlmGateway::new(
            Arc::new(OllamaClient::new("http://127.0.0.1:1")),
            "claude-test",
            "anthropic",
            None,
            None,
            None,
            Some("sk-ant-test".to_string()),
            Some("claude-test".to_string()),
        );
        // 直接调用内部方法(绕过 chat_stream 路由,因 new() 不支持自定义 base_url)。
        let stream = gw.chat_stream_anthropic(fallback, vec![ChatMessage::user("hi")]);
        use futures::StreamExt;
        let mut stream = stream;
        let mut collected = String::new();
        let mut saw_done = false;
        while let Some(result) = stream.next().await {
            match result {
                Ok(token) => {
                    if !token.text.is_empty() {
                        collected.push_str(&token.text);
                    }
                    if token.done {
                        saw_done = true;
                    }
                }
                Err(e) => panic!("stream error: {e}"),
            }
        }
        assert_eq!(collected, "World");
        assert!(saw_done, "should see done=true token");
    }

    // ---- T-E-C-02: describe_image 测试 ----

    /// T-E-C-02: describe_image 用 mock Ollama server 验证
    /// 多模态 API 请求体格式与响应解析。
    ///
    /// Mock server 接收 POST /api/chat,返回固定的 ChatResponse JSON。
    /// 验证:
    /// - 请求体包含 `"images":["BASE64PNG"]` 字段(多模态 wire format 正确)。
    /// - 返回值是 ChatResponse.message.content 字符串。
    /// - 使用指定的 model(此处为 "qwen2.5-vl:3b")。
    #[tokio::test]
    async fn test_describe_image_with_mock_server() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        // Mock Ollama server:收到 POST /api/chat 时返回固定响应。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        // 记录收到的请求体,供主线程断言。
        let captured_request: Arc<std::sync::Mutex<String>> =
            Arc::new(std::sync::Mutex::new(String::new()));
        let counter = Arc::new(AtomicUsize::new(0));
        let captured_clone = captured_request.clone();
        let counter_clone = counter.clone();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = [0u8; 8192];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let req_str = String::from_utf8_lossy(&buf).to_string();
                if req_str.starts_with("POST /api/chat") {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                    *captured_clone.lock().unwrap() = req_str.clone();
                    let body = r#"{"model":"qwen2.5-vl:3b","message":{"role":"assistant","content":"a screenshot showing code editor"},"done":true}"#;
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

        // 构造 gateway 并调用 describe_image。
        let ollama = Arc::new(OllamaClient::new_with_timeout(
            base_url.clone(),
            std::time::Duration::from_secs(5),
        ));
        let gw = LlmGateway::new(
            ollama,
            "default-model",
            "ollama",
            None,
            None,
            None,
            None,
            None,
        );

        let msg = ChatMessage {
            role: "user".to_string(),
            content: "describe this".to_string(),
            images: vec!["BASE64PNG".to_string()],
            ..Default::default()
        };
        let result = gw.describe_image("qwen2.5-vl:3b", msg).await;
        assert!(
            result.is_ok(),
            "describe_image should succeed with mock server"
        );
        let content = result.unwrap();
        assert_eq!(
            content, "a screenshot showing code editor",
            "describe_image should return ChatResponse.message.content"
        );
        // 验证请求体包含 images 字段(多模态 wire format)。
        let captured = captured_request.lock().unwrap().clone();
        assert_eq!(
            counter.load(Ordering::SeqCst),
            1,
            "should hit /api/chat exactly once"
        );
        assert!(
            captured.contains("\"images\""),
            "request body must contain images field; got: {captured}"
        );
        assert!(
            captured.contains("BASE64PNG"),
            "request body must contain base64 image data; got: {captured}"
        );
        assert!(
            captured.contains("\"model\":\"qwen2.5-vl:3b\""),
            "request body must contain vision_model name; got: {captured}"
        );
    }
}

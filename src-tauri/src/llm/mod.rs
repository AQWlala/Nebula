//! `nebula::llm` — model gateway and concrete clients.
//!
//! Two responsibilities live here:
//!
//! * [`ollama`] — a thin async HTTP wrapper around the local Ollama
//!   server (`/api/chat`, `/api/generate`, `/api/embeddings`).
//! * [`gateway`] — a higher-level [`LlmGateway`] that handles prompt
//!   caching, request rate limiting, and graceful degradation to a
//!   remote fallback endpoint when the local server is unavailable.
//! * [`anthropic`] — v1.1 P0-1: Anthropic Claude HTTP client.
//! * [`semantic_cache`] — T-E-A-01: L0.5 语义缓存层（cosine ≥ 0.92 短路）。
//! * [`cost_tracker`] — T-E-A-06: Token 费用追踪（按模型计费 + 按日/月聚合）。

pub mod gateway;
pub mod ollama;
// v1.1 P0-1: Anthropic Claude provider
pub mod anthropic;
// T-E-A-01: L0.5 SemanticCache
pub mod semantic_cache;
// T-E-A-06: Token 费用追踪
pub mod cost_tracker;
// M5 #71: CostPolicy 统一 — max_tokens_per_task + daily_task_limit
pub mod cost_policy;
// T-E-A-02: TokenJuice 三级压缩
pub mod token_juice;
// T-E-A-03: ModelRouter 智能路由
pub mod model_router;
// T-E-B-17: ReasoningChain
pub mod reasoning;
// T-E-S-40: OpenAI 兼容层(vLLM / LMStudio / OpenRouter / DeepSeek 等)
pub mod openai_compat;
// T-E-S-41: models.json 动态配置(provider/model 注册表)。
pub mod models_config;
// T-E-S-39: SOUL.md / AGENTS.md / TOOLS.md persona injection.
pub mod persona;
// T-E-A-11: Smart Prefetch — 打开文件时三路检索历史对话预热 SemanticCache。
pub mod prefetch;
// T-E-A-14: Arena A/B 测试 — 模型对战 + ELO 评分 + SQLite 持久化。
pub mod arena;
// ADR-003 v3.1: 统一模型调度层。
// P0-2: unified-dispatcher feature 已改为默认启用，不再需要 cfg gate。
// 运行时仍可通过 UNIFIED_DISPATCHER_ENABLED=0 禁用作为安全网（ADR-004）。
pub mod dispatcher;

pub use anthropic::{AnthropicClient, Role as AnthropicRole};
pub use arena::{ArenaLeaderboard, ArenaMatch};
pub use cost_tracker::{
    CostRecord, CostTracker, DailyAggregate, MonthlyAggregate, ProviderBucket, WeeklyAggregate,
};
// M5 #71: CostPolicy 统一预算门禁
pub use cost_policy::{CostDecision, CostPolicy};
// ADR-003: CircuitBreaker 重新导出，供 dispatcher.rs 使用。
pub use gateway::{CircuitBreaker, LlmGateway, StreamToken};
pub use model_router::{ModelRouter, Route};
pub use models_config::{
    ModelConfig, ModelsConfig, Pricing, ProviderConfig, ProviderKind, WorkTypeOverrideEntry,
};
pub use ollama::{
    ChatMessage, ChatResponse, FunctionCall, FunctionSpec, OllamaClient, Role, ToolCall, ToolSpec,
};
pub use openai_compat::OpenAICompatClient;
pub use persona::PersonaConfig;
pub use prefetch::{ChatTurnPair, PrefetchConfig, PrefetchEngine, PrefetchStats};
pub use reasoning::{ReasoningChain, ReasoningStep};
pub use semantic_cache::SemanticCache;
pub use token_juice::{TokenJuiceCompressor, TokenJuiceConfig};

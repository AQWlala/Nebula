//! ADR-003: 统一模型调度层（Unified Model Dispatcher）
//!
//! 所有 LLM 调用（聊天 / Swarm / Master / 进化 / Soul 编译 / 分类）的单一入口。
//! 参考 JiuwenSwarm（openJiuwen 社区）的成员级模型路由设计：不同工作类型
//! 用不同模型，"因材施教"。
//!
//! ## 核心设计
//!
//! - **WorkType 枚举**：按"工作类型"分类（Chat/SwarmWorker/Evolution/...），
//!   与现有 `CostSource`（按"触发场景"分类）正交，构成双维度成本统计。
//! - **ModelPolicy**：WorkType → (provider, model) 路由策略，支持用户在
//!   `models.json` 中通过 `work_type_overrides` 自定义覆盖。
//! - **is_local_only() 强制约束**：Evolution/SoulCompile/Classifier 强制
//!   走本地 Ollama，忽略非本地 override（P0-2 修复）。
//! - **独立断路器**：本地路径持有独立 `CircuitBreaker`，与远端 Gateway 的
//!   breaker 解耦（P0-4 修复）。
//! - **流式接口**：`dispatch_stream()` 返回 `BoxStream<Result<StreamToken>>`
//!   （P0-3 修复）。
//!
//! ## Feature Gate
//!
//! 整个模块由 `unified-dispatcher` feature 控制（默认 off）。
//! 运行时还需环境变量 `UNIFIED_DISPATCHER_ENABLED=1` 才会真正启用。
//! 参见 ADR-004 Feature Flag 策略。
//!
//! ## 迁移路径（参见 ADR-003 §7.1）
//!
//! - Phase 1（M0c，本文件）：创建 Dispatcher 骨架，新增功能直接使用。
//! - Phase 2（M3）：迁移 ModelRouter，不再直连 OllamaClient。
//! - Phase 3（M3）：迁移 SwarmWorker，注入 `Arc<UnifiedModelDispatcher>`。
//! - Phase 4（M7a）：迁移普通聊天，feature flag 双路径可回滚。

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use parking_lot::RwLock;
use tracing::instrument;

use super::cost_tracker::CostTracker;
use super::gateway::{CircuitBreaker, StreamToken};
use super::ollama::{ChatMessage, ChatResponse};
use super::semantic_cache::SemanticCache;
use super::LlmGateway;

// ---------------------------------------------------------------------------
// WorkType 枚举
// ---------------------------------------------------------------------------

/// 统一模型调度的工作类型。
/// 每种类型可配置不同的模型和 provider。
///
/// ## M3 #50 精简：从 11 个变体合并为 7 个
///
/// - `MasterDecompose` + `MasterValidate` → `MasterTask`
/// - `EvolutionExtract` + `EvolutionCompile` + `EvolutionReflect` + `EvolutionSoul` → `Evolution`
///
/// 合并理由：被合并的变体路由到相同的 provider+model（Master 都走远端默认，
/// Evolution 都走本地 evolution 模型），区分由调用方（EvolutionEngine 的 phase
/// 参数 / MasterOrchestrator 的 decompose vs validate 方法）处理，不影响路由。
///
/// 向后兼容：`parse_work_type()` 仍接受旧字符串键（`master_decompose` /
/// `evolution_extract` 等），映射到合并后的变体。
///
/// 注意：**Embedding 不纳入此枚举**。
/// Embedding 的返回类型是 `Vec<f32>`（不是 `ChatResponse`），
/// 缓存语义是精确匹配（不是语义近邻），走专用路径 `OllamaClient::embed()`。
///
/// P0-8 修复：已从枚举中移除 `Embedding` 变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkType {
    /// 普通聊天（用户对话）
    Chat,
    /// Swarm Worker 执行（fan-out 并行）
    SwarmWorker,
    /// Swarm 结果综合（Negotiator / Master synthesize）
    SwarmSynthesize,
    /// MasterAgent 任务（拆解 + 校验，M3 #50 合并）
    MasterTask,
    /// EvolutionEngine 全 Phase（M3 #50 合并 4 个 Phase）
    Evolution,
    /// SoulCompiler 编译（SOUL.md → CompiledSoul）
    SoulCompile,
    /// ModelRouter 分类器（复杂度判定）
    Classifier,
}

impl WorkType {
    /// WorkType 的字符串形式（存入 `CostRecord.task` 字段）。
    ///
    /// 双维度成本统计：`CostSource`（触发场景）保持不变，
    /// `WorkType`（工作类型）通过 `task` 字段记录，前端可按两个维度聚合。
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkType::Chat => "chat",
            WorkType::SwarmWorker => "swarm_worker",
            WorkType::SwarmSynthesize => "swarm_synthesize",
            WorkType::MasterTask => "master_task",
            WorkType::Evolution => "evolution",
            WorkType::SoulCompile => "soul_compile",
            WorkType::Classifier => "classifier",
        }
    }

    /// 是否强制本地路由（不可 override 到远端）。
    ///
    /// `is_local_only()` 的 WorkType：
    /// - `resolve()` 中强制忽略非本地 override（P0-2 修复）
    /// - `dispatch()` 中走 `dispatch_local()` 而非 `dispatch_remote()`
    /// - 不走 SemanticCache（is_local_only 的生成式调用不缓存）
    ///
    /// 包含：Evolution（M3 #50 合并后为单一变体） + SoulCompile + Classifier。
    /// 这些工作类型涉及用户隐私 / 零成本设计，必须本地执行。
    pub fn is_local_only(&self) -> bool {
        matches!(
            self,
            WorkType::Evolution | WorkType::SoulCompile | WorkType::Classifier
        )
    }
}

// ---------------------------------------------------------------------------
// ResolvedModel
// ---------------------------------------------------------------------------

/// `ModelPolicy::resolve()` 的输出：解析后的目标 provider + model。
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider: String,
    pub model: String,
    /// 温度覆盖（可选）
    pub temperature: Option<f32>,
    /// 最大 token 覆盖（可选）
    pub max_tokens: Option<u32>,
}

impl ResolvedModel {
    /// 构造一个本地路由结果。
    pub fn local(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            temperature: None,
            max_tokens: None,
        }
    }

    /// 构造一个远端路由结果。
    pub fn remote(provider: &str, model: &str) -> Self {
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            temperature: None,
            max_tokens: None,
        }
    }

    /// 是否路由到本地 provider（与 `ModelPolicy.local_provider_id` 比对）。
    pub fn is_local(&self, local_provider_id: &str) -> bool {
        self.provider == local_provider_id
    }
}

// ---------------------------------------------------------------------------
// ModelPolicy
// ---------------------------------------------------------------------------

/// 工作类型 → 模型策略映射。
///
/// 优先级：`is_local_only()` 强制约束 > 用户 override > 默认策略。
///
/// P0-2 修复：对 `is_local_only()` 的 WorkType，`resolve()` 强制忽略
/// 非本地 override 并 warn 日志。
///
/// Override 配置使用 `WorkTypeOverrideEntry`（定义在 `models_config.rs`，
/// 始终编译），从 `models.json` 的 `work_type_overrides` 字段解析。
/// JSON 中的键是 `WorkType::as_str()` 的字符串形式，加载时转换为
/// `WorkType` 枚举键（未知键 warn 跳过）。
#[derive(Clone)]
pub struct ModelPolicy {
    /// 默认 provider（来自 `models.json` 的 `default_provider`）。
    default_provider: String,
    /// 默认模型（来自 `models.json` 的 `default_model`）。
    default_model: String,
    /// 本地 provider ID（通常 "ollama"）。
    local_provider_id: String,
    /// 本地分类器模型（通常 "qwen2.5:3b"）。
    local_classifier_model: String,
    /// 本地进化模型（通常 "qwen2.5:7b" 或 "qwen2.5:14b"）。
    local_evolution_model: String,
    /// 本地 Soul 编译模型。
    local_soul_model: String,
    /// SwarmWorker 默认本地模型。
    worker_local_model: String,
    /// 每种 WorkType 的用户覆盖配置（可选）。
    overrides: HashMap<WorkType, super::models_config::WorkTypeOverrideEntry>,
}

impl ModelPolicy {
    /// 构造 ModelPolicy。
    pub fn new(
        default_provider: String,
        default_model: String,
        local_provider_id: String,
        local_classifier_model: String,
        local_evolution_model: String,
        local_soul_model: String,
        worker_local_model: String,
        overrides: HashMap<WorkType, super::models_config::WorkTypeOverrideEntry>,
    ) -> Self {
        Self {
            default_provider,
            default_model,
            local_provider_id,
            local_classifier_model,
            local_evolution_model,
            local_soul_model,
            worker_local_model,
            overrides,
        }
    }

    /// 从 `ModelsConfig` 构造 ModelPolicy。
    ///
    /// 自动解析 `work_type_overrides` 的字符串键为 `WorkType` 枚举，
    /// 未知键 warn 跳过（不阻断启动）。
    pub fn from_models_config(cfg: &super::models_config::ModelsConfig) -> Self {
        let local_provider_id = cfg.local_provider.clone();
        let local_classifier_model = cfg.local_classifier_model.clone();
        let local_evolution_model = cfg.local_evolution_model.clone();
        let local_soul_model = cfg.local_soul_model.clone();
        let worker_local_model = cfg.worker_local_model.clone();

        // 解析 work_type_overrides：字符串键 → WorkType 枚举
        let mut overrides = HashMap::new();
        for (key, entry) in &cfg.work_type_overrides {
            match parse_work_type(key) {
                Some(wt) => {
                    overrides.insert(wt, entry.clone());
                }
                None => {
                    tracing::warn!(
                        key = %key,
                        "unknown work_type key in work_type_overrides, skipping"
                    );
                }
            }
        }

        Self::new(
            cfg.default_provider.clone(),
            cfg.default_model.clone(),
            local_provider_id,
            local_classifier_model,
            local_evolution_model,
            local_soul_model,
            worker_local_model,
            overrides,
        )
    }

    /// 解析工作类型 → `ResolvedModel`。
    ///
    /// 优先级：`is_local_only()` 强制约束 > 用户 override > 默认策略。
    ///
    /// P0-2 修复：对 `is_local_only()` 的 WorkType，强制忽略非本地 override。
    pub fn resolve(&self, work_type: WorkType) -> ResolvedModel {
        // 1. 检查用户 override
        if let Some(cfg) = self.overrides.get(&work_type) {
            // P0-2: is_local_only WorkType 强制本地，忽略非本地 override
            if work_type.is_local_only() && cfg.provider != self.local_provider_id {
                tracing::warn!(
                    work_type = ?work_type,
                    override_provider = %cfg.provider,
                    "override rejected: is_local_only enforced, falling back to default"
                );
                return self.default_route(work_type);
            }
            return ResolvedModel {
                provider: cfg.provider.clone(),
                model: cfg.model.clone(),
                temperature: cfg.temperature,
                max_tokens: cfg.max_tokens,
            };
        }
        // 2. 默认策略
        self.default_route(work_type)
    }

    /// 默认路由策略（无用户 override 时）。
    ///
    /// 参见 ADR-003 §3.5 默认路由表。
    fn default_route(&self, work_type: WorkType) -> ResolvedModel {
        match work_type {
            // 远端：主模型
            WorkType::Chat => ResolvedModel::remote(&self.default_provider, &self.default_model),
            WorkType::SwarmSynthesize => {
                ResolvedModel::remote(&self.default_provider, &self.default_model)
            }
            WorkType::MasterTask => {
                ResolvedModel::remote(&self.default_provider, &self.default_model)
            }

            // 本地优先：简单 Worker 任务用本地模型
            // （可被 ModelRouter 动态升级为远端，见 §6.5）
            WorkType::SwarmWorker => {
                ResolvedModel::local(&self.local_provider_id, &self.worker_local_model)
            }

            // 强制本地：进化引擎全 Phase（M3 #50 合并为单一 Evolution 变体）
            WorkType::Evolution => {
                ResolvedModel::local(&self.local_provider_id, &self.local_evolution_model)
            }

            // 强制本地：Soul 编译
            WorkType::SoulCompile => {
                ResolvedModel::local(&self.local_provider_id, &self.local_soul_model)
            }

            // 强制本地：分类器
            WorkType::Classifier => {
                ResolvedModel::local(&self.local_provider_id, &self.local_classifier_model)
            }
        }
    }

    /// 更新用户 override 配置（热更新）。
    pub fn set_overrides(
        &mut self,
        overrides: HashMap<WorkType, super::models_config::WorkTypeOverrideEntry>,
    ) {
        self.overrides = overrides;
    }

    /// 获取本地 provider ID。
    pub fn local_provider_id(&self) -> &str {
        &self.local_provider_id
    }
}

/// 解析 WorkType 字符串键（来自 models.json 的 work_type_overrides）。
///
/// 未知字符串返回 None（调用方 warn 跳过）。
///
/// M3 #50 向后兼容：旧字符串键（`master_decompose` / `master_validate` /
/// `evolution_extract` / `evolution_compile` / `evolution_reflect` /
/// `evolution_soul`）仍被接受，映射到合并后的变体。
fn parse_work_type(s: &str) -> Option<WorkType> {
    match s {
        // 新键（M3 #50 精简后）
        "chat" => Some(WorkType::Chat),
        "swarm_worker" => Some(WorkType::SwarmWorker),
        "swarm_synthesize" => Some(WorkType::SwarmSynthesize),
        "master_task" => Some(WorkType::MasterTask),
        "evolution" => Some(WorkType::Evolution),
        "soul_compile" => Some(WorkType::SoulCompile),
        "classifier" => Some(WorkType::Classifier),

        // 旧键（M3 #50 向后兼容，映射到合并后的变体）
        "master_decompose" | "master_validate" => Some(WorkType::MasterTask),
        "evolution_extract" | "evolution_compile" | "evolution_reflect"
        | "evolution_soul" => Some(WorkType::Evolution),

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// UnifiedModelDispatcher
// ---------------------------------------------------------------------------

/// 统一模型调度器 — 所有 LLM 调用的单一入口。
///
/// 参考 JiuwenSwarm 的成员级模型路由设计：不同工作类型用不同模型。
/// 所有调用自动享受：断路器、语义缓存、重试机制、TokenJuice 压缩、
/// 成本统计。
///
/// ## 双断路器设计（P0-4 修复）
///
/// - `local_breaker`：观测本地 Ollama 服务的健康状态（独立实例）
/// - Gateway 内部的 `breaker`：观测远端 provider 的健康状态
///
/// 两者完全解耦，本地 Ollama 宕机不会影响远端调用的断路器状态。
///
/// ## 本地并发限流（P1-3 EA-3 修复）
///
/// `local_semaphore` 限制同时发往 Ollama 的请求数，防止打爆本地推理服务。
/// 默认并发数 = 2（可通过构造器配置）。
pub struct UnifiedModelDispatcher {
    /// 底层 Gateway（远端路径复用现有基础设施）。
    gateway: Arc<LlmGateway>,
    /// 本地断路器（P0-4 修复：本地路径独立断路器，与远端解耦）。
    local_breaker: Arc<CircuitBreaker>,
    /// 本地语义缓存（P0-4 修复：本地路径独立缓存）。
    /// 复用 `SemanticCache` 逻辑但独立实例，避免本地/远端缓存污染。
    local_cache: Option<Arc<SemanticCache>>,
    /// 路由策略。
    policy: Arc<RwLock<ModelPolicy>>,
    /// 成本追踪器。
    cost_tracker: Option<Arc<CostTracker>>,
    /// 本地并发限流信号量（P1-3 EA-3 修复）。
    local_semaphore: Arc<tokio::sync::Semaphore>,
}

impl UnifiedModelDispatcher {
    /// 构造 Dispatcher。
    ///
    /// # 参数
    /// - `gateway`: 底层 LlmGateway（远端路径复用）
    /// - `policy`: 模型路由策略
    /// - `cost_tracker`: 成本追踪器（可选）
    /// - `local_cache`: 本地语义缓存（可选，None 时跳过缓存）
    /// - `max_local_concurrency`: 本地 Ollama 最大并发数（默认 2）
    pub fn new(
        gateway: Arc<LlmGateway>,
        policy: ModelPolicy,
        cost_tracker: Option<Arc<CostTracker>>,
        local_cache: Option<Arc<SemanticCache>>,
        max_local_concurrency: usize,
    ) -> Self {
        Self {
            gateway,
            local_breaker: Arc::new(CircuitBreaker::default()),
            local_cache,
            policy: Arc::new(RwLock::new(policy)),
            cost_tracker,
            local_semaphore: Arc::new(tokio::sync::Semaphore::new(
                max_local_concurrency.max(1),
            )),
        }
    }

    /// 主入口：按工作类型调度 LLM 调用（非流式）。
    ///
    /// 流程：
    /// 1. 解析 `WorkType` → `(provider, model)` via `ModelPolicy::resolve()`
    /// 2. 判断是否本地路由（`is_local()` 或 `is_local_only()` 强制）
    ///    - 本地：走 `dispatch_local()`，独立断路器 + 独立缓存 + Semaphore 限流
    ///    - 远端：走 `dispatch_remote()`，复用 Gateway 的断路器/缓存/重试/压缩
    /// 3. 自动记录 `CostSource`（触发场景）+ `WorkType`（工作类型）
    /// 4. 返回 `ChatResponse`
    ///
    /// #18: tracing span — `otel.kind = "llm_dispatch"`, `work_type` 字段。
    #[instrument(
        target = "nebula.llm.dispatcher",
        skip(self, messages),
        fields(otel.kind = "llm_dispatch", work_type = ?work_type)
    )]
    pub async fn dispatch(
        &self,
        work_type: WorkType,
        messages: Vec<ChatMessage>,
    ) -> Result<ChatResponse> {
        let resolved = self.policy.read().resolve(work_type);

        if resolved.is_local(self.policy.read().local_provider_id()) {
            self.dispatch_local(&resolved, &messages, work_type)
                .await
        } else {
            self.dispatch_remote(&resolved, messages, work_type)
                .await
        }
    }

    /// 流式入口（P0-3 修复）：按工作类型调度流式 LLM 调用。
    ///
    /// 仅在 `Chat` / `SwarmSynthesize` / `MasterTask` 上启用流式。
    /// `Evolution` / `SoulCompile` / `Classifier` 仍走非流式
    /// （本地推理快，无需流式开销）。
    ///
    /// 远端走 `gateway.chat_stream()`（已支持 DeepSeek SSE / Ollama NDJSON
    /// / Anthropic event-stream）。
    /// 本地走 `OllamaClient::chat_stream()`（M5 #74 实现）。
    #[instrument(
        target = "nebula.llm.dispatcher",
        skip(self, messages),
        fields(otel.kind = "llm_dispatch_stream", work_type = ?work_type)
    )]
    pub fn dispatch_stream(
        &self,
        work_type: WorkType,
        messages: Vec<ChatMessage>,
    ) -> futures::stream::BoxStream<'static, Result<StreamToken>> {
        let resolved = self.policy.read().resolve(work_type);
        let is_local = resolved.is_local(self.policy.read().local_provider_id());

        if is_local {
            // M5 #74: 本地走 OllamaClient::chat_stream()（NDJSON 流式）。
            // 复用 gateway.ollama_client()（共享 OllamaClient 实例），以共享
            // M3 #49 的 Semaphore 限流（max_concurrency 默认 2）。
            // resolved.model 是 ModelPolicy 解析后的本地模型名。
            tracing::debug!(
                work_type = ?work_type,
                model = %resolved.model,
                "dispatch_stream: local ollama streaming"
            );
            return self.gateway.ollama_client()
                .chat_stream(&resolved.model, messages);
        }

        // 远端流式：复用 Gateway 的 chat_stream()。
        // TODO(#13 P0-5): 迁移到 chat_with_task_context_stream() 以记录 work_type。
        // 当前 chat_stream() 不接受 task 参数，work_type 不会被记录到 CostRecord。
        self.gateway.chat_stream(messages)
    }

    /// 本地调度：直连 OllamaClient，通过独立断路器保护。
    ///
    /// P0-4 修复：不再绕过断路器/缓存。
    ///
    /// 流程：
    /// 1. 断路器检查（独立于 Gateway 的 breaker）
    /// 2. 语义缓存检查（本地独立缓存，仅适用于生成式 WorkType）
    /// 3. Semaphore 限流（防止打爆 Ollama）
    /// 4. 调用 Ollama（通过 Gateway 的 OllamaClient，复用 chat_with_retry）
    /// 5. 断路器记录（成功/失败）
    /// 6. 缓存存储（仅生成式调用）
    /// 7. 成本记录（work_type 存入 CostRecord.task 字段）
    async fn dispatch_local(
        &self,
        resolved: &ResolvedModel,
        messages: &[ChatMessage],
        work_type: WorkType,
    ) -> Result<ChatResponse> {
        // 1. 断路器检查（独立于 Gateway 的 breaker）
        self.local_breaker.check()?;

        // 2. 语义缓存检查（本地独立缓存，仅适用于非 is_local_only 的生成式调用）
        //    is_local_only() 的调用（Evolution/Soul/Classifier）不缓存，
        //    因为它们的输出是结构化的（不是对话），缓存命中率低且可能污染。
        if let Some(cache) = &self.local_cache {
            if !work_type.is_local_only() || work_type == WorkType::SwarmWorker {
                if let Some(query) = last_user_message(messages) {
                    if let Some(cached) = cache.check(query).await {
                        tracing::debug!(
                            work_type = ?work_type,
                            "local semantic cache hit"
                        );
                        return Ok(cached_response(&resolved.model, &cached));
                    }
                }
            }
        }

        // 3. Semaphore 限流（防止打爆 Ollama）
        let _permit = self
            .local_semaphore
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("local semaphore closed"))?;

        // 4. 调用 Ollama（通过 Gateway 的 OllamaClient，复用 chat_with_retry）
        let result = self
            .gateway
            .ollama_client()
            .chat_with_retry(&resolved.model, messages)
            .await;

        // 5. 断路器记录
        match &result {
            Ok(_) => self.local_breaker.record_success(),
            Err(_) => self.local_breaker.record_failure(),
        }

        let resp = result?;

        // 6. 缓存存储（仅生成式调用）
        //    SemanticCache::store 返回 ()（best-effort，内部已处理错误）
        if let Some(cache) = &self.local_cache {
            if !work_type.is_local_only() || work_type == WorkType::SwarmWorker {
                if let Some(query) = last_user_message(messages) {
                    cache.store(query, &resp.message.content).await;
                }
            }
        }

        // 7. 成本记录（复用 task 字段记录 work_type）
        //    Ollama API 不返回 prompt_tokens，仅 eval_count（生成 token 数）。
        //    prompt_tokens 记为 0，completion 取 eval_count（与 gateway.rs 一致）。
        if let Some(tracker) = &self.cost_tracker {
            let completion = resp.eval_count.unwrap_or(0);
            tracker.record_with_context(
                &resp.model,
                0,
                completion,
                Some("ollama".to_string()),
                Some(work_type.as_str().to_string()),
                None,
            );
        }

        Ok(resp)
    }

    /// 远端调度：走 LlmGateway，自动享受所有基础设施。
    ///
    /// P0-5 修复：使用新增的 `chat_with_task_context()` 方法
    /// （绕过 ModelRouter，由 Dispatcher 已决策 provider）。
    ///
    /// TODO(#13 P0-5): `LlmGateway::chat_with_task_context()` 尚未实现。
    /// 当前回退到 `gateway.chat()`，work_type 不会被记录到 CostRecord。
    ///
    /// M3 #47 修复：当 ModelRouter 注入了 Dispatcher 后,
    /// `gateway.chat()` → `router.classify()` → `dispatcher.dispatch(Classifier)`
    /// → `dispatch_remote()` 形成 async fn 递归。Classifier 是 local-only
    /// (运行时实际走 `dispatch_local()`),编译器无法静态证明,
    /// 因此用 `Box::pin` 引入 indirection 破除无限大 future。
    ///
    /// M3 #51: 远端 provider fallback 到本地 — `gateway.chat()` 内置
    /// 完整 fallback 链(DeepSeek → Ollama → Anthropic → Remote):
    /// 1. 主 provider(如 DeepSeek)调用失败 → 自动尝试 Ollama(本地)
    /// 2. Ollama 失败 → 尝试 Anthropic(若配置)
    /// 3. Anthropic 失败 → 尝试 Remote(OpenAI 兼容,若配置)
    /// 4. 日预算超限 → 强制走 Ollama(本地,零成本)
    /// 因此 `dispatch_remote` 本身**无需**额外 fallback 逻辑,
    /// 复用 Gateway 既有 fallback 链即可。
    fn dispatch_remote(
        &self,
        _resolved: &ResolvedModel,
        messages: Vec<ChatMessage>,
        work_type: WorkType,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ChatResponse>> + Send + '_>> {
        // TODO(#13 P0-5): 替换为 chat_with_task_context(messages, work_type.as_str())
        // 当前回退到普通 chat()，work_type 不被记录（功能可用但成本统计不完整）。
        tracing::debug!(
            work_type = ?work_type,
            "remote dispatch via gateway.chat() (chat_with_task_context not yet implemented)"
        );
        Box::pin(self.gateway.chat(messages))
    }

    /// 获取底层 Gateway 引用（供需要直接访问 Gateway 的场景使用）。
    pub fn gateway(&self) -> &LlmGateway {
        &self.gateway
    }

    /// 获取本地断路器引用（供测试 / 健康检查使用）。
    pub fn local_breaker(&self) -> &CircuitBreaker {
        &self.local_breaker
    }

    /// 更新路由策略（热更新）。
    pub fn update_policy(&self, policy: ModelPolicy) {
        *self.policy.write() = policy;
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 提取消息列表中最后一条 user 消息的 content 作为缓存 query。
///
/// 与 `gateway.rs` 中 semantic_cache 查询逻辑一致。
fn last_user_message(messages: &[ChatMessage]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.as_str())
}

/// 从缓存命中的 response 文本构造 `ChatResponse`。
///
/// 与 `gateway.rs` 中 semantic cache hit 路径一致。
fn cached_response(model: &str, content: &str) -> ChatResponse {
    ChatResponse {
        model: model.to_string(),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_type_as_str_roundtrip() {
        // M3 #50: 精简后的 7 个变体。
        let cases = [
            (WorkType::Chat, "chat"),
            (WorkType::SwarmWorker, "swarm_worker"),
            (WorkType::SwarmSynthesize, "swarm_synthesize"),
            (WorkType::MasterTask, "master_task"),
            (WorkType::Evolution, "evolution"),
            (WorkType::SoulCompile, "soul_compile"),
            (WorkType::Classifier, "classifier"),
        ];
        for (wt, expected) in &cases {
            assert_eq!(wt.as_str(), *expected);
        }
    }

    #[test]
    fn is_local_only_enforced_for_evolution_soul_classifier() {
        // M3 #50: Evolution 合并 4 个旧 phase，仍是 local-only。
        assert!(WorkType::Evolution.is_local_only());
        // Soul 编译
        assert!(WorkType::SoulCompile.is_local_only());
        // 分类器
        assert!(WorkType::Classifier.is_local_only());
    }

    #[test]
    fn is_local_only_false_for_chat_swarm_master() {
        assert!(!WorkType::Chat.is_local_only());
        assert!(!WorkType::SwarmWorker.is_local_only());
        assert!(!WorkType::SwarmSynthesize.is_local_only());
        assert!(!WorkType::MasterTask.is_local_only());
    }

    #[test]
    fn resolve_default_routes_chat_to_remote() {
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            HashMap::new(),
        );
        let r = policy.resolve(WorkType::Chat);
        assert_eq!(r.provider, "deepseek");
        assert_eq!(r.model, "deepseek-chat");
        assert!(!r.is_local("ollama"));
    }

    #[test]
    fn resolve_default_routes_evolution_to_local() {
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            HashMap::new(),
        );
        let r = policy.resolve(WorkType::Evolution);
        assert_eq!(r.provider, "ollama");
        assert_eq!(r.model, "qwen2.5:7b");
        assert!(r.is_local("ollama"));
    }

    #[test]
    fn resolve_default_routes_classifier_to_local() {
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            HashMap::new(),
        );
        let r = policy.resolve(WorkType::Classifier);
        assert_eq!(r.provider, "ollama");
        assert_eq!(r.model, "qwen2.5:3b");
    }

    #[test]
    fn resolve_default_routes_swarm_worker_to_local() {
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            HashMap::new(),
        );
        let r = policy.resolve(WorkType::SwarmWorker);
        assert_eq!(r.provider, "ollama");
        assert_eq!(r.model, "qwen2.5:7b");
    }

    // P0-2: is_local_only WorkType 忽略非本地 override
    #[test]
    fn resolve_rejects_remote_override_for_local_only_worktype() {
        let mut overrides = HashMap::new();
        // 用户尝试把 Evolution override 到 deepseek（远端）
        overrides.insert(
            WorkType::Evolution,
            super::super::models_config::WorkTypeOverrideEntry {
                provider: "deepseek".to_string(),
                model: "deepseek-chat".to_string(),
                temperature: None,
                max_tokens: None,
            },
        );
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            overrides,
        );
        // resolve 应忽略非本地 override，回退到默认本地路由
        let r = policy.resolve(WorkType::Evolution);
        assert_eq!(r.provider, "ollama", "is_local_only override should be rejected");
        assert_eq!(r.model, "qwen2.5:7b");
    }

    // P0-2: is_local_only WorkType 接受本地 override
    #[test]
    fn resolve_accepts_local_override_for_local_only_worktype() {
        let mut overrides = HashMap::new();
        // 用户把 Evolution override 到 ollama 的更大模型
        overrides.insert(
            WorkType::Evolution,
            super::super::models_config::WorkTypeOverrideEntry {
                provider: "ollama".to_string(),
                model: "qwen2.5:14b".to_string(),
                temperature: Some(0.5),
                max_tokens: None,
            },
        );
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            overrides,
        );
        let r = policy.resolve(WorkType::Evolution);
        assert_eq!(r.provider, "ollama");
        assert_eq!(r.model, "qwen2.5:14b");
        assert_eq!(r.temperature, Some(0.5));
    }

    // 非 is_local_only 的 WorkType 接受任意 override
    #[test]
    fn resolve_accepts_any_override_for_non_local_only_worktype() {
        let mut overrides = HashMap::new();
        overrides.insert(
            WorkType::Chat,
            super::super::models_config::WorkTypeOverrideEntry {
                provider: "anthropic".to_string(),
                model: "claude-sonnet-4".to_string(),
                temperature: Some(0.7),
                max_tokens: Some(4096),
            },
        );
        let policy = ModelPolicy::new(
            "deepseek".to_string(),
            "deepseek-chat".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            overrides,
        );
        let r = policy.resolve(WorkType::Chat);
        assert_eq!(r.provider, "anthropic");
        assert_eq!(r.model, "claude-sonnet-4");
        assert_eq!(r.temperature, Some(0.7));
        assert_eq!(r.max_tokens, Some(4096));
    }

    #[test]
    fn parse_work_type_roundtrip() {
        // M3 #50: 精简后的 7 个变体。
        let cases = [
            WorkType::Chat,
            WorkType::SwarmWorker,
            WorkType::SwarmSynthesize,
            WorkType::MasterTask,
            WorkType::Evolution,
            WorkType::SoulCompile,
            WorkType::Classifier,
        ];
        for wt in &cases {
            let s = wt.as_str();
            let parsed = parse_work_type(s);
            assert_eq!(parsed, Some(*wt), "parse_work_type({s}) should roundtrip");
        }
    }

    #[test]
    fn parse_work_type_returns_none_for_unknown() {
        assert_eq!(parse_work_type("unknown"), None);
        assert_eq!(parse_work_type(""), None);
        assert_eq!(parse_work_type("embedding"), None);
    }

    /// M3 #50: 旧 11 变体字符串向后兼容映射。
    /// - `master_decompose` / `master_validate` → `MasterTask`
    /// - `evolution_extract` / `evolution_compile` / `evolution_reflect` / `evolution_soul` → `Evolution`
    #[test]
    fn parse_work_type_backward_compat_with_legacy_keys() {
        assert_eq!(parse_work_type("master_decompose"), Some(WorkType::MasterTask));
        assert_eq!(parse_work_type("master_validate"), Some(WorkType::MasterTask));
        assert_eq!(parse_work_type("evolution_extract"), Some(WorkType::Evolution));
        assert_eq!(parse_work_type("evolution_compile"), Some(WorkType::Evolution));
        assert_eq!(parse_work_type("evolution_reflect"), Some(WorkType::Evolution));
        assert_eq!(parse_work_type("evolution_soul"), Some(WorkType::Evolution));
    }

    #[test]
    fn last_user_message_extracts_correctly() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "you are helpful".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: "first question".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: "answer".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: "second question".to_string(),
                ..Default::default()
            },
        ];
        assert_eq!(last_user_message(&messages), Some("second question"));
    }

    #[test]
    fn last_user_message_returns_none_when_no_user() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: "hi".to_string(),
            ..Default::default()
        }];
        assert_eq!(last_user_message(&messages), None);
    }
}

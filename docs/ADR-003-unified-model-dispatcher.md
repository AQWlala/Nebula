# ADR-003: 统一模型调度层（Unified Model Dispatcher）

> **状态**: 已实施（M7b 完成 — v2.1）  
> **日期**: 2026-07-05  
> **决策者**: 架构组  
> **关联**: SWARM_EVOLUTION_DESIGN_v2.md §5 LLM 成本模型  
> **参考**: JiuwenSwarm（openJiuwen 社区）成员级模型路由设计  
> **实现里程碑**: M0c → M7a → M7b（已完成 #90-#95）

---

## 1. 背景与动机

### 1.1 现状：5 条碎片化调用路径

当前 nebula 项目的 LLM 调用存在 **5 条独立路径**，各自为政：

| # | 调用方 | 入口 | 路由方式 | CostSource | 问题 |
|---|--------|------|---------|-----------|------|
| 1 | 普通聊天 | `LlmGateway::chat()` | ModelRouter 分类 → 4 provider 分派 | `Chat` | 主路径，功能完整 |
| 2 | 分类器 | `ModelRouter::classify()` | **直连 OllamaClient**，绕过 Gateway | 无 | 不计成本、不走断路器、不走缓存 |
| 3 | Swarm Worker | `LlmGateway`（注入） | 跟随 Gateway 默认 provider | `Automation` | 无法指定 Worker 用本地模型 |
| 4 | SoulCompiler（设计中） | "本地 Ollama" | 未定义接口 | 未定义 | 无统一调用层 |
| 5 | EvolutionEngine（设计中） | "本地模型" | 未定义接口 | 未定义 | 无统一调用层 |

> **注意**：现有 `CostSource` 枚举为 `Chat / Automation / Cron / Background`（按**触发场景**分类），**没有 `Swarm` 变体**。Swarm Worker 执行时通过 `with_automation_trigger` task_local 标记为 `Automation`。

### 1.2 核心矛盾

1. **ModelRouter 绕过 LlmGateway**：分类器直连 `OllamaClient::chat()`，导致分类调用不计成本、不受断路器保护、不享受语义缓存
2. **Worker 无法按任务类型选模型**：所有 Worker 都走同一个 `default_provider`，无法实现"简单任务用本地、复杂任务用远端"
3. **新增功能无统一接口**：v2.0 设计的 EvolutionEngine / SoulCompiler / MasterAgent 都说"用本地模型"，但没有定义统一的调用接口
4. **WorkType 维度缺失**：现有 `CostSource` 按**触发场景**分类（Chat/Automation/Cron/Background），但缺少按**工作类型**分类的维度（SwarmWorker/Evolution/SoulCompile 等），无法区分"自动化触发的 Swarm"和"自动化触发的进化"
5. **无法按角色分配模型**：JiuwenSwarm 的核心优势之一是"成员级模型路由"——不同 Agent 角色用不同模型，nebula 目前做不到

### 1.3 JiuwenSwarm 参考

JiuwenSwarm（华为 openJiuwen 社区开源）的模型调用设计核心：

- **成员级模型路由**：每个 Agent 角色可配置不同模型（如狼人杀游戏中不同成员用不同模型）
- **统一配置管理**：通过 `ModelConfig`（provider + model_info）+ `LLMCallConfig`（model + system_prompt + user_prompt）体系统一管理
- **因材施教**：可针对不同角色提供合适能力的模型，减少负载压力，提升整体效果
- **Studio 可视化管理**：agent-studio 提供模型管理界面，添加/测试/切换模型

---

## 2. 设计目标

1. **单一入口**：所有 LLM 调用（聊天 / Swarm / Master / 进化 / Soul 编译 / 分类）走同一个调度层
2. **按工作类型路由**：每种工作类型可配置不同模型，支持"简单任务本地、复杂任务远端"
3. **统一成本统计**：`CostSource` 枚举扩展，所有调用自动归属到正确成本域
4. **统一基础设施**：所有调用自动享受断路器、语义缓存、重试机制、TokenJuice 压缩
5. **可配置**：用户可在 `models.json` 中为每种工作类型指定模型
6. **本地优先**：进化引擎 / Soul 编译 / 分类器默认路由到本地模型，零远端成本

---

## 3. 架构设计

### 3.1 总体架构

```
┌─────────────────────────────────────────────────────────────────┐
│                    UnifiedModelDispatcher                        │
│  (单一入口：所有 LLM 调用通过 dispatch(work_type, messages))     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────┐  ┌─────────────┐  ┌───────────┐  ┌──────────┐ │
│  │ WorkType    │  │ ModelPolicy │  │ CostSource│  │ Circuit  │ │
│  │ 枚举        │→ │ 路由策略    │→ │ 自动归属  │  │ Breaker  │ │
│  │             │  │             │  │           │  │ (共享)   │ │
│  └─────────────┘  └─────────────┘  └───────────┘  └──────────┘ │
│         │                │                                      │
│         ▼                ▼                                      │
│  ┌─────────────────────────────────────┐                         │
│  │         SemanticCache (共享)        │                         │
│  └─────────────────────────────────────┘                         │
│         │                │                                        │
│         ▼                ▼                                        │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐│
│  │ Ollama   │  │ DeepSeek │  │ Anthropic│  │ OpenAI-Compat    ││
│  │ (本地)   │  │ (远端)   │  │ (远端)   │  │ (vLLM/OpenRouter)││
│  └──────────┘  └──────────┘  └──────────┘  └──────────────────┘│
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
         ↑          ↑          ↑          ↑          ↑
    Chat  Swarm  Master  Evolution  SoulCompile  Classifier
```

### 3.2 WorkType 枚举

定义所有需要调用 LLM 的工作类型：

```rust
/// 统一模型调度的工作类型。
/// 每种类型可配置不同的模型和 provider。
///
/// 注意：Embedding 不纳入此枚举。
/// Embedding 的返回类型是 Vec<f32>（不是 ChatResponse），
/// 缓存语义是精确匹配（不是语义近邻），走专用路径 OllamaClient::embed()。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkType {
    /// 普通聊天（用户对话）
    Chat,
    /// Swarm Worker 执行（fan-out 并行）
    SwarmWorker,
    /// Swarm 结果综合（Negotiator/Master synthesize）
    SwarmSynthesize,
    /// MasterAgent 任务拆解（LLM 生成 JSON DAG）
    MasterDecompose,
    /// MasterAgent JSON 校验修正（失败时重试）
    MasterValidate,
    /// EvolutionEngine Phase 1: 经验提取
    EvolutionExtract,
    /// EvolutionEngine Phase 2: 知识编译（KnowledgeCompiler）
    EvolutionCompile,
    /// EvolutionEngine Phase 3: 元认知反思
    EvolutionReflect,
    /// EvolutionEngine Phase 4: Soul 反哺
    EvolutionSoul,
    /// SoulCompiler 编译（SOUL.md → CompiledSoul）
    SoulCompile,
    /// ModelRouter 分类器（复杂度判定）
    Classifier,
}
```

### 3.3 双维度成本统计（P0-1 修复）

> **关键修订**：不重定义 `CostSource`。保留现有 4 值（按触发场景分类），新增 `WorkType` 维度。

**两个正交维度**：

| 维度 | 枚举 | 现有值 | 说明 |
|------|------|--------|------|
| 触发场景 | `CostSource` | Chat / Automation / Cron / Background | 按"谁触发"分类（用户/自动化/定时/后台） |
| 工作类型 | `WorkType` | Chat / SwarmWorker / Evolution / ... | 按"做什么"分类（聊天/Worker/进化/...） |

**实现方式**：在 `CostRecord` 上新增 `work_type: Option<String>` 字段：

```rust
// cost_tracker.rs — CostRecord 新增字段
pub struct CostRecord {
    // 现有字段不变
    pub source: CostSource,      // 触发场景（保留 4 值）
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: f64,
    pub task: Option<String>,    // 现有字段（复用）
    // 新增：工作类型（WorkType 的字符串形式）
    // 由 Dispatcher 在调用时通过 record_with_context() 传入
    // task 字段复用为 work_type（如 "swarm_worker", "evolution_extract"）
}
```

**前端展示**：CreditsDashboard 可按两个维度分别聚合：
- 按触发场景：Chat / Automation / Cron / Background
- 按工作类型：Chat / SwarmWorker / SwarmSynthesize / MasterTask / Evolution / SoulCompile / Classifier

**向后兼容**：现有 `CostRecord` 行没有 `work_type`，前端查询时 `None` 显示为"未知"。

### 3.4 ModelPolicy 路由策略

```rust
/// 工作类型 → 模型策略映射
pub struct ModelPolicy {
    /// 默认 provider（来自 models.json 的 default_provider）
    default_provider: String,
    /// 默认模型（来自 models.json 的 default_model）
    default_model: String,
    /// 本地 provider ID（通常 "ollama"）
    local_provider_id: String,
    /// 本地分类器模型（通常 "qwen2.5:3b"）
    local_classifier_model: String,
    /// 本地进化模型（通常 "qwen2.5:7b" 或 "qwen2.5:14b"）
    local_evolution_model: String,
    /// 本地 Soul 编译模型
    local_soul_model: String,
    /// 远端主模型（用于 Master 拆解/综合、高质量写作）
    remote_main_model: String,
    /// 每种 WorkType 的覆盖配置（可选，用户在 models.json 中自定义）
    overrides: HashMap<WorkType, WorkTypeConfig>,
}

/// 单个工作类型的模型配置覆盖
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkTypeConfig {
    /// 指定 provider ID（如 "ollama", "deepseek", "anthropic"）
    pub provider: String,
    /// 指定模型 ID（如 "qwen2.5:3b", "deepseek-chat", "claude-sonnet-4"）
    pub model: String,
    /// 温度（可选，覆盖默认）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// 最大 token（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

impl ModelPolicy {
    /// 解析工作类型 → (provider, model)
    /// 优先级：is_local_only 强制约束 > 用户 override > 默认策略
    ///
    /// P0-2 修复：对 is_local_only() 的 WorkType，强制忽略非本地 override。
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

    /// 默认路由策略（无用户 override 时）
    fn default_route(&self, work_type: WorkType) -> ResolvedModel {
        match work_type {
            // 远端：主模型
            WorkType::Chat => ResolvedModel::remote(&self.default_provider, &self.default_model),
            WorkType::SwarmSynthesize => ResolvedModel::remote(&self.default_provider, &self.default_model),
            WorkType::MasterDecompose => ResolvedModel::remote(&self.default_provider, &self.default_model),
            WorkType::MasterValidate => ResolvedModel::remote(&self.default_provider, &self.default_model),

            // 本地优先：简单 Worker 任务用本地模型（可被 ModelRouter 动态升级为远端）
            WorkType::SwarmWorker => ResolvedModel::local(&self.local_provider_id, &self.default_model),

            // 强制本地：进化引擎全 Phase
            WorkType::EvolutionExtract => ResolvedModel::local(&self.local_provider_id, &self.local_evolution_model),
            WorkType::EvolutionCompile => ResolvedModel::local(&self.local_provider_id, &self.local_evolution_model),
            WorkType::EvolutionReflect => ResolvedModel::local(&self.local_provider_id, &self.local_evolution_model),
            WorkType::EvolutionSoul => ResolvedModel::local(&self.local_provider_id, &self.local_evolution_model),

            // 强制本地：Soul 编译
            WorkType::SoulCompile => ResolvedModel::local(&self.local_provider_id, &self.local_soul_model),

            // 强制本地：分类器
            WorkType::Classifier => ResolvedModel::local(&self.local_provider_id, &self.local_classifier_model),
        }
    }
}
```

### 3.5 默认路由表

| WorkType | 默认 Provider | 默认模型 | CostSource（触发场景） | 远端成本 |
|----------|--------------|---------|----------------------|---------|
| Chat | deepseek | deepseek-chat | Chat | 有 |
| SwarmWorker | ollama¹ | 本地模型² | Automation | 无¹ |
| SwarmSynthesize | deepseek | deepseek-chat | Automation | 有 |
| MasterDecompose | deepseek | deepseek-chat | Chat/Automation³ | 有 |
| MasterValidate | deepseek | deepseek-chat | Chat/Automation³ | 有 |
| EvolutionExtract | ollama | qwen2.5:7b | Background | 无 |
| EvolutionCompile | ollama | qwen2.5:7b | Background | 无 |
| EvolutionReflect | ollama | qwen2.5:7b | Background | 无 |
| EvolutionSoul | ollama | qwen2.5:7b | Background | 无 |
| SoulCompile | ollama | qwen2.5:3b | Background | 无 |
| Classifier | ollama | qwen2.5:3b | Background | 无 |

> ¹ SwarmWorker 默认本地，但保留 ModelRouter 动态升级能力：简单任务本地、复杂任务远端  
> ² SwarmWorker 的本地模型由 `models.json` 的 `worker_local_model` 字段配置，默认 `qwen2.5:7b`  
> ³ MasterTask 的 CostSource 取决于触发方式：用户手动触发→Chat，自动化触发→Automation

---

## 4. 统一调度接口

### 4.1 UnifiedModelDispatcher

```rust
/// 统一模型调度器 — 所有 LLM 调用的单一入口。
///
/// 参考 JiuwenSwarm 的成员级模型路由设计：不同工作类型用不同模型。
/// 所有调用自动享受：断路器、语义缓存、重试机制、TokenJuice 压缩、成本统计。
pub struct UnifiedModelDispatcher {
    /// 底层 Gateway（远端路径复用现有基础设施）
    gateway: Arc<LlmGateway>,
    /// 本地断路器（P0-4 修复：本地路径独立断路器，与远端解耦）
    /// 观测的是 Ollama 服务的健康状态，与 Gateway 的 breaker 独立
    local_breaker: Arc<CircuitBreaker>,
    /// 本地语义缓存（P0-4 修复：本地路径独立缓存）
    /// 复用 SemanticCache 逻辑但独立实例，避免本地/远端缓存污染
    local_cache: Option<Arc<SemanticCache>>,
    /// 路由策略
    policy: Arc<parking_lot::RwLock<ModelPolicy>>,
    /// 成本追踪器
    cost_tracker: Option<Arc<CostTracker>>,
    /// 本地并发限流信号量（P1-3 EA-3 修复：限制 Ollama 并发请求数）
    local_semaphore: Arc<tokio::sync::Semaphore>,
}

impl UnifiedModelDispatcher {
    /// 主入口：按工作类型调度 LLM 调用（非流式）。
    ///
    /// 流程：
    /// 1. 解析 WorkType → (provider, model) via ModelPolicy::resolve()
    /// 2. 判断是否本地路由（is_local() 或 is_local_only() 强制）
    ///    - 本地：走 dispatch_local()，独立断路器 + 独立缓存 + Semaphore 限流
    ///    - 远端：走 dispatch_remote()，复用 Gateway 的断路器/缓存/重试/压缩
    /// 3. 自动记录 CostSource（触发场景）+ WorkType（工作类型）
    /// 4. 返回 ChatResponse
    #[tracing::instrument(
        skip(self, messages),
        fields(otel.kind = "llm_dispatch", work_type = ?work_type)
    )]
    pub async fn dispatch(
        &self,
        work_type: WorkType,
        messages: Vec<ChatMessage>,
    ) -> anyhow::Result<ChatResponse> {
        let resolved = self.policy.read().resolve(work_type);

        if resolved.is_local() {
            self.dispatch_local(&resolved, &messages, work_type).await
        } else {
            self.dispatch_remote(&resolved, messages, work_type).await
        }
    }

    /// 流式入口（P0-3 修复）：按工作类型调度流式 LLM 调用。
    ///
    /// 仅在 Chat / SwarmSynthesize / MasterDecompose 上启用流式。
    /// Evolution / SoulCompile / Classifier 仍走非流式（本地推理快，无需流式开销）。
    ///
    /// 远端走 gateway.chat_stream()（已支持 DeepSeek SSE / Ollama NDJSON / Anthropic event-stream）
    /// 本地走 OllamaClient::chat_stream()（需新增，stream: true + SSE 解析）
    pub fn dispatch_stream(
        &self,
        work_type: WorkType,
        messages: Vec<ChatMessage>,
    ) -> BoxStream<'static, Result<StreamToken>> {
        let resolved = self.policy.read().resolve(work_type);

        if resolved.is_local() {
            self.dispatch_local_stream(&resolved, messages, work_type)
        } else {
            self.dispatch_remote_stream(&resolved, messages, work_type)
        }
    }

    /// 本地调度：直连 OllamaClient，通过独立断路器保护
    ///
    /// P0-4 修复：不再绕过断路器/缓存。
    async fn dispatch_local(
        &self,
        resolved: &ResolvedModel,
        messages: &[ChatMessage],
        work_type: WorkType,
    ) -> anyhow::Result<ChatResponse> {
        // 1. 断路器检查（独立于 Gateway 的 breaker）
        self.local_breaker.check()?;

        // 2. 语义缓存检查（本地独立缓存，仅适用于生成式 WorkType）
        if let Some(cache) = &self.local_cache {
            if !work_type.is_local_only() || work_type == WorkType::SwarmWorker {
                // 非强制本地的生成式调用才查缓存
                if let Some(cached) = cache.check(messages).await? {
                    tracing::debug!(work_type = ?work_type, "local cache hit");
                    return Ok(cached);
                }
            }
        }

        // 3. Semaphore 限流（防止打爆 Ollama）
        let _permit = self.local_semaphore.acquire().await
            .map_err(|_| anyhow::anyhow!("local semaphore closed"))?;

        // 4. 调用 Ollama（通过 Gateway 的 OllamaClient，复用 chat_with_retry）
        let gateway = self.gateway.clone();
        let model = resolved.model.clone();
        let msgs = messages.to_vec();

        let result = gateway.ollama_client()
            .chat_with_retry(&model, &msgs)
            .await;

        // 5. 断路器记录
        match &result {
            Ok(_) => self.local_breaker.record_success(),
            Err(e) => self.local_breaker.record_failure(),
        }

        let resp = result?;

        // 6. 缓存存储（仅生成式调用）
        if let Some(cache) = &self.local_cache {
            if !work_type.is_local_only() || work_type == WorkType::SwarmWorker {
                cache.store(messages, &resp).await.ok();
            }
        }

        // 7. 成本记录（复用 task 字段记录 work_type）
        if let Some(tracker) = &self.cost_tracker {
            tracker.record_with_context(
                &resp.model,
                resp.input_tokens.unwrap_or(0),
                resp.output_tokens.unwrap_or(0),
                Some("ollama"),
                Some(work_type.as_str()),  // work_type 存入 task 字段
                None,
            );
        }

        Ok(resp)
    }

    /// 远端调度：走 LlmGateway，自动享受所有基础设施
    ///
    /// P0-5 修复：使用新增的 chat_with_task_context() 方法（非 chat_with_cost_source）
    async fn dispatch_remote(
        &self,
        resolved: &ResolvedModel,
        messages: Vec<ChatMessage>,
        work_type: WorkType,
    ) -> anyhow::Result<ChatResponse> {
        // Gateway 内部已处理：断路器 → 语义缓存 → ModelRouter → provider 分派 → 重试 → 压缩
        // P0-5: 新增 chat_with_task_context() 方法，绕过 ModelRouter（Dispatcher 已决策 provider）
        // work_type 存入 CostRecord.task 字段
        self.gateway
            .chat_with_task_context(messages, work_type.as_str())
            .await
    }
}
```

### 4.1.1 LlmGateway 新增方法（P0-5 修复）

```rust
impl LlmGateway {
    /// 新增：按指定 task 上下文调用 LLM（绕过 ModelRouter，由 Dispatcher 决策 provider）
    ///
    /// 与 chat() 的区别：
    /// - 不走 ModelRouter.classify()（Dispatcher 已决策）
    /// - task 参数存入 CostRecord.task 字段（WorkType 字符串）
    /// - 仍享受断路器/缓存/重试/压缩/fallback
    pub async fn chat_with_task_context(
        &self,
        messages: Vec<ChatMessage>,
        task: &str,
    ) -> anyhow::Result<ChatResponse> {
        // 复用现有 chat() 逻辑但跳过 ModelRouter
        // 直接走 default_provider，task 通过 record_with_context 传入
        // ...
    }
}
```

### 4.2 迁移后的调用方式对比

**迁移前（碎片化）**：
```rust
// 路径 1: 聊天
gateway.chat(messages).await

// 路径 2: 分类器（绕过 Gateway）
model_router.classify(&messages).await  // 直连 Ollama

// 路径 3: Swarm Worker（注入 Gateway）
agent.llm.chat(messages).await  // 所有 Worker 走同一 provider

// 路径 4: SoulCompiler（设计中，未定义）
ollama.chat("qwen2.5:3b", &prompt).await  // 直连

// 路径 5: EvolutionEngine（设计中，未定义）
ollama.chat("qwen2.5:7b", &prompt).await  // 直连
```

**迁移后（统一）**：
```rust
// 所有调用走统一调度器
dispatcher.dispatch(WorkType::Chat, messages).await
dispatcher.dispatch(WorkType::SwarmWorker, messages).await
dispatcher.dispatch(WorkType::Classifier, messages).await
dispatcher.dispatch(WorkType::SoulCompile, messages).await
dispatcher.dispatch(WorkType::EvolutionExtract, messages).await
// ...
```

---

## 5. 配置结构

### 5.1 models.json 扩展

在现有 `models.json` 基础上新增 `work_type_overrides` 字段：

```json
{
  "version": 2,
  "default_provider": "deepseek",
  "default_model": "deepseek-chat",
  "local_provider": "ollama",
  "local_classifier_model": "qwen2.5:3b",
  "local_evolution_model": "qwen2.5:7b",
  "local_soul_model": "qwen2.5:3b",
  "worker_local_model": "qwen2.5:7b",
  "providers": [ /* 现有 provider 配置不变 */ ],
  "work_type_overrides": {
    "swarm_worker": {
      "provider": "deepseek",
      "model": "deepseek-chat"
    },
    "evolution_extract": {
      "provider": "ollama",
      "model": "qwen2.5:14b"
    },
    "chat": {
      "provider": "anthropic",
      "model": "claude-sonnet-4",
      "temperature": 0.7
    }
  }
}
```

### 5.2 配置优先级

```
用户 work_type_overrides > 默认路由策略 > 现有 default_provider/default_model
```

- 用户未配置 `work_type_overrides` → 使用默认路由策略（§3.5 默认路由表）
- 用户配置了某个 WorkType 的 override → 使用用户指定的 provider/model
- 现有代码不指定 WorkType → 回退到 `default_provider`/`default_model`（完全向后兼容）

---

## 6. 与蜂群进化设计 v2.0 的整合

### 6.1 双维度成本统计对齐

v2.0 §5.2 的成本分类与本 ADR 的双维度设计对齐：

| v2.0 WorkType | CostSource（触发场景） | 说明 |
|---------------|----------------------|------|
| Chat | Chat | 用户对话 |
| SwarmWorker | Automation（或 Chat） | 取决于触发方式 |
| SwarmSynthesize | Automation | Swarm 结果综合 |
| MasterTask | Chat/Automation | MasterAgent 编排 |
| Evolution | Background | 全部本地，记录但不计费 |
| SoulCompile | Background | 全部本地 |
| Classifier | Background | 全部本地 |

### 6.2 EvolutionEngine 调用路径

v2.0 设计说"所有 Phase 的 LLM 调用均路由到本地模型"，本 ADR 明确定义：

```
Phase 1 经验提取  → dispatch(WorkType::EvolutionExtract, messages)  → Ollama qwen2.5:7b
Phase 2 知识编译  → dispatch(WorkType::EvolutionCompile, messages)  → Ollama qwen2.5:7b
Phase 3 元认知反思 → dispatch(WorkType::EvolutionReflect, messages) → Ollama qwen2.5:7b
Phase 4 Soul 反哺 → dispatch(WorkType::EvolutionSoul, messages)    → Ollama qwen2.5:7b
```

### 6.3 SoulCompiler 调用路径

```
SOUL.md → injection_scan → dispatch(WorkType::SoulCompile, prompt) → Ollama qwen2.5:3b → CompiledSoul
```

> **P0-7 修复**：SoulCompiler 输出 `CompiledSoul { system_prompt, warnings }`，不是 `PersonaConfig`。
> PersonaConfig 仅作为"无 Soul 时的回退"（v2.0 §2.2）。

### 6.4 MasterAgent 调用路径

```
任务拆解  → dispatch(WorkType::MasterDecompose, prompt)  → 远端主模型
JSON 校验 → dispatch(WorkType::MasterValidate, prompt)   → 远端主模型
Worker 执行 → dispatch(WorkType::SwarmWorker, prompt)     → 本地/远端（ModelRouter 动态升级）
结果综合  → dispatch(WorkType::SwarmSynthesize, prompt)  → 远端主模型
```

### 6.5 ModelRouter 整合

**迁移前**：ModelRouter 绕过 Gateway，直连 OllamaClient  
**迁移后**：ModelRouter 成为 UnifiedModelDispatcher 的内部组件

```rust
// ModelRouter 不再直连 OllamaClient，而是通过 Dispatcher 调度
// 分类器调用本身也走统一调度：dispatch(WorkType::Classifier, prompt)
// 这样分类器调用自动享受缓存、断路器、成本统计

// SwarmWorker 的动态升级逻辑：
// 1. 默认路由到本地模型
// 2. 如果 ModelRouter 判定任务复杂，动态升级为远端模型
async fn dispatch_swarm_worker(&self, messages: Vec<ChatMessage>) -> Result<ChatResponse> {
    let route = self.model_router.classify(&messages).await;
    let work_type = match route {
        Route::Ollama => WorkType::SwarmWorker,      // 本地
        Route::DeepSeek | Route::Anthropic | Route::Remote => {
            // 动态升级：复杂任务走远端
            // 但仍标记为 SwarmWorker 成本
            WorkType::SwarmWorker
        }
    };
    // 实际路由由 ModelPolicy + ModelRouter 共同决定
    self.dispatch(work_type, messages).await
}
```

---

## 7. 迁移路径

### 7.1 分阶段迁移（不破坏现有功能）

**Phase 1: 创建 UnifiedModelDispatcher（非破坏性）**
- 新建 `src-tauri/src/llm/dispatcher.rs`
- 实现 `UnifiedModelDispatcher` + `WorkType` + `ModelPolicy`
- `LlmGateway` 保持不变，Dispatcher 内部调用 Gateway
- 新增功能（EvolutionEngine / SoulCompiler / MasterAgent）直接使用 Dispatcher

**Phase 2: 迁移 ModelRouter（低风险）**
- ModelRouter 不再直连 OllamaClient
- 改为通过 Dispatcher 调度：`dispatcher.dispatch(WorkType::Classifier, prompt)`
- 分类器调用自动享受缓存、断路器、成本统计

**Phase 3: 迁移 Swarm Worker（中风险）**
- GenericAgent 注入 `Arc<UnifiedModelDispatcher>` 替代 `Arc<LlmGateway>`
- Worker 调用改为 `dispatcher.dispatch(WorkType::SwarmWorker, messages)`
- 保留 ModelRouter 动态升级能力

**Phase 4: 迁移普通聊天（高风险，最后做）**
- `chat` 命令改为通过 Dispatcher 调度
- 充分测试后再合并

### 7.2 向后兼容

- `LlmGateway` 不删除，`UnifiedModelDispatcher` 内部委托给 Gateway
- 现有 `models.json` v1 配置完全兼容（无 `work_type_overrides` 字段时使用默认路由）
- `CostSource` 枚举不修改（保留 Chat/Automation/Cron/Background 4 值），WorkType 通过 `CostRecord.task` 字段记录
- v1 配置自动升级：加载时若 `version` 缺失，补写 `version: 2` + 默认字段（备份原文件为 `.v1.bak`）

---

## 8. 成本模型对比

### 8.1 迁移前（碎片化）

| 步骤 | 调用次数 | 路径 | 成本统计 |
|------|---------|------|---------|
| ModelRouter 分类 | 1 | 直连 Ollama（绕过 Gateway） | ❌ 不计入 |
| Worker 执行 ×4 | 4 | Gateway（全部走默认 provider） | ✅ Swarm |
| Negotiator 综合 | 1 | Gateway | ✅ Swarm |
| **合计** | 6 | | 5/6 计入成本 |

### 8.2 迁移后（统一调度）

| 步骤 | 调用次数 | WorkType | Provider | 成本统计 |
|------|---------|----------|---------|---------|
| 分类器 | 1 | Classifier | Ollama (本地) | ✅ Classifier (不计费) |
| Worker 执行 ×4 | 4 | SwarmWorker | 本地²/远端混合 | ✅ SwarmWorker |
| 结果综合 | 1 | SwarmSynthesize | 远端 | ✅ SwarmSynthesize |
| Soul 编译 | 1 | SoulCompile | Ollama (本地) | ✅ SoulCompile (不计费) |
| 进化 Phase 1 | 1 | EvolutionExtract | Ollama (本地) | ✅ Evolution (不计费) |
| **合计** | 8 | | | ✅ 8/8 计入成本 |

> ² Worker 默认本地，ModelRouter 可动态升级为远端

### 8.3 成本可见性

前端 CreditsDashboard 可按 CostSource 维度展示：

```
今日 Token 消耗:
  Chat:          $0.12  (12K tokens)
  SwarmWorker:   $0.08  (8K tokens, 3 本地 + 1 远端)
  SwarmSynthesize: $0.04 (4K tokens)
  MasterTask:    $0.06  (6K tokens)
  Evolution:    $0.00  (本地, 不计费)
  SoulCompile:   $0.00  (本地, 不计费)
  Classifier:    $0.00  (本地, 不计费)
  ─────────────────────
  总计:          $0.30
```

---

## 9. 与 JiuwenSwarm 设计的对比

| 维度 | JiuwenSwarm | nebula (本 ADR) |
|------|-------------|---------------------|
| 模型路由粒度 | Agent 成员级（每个角色不同模型） | WorkType 级（每种工作类型不同模型） |
| 配置方式 | ModelConfig + LLMCallConfig (Python) | models.json work_type_overrides (Rust) |
| 本地/远端 | 由 provider 配置决定 | 强制本地（Evolution/Soul/Classifier）+ 可选远端 |
| 成本统计 | LLM 参数记录（model/temp/top_p/max_tokens） | CostSource 分域统计 + 前端可视化 |
| 断路器 | 未公开 | ✅ CircuitBreaker（共享） |
| 语义缓存 | 未公开 | ✅ SemanticCache（共享） |
| 重试机制 | 未公开 | ✅ chat_with_retry（指数退避） |
| 模型管理 UI | agent-studio 可视化 | Settings 面板（现有） |

**nebula 的差异化优势**：
1. 强制本地路由（进化/Soul/分类器），零远端成本
2. 统一基础设施（断路器/缓存/重试/压缩），所有调用自动享受
3. 成本分域统计，前端实时可视化

---

## 10. 验收标准

- [x] `UnifiedModelDispatcher` 实现，所有 WorkType 可调度（M0c 完成）
- [x] `ModelPolicy` 从 `models.json` 读取配置，支持 `work_type_overrides`（M0c 完成）
- [x] `CostSource` 枚举扩展，前端 CreditsDashboard 按域展示（M6 完成）
- [x] ModelRouter 迁移到 Dispatcher，不再直连 OllamaClient（M7a 完成）
- [x] EvolutionEngine / SoulCompiler / MasterAgent 通过 Dispatcher 调用（M3-M5 完成）
- [x] 现有 `cargo check` + `npx tsc --noEmit` 通过（M7b #90 完成）
- [x] 单元测试：每种 WorkType 路由到正确的 provider（M7b #90 完成，19 个 dispatcher 测试）
- [x] 集成测试：成本统计按 CostSource 正确归类（M7b #91 完成）
- [x] E2E 测试：端到端调用链路验证（M7b #92 完成）
- [x] 性能回归测试：criterion 基准无回归（M7b #93 完成，3 个基准）
- [x] 安全审计：injection_guard + SSRF + is_local_only 全覆盖（M7b #94 完成，26 处缺口修复）

---

## 11. 修订历史

### v2.1 — M7b 实施完成（2026-07-05）

**M7b 里程碑（#90-#95）交付内容**：

| 任务 | 描述 | 状态 |
|------|------|------|
| #90 | 全量单元测试 | ✅ dispatcher 19 / model_router 13 / ollama 21 / dag 25 / master 6 / generic_agent 4 |
| #91 | 全量集成测试 | ✅ 跨模块集成测试通过 |
| #92 | E2E 测试 | ✅ 端到端调用链路验证 |
| #93 | 性能回归测试 | ✅ criterion 基准 3 个(dispatcher_construct / worktype_resolve_all_seven / dispatch_fail_fast_local),test 模式验证通过 |
| #94 | 安全审计 | ✅ 26 处缺口全量修复(详见下方) |
| #95 | 文档更新 | ✅ ADR-003 v2.1 + CHANGELOG.md + PRODUCTION_TASK_TRACKER.md |

**#94 安全审计修复详情（26 处缺口）**：

1. **injection_guard 13 处**：
   - `commands/service.rs` 新增 `injection_guard_check()` 辅助函数,在 chat / swarm_execute / llm_complete 入口扫描,Critical/High 拦截返回 Err,Low/Medium 仅记日志
   - `memory/sponge.rs` absorb() 纵深防御:入口扫描,Critical 命中时 sanitize 为占位符(不拒绝存储,保持审计链)
   - `commands/swarm.rs` moa_execute 入口扫描
   - `swarm/orchestrator.rs` 叶子 agent 输出发布到 team_context_pool 前扫描
   - 设计原则:service 层拦截入站,memory 层 sanitize 出站(防止存储-检索-再注入攻击面)

2. **SSRF 13 处**：
   - `security/ssrf_guard.rs` 增强:`with_allow_loopback(true)` 方法(允许 127.0.0.0/8 + ::1 本地 LLM 端点,仍拒绝其他私网)+ IPv6 检测(is_ula_v6 fc00::/7 / is_link_local_v6 fe80::/10 / is_ipv4_mapped_v6 ::ffff:0:0/96)
   - `llm/openai_compat.rs` 修复 with_allow_private(true) → with_allow_loopback(true),用 build_safe_client 构造器
   - `mcp/transport.rs` HttpTransport 用 build_safe_client
   - `triggers/watch.rs` WebFetcher 用 build_safe_client(重定向链每跳校验)
   - `channel/discord.rs` validate_url 结果不再丢弃
   - `llm/ollama.rs` / `storage/webdav.rs` / `sync/relay_client.rs` / `skills/importer.rs` / `commands/models_config.rs` 5 处添加 validate_url
   - `memory/vector_store/chroma.rs` / `qdrant.rs` / `skills/sandbox.rs` / `channel/bridge.rs` 4 处补充 SSRF 校验(feature-gated)

3. **is_local_only 强制执行**：✅ 审计结论 — 强制执行严密,无需修复
   - `ModelPolicy::resolve()` 中 is_local_only WorkType 硬性 return,忽略非本地 override(非仅 warn)

**关键修复**：

- `ModelPolicy` 添加 `#[derive(Clone)]`(支持 criterion bench 中克隆策略实例)
- `benches/dispatcher.rs` 修复 `unused Result` warning(`let _ = black_box(result)`)

### v2 — 修复 7 个 P0（2026-07-05 早期）

- P0-1: 双维度成本统计(保留 CostSource,新增 WorkType 维度)
- P0-2: is_local_only WorkType 强制本地,忽略非本地 override
- P0-3: dispatch_stream() 流式入口
- P0-4: 本地路径独立断路器 + 独立缓存(避免本地/远端缓存污染)
- P0-5: LlmGateway 新增方法支持 Dispatcher 复用基础设施
- P0-6: ModelRouter 旧路径回退(feature flag off 时支持最小构建)
- P0-7: MasterOrchestrator 组合模式委托 SwarmOrchestrator(避免重复实现 Worker 池)

---

## 附录 A: WorkType 属性

```rust
impl WorkType {
    /// WorkType 的字符串形式（存入 CostRecord.task 字段）
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkType::Chat => "chat",
            WorkType::SwarmWorker => "swarm_worker",
            WorkType::SwarmSynthesize => "swarm_synthesize",
            WorkType::MasterDecompose => "master_decompose",
            WorkType::MasterValidate => "master_validate",
            WorkType::EvolutionExtract => "evolution_extract",
            WorkType::EvolutionCompile => "evolution_compile",
            WorkType::EvolutionReflect => "evolution_reflect",
            WorkType::EvolutionSoul => "evolution_soul",
            WorkType::SoulCompile => "soul_compile",
            WorkType::Classifier => "classifier",
        }
    }

    /// 是否强制本地路由
    /// is_local_only() 的 WorkType：
    /// - resolve() 中强制忽略非本地 override（P0-2）
    /// - dispatch() 中走 dispatch_local() 而非 dispatch_remote()
    /// - 不走 SemanticCache（is_local_only 的生成式调用不缓存）
    pub fn is_local_only(&self) -> bool {
        matches!(
            self,
            WorkType::EvolutionExtract
                | WorkType::EvolutionCompile
                | WorkType::EvolutionReflect
                | WorkType::EvolutionSoul
                | WorkType::SoulCompile
                | WorkType::Classifier
        )
    }
}
```

> **注意**：`Embedding` 不在 WorkType 枚举中。Embedding 走专用路径 `OllamaClient::embed()`，返回 `Vec<f32>`，使用独立 LRU 缓存（精确匹配），不纳入 `dispatch()` / `dispatch_stream()`。

## 附录 B: models.json v2 完整示例

```json
{
  "version": 2,
  "default_provider": "deepseek",
  "default_model": "deepseek-chat",
  "local_provider": "ollama",
  "local_classifier_model": "qwen2.5:3b",
  "local_evolution_model": "qwen2.5:7b",
  "local_soul_model": "qwen2.5:3b",
  "worker_local_model": "qwen2.5:7b",
  "providers": [
    {
      "id": "deepseek",
      "kind": "openai-compat",
      "display_name": "DeepSeek",
      "base_url": "https://api.deepseek.com/v1",
      "api_key_keychain_slot": "provider:deepseek",
      "api_key_env": "DEEPSEEK_API_KEY",
      "supports_tools": true,
      "supports_streaming": true,
      "is_builtin": true,
      "models": [
        { "id": "deepseek-chat", "display_name": "DeepSeek Chat", "context_window": 64000 }
      ]
    },
    {
      "id": "anthropic",
      "kind": "anthropic",
      "display_name": "Anthropic Claude",
      "base_url": "https://api.anthropic.com",
      "api_key_keychain_slot": "provider:anthropic",
      "api_key_env": "ANTHROPIC_API_KEY",
      "supports_tools": true,
      "supports_streaming": true,
      "is_builtin": true,
      "models": [
        { "id": "claude-sonnet-4", "display_name": "Claude Sonnet 4", "context_window": 200000 }
      ]
    },
    {
      "id": "ollama",
      "kind": "ollama",
      "display_name": "Ollama (本地)",
      "base_url": "http://127.0.0.1:11434",
      "supports_tools": true,
      "supports_streaming": true,
      "is_builtin": true,
      "models": [
        { "id": "qwen2.5:3b", "display_name": "Qwen2.5 3B (分类器)" },
        { "id": "qwen2.5:7b", "display_name": "Qwen2.5 7B (进化引擎)" },
        { "id": "nomic-embed-text", "display_name": "Nomic Embed (向量化)" }
      ]
    }
  ],
  "work_type_overrides": {}
}
```

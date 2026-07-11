# LLM 网关 行为契约

> **领域**: llm
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

LLM 网关是 Nebula 所有 AI 调用的统一入口,由 UnifiedModelDispatcher(ADR-003)编排,支持多提供商(Anthropic / Ollama / OpenAI 兼容)、成本追踪、语义缓存、模型健康检查与竞技场(Arena)A/B 测试。所有 LLM 调用——聊天、蜂群、Master、进化、Soul 编译、分类——均经此网关。

## Requirements

### Requirement: 统一模型调度
The system SHALL dispatch all LLM calls through the UnifiedModelDispatcher (ADR-003).
- 单一入口:聊天 / Swarm / Master / 进化 / Soul 编译 / 分类均经 `UnifiedModelDispatcher`
- `WorkType` 枚举按工作类型分类:Chat / SwarmWorker / Evolution / SoulCompile / Classifier / MasterDecompose 等
- `ModelPolicy`:WorkType → (provider, model) 路由,用户可通过 `models.json` 的 `work_type_overrides` 自定义
- `is_local_only()` 强制约束:Evolution / SoulCompile / Classifier 强制走本地 Ollama,忽略非本地 override
- 独立断路器:本地路径持有独立 `CircuitBreaker`,与远端 Gateway breaker 解耦
- 流式接口:`dispatch_stream()` 返回 `BoxStream<Result<StreamToken>>`
- Feature 门控:`unified-dispatcher` feature 默认启用;`UNIFIED_DISPATCHER_ENABLED=0` 可运行时禁用

#### Scenario: 工作类型路由
- **WHEN** 蜂群 Worker 发起 LLM 调用(WorkType = SwarmWorker)
- **THEN** Dispatcher 根据 `models.json` 的 `work_type_overrides` 路由到对应 (provider, model)
- **AND** 若无 override,使用默认路由策略

#### Scenario: 本地强制约束
- **WHEN** 进化引擎发起 LLM 调用(WorkType = Evolution)
- **THEN** `is_local_only()` 返回 true,强制路由到本地 Ollama
- **AND** 即使用户配置了非本地 override,该 override 被忽略

### Requirement: 多提供商支持
The system SHALL support multiple LLM providers: Anthropic, Ollama, and OpenAI-compatible endpoints.
- `ollama` — 本地 Ollama HTTP 客户端(`/api/chat` / `/api/generate` / `/api/embeddings`)
- `anthropic` — Anthropic Claude HTTP 客户端
- `openai_compat` — OpenAI 兼容层(vLLM / LMStudio / OpenRouter / DeepSeek 等)
- `models.json` 动态配置:provider/model 注册表(`ModelsConfig` / `ProviderConfig` / `ModelConfig`)
- `ProviderKind` 枚举标识提供商类型
- `Pricing` 字段记录模型单价(input_per_1M / output_per_1M USD)

#### Scenario: DeepSeek 经 OpenAI 兼容层
- **WHEN** 用户配置 DeepSeek API 端点
- **THEN** 通过 `openai_compat::OpenAICompatClient` 发起请求
- **AND** DeepSeek 作为优先提供商(若 `work_type_overrides` 配置)

#### Scenario: Ollama 本地 fallback
- **WHEN** 远端提供商(Anthropic / DeepSeek)不可用
- **THEN** Dispatcher 回退到本地 Ollama
- **AND** 本地路径独立断路器不受远端故障影响

### Requirement: 成本追踪
The system SHALL track token costs per model with daily/monthly aggregation.
- `CostTracker` 按 token 用量换算美元:input_tokens × input_price + output_tokens × output_price
- 单位:micro-cent(1 USD = 10^8 micro-cent,避免浮点)
- 聚合维度:`DailyAggregate` / `WeeklyAggregate` / `MonthlyAggregate` / `ProviderBucket`
- `CostSource` 分类:Chat / Automation / Cron / Background(区分人工与自动化成本)
- `CostPolicy` 预算门禁:`max_tokens_per_task` + `daily_task_limit`
- `CostDecision` 返回 Allow / Deny / Throttle
- 全局计数器:`metrics::global().token_cost_usd`

#### Scenario: 预算超限拒绝
- **WHEN** 当日累计 token 成本超过 `daily_task_limit`
- **THEN** `CostPolicy` 返回 Deny
- **AND** LLM 调用被拒绝,提示用户预算已耗尽

#### Scenario: 自动化成本区分
- **WHEN** Cron 触发的自动化任务产生 LLM 调用
- **THEN** `CostSource` 标记为 `Cron`
- **AND** 成本聚合中与人工 Chat 成本分开统计

### Requirement: 语义缓存
The system SHALL short-circuit LLM calls via a semantic cache when query similarity exceeds threshold.
- `SemanticCache` 插入在 L0 精确缓存与 LLM 调用之间
- 相似度阈值:cosine ≥ 0.92(默认 `DEFAULT_SIMILARITY_THRESHOLD`)
- TTL:默认 1 小时,过期视为 miss
- 复用基础设施:直接持有 `VectorStore` + `Embedder`,零新增依赖
- 失败降级:embed / 向量查询错误返回 None,不阻断主调用链
- 指标:`semantic_cache_hits` / `semantic_cache_misses` 原子计数器
- LRU 缓存:`lru` crate 提供进程内 LRU 驱逐

#### Scenario: 语义缓存命中
- **WHEN** 用户查询与缓存中某条目 cosine ≥ 0.92 且未过 TTL
- **THEN** 直接返回缓存响应,跳过 LLM 调用
- **AND** 累加 `semantic_cache_hits`

#### Scenario: 语义缓存降级
- **WHEN** Embedder 或向量存储发生错误
- **THEN** 返回 None(视为 miss),LLM 正常调用
- **AND** 不阻断主调用链,仅记录 debug 日志

### Requirement: 模型健康检查
The system SHALL track model health metrics including latency, error rate, and circuit breaker state.
- `ModelHealthTracker` 追踪每个 provider 的健康指标
- `ProviderMetrics`:延迟 / 错误率 / 断路器状态
- `CircuitBreaker`:连续失败触发熔断,半开状态试探恢复
- 前端 `ModelHealthPanel` 展示健康状态
- Ollama 健康检查:每 30 秒轮询(前端 `checkOllama`)

#### Scenario: 断路器熔断与恢复
- **WHEN** 远端 provider 连续失败达到阈值
- **THEN** CircuitBreaker 熔断,后续请求直接失败(不走远端)
- **AND** 半开状态下试探性发送一个请求,成功则恢复

### Requirement: 竞技场
The system SHALL provide an A/B testing arena for model comparison with ELO scoring.
- `Arena` 模块支持模型对战(A/B 测试)
- `ArenaMatch` 记录对战结果
- ELO 评分系统:根据胜负调整模型评分
- `ArenaLeaderboard` 排行榜持久化到 SQLite(migration 034)
- 前端 `ArenaPanel` 展示对战与排行

#### Scenario: 模型对战与 ELO 评分
- **WHEN** 两个模型对同一 prompt 生成响应,用户选择更优者
- **THEN** `ArenaMatch` 记录胜负
- **AND** ELO 评分更新,排行榜刷新

### Requirement: LRU 缓存
The system SHALL use an LRU cache for hot-path in-process caching.
- `lru` crate(v0.16)提供 `LruCache`
- 用于 SemanticCache 的进程内条目驱逐
- `NonZeroUsize` 保证容量非零
- 热路径缓存:embed 结果 / 短期响应缓存

#### Scenario: LRU 驱逐
- **WHEN** LRU 缓存达到容量上限
- **THEN** 最久未使用的条目被驱逐
- **AND** 新条目插入缓存

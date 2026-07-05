# 蜂群进化 v2.0 + ADR-003 统一模型调度层 — 七专家合并评审综合报告

> **评审日期**: 2026-07-05  
> **评审对象**: SWARM_EVOLUTION_DESIGN_v2.md + ADR-003-unified-model-dispatcher.md  
> **评审团**: EA-1 架构师 / EA-2 记忆专家 / EA-3 蜂群工程师 / EA-4 安全工程师 / EA-5 工程经理 / EA-6 模型调度专家 / EA-7 生产交付专家  
> **总体评级**: **有条件通过** — 设计方向正确，但存在 11 个 P0 阻塞项必须先修订

---

## 1. 评审总览

| 专家 | P0 | P1 | P2 | 评级 |
|------|----|----|----|----|
| EA-1 架构师 | 2 | 7 | 7 | 有条件通过 |
| EA-2 记忆专家 | 5 | 8 | 3 | B-（需修订） |
| EA-3 蜂群工程师 | 3 | 6 | 5 | B-（需修订） |
| EA-4 安全工程师 | 3 | 5 | 5 | B-（需修订） |
| EA-5 工程经理 | 6 | 6 | 4 | C+（需补 P0） |
| EA-6 模型调度专家 | 4 | 5 | 4 | B-（需修订） |
| EA-7 生产交付专家 | 5 | 8 | 5 | 黄灯 |
| **原始合计** | **28** | **45** | **33** | |
| **去重后独立** | **11** | **22** | **18** | |

---

## 2. P0 发现（去重后 11 个，必须 M0 前修复）

### P0-1 CostSource 枚举与现有代码严重不符（EA-3/EA-6/EA-7 共识）

**发现者**: EA-3 P0-1 / EA-6 P0-1 / EA-7 P0-1  
**位置**: ADR-003 §3.3 vs `cost_tracker.rs:41-53`

ADR-003 声称现有 CostSource 有 `Chat/Swarm`，实际是 `Chat/Automation/Cron/Background`。ADR-003 的扩展方案会删除 `Automation/Cron/Background`，破坏 T-E-A-12 自动化 Credits 系统、migration 027 数据、预算告警逻辑。

**修复方案**: 不重定义 CostSource。保留现有 4 值（按"触发场景"分类），在 `CostRecord` 上新增 `work_type: Option<String>` 字段（按"工作类型"分类）。两个维度正交，前端可按任一维度聚合。

### P0-2 `is_local_only()` 约束在 `resolve()` 中未被强制执行（EA-1/EA-4 共识）

**发现者**: EA-1 FINDING-1.5 / EA-4 P0-1  
**位置**: ADR-003 §3.4 `ModelPolicy::resolve()`

用户可在 `work_type_overrides` 中为 `EvolutionExtract` 配置 `"provider": "deepseek"`，`resolve()` 直接返回用户配置，不检查 `is_local_only()`。导致进化引擎的 L1 用户对话记录（含隐私内容）被发送到远端 provider。

**修复方案**: `resolve()` 对 `is_local_only()` 返回 true 的 WorkType 强制忽略非本地 override，并 `tracing::warn!` 记录拒绝日志。`ModelsConfig::validate()` 启动期 fail-fast。

### P0-3 `dispatch()` 不支持流式输出（EA-3/EA-6 共识）

**发现者**: EA-3 P0-2 / EA-6 P1-2  
**位置**: ADR-003 §4.1 vs v2.0 §9 Layer 3

`dispatch()` 返回 `ChatResponse`（一次性），不支持流式。v2.0 §9 Layer 3 要求 MasterAgent synthesize 阶段逐 token 渲染（< 200ms 延迟）。现有 `LlmGateway` 已有 `chat_stream()` 返回 `BoxStream`。

**修复方案**: 新增 `dispatch_stream(work_type, messages) -> BoxStream<Result<StreamToken>>`，仅在 Chat/SwarmSynthesize/MasterDecompose 上启用。

### P0-4 `dispatch_local()` 绕过断路器/缓存/压缩（EA-3/EA-6 共识）

**发现者**: EA-3 P0-3 / EA-6 P0-4  
**位置**: ADR-003 §4.1 `dispatch_local()`

本地路径直连 `OllamaClient::chat_with_retry()`，完全绕过 Gateway 的断路器、SemanticCache、TokenJuice 压缩。4-6 Worker 并发 + EvolutionEngine 同时跑时，若 Ollama 宕机，所有调用并发打挂无熔断，雪崩。

**修复方案**: 为本地路径创建独立 `CircuitBreaker`（与远端 breaker 解耦），或让 `dispatch_local` 复用 Gateway 的 Ollama 路径。

### P0-5 `chat_with_cost_source` 方法不存在（EA-3/EA-6 共识）

**发现者**: EA-3 P1-3 / EA-6 P0-3  
**位置**: ADR-003 §4.1 `dispatch_remote()`

ADR 调用 `self.gateway.chat_with_cost_source()`，但 `LlmGateway` 没有这个方法。现有方法：`chat` / `chat_with_tools` / `chat_with_model` / `chat_with_provider` / `chat_stream`。

**修复方案**: 在 LlmGateway 新增 `chat_with_task_context(messages, task: &str)` 方法，内部走 `record_with_context(model, in, out, Some(provider), Some(task), None)`。

### P0-6 MasterAgent fan-out 职责矛盾（EA-1 独立发现）

**发现者**: EA-1 FINDING-1.1  
**位置**: v2.0 §1.1 vs §8.1

§1.1 说"SwarmOrchestrator 负责 fan-out"，§8.1 说"MasterAgent 自己 fan-out"（直接 acquire Worker）。两条路径互斥。

**修复方案**: 采用方案 A——MasterOrchestrator 完全委托 SwarmOrchestrator 做 fan-out。删除 §8.1 的 `dispatch_round()` 伪代码。MasterAgent 只做拆解/synthesize。

### P0-7 SoulCompiler 输出类型矛盾（EA-1 独立发现）

**发现者**: EA-1 FINDING-1.2  
**位置**: v2.0 §2.5 vs ADR-003 §3.2/§6.3

v2.0 说 SoulCompiler 输出 `CompiledSoul`，ADR-003 说输出 `PersonaConfig`。类型签名根本性矛盾。

**修复方案**: 修正 ADR-003 §3.2 注释和 §6.3 终点为 `CompiledSoul { system_prompt, warnings }`。

### P0-8 Embedding WorkType 返回类型不兼容 + 维度冲突（EA-2 独立发现）

**发现者**: EA-2 P0-1/P0-2  
**位置**: ADR-003 §3.2 WorkType::Embedding vs embedder.rs

`dispatch()` 返回 `ChatResponse`，但 Embedding 需要 `Vec<f32>`。现有 Embedder 硬编码 BGE-small-zh-v1.5（512 维），ADR 默认 `nomic-embed-text`（768 维），切换会让现有向量库作废。

**修复方案**: Embedding 不纳入 `dispatch()`。保留 Embedder 直连 Ollama `/api/embeddings`，走专用路径。从 WorkType 枚举中移除 `Embedding`。

### P0-9 Memory domain 字段改动量被严重低估（EA-2 独立发现）

**发现者**: EA-2 P0-4/P0-5  
**位置**: v2.0 §7.2 vs types.rs/sqlite_store.rs/acl.rs

Memory struct 缺 `domain` 字段，memories 表缺 `domain` 列，MemoryAcl 接口语义与现有实现完全不兼容（需重写不是扩展），TRUSTED_PRINCIPALS 硬编码与 PrincipalDomainMap 直接冲突。M2 的 10-14 天估算偏紧。

**修复方案**: M2 拆分为 M2a（schema + struct）+ M2b（ACL + PrincipalDomainMap），上调到 14-18 天。先做 1 天调用点审计。

### P0-10 ADR-001/ADR-002 文档不存在 + petgraph 未引入（EA-5 独立发现）

**发现者**: EA-5 P0-01/P0-02  
**位置**: v2.0 引用 ADR-001/002 但 `docs/` 下只有 ADR-003；Cargo.toml 无 petgraph

v2.0 把"ADR-001 已决定组合模式"、"ADR-002 已决定 petgraph DAG"作为已定事实，但两份 ADR 根本不存在。petgraph 依赖未引入 Cargo.toml。

**修复方案**: M0a 补齐 ADR-001 + ADR-002。M0b 引入 petgraph + CI 烟囱测试。

### P0-11 缺少 feature flag + PR 拆分策略（EA-5/EA-7 共识）

**发现者**: EA-5 P0-05 / EA-7 P0-3  
**位置**: v2.0 全文 + ADR-003 全文

2000+ 行新代码无 feature flag 无法灰度发布。无 PR 拆分策略无法安全评审。

**修复方案**: Cargo.toml 新增 4 个 feature flag（`soul_system` / `master_orchestrator` / `evolution_engine` / `unified_dispatcher`），默认全 off。补 ADR-004 feature flag 策略。

---

## 3. P1 发现汇总（去重后 22 个）

| # | 发现 | 发现者 | 影响范围 |
|---|------|--------|---------|
| P1-1 | 进化引擎模型参数不一致（3b vs 7b） | EA-1 | 进化效果 |
| P1-2 | DAG 节点缺少 work_type_hint 字段 | EA-1 | Worker 路由 |
| P1-3 | ModelRouter 双层分类 + 分类结果被丢弃 | EA-1/EA-6 | 动态升级失效 |
| P1-4 | MasterTask CostSource 归属不一致 | EA-1 | 成本统计 |
| P1-5 | Dispatcher 远端调用与 Gateway ModelRouter 冲突 | EA-1 | 路由权归属 |
| P1-6 | SemanticCache 不应套用于 Embedding | EA-2 | 向量库污染 |
| P1-7 | DAG 缓存与 SemanticCache 关系未定义 | EA-2 | 缓存冲突 |
| P1-8 | EvolutionEngine 记忆读写路径未明确 | EA-2 | domain 隔离盲点 |
| P1-9 | SpongeEngine.absorb 签名需加 principal 参数 | EA-2 | 调用面广 |
| P1-10 | Negotiator 仍持 &LlmGateway 绕过 Dispatcher | EA-3 | 双路径并存 |
| P1-11 | OllamaClient 并发限制未提及 | EA-3/EA-6 | 本地"并行"实为串行 |
| P1-12 | WorkType 12 变体过度细分 | EA-3/EA-6 | 配置膨胀 |
| P1-13 | SoulCompiler 注入扫描未覆盖 L2/L3/L5 | EA-4 | 跨域污染 |
| P1-14 | SOUL.md 进化写入非原子性 | EA-4 | 崩溃后身份损坏 |
| P1-15 | MasterDecompose 默认远端泄露用户任务描述 | EA-4 | 隐私 |
| P1-16 | models.json provider base_url 缺失 SSRF 校验 | EA-4 | SSRF |
| P1-17 | 里程碑依赖映射缺失 | EA-5 | 排期风险 |
| P1-18 | ADR-003 测试计划缺失 | EA-5/EA-7 | 质量保证 |
| P1-19 | 迁移不可逆，回滚策略缺失 | EA-5/EA-7 | 无法回退 |
| P1-20 | Dispatcher 缺少 tracing span | EA-7 | 可观测性断链 |
| P1-21 | EventEnvelope 未覆盖 MasterEvent | EA-7 | trace_id 丢失 |
| P1-22 | 配置热重载策略不一致 | EA-7 | 生效时机 |

---

## 4. 共识结论

### 4.1 七专家一致认同

1. **设计方向正确**：v2.0 的"双主智能体 + 自主进化"需要 ADR-003 的"统一模型调度层"作为基础设施
2. **WorkType 路由概念合理**：参考 JiuwenSwarm 的"成员级路由"，nebula 的"WorkType 级"是合理的本土化
3. **本地优先策略正确**：Evolution/Soul/Classifier 强制本地，零远端成本
4. **渐进迁移路径可行**：Phase 1-4 分阶段迁移 + 向后兼容策略完备

### 4.2 七专家共同关切

1. **CostSource 是地基性错误**（3 位专家独立发现）：ADR-003 对现有代码的认知存在硬错误，会破坏生产数据
2. **本地路径绕过基础设施**（2 位专家独立发现）：断路器/缓存/压缩全部失效
3. **bus factor=1 风险**：所有专家都提到，但无解法——只能通过文档完备性降低风险
4. **与现有 evolution/ 模块冲突**（EA-5）：PromptSelfMutator + SkillAutoEvolver 与 EvolutionEngine 功能重叠

### 4.3 修订条件（M0 前置必须完成）

| 序号 | 修订项 | 预计工时 |
|------|--------|---------|
| 1 | 修正 ADR-003 CostSource 设计（P0-1） | 0.5 天 |
| 2 | `resolve()` 强制 `is_local_only()` 约束（P0-2） | 0.5 天 |
| 3 | 新增 `dispatch_stream()` 流式接口（P0-3） | 1 天 |
| 4 | 本地路径接入断路器/缓存（P0-4） | 1 天 |
| 5 | 明确 `chat_with_task_context` 新方法签名（P0-5） | 0.5 天 |
| 6 | 修订 v2.0 §1.1 与 §8.1 fan-out 矛盾（P0-6） | 0.5 天 |
| 7 | 修正 SoulCompiler 输出类型（P0-7） | 0.5 天 |
| 8 | 移除 Embedding WorkType（P0-8） | 0.5 天 |
| 9 | M2 拆分 + 调用点审计（P0-9） | 1 天 |
| 10 | 补齐 ADR-001/002 + petgraph 引入（P0-10） | 2 天 |
| 11 | feature flag + PR 拆分策略（P0-11） | 1 天 |
| **合计** | | **8.5 天** |

---

## 5. 修订后合并工时估算

| 阶段 | 内容 | P50 | P90 | 前置依赖 |
|------|------|-----|-----|---------|
| **M0a** | 补齐 ADR-001 + ADR-002 | 2d | 3d | — |
| **M0b** | petgraph 引入 + CI 烟囱测试 | 1d | 2d | M0a |
| **M0c** | 修订 ADR-003（11 个 P0 修复）+ Dispatcher 骨架 | 5d | 7d | M0b |
| **M1** | Soul 系统 + SoulCompiler + 注入扫描 | 8d | 11d | M0c |
| **M2a** | domain schema + Memory struct + 调用点审计 | 7d | 10d | M1 |
| **M2b** | ACL 重写 + PrincipalDomainMap + TRUSTED_PRINCIPALS | 7d | 10d | M2a |
| **M3** | MasterOrchestrator + DAG + ModelRouter 迁移 + SwarmWorker 迁移 | 16d | 22d | M2b, M0c |
| **M4** | EvolutionEngine 4 Phase + 与现有 evolution/ 整合 | 12d | 16d | M3 |
| **M5** | L4 审批 + 成本路由统一 + 流式 MVP | 9d | 13d | M4 |
| **M6** | 前端 Soul 编辑器 + 进化日志 + 蜂群画布 + 流式 | 13d | 17d | M5 |
| **M7a** | chat 命令迁移到 Dispatcher | 4d | 6d | M6 |
| **M7b** | 集成测试 + 回归 + 发布准备 | 6d | 9d | M7a |
| **合计** | | **90d** | **126d** | |

关键路径（slack=0）：M0a → M0b → M0c → M1 → M2a → M2b → M3 → M4 → M5 → M6 → M7a → M7b

---

## 6. 测试计划合并

| 类型 | v2.0 原有 | ADR-003 新增 | 合并后 |
|------|----------|-------------|--------|
| 单元测试 | 107 | ~48 | 155 |
| 集成测试 | 24 | ~22 | 46 |
| E2E 测试 | 7 | ~3 | 10 |
| **合计** | 138 | ~73 | **211** |

---

## 7. 总体评级

**🟡 有条件通过**

两份文档在战略方向上高度一致，设计质量较高。但 ADR-003 与现有代码存在硬冲突（CostSource 枚举错误），本地路径绕过基础设施（断路器/缓存），缺少 feature flag 和回滚策略。

**通过条件**：完成 11 个 P0 修订（预计 8.5 天），修订后可进入 M1 实施。

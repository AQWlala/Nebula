# 记忆系统 行为契约

> **领域**: memory
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

记忆系统是 Nebula 的认知核心,负责对话历史留存、知识抽取、事实压缩、价值层评估与反思。系统采用 6 层活跃架构(L0-L5,L6-L7 预留),由黑洞压缩引擎(BlackholeEngine)与海绵吸收引擎(SpongeEngine)驱动,后端为 SQLite(结构化)+ LanceDB(向量),检索采用 BM25 + 向量混合检索。

## Requirements

### Requirement: 六层记忆架构
The system SHALL maintain a six-layer active memory hierarchy (L0-L5), with L6-L7 reserved for future use.
- L0 — 临时缓存(单轮对话,不参与检索,TTL=0 即不自动驱逐)
- L1 — 会话内滚动消息历史(TTL=1 天,可检索,可共享,可压缩)
- L2 — 跨会话经验(TTL=7 天,默认摘要桶=1)
- L3 — 具体事实(TTL=30 天,默认摘要桶=1)
- L4 — 蒸馏知识(TTL=90 天,默认摘要桶=2)
- L5 — 反思教训(TTL=365 天,默认摘要桶=2)
- L6 — 可复用原则(预留,TTL=0,默认摘要桶=3)
- L7 — 奇点核心价值(预留,永不可压缩,`is_immutable` 返回 true)
- 进入 LLM 上下文窗口的层:L1 / L2 / L3 / L4 / L6(L0 / L5 / L7 不直接进入)
- 自动提升:L3 访问 >10 次且重要性 >0.7 → 提升至 L4;L4 访问 >20 次且重要性 >0.8 → 触发 L5 反思关注(L6 提升暂返回 None,推迟到 v2.5+)

#### Scenario: L0 不参与检索
- **WHEN** 执行混合检索(BM25 + 向量)
- **THEN** L0 层记忆被排除(`searchable = false`)
- **AND** L0 仅在单轮对话存活,不跨轮持久

#### Scenario: L7 永不压缩
- **WHEN** BlackholeEngine 执行压缩 pass
- **THEN** L7 层记忆被跳过(`compressible = false`)
- **AND** `pinned = true` 的记忆同样被跳过

#### Scenario: 自动层提升
- **WHEN** 一条 L3 记忆被访问 11 次且重要性评分 0.75
- **THEN** `check_auto_promote` 返回 `Some(L4)`
- **AND** 该记忆被提升至 L4 层

### Requirement: 黑洞压缩引擎
The system SHALL compress low-importance, stale memories into higher-level semantic capsules via the BlackholeEngine, never deleting originals.
- 触发条件(两者同时满足):记录在 `threshold_days` 天内未被访问,且重要性 ≤ `BLACKHOLE_IMPORTANCE_FLOOR`
- 压缩语义:合并相关记录组为单条更高层摘要,保留溯源链(provenance chain)指向原始记录
- L7 层与 `pinned = true` 记忆永不被压缩
- 压缩产出 `CompressionReport`(scanned / compressed / skipped / summaries_created)
- 遗忘引擎(ForgettingEngine)归档后可调用 `run_pass_archived()` 形成"归档 → 压缩"闭环

#### Scenario: 低重要性记忆被压缩
- **WHEN** 一条 L3 记忆 30 天未访问且重要性 0.2(低于 floor)
- **THEN** BlackholeEngine 将其与相关记录合并为一条 L4 摘要
- **AND** 原始记录保留溯源链,不被物理删除

#### Scenario: pinned 记忆豁免压缩
- **WHEN** 一条记忆 `pinned = true` 且满足压缩触发条件
- **THEN** 该记忆被跳过,不计入 `compressed`

### Requirement: 海绵吸收引擎
The system SHALL incrementally absorb conversation turns via the SpongeEngine, performing de-duplication, normalization, and relation linking.
- 对话增量吸收:每轮对话后触发,将消息写入 L1 并抽取候选记忆
- 去重:与现有记忆做语义相似度比对,重复内容不重复存储
- 归一化:统一格式、实体链接、时间戳标准化
- 关系链接:抽取 `MemoryRelation`(因果关系 / 证据关系等),写入图结构
- 多粒度摘要桶:按层策略 `default_summary_bucket` 选择摘要粒度

#### Scenario: 对话增量吸收
- **WHEN** 用户发送一条消息并收到助手回复
- **THEN** SpongeEngine 将对话写入 L1
- **AND** 抽取候选事实写入 L3,标注 `SourceKind` 与时间戳
- **AND** 重复内容被去重,不产生冗余记忆

### Requirement: 存储后端
The system SHALL persist memories to SQLite (structured) and LanceDB (dense vectors), with in-memory fallback when LanceDB is unavailable.
- SQLite:`SqliteStore` 管理结构化记忆条目、关系、ACL、版本控制
- LanceDB:`LanceStore` 管理 `(id, vector)` 向量索引,由 `vector-store` feature 门控
- 降级:`--no-default-features` 编译时,LanceStore 回退到内存线性余弦扫描
- 加密:可选 SQLCipher(`sqlcipher` feature)提供 SQLite 透明加密
- 迁移:38 个编号迁移文件(`migrations/001_initial.sql` 至 `038_long_tasks.sql`)

#### Scenario: LanceDB 不可用时降级
- **WHEN** `vector-store` feature 未启用或 LanceDB 初始化失败
- **THEN** 向量检索降级为内存线性扫描
- **AND** 系统其余功能不受影响

### Requirement: 混合检索
The system SHALL retrieve memories using BM25 keyword search fused with vector similarity search.
- BM25(经 SQLite FTS5):擅长精确关键词匹配(函数名、文件路径、标识符)
- 向量检索(经 LanceDB):擅长语义相似(改写查询、概念查找)
- 两路并行执行(`tokio::join!`),分数归一化至 [0,1] 后融合
- 融合公式:`final_score = alpha * vector_score + (1 - alpha) * bm25_score`,默认 `alpha = 0.6`(向量倾向)
- 过取因子:每路检索 `limit * over_fetch` 条候选(默认 `over_fetch = 3`),融合后去重并截断至 `limit`

#### Scenario: 语义相似查询
- **WHEN** 用户查询"如何重置路由器",但记忆中存储的是"路由器恢复出厂设置步骤"
- **THEN** 向量检索命中(BM25 可能未命中)
- **AND** 融合后返回该记忆条目

#### Scenario: 精确标识符查询
- **WHEN** 用户查询函数名 `parse_argv`
- **THEN** BM25 精确命中
- **AND** 向量检索可能稀释信号,但融合后 BM25 权重保证命中

### Requirement: 重要性评分
The system SHALL score each memory's importance using the formula defined in `docs/ARCHITECTURE.md` §10.1.
- 公式:`score = base + access_coef * min(access_count, 100)/100 + recency_coef * recency_30d + feedback_coef * feedback + type_weight(memory_type)`
- 默认权重:`base = 0.5`, `access = 0.05`, `recency = 0.20`, `feedback = 0.20`
- recency 半衰期:30 天(非 7 天)
- 类型权重:`Metacognitive = 0.3`, `Emotional = 0.2`,其余 = 0.0
- 最终值 clamp 至 [0.0, 1.0]

#### Scenario: 新存储记忆的基线分
- **WHEN** 一条新记忆被存储(0 次访问,feedback=0,Semantic 类型)
- **THEN** 重要性评分 = 0.5(base)
- **AND** 评分不低于"未读但非平凡"水平

#### Scenario: 元认知记忆加分
- **WHEN** 一条 Metacognitive 类型记忆被存储
- **THEN** 评分额外加 0.3(type_weight)
- **AND** 反映元认知对用户自我模型的重要性

### Requirement: 遗忘机制
The system SHALL provide a ForgettingEngine that archives low-importance memories based on a tick-based candidate selection.
- 默认重要性阈值:`importance_threshold = 0.3`
- 支持 `dry_run` 模式(仅列出候选,不实际归档)
- 归档后可触发 BlackholeEngine 压缩闭环(通过 `with_blackhole` 注入)
- 候选输出 `ForgettingCandidate`(id / layer / importance / last_access / ttl_days / reason)

#### Scenario: 低重要性记忆被遗忘
- **WHEN** ForgettingEngine tick 执行,发现一条重要性 0.2 的 L3 记忆
- **THEN** 该记忆被标记为遗忘候选并归档
- **AND** 若注入了 BlackholeEngine,归档后触发压缩 pass
- **AND** `dry_run = true` 时仅输出候选列表,不实际归档

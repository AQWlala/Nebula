# Delta for Memory

> **变更**: v2.0.0-feature-completion
> **领域**: memory
> **说明**: 历史回填型 delta，记录 v2.0.0 功能补齐对记忆系统的变更

## ADDED Requirements

### Requirement: 记忆引擎编排器
The system SHALL orchestrate memory processing through a MemoryOrchestrator that coordinates multiple specialized engines.
- 引擎包括: BlackholeEngine（压缩）、SpongeEngine（吸收）、ForgettingEngine（遗忘）、ImportanceEngine（重要性）、MocEngine（MOC 整理）、ReflectEngine（反思）
- 编排顺序: Sponge 吸收 → Importance 评估 → Blackhole 压缩 → Moc 整理 → Reflect 反思 → Forgetting 清理

#### Scenario: 新记忆写入触发编排
- **WHEN** 一条新记忆写入 L0
- **THEN** MemoryOrchestrator 按 SOP 顺序调度各引擎
- **AND** 每个引擎的处理结果记录到 provenance

### Requirement: 黑洞压缩引擎
The system SHALL compress L3 facts into semantic capsules using the BlackholeEngine.
- 压缩触发条件: L3 容量超过阈值
- 压缩输出: 语义胶囊（结构化压缩表示）
- 压缩是无损的——原始事实可通过版本控制回溯

#### Scenario: L3 容量超限触发压缩
- **WHEN** L3 存储容量超过配置阈值
- **THEN** BlackholeEngine 启动压缩流程
- **AND** 生成语义胶囊
- **AND** 原始事实标记为"已压缩"但保留可回溯

### Requirement: 记忆 ACL 权限控制
The system SHALL enforce access control on memory items based on user-defined ACL rules.
- 每条记忆携带 ACL 元数据（migration 013）
- 支持的权限级别: private / shared / public
- 默认权限: private

#### Scenario: 代理尝试读取 private 记忆
- **WHEN** 一个非主人代理尝试读取 private 级别的记忆
- **THEN** 系统拒绝访问并记录审计日志
- **AND** 返回空结果（不泄露记忆存在性）

### Requirement: 隐私守卫
The system SHALL redact sensitive data in memories before they are exposed to LLM context or external services.
- 敏感数据类型: 密钥、令牌、个人信息（PII）、财务数据
- 脱敏方式: 占位符替换（可逆映射存储在加密的映射表中）
- 触发时机: 记忆写入时检测、检索结果输出前脱敏

#### Scenario: 记忆包含 API 密钥
- **WHEN** 用户对话中包含 API 密钥并被写入记忆
- **THEN** PrivacyGuard 检测到密钥模式
- **AND** 将密钥替换为占位符 `[REDACTED:api_key]`
- **AND** 原始密钥存储在加密映射表中，仅用户可解锁

### Requirement: 记忆版本控制
The system SHALL maintain version history for each memory item with full provenance tracking.
- 每次修改创建新版本（migration 016）
- provenance 记录: 修改者（用户/代理 ID）、修改时间、修改原因、变更 diff
- 支持回滚到任意历史版本

#### Scenario: 用户手动编辑记忆
- **WHEN** 用户通过 Memory Inspector 编辑一条记忆
- **THEN** 系统创建新版本，保留旧版本
- **AND** provenance 记录修改者为用户 ID
- **AND** 变更 diff 存入版本历史

#### Scenario: 回滚记忆到历史版本
- **WHEN** 用户选择回滚到某个历史版本
- **THEN** 系统将该版本恢复为当前版本
- **AND** 回滚操作本身也记录为一个新版本
- **AND** 历史版本链不被破坏

### Requirement: 混合检索
The system SHALL provide hybrid search combining BM25 keyword matching and vector similarity search.
- BM25（bm25.rs）: 关键词精确匹配
- 向量检索: 语义相似度（支持 chroma/lance/qdrant 三后端）
- 融合策略: 加权分数融合（权重可配置）
- 图谱检索（graph_search.rs）: 因果关系遍历

#### Scenario: 用户搜索记忆
- **WHEN** 用户输入查询关键词
- **THEN** 系统并行执行 BM25 和向量检索
- **AND** 融合两路结果并按相关性排序
- **AND** 返回 top-K 结果（K 可配置）

### Requirement: 记忆图谱
The system SHALL maintain a causal graph and MDRM graph for memory relationships.
- 因果图（causal_graph.rs）: 记忆间的因果关系
- MDRM 图（mdrm_graph.rs）: 多维关系映射
- 一致性检查（consistency.rs）: 检测图谱矛盾

#### Scenario: 新记忆与已有记忆存在因果关系
- **WHEN** 写入的新记忆引用了已有记忆
- **THEN** 系统在因果图中建立边
- **AND** 一致性检查器验证无矛盾
- **AND** 若检测到矛盾，标记为待人工审核

## MODIFIED Requirements

### Requirement: 记忆层级
The system MUST support L0-L5 memory layers with clear lifecycle management. (Previously: L0-L5 layers defined but without engine orchestration)
- L0: 原始上下文（短期）— l0_cache.rs 快速存取
- L1: 对话摘要（中期）— summarizer.rs 自动压缩
- L2: 知识抽取（长期）— entity_extractor.rs 实体识别
- L3: 事实记忆（持久）— sqlite_store.rs + lance_store.rs
- L4: 价值观记忆（宪法）— constitutional.rs
- L5: 反思记忆（元认知）— reflect.rs + self_reflection.rs
- 层级间转换由 MemoryOrchestrator 编排

#### Scenario: L0→L1 自动压缩
- **WHEN** 对话超过 20 轮
- **THEN** SummarizerEngine 自动生成 L1 摘要
- **AND** 清理 L0 中的已压缩内容
- **AND** provenance 记录压缩操作

### Requirement: 记忆存储
The system SHALL store memories in encrypted SQLite (cipher) plus Lance vector store, with L0 cache for hot context. (Previously: SQLite only)
- SQLite Cipher（sqlite_cipher.rs）: 加密的关系存储
- Lance（lance_store.rs）: 向量存储
- L0 Cache（l0_cache.rs）: 热点上下文内存缓存
- 迁移管理（migration.rs）: schema 版本演进

#### Scenario: 记忆写入存储链
- **WHEN** 一条新记忆写入
- **THEN** 系统将其存入 SQLite Cipher（结构化数据）
- **AND** 生成 embedding 存入 Lance（向量数据）
- **AND** 若为热点上下文，同时缓存到 L0 Cache

## REMOVED Requirements

(none)

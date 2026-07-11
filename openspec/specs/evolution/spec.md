# 进化系统 行为契约

> **领域**: evolution
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

进化系统(Evolution)是 Nebula 的闭环自我进化引擎,由 EvolutionEngine 驱动三层共存进化:基因突变器(GeneMutator)、提示变异器(PromptMutator)与技能进化器(SkillEvolver)。系统通过 Cron 调度定时运行,收集目标信号(胜率)作为进化方向,默认关闭,需用户在 Settings 中显式启用。

## Requirements

### Requirement: EvolutionEngine 三层共存
The system SHALL run a three-layer evolution engine: gene mutation, prompt mutation, and skill evolution.
- `EvolutionEngine` 4-Phase 进化管线:L1 → L2 → L3 → L5 → SOUL.md
- 三层共存:
  1. 基因突变器(`GeneMutator`)— 行为模式编码为基因,通过变异/交叉/选择自进化
  2. 提示变异器(`PromptMutator`)— LLM prompt 自我变异 + 快照 + 回滚
  3. 技能进化器(`SkillEvolver`)— 技能自动归档/变异/进化
- Feature 门控:`self-evolution` feature(默认 off),`evolution-engine` feature 隐含 `self-evolution`
- 进化产出写入 `EvolutionLog`,支持回滚(`rollback.rs`)

#### Scenario: 进化管线执行
- **WHEN** EvolutionEngine 触发一次进化 pass
- **THEN** 执行 4-Phase 管线:L1 对话 → L2 经验 → L3 事实 → L5 反思 → SOUL.md 更新
- **AND** 每阶段产出记录到 EvolutionLog
- **AND** 失败时可回滚至上一快照

### Requirement: 主开关与安全约束
The system SHALL default the evolution master switch to OFF, requiring explicit user opt-in.
- `EVOLUTION_ENABLED` 静态原子布尔,默认 `false`
- `evolution_enabled()` 查询开关状态,所有 mutator/evolver 执行前必须先检查
- 用户通过 Settings UI 翻转开关
- 即便 feature 编译启用,运行时默认不改变任何行为
- `EvolutionConfig`:enabled / archive_rate_floor / archive_min_usage / background_period_secs / prompt_mutator_window / goal_confidence_threshold

#### Scenario: 默认关闭不改变行为
- **WHEN** 用户升级到包含 evolution 模块的版本但未开启开关
- **THEN** `evolution_enabled()` 返回 false
- **AND** 所有 mutator/evolver 跳过执行,运行时行为与无 evolution 模块一致

#### Scenario: 用户显式启用
- **WHEN** 用户在 Settings 中开启"自我进化"
- **THEN** `set_evolution_enabled(true)` 设置开关
- **AND** 后台 worker 开始按 `background_period_secs` 周期运行

### Requirement: 基因突变器
The system SHALL encode behavioral patterns as genes and evolve them via mutation, crossover, and selection.
- `GeneMutator` 将行为模式编码为基因(可序列化结构)
- 变异:对单个基因施加随机扰动
- 交叉:两个父代基因重组产生子代
- 选择:根据适应度(目标信号)筛选保留优秀基因
- 基因库持久化到 SQLite

#### Scenario: 基因变异与选择
- **WHEN** GeneMutator 执行一次进化迭代
- **THEN** 从基因库中选择父代,施加变异/交叉产生子代
- **AND** 根据适应度(目标信号)筛选,保留高适应度基因
- **AND** 低适应度基因被淘汰

### Requirement: 提示变异器
The system SHALL self-mutate LLM prompts with snapshot storage and rollback support.
- `PromptSelfMutator` 对每个 agent 的 prompt 施加变异
- 快照存储:每次变异前保存快照(`snapshot_id`),支持回滚
- 变异窗口:从最近 `prompt_mutator_window`(默认 30)条 outcome 中学习
- 目标信号:confidence ≥ `goal_confidence_threshold`(默认 0.7)视为"获胜"
- 覆盖 agent:coder / writer / reviewer / researcher / planner
- `SqlitePromptSelfMutator` 为 SQLite 后端实现

#### Scenario: Prompt 变异与回滚
- **WHEN** PromptMutator 对 coder agent 的 prompt 施加变异
- **THEN** 保存变异前快照(`snapshot_id`)
- **AND** 变异后 prompt 在后续任务中测试
- **AND** 若性能下降,可回滚至 `snapshot_id`

### Requirement: 技能进化器
The system SHALL auto-archive, mutate, and evolve skills based on usage and rating signals.
- `SkillAutoEvolver` + `EvolutionPolicy` + `SkillArchive`
- 归档条件:`success_count >= usage_count * (1.0 - archive_rate_floor)` 且 `usage_count >= archive_min_usage`
- 默认阈值:`archive_rate_floor = 0.5`,`archive_min_usage = 20`
- 低效技能被归档,高效技能被变异优化
- `SkillArchive` 记录归档历史

#### Scenario: 低效技能归档
- **WHEN** 一个技能被使用 25 次,成功率 30%(低于 `archive_rate_floor = 0.5`)
- **THEN** `SkillAutoEvolver` 将其标记为归档候选
- **AND** 技能移入 `SkillArchive`,不再默认推荐

### Requirement: Cron 调度
The system SHALL schedule evolution tasks via a cron-based scheduler with three timing mechanisms.
- 三计时机制(统一由 `TimerEngine` 管理):
  1. Cron 调度(`CronScheduler`)— 03:00 合并 / 12:00 自检 / 21:00 回顾
  2. 事件触发(Event)— 基于系统事件即时执行
  3. 轮询(Poll)— 周期性检查条件
- `CronEngine` 通用 cron 调度:任务注册 + 执行追踪 + 失败重试
- `CronExpr` 5 字段 cron 表达式解析器(minute hour day month weekday)
- `AutomationTemplates` 预定义自动化场景模板
- Feature 门控:`self-evolution`(CronScheduler 依赖)

#### Scenario: 定时合并任务
- **WHEN** 系统时间到 03:00
- **THEN** CronScheduler 触发合并任务(记忆合并 / 压缩)
- **AND** 执行结果记录到 EvolutionLog

### Requirement: 目标信号收集
The system SHALL collect goal signals (win rate) from task outcomes to guide evolution direction.
- `goal_signal` 模块从 `OutcomeLedger` 收集任务结果
- 目标函数:胜率 = 胜任任务数 / 总任务数
- `confidence ≥ goal_confidence_threshold`(默认 0.7)视为"获胜"
- `OutcomeCollectors` 从 skills / swarm / chat 自动发射 outcome
- `TaskOutcome` DTO + `OutcomeLedger` + SQLite 后端
- 后台 worker 按 `background_period_secs`(默认 3600 秒)周期运行

#### Scenario: 胜率计算与反馈
- **WHEN** 后台 worker 周期触发目标信号推导
- **THEN** 从 OutcomeLedger 收集近期任务结果
- **AND** 计算各 agent 的胜率
- **AND** 胜率作为适应度反馈给 GeneMutator / PromptMutator

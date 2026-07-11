# 蜂群协作 行为契约

> **领域**: swarm
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

蜂群系统(Swarm)实现多 Agent 并行协作,由 MasterOrchestrator 编排顶层任务,通过 TaskDag(petgraph)管理子任务依赖,支持领导者轮值选举、CRDT 同步与死锁检测。Worker 类型包含 coder/writer/reviewer/researcher/planner,以及 v2.0 引入的通用 GenericAgent。

## Requirements

### Requirement: MasterOrchestrator 编排
The system SHALL orchestrate top-level tasks via MasterOrchestrator, which decomposes tasks into a DAG and executes sub-tasks by topological layer.
- MasterOrchestrator 持有 `Arc<SwarmOrchestrator>`,不直接持有 Worker 池
- 任务拆解:顶层任务 → DAG 节点(SubTask)→ 按拓扑层执行
- 结果综合:收集各 SubTask 结果,由 MasterAgent 综合最终输出
- BypassMode:通过 `ExecuteMode` 参数传入 `execute_with_mode`
- 复用 SwarmOrchestrator 全部子系统(RAG / Leader / Negotiator / CRDT)
- Feature 门控:`master-orchestrator` feature(默认 off)

#### Scenario: 任务拆解与拓扑执行
- **WHEN** MasterOrchestrator 接收一个顶层任务
- **THEN** 调用 LLM(WorkType = MasterDecompose)拆解为 SubTask DAG
- **AND** 按拓扑排序逐层执行,前驱完成后后驱才启动
- **AND** 收集所有 SubTask 结果后综合最终输出

### Requirement: TaskDag 依赖管理
The system SHALL manage sub-task dependencies using a petgraph DiGraph with topological sort and cycle detection.
- `TaskDag` 包装 `petgraph::graph::DiGraph`,节点为 `SubTask`,边为 `DependencyEdge`
- 拓扑排序:`petgraph::algo::toposort` 确定执行顺序
- 循环检测:`petgraph::algo::is_cyclic_directed` 在添加边时检测,有环则拒绝
- SubTask 携带 `work_type_hint` 字段,供路由层选择模型
- 失败策略:`FailureStrategy`(Retry / Skip / Fail / Manual)
- 结果收集:`SubTaskResultMap` 防 placeholder 注入
- Worker 能力枚举 `WorkerCapability`(Summarize/WriteShort/WriteLong/Search/Generate/CodeExecute/FileOperate/MediaProcess)

#### Scenario: 循环依赖检测
- **WHEN** 添加一条边后 DAG 出现环
- **THEN** `is_cyclic_directed` 返回 true,该边被拒绝
- **AND** 返回错误提示"检测到循环依赖"

#### Scenario: 拓扑层执行
- **WHEN** DAG 包含 A → B → C 的依赖链
- **THEN** A 先执行,A 完成后 B 启动,B 完成后 C 启动
- **AND** 无依赖的节点可并行执行

### Requirement: Worker 类型
The system SHALL support typed worker agents: coder, writer, reviewer, researcher, planner, and generic.
- v2.0 起,默认生成 2-6 个 `GenericAgent` 并行工作(jiuwenswarm 风格)
- 角色化 Agent(Coder/Writer/Reviewer/Researcher/Planner)v2.2 重新启用,含独立 tool_set/knowledge_scope
- `agent_count` clamp 到 `2..=6`
- 所有 Agent 并发执行(`futures::future::join_all`)
- 单个 Agent 失败不中止整个 run,标记为 errored 后继续
- 每个 Agent 重试带指数退避

#### Scenario: 通用 Agent 并行
- **WHEN** 提交一个 SwarmTask,`agent_count = 4`
- **THEN** 生成 4 个 GenericAgent 并行处理同一任务
- **AND** 单个 Agent 失败不中止其余 Agent
- **AND** 结果汇总后返回

#### Scenario: 显式角色分工
- **WHEN** SwarmTask 的 `agents` 字段指定 `["coder", "reviewer"]`
- **THEN** 生成对应角色的 Agent,各自携带独立 tool_set
- **AND** 角色化 Agent 优先于 `agent_count` 默认池

### Requirement: 领导者选举
The system SHALL elect a leader agent per task using a weighted random rotation algorithm (not Raft consensus).
- 评分公式:`score = capability_score * 0.5 + history_success_rate * 0.3 + (1 - current_load) * 0.2`
- `capability_score` [0,1],默认 0.5
- `history_success_rate = successful_tasks / total_tasks`
- `current_load` [0,1],0=空闲,1=满载
- 按分数加权随机选出 Leader
- Leader 职责:最终决策、触发协商(输出冲突时)、协商阶段享有更高权重

#### Scenario: 高能力 Agent 当选
- **WHEN** Agent A 能力分 0.9、成功率 0.8、负载 0.2;Agent B 能力分 0.5、成功率 0.5、负载 0.5
- **THEN** Agent A 的综合分数高于 Agent B
- **AND** 加权随机选举中 Agent A 有更高概率当选 Leader

### Requirement: CRDT 同步
The system SHALL synchronize memory changes across agents via SwarmCrdtSync using LWW (Last-Writer-Wins) merge semantics.
- 复用 `crate::sync::crdt::CrdtEngine` 的 LWW 合并语义
- 本地副本:`memory_id -> CrdtVersion` 映射
- Agent 修改记忆时通过 `AgentBus` 广播 `CrdtSync` 消息
- 接收方调用 `merge_remote` 进行 LWW 合并
- ACL 强制过滤:`merge_remote` 写入前检查 Write 权限,`get_memory`/`list_memories` 读取前检查 Read 权限
- 整版本级合并(`merge_lww`),字段级合并(`merge_fields`)可选

#### Scenario: 跨 Agent 记忆同步
- **WHEN** Agent A 修改了一条记忆
- **THEN** 通过 AgentBus 广播 CrdtSync 消息
- **AND** Agent B 接收后检查 ACL Write 权限,通过后 LWW 合并
- **AND** 无 Write 权限的 Agent 跳过合并

### Requirement: 死锁检测
The system SHALL detect deadlocks among agents using a Wait-For Graph.
- `WaitForGraph` 维护 `waiter -> {wait_for...}` 边集
- 添加/移除等待关系:`add_wait` / `remove_wait`
- 检测环:图中出现环则判定死锁
- 检测到死锁后触发恢复策略(终止一个或多个等待方)

#### Scenario: 死锁检测与恢复
- **WHEN** Agent A 等待 Agent B,Agent B 等待 Agent A(环)
- **THEN** WaitForGraph 检测到环,判定死锁
- **AND** 触发恢复策略,终止其中一个等待方以打破环

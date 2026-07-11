# Delta for Swarm

> **变更**: v2.0.0-feature-completion
> **领域**: swarm
> **说明**: 历史回填型 delta，记录 v2.0.0 功能补齐对蜂群系统的变更

## ADDED Requirements

### Requirement: Master-Orchestrator 编排
The system SHALL coordinate swarm agents through a MasterOrchestrator that decomposes user intent into a DAG of agent tasks.
- Master（master.rs）: 接收用户意图，分解为任务图
- Orchestrator（orchestrator.rs）: 调度任务图执行
- Composer（composer.rs）: 合并多代理输出
- Negotiator（negotiator.rs）: 代理间冲突协商

#### Scenario: 用户提交复杂请求
- **WHEN** 用户提交一个需要多代理协作的请求
- **THEN** MasterOrchestrator 将请求分解为 DAG 任务图
- **AND** 为每个任务分配合适的代理角色
- **AND** Orchestrator 按 DAG 依赖顺序调度执行

#### Scenario: 代理间输出冲突
- **WHEN** 两个代理对同一任务产生冲突的输出
- **THEN** Negotiator 启动协商流程
- **AND** 根据代理置信度和角色权重裁定
- **AND** 裁定结果记录到审计日志

### Requirement: 代理角色系统
The system SHALL support specialized agent roles: Planner, Coder, Researcher, Reviewer, Writer, and configurable Generic agents.
- PlannerAgent（planner.rs）: 任务分解与规划
- CoderAgent（coder.rs）: 代码实现
- ResearcherAgent（researcher.rs）: 信息检索与分析
- ReviewerAgent（reviewer.rs）: 代码/内容审查
- WriterAgent（writer.rs）: 内容创作
- GenericAgent（generic_agent.rs）: 可配置角色（通过 scenario_profiles.rs）
- 8 人格系统（personality/mod.rs）: 每个代理可绑定人格

#### Scenario: Planner 分解任务
- **WHEN** Master 将一个复杂请求交给 PlannerAgent
- **THEN** Planner 生成任务分解方案
- **AND** 为每个子任务推荐代理角色
- **AND** 输出 DAG 结构供 Orchestrator 执行

#### Scenario: Coder 与 Reviewer 协作
- **WHEN** CoderAgent 完成代码实现
- **THEN** Orchestrator 自动调度 ReviewerAgent 审查
- **AND** 若审查未通过，回流给 Coder 修改
- **AND** 循环直到审查通过或达到最大迭代数

### Requirement: AgentBus 代理总线
The system SHALL provide an AgentBus for inter-agent communication with event streaming support.
- AgentBus（agent_bus.rs + bus.rs）: 代理间消息传递
- EventBus（event_bus.rs）: 事件发布订阅
- EventStream（event_stream.rs）: 协议化事件流
- Context（context.rs）: 共享上下文
- ContextPool（context_pool.rs）: 上下文池管理

#### Scenario: 代理间传递上下文
- **WHEN** 一个代理完成任务并需要传递结果给下游代理
- **THEN** 通过 AgentBus 发送消息
- **AND** 消息携带 Context 引用
- **AND** 下游代理从 ContextPool 获取完整上下文

#### Scenario: 事件流订阅
- **WHEN** 一个代理需要监听特定类型的事件
- **THEN** 通过 EventBus 订阅事件
- **AND** 事件以协议化格式（EventStream）推送
- **AND** 订阅者异步处理不阻塞发布者

### Requirement: DAG 任务图
The system SHALL manage task dependencies through a DAG (Directed Acyclic Graph) with deadlock detection.
- DAG（dag.rs）: 任务依赖图
- Deadlock（deadlock.rs）: 死锁检测与解除
- 任务状态: pending / running / completed / failed / cancelled

#### Scenario: DAG 中存在循环依赖
- **WHEN** 任务图构建时检测到循环依赖
- **THEN** 系统拒绝构建并报错
- **AND** 提示循环路径供用户修正

#### Scenario: 运行时检测到死锁
- **WHEN** 多个代理互相等待对方释放资源
- **THEN** Deadlock 检测器识别死锁
- **AND** 根据优先级牺牲最低优先级代理
- **AND** 被牺牲代理的任务回流到队列

### Requirement: 运行时画布
The system SHALL provide a runtime canvas for visualizing agent execution with replay and CRDT sync support.
- RuntimeCanvas（runtime_canvas.rs）: 执行可视化
- CanvasInteraction（canvas_interaction.rs）: 用户操控画布
- ExecutionReplay（execution_replay.rs）: 历史回放
- CRDTSync（crdt_sync.rs）: 多代理画布同步

#### Scenario: 用户查看执行过程
- **WHEN** 蜂群正在执行任务
- **THEN** RuntimeCanvas 实时渲染代理状态和任务进度
- **AND** 用户可通过 CanvasInteraction 干预（暂停/取消/重定向）

#### Scenario: 回放历史执行
- **WHEN** 用户选择回放某次蜂群执行
- **THEN** ExecutionReplay 按时间轴重现执行过程
- **AND** 支持快进/后退/定点跳转

### Requirement: Loop Engineering
The system SHALL support declarative loop definitions with phase rings, budgets, and audit logging.
- LoopDef（loop_def.rs）: 循环结构声明
- LoopDesign（loop_design.rs）: 循环编排
- LoopPhaseRing（loop_phase_ring.rs）: 阶段环管理
- LoopBudget（loop_budget.rs）: 成本/迭代限制
- LoopAuditLog（loop_audit_log.rs + migration 038）: 审计日志
- ToolLoop（tool_loop.rs）: 工具调用循环

#### Scenario: 循环达到预算上限
- **WHEN** 一个 Loop 的累计成本超过 LoopBudget 设定上限
- **THEN** 系统强制终止循环
- **AND** 记录终止原因到 LoopAuditLog
- **AND** 通知用户并请求授权继续或停止

#### Scenario: 循环阶段转换
- **WHEN** 一个 Loop 完成当前阶段的所有任务
- **THEN** LoopPhaseRing 推进到下一阶段
- **AND** 阶段转换记录到审计日志
- **AND** 若无下一阶段，标记循环完成

### Requirement: 领选者选举
The system SHALL elect a leader agent for coordination when no explicit Master is designated.
- LeaderElector（leader_elector.rs）: 领选者选举
- 选举策略: 基于角色权重 + 负载 + 历史成功率
- 选举触发: Master 不可用时自动触发

#### Scenario: Master 不可用时选举新领选者
- **WHEN** 当前 Master 代理失去响应超过阈值时间
- **THEN** LeaderElector 启动选举
- **AND** 基于权重选出新领选者
- **AND** 新领选者接管未完成的任务调度

## MODIFIED Requirements

### Requirement: 蜂群协作
The system SHALL enable multiple agents to collaborate on complex tasks through orchestrated DAG execution with full auditability. (Previously: basic multi-agent without DAG orchestration)
- 协作模式: Master 编排 + 角色分工 + DAG 依赖
- 通信: AgentBus 消息传递 + EventBus 事件流
- 可视化: RuntimeCanvas 实时画布
- 可审计: ExecutionReplay 回放 + LoopAuditLog 审计

#### Scenario: 多代理协作完成代码任务
- **WHEN** 用户请求"实现一个功能并审查"
- **THEN** Master 分解为: Planner 规划 → Coder 实现 → Reviewer 审查
- **AND** DAG 按依赖顺序执行
- **AND** 全程通过 RuntimeCanvas 可视化
- **AND** 执行过程可通过 ExecutionReplay 回放

## REMOVED Requirements

(none)

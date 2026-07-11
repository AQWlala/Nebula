# Delta: swarm

> **change**: add-agent-eval
> **领域**: swarm
> **状态**: draft
> **创建**: 2026-07-11

## MODIFIED Requirements

### Requirement: MasterOrchestrator 编排
The system SHALL orchestrate top-level tasks via MasterOrchestrator, which decomposes tasks into a DAG and executes sub-tasks by topological layer.

**变更说明**: Master 任务执行时可选触发 Trace 收集和评估。

- `execute_loop()` 添加 TraceCollector 钩子
- 钩子通过 `NEBULA_EVAL_TRACING=1` 环境变量启用,默认关闭(零开销)
- 启用时,Master 拆解生成 MasterDecompose span,结果综合生成 MasterSynthesize span
- 每个 SubTask 的 Worker 执行生成 SwarmWorker span,parent_id 指向 Master span

#### Scenario: Trace 钩子默认关闭
- **WHEN** 未设置 `NEBULA_EVAL_TRACING` 环境变量
- **THEN** execute_loop() 不触发 Trace 收集
- **AND** 性能与无评估模块时一致

#### Scenario: 启用 Trace 后记录 Master span
- **WHEN** 设置 `NEBULA_EVAL_TRACING=1` 并执行 Master 任务
- **THEN** execute_loop() 开头创建 MasterDecompose span
- **AND** 每个 SubTask 创建 SwarmWorker span,parent_id = Master span id
- **AND** 结果综合时创建 MasterSynthesize span

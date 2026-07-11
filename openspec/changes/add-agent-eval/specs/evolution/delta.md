# Delta: evolution

> **change**: add-agent-eval
> **领域**: evolution
> **状态**: draft
> **创建**: 2026-07-11

## MODIFIED Requirements

### Requirement: EvolutionEngine 三层共存
The system SHALL run a three-layer evolution engine: gene mutation, prompt mutation, and skill evolution.

**变更说明**: 进化管线接入评估器反馈,评估分数作为变异信号。

- 进化 pass 完成后,自动运行评测集(EvalRunner)
- 评估分数与基线对比:
  - 分数 ≥ 基线 → 变异方向正确,更新基线
  - 分数 < 基线 → 变异方向错误,回滚到上一快照
- 最小改进阈值(默认 0.05),避免噪声导致回滚
- 评估结果写入 EvolutionLog,但不写入 L0-L7 记忆层(避免污染)

#### Scenario: 评估分数提升时保留变异
- **WHEN** 进化 pass 完成后运行评测集
- **AND** 评估分数 > 基线 + 最小改进阈值
- **THEN** 保留变异结果
- **AND** 更新基线分数
- **AND** 记录到 EvolutionLog

#### Scenario: 评估分数下降时回滚
- **WHEN** 进化 pass 完成后运行评测集
- **AND** 评估分数 < 基线
- **THEN** 回滚到上一快照
- **AND** 记录回滚原因到 EvolutionLog
- **AND** 不更新基线

#### Scenario: 评估结果不污染记忆层
- **WHEN** 评估器完成一次评测
- **THEN** 评估分数和评分卡只写入 EvolutionLog + eval_runs 表
- **AND** 不写入 L0-L7 记忆层
- **AND** 不影响后续任务的上下文注入

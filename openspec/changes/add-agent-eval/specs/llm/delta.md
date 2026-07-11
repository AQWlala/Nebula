# Delta: llm

> **change**: add-agent-eval
> **领域**: llm
> **状态**: draft
> **创建**: 2026-07-11

## MODIFIED Requirements

### Requirement: 统一模型调度
The system SHALL dispatch all LLM calls through the UnifiedModelDispatcher (ADR-003).

**变更说明**: 新增 `WorkType::EvalJudge` 工作类型,用于 LLM-as-Judge 评估器。

- 新增 `WorkType::EvalJudge` — LLM 评估器调用
- `is_local_only()` 强制约束:EvalJudge 强制走本地 Ollama(与 Evolution / SoulCompile / Classifier 一致)
- 评估器调用经 `dispatch(WorkType::EvalJudge, ...)` 统一路由
- 评估器调用的 cost 追踪独立分类(eval 类别)

#### Scenario: EvalJudge 工作类型路由
- **WHEN** JudgeEngine 发起 LLM 调用(WorkType = EvalJudge)
- **THEN** Dispatcher 根据 `models.json` 的 `work_type_overrides` 路由
- **AND** `is_local_only()` 返回 true,强制路由到本地 Ollama
- **AND** 即使用户配置了非本地 override,该 override 被忽略

#### Scenario: EvalJudge 成本独立追踪
- **WHEN** 评估器完成一次 LLM 调用
- **THEN** 成本记录到 `cost_tracker` 的 eval 类别
- **AND** 不与 Chat / SwarmWorker / Evolution 成本混淆

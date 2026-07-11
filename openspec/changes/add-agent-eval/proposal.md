---
change: add-agent-eval
domain: evaluation
description: Agent 输出质量评估体系(Trace 导出 + 评测集 + LLM-as-Judge + 节点隔离测试)
status: draft
created: 2026-07-11
---

# Proposal: add-agent-eval

> **领域**: evaluation
> **一句话描述**: Agent 输出质量评估体系(Trace 导出 + 评测集 + LLM-as-Judge + 节点隔离测试)
> **状态**: draft

## 为什么 (Why)

### 当前问题

Nebula 已建成完整的 Agent 协作系统(Master / Swarm / Evolution / Skills),但 **缺少 Agent 输出质量的系统化评估手段**。当前仅有代码级测试(`cargo test` 2576+ 用例)验证 API 契约正确性,无法回答:

```
1. Master 拆解的任务 DAG 质量如何? 是否漏了关键子任务?
2. 蜂群 Worker 的回答是否真的解决了用户问题?
3. PromptMutator 变异后的 prompt 比上一版"更好"还是"更差"?
4. SkillEvolver 进化出的新技能是否真的可用?
5. L5 反思记忆是否真正提升了下一次任务的表现?
```

引用相关 spec:

- `specs/llm/spec.md` "统一模型调度" — 网关只负责路由,不评估输出质量
- `specs/evolution/spec.md` "进化引擎三层共存" — GeneMutator/PromptMutator/SkillEvolver 用"胜率"作为信号,但胜率采集依赖人工标注或代理指标,没有 LLM-as-Judge 闭环
- `specs/swarm/spec.md` "MasterOrchestrator 编排" — 任务拆解后只检查 SubTask 是否完成,不评估 SubTask 输出质量

参考《AI Agent 要如何评估》文章的核心观点:

> "Agent 的输出质量很多时候是主观的,不能非黑即白地评估。需要用 LLM 即评委 + 二元评判法 + Trace 追踪来量化。"

### 不解决的后果

若不引入评估体系:

- **进化系统无法收敛** — PromptMutator 和 GeneMutator 缺少量化反馈,变异方向是随机的,可能"越进化越差"
- **Master 拆解质量不可追溯** — 任务 DAG 失败时无法定位是"拆解错误"还是"Worker 执行错误"
- **竞品差距拉大** — OpenSpec / LangSmith / Langfuse / Braintrust 已有完整 Trace + Eval 工具链,Nebula 在评估维度完全空白
- **v2.4.0"质量提升"主题无法验证** — 没有基线就没有改进依据
- **用户信任成本高** — 用户无法判断 Agent 回答是否可信,只能盲目接受或全盘否定

### 为什么现在

- v2.3.0 macOS 风格重设计已稳定,CI 全绿,可以聚焦"质量"而非"功能"
- EvolutionEngine 在 v2.3.0 已落地三层架构,但缺少反馈闭环,是引入评估器的窗口
- ROADMAP v3.1 的 131 个 T-E-* 功能任务已完成,后续迭代需要"质量提升"主题
- 学习 OpenSpec 项目的 spec-driven 方法论后,本 change 是第一个使用 Delta Specs 描述增量的实践

## 做什么 (What)

### 变更摘要

新增 **Agent 评估体系**,包含四个子系统:

1. **Trace 导出与分析** — 记录 Master/Swarm/Evolution 每一步中间输出,导出为 JSONL 供离线分析,UI 可查看 Trace 时间线
2. **评测集管理 (EvalSet)** — 10-20 个代表性样本(对话/任务拆解/技能执行),衡量修改是否有效,支持基线对比
3. **LLM-as-Judge 评估器** — 用本地 Ollama 作为评委,按二元评判法(满足/不满足标准,+1/-1)打分,输出结构化评分卡
4. **节点隔离测试** — 针对单个 Agent 节点(Master/Reviewer/Worker)隔离运行评测集,节省 LLM 调用成本

评估器**强制走本地 Ollama**(WorkType = EvalJudge),遵循数据主权红线:用户对话内容不外发给远端 LLM 评判。

### 受影响的 Requirement

| Requirement | 变更类型 | 说明 |
|------------|---------|------|
| Trace 导出 | ADDED | 新增 Trace 导出 requirement |
| 评测集管理 | ADDED | 新增 EvalSet requirement |
| LLM-as-Judge 评估器 | ADDED | 新增 LLM 评委 requirement |
| 节点隔离测试 | ADDED | 新增节点隔离测试 requirement |
| 统一模型调度 | MODIFIED | 新增 WorkType::EvalJudge,强制本地 Ollama |
| EvolutionEngine 三层共存 | MODIFIED | 进化管线接入评估器反馈,胜率信号改为评估器打分 |
| MasterOrchestrator 编排 | MODIFIED | 任务完成后可选触发评估器,记录 Trace |

### 不做什么 (Out of Scope)

本 change 不涉及:

- **在线 A/B 测试框架** — 评估器是离线/批处理模式,不接入实时流量分流(由 `llm/arena` 领域独立处理)
- **人工标注平台** — 不建标注 UI,人工标注通过 YAML 文件管理
- **评估结果的记忆层沉淀** — 评估分数不写入 L0-L7 记忆层(避免污染记忆),只写入 EvolutionLog
- **第三方评估工具集成** — 不集成 LangSmith / Langfuse / Braintrust,使用自建工具
- **前端评估仪表盘** — 本期只做命令行 + JSONL 输出,UI 展示留待后续 change

## 影响评估

### 受影响的文件

```
新增:
- src-tauri/src/eval/mod.rs              — 评估模块入口
- src-tauri/src/eval/trace.rs            — Trace 导出器
- src-tauri/src/eval/evalset.rs          — 评测集管理
- src-tauri/src/eval/judge.rs            — LLM-as-Judge 评估器
- src-tauri/src/eval/isolation.rs        — 节点隔离测试
- src-tauri/src/eval/scrub.rs            — PII 脱敏(评估前)
- src/migrations/0xxx_eval_traces.sql    — Trace 表迁移
- src/migrations/0xxx_eval_runs.sql      — 评估运行记录表
- src/migrations/0xxx_evalsets.sql       — 评测集表迁移
- src-tauri/tests/eval_*.rs              — 评估模块测试
- evalsets/default.yaml                  — 默认评测集(10 样本)
- evalsets/regression.yaml               — 回归评测集(5 样本)

修改:
- src-tauri/src/llm/dispatcher.rs        — 新增 WorkType::EvalJudge
- src-tauri/src/llm/mod.rs               — WorkType 枚举扩展
- src-tauri/src/swarm/master.rs          — Trace 钩子
- src-tauri/src/swarm/orchestrator.rs    — Trace 钩子
- src-tauri/src/evolution/engine.rs      — 接入评估器反馈
- src-tauri/src/evolution/prompt_mutator.rs — 评估器作为变异信号
- src-tauri/src/lib.rs                   — 注册 eval 模块
- src-tauri/Cargo.toml                   — 新增 feature: eval
- src-tauri/src/features.rs              — feature 定义
- openspec/specs/llm/spec.md             — Delta: 新增 EvalJudge
- openspec/specs/evolution/spec.md       — Delta: 接入评估器
- openspec/specs/swarm/spec.md           — Delta: Trace 钩子
```

### 受影响的领域

```
- evaluation/ (主要) — 全新领域,4 个新 requirement
- llm/ (次要) — 新增 WorkType::EvalJudge
- evolution/ (次要) — 进化管线接入评估器反馈
- swarm/ (次要) — Master/Swarm 添加 Trace 钩子
```

## 验收标准

- [ ] Delta spec 中的所有 scenario 有对应测试
- [ ] `cargo test --lib --features eval` 全通过
- [ ] `cargo test --test eval_*` 全通过
- [ ] `npm test` 全通过(无前端变更,应保持绿)
- [ ] `tsc --noEmit` 无错误
- [ ] `cargo clippy --features eval` 无 warning
- [ ] CI 全绿
- [ ] sovereignty-check.md 所有项通过(EvalJudge 强制本地)
- [ ] memory-impact.md 已评估(评估分数不污染记忆)
- [ ] 默认评测集 `evalsets/default.yaml` 至少 10 个样本
- [ ] `nebula eval run --set default` 命令可执行并输出评分卡
- [ ] Trace 导出 JSONL 格式符合 schema

## 关联

- **关联 change**: `add-openspec-system`(spec-driven 系统本身)
- **关联 ADR**: ADR-003(统一模型调度),ADR-007(数据主权)
- **关联 issue**: 无
- **关联 ROADMAP 项**: v2.4.0 "质量提升"主题(规划中)
- **参考来源**: 《AI Agent 要如何评估》文章 + OpenSpec 项目 + LangSmith 工具链设计

---

*本文件是 change `add-agent-eval` 的提案。技术方案见 [design.md](./design.md),
实现清单见 [tasks.md](./tasks.md),
Delta spec 见 [specs/evaluation/delta.md](./specs/evaluation/delta.md) 等文件。*

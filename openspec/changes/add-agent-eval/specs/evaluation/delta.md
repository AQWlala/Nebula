# Delta: evaluation

> **change**: add-agent-eval
> **领域**: evaluation
> **状态**: draft
> **创建**: 2026-07-11

## ADDED Requirements

### Requirement: Trace 导出
The system SHALL provide a TraceCollector that records Agent execution spans and exports them as JSONL.

- `TraceCollector` 记录 Master / Swarm / Evolution / Skills 每一步中间输出
- 每个 span 包含:id / parent_id / trace_id / span_kind / started_at / ended_at / input / output / llm_meta / child_task_ids / error
- `SpanKind` 枚举:MasterDecompose / MasterSynthesize / SwarmWorker / Reviewer / EvolutionPass / PromptMutation / SkillExec / LlmCall
- `NEBULA_EVAL_TRACING=1` 环境变量启用 Trace 收集,默认关闭(零开销)
- 存储到 SQLite `eval_traces` + `eval_spans` 表
- `nebula eval trace --task <id> --format jsonl` 命令导出

#### Scenario: Trace 收集默认关闭
- **WHEN** 未设置 `NEBULA_EVAL_TRACING` 环境变量
- **THEN** TraceCollector 是 no-op,不记录任何 span
- **AND** 对运行时性能零影响

#### Scenario: 启用 Trace 后记录 span
- **WHEN** 设置 `NEBULA_EVAL_TRACING=1` 并执行 Master 任务
- **THEN** Master 拆解生成一个 MasterDecompose span
- **AND** 每个 Worker 执行生成一个 SwarmWorker span,parent_id 指向 Master span
- **AND** 所有 span 共享同一 trace_id

#### Scenario: 导出 JSONL
- **WHEN** 执行 `nebula eval trace --task <id> --format jsonl --output trace.jsonl`
- **THEN** 从 SQLite 读取该任务的所有 span
- **AND** 按时间顺序写入 JSONL(每行一个 JSON 对象)

---

### Requirement: 评测集管理
The system SHALL manage evaluation sets in YAML format, each containing 10-20 representative samples with binary criteria.

- 评测集格式:YAML,包含 name / description / version / samples
- 每个 Sample 包含:id / kind / input / expected.criteria
- Criteria 为二元标准:满足(+1)/ 不满足(0),可设置 weight
- `SampleKind` 枚举:chat / master_decompose / skill_exec / reviewer / evolution
- 默认评测集 `evalsets/default.yaml` 至少 10 个样本
- 回归评测集 `evalsets/regression.yaml` 5 个样本
- 评测运行记录存储到 SQLite `eval_runs` + `eval_run_samples` 表

#### Scenario: 加载评测集
- **WHEN** 执行 `nebula eval run --set default`
- **THEN** 从 `evalsets/default.yaml` 加载评测集
- **AND** 解析所有样本和标准

#### Scenario: 指定样本子集运行
- **WHEN** 执行 `nebula eval run --set default --sample id1,id2`
- **THEN** 只运行指定的样本,跳过其他

---

### Requirement: LLM-as-Judge 评估器
The system SHALL use a local Ollama model as judge, applying binary verdict (PASS/FAIL) to each criterion.

- Judge LLM **强制本地 Ollama**(WorkType = EvalJudge,`is_local_only()` = true)
- 评判方法:二元评判法,对每个标准判断 PASS / FAIL,不使用模糊评分
- PII 脱敏:在送入 judge 前,用 Regex 脱敏邮箱/电话/身份证/银行卡
- Judge prompt 要求输出严格 JSON
- 容错解析:JSON parse 失败时,用正则提取 PASS/FAIL
- 评分卡输出:终端表格 + JSONL 详细输出

#### Scenario: Judge 强制本地
- **WHEN** EvalJudge 工作类型发起 LLM 调用
- **THEN** `is_local_only()` 返回 true
- **AND** 强制路由到本地 Ollama
- **AND** 即使用户配置了非本地 override,该 override 被忽略

#### Scenario: PII 脱敏
- **WHEN** Agent 输出包含邮箱 `user@example.com`
- **THEN** Scrubber 将其替换为 `[REDACTED]`
- **AND** Judge 收到的 input 中不包含原始邮箱

#### Scenario: Judge 输出非 JSON 时的容错
- **WHEN** Judge LLM 输出 `"1. PASS - 包含3个工作项 2. FAIL - 没有优先级排序"`
- **THEN** 容错解析器用正则提取 PASS / FAIL
- **AND** 生成结构化评分卡

---

### Requirement: 节点隔离测试
The system SHALL support isolated testing of individual Agent nodes without launching the full swarm.

- `IsolationRunner` 支持隔离测试单个节点:Master / Reviewer / Worker / SkillExec
- `IsolatedNode` 枚举:MasterDecompose / MasterSynthesize / Reviewer / SingleWorker / SkillExec
- 隔离测试只调用指定节点,不启动完整蜂群
- 节省 LLM 调用成本(1 次调用 vs 5+ 次)
- `nebula eval isolate --node <node> --sample <count>` 命令

#### Scenario: 隔离测试 Master 拆解
- **WHEN** 执行 `nebula eval isolate --node master-decompose --sample 3`
- **THEN** 只调用 MasterAgent::decompose(),不执行子任务
- **AND** 评估拆解质量(子任务数量 / 依赖关系 / 覆盖度)

#### Scenario: 隔离测试 Reviewer
- **WHEN** 执行 `nebula eval isolate --node reviewer --sample all`
- **THEN** 给定预制 Worker 输出,调用 ReviewerAgent::review()
- **AND** 评估审查质量(是否发现问题 / 建议是否具体)

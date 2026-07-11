# Design: add-agent-eval

> **change**: add-agent-eval
> **状态**: draft
> **创建**: 2026-07-11

## 技术方案概览

```
┌─────────────────────────────────────────────────────────────┐
│                    nebula eval CLI                          │
│  nebula eval run --set default                              │
│  nebula eval trace --task <id> --format jsonl               │
│  nebula eval isolate --node master --sample 3               │
└────────────────────────┬────────────────────────────────────┘
                         │
       ┌─────────────────┼─────────────────┐
       ▼                 ▼                 ▼
┌─────────────┐  ┌──────────────┐  ┌────────────────┐
│ TraceCollector│  │  EvalRunner  │  │ IsolationRunner│
│ (运行时钩子) │  │  (批处理)    │  │  (单节点)      │
└──────┬──────┘  └──────┬───────┘  └────────┬───────┘
       │                │                   │
       │                ▼                   │
       │       ┌────────────────┐           │
       │       │  JudgeEngine   │           │
       │       │ (LLM-as-Judge) │           │
       │       └────────┬───────┘           │
       │                │                   │
       │                ▼                   │
       │       ┌────────────────┐           │
       │       │ Scrubber (PII) │           │
       │       └────────┬───────┘           │
       │                │                   │
       ▼                ▼                   ▼
┌─────────────────────────────────────────────────────────────┐
│                  SQLite (eval_traces / eval_runs)           │
└─────────────────────────────────────────────────────────────┘
       │
       ▼
┌─────────────────────────────────────────────────────────────┐
│  EvolutionEngine ← 评估分数反馈(胜率信号)                  │
└─────────────────────────────────────────────────────────────┘
```

## 一、Trace 导出器 (TraceCollector)

### 1.1 设计目标

记录 Master / Swarm / Evolution 每一步中间输出,生成可离线分析的 JSONL 文件。

### 1.2 数据模型

```rust
/// 一条 Trace 记录,对应 Agent 执行的一个步骤
#[derive(Serialize, Deserialize, Clone)]
pub struct TraceSpan {
    /// 唯一 ID(UUID v7,时序可排序)
    pub id: String,
    /// 父 span ID(形成调用树)
    pub parent_id: Option<String>,
    /// Trace 根 ID(同一任务的所有 span 共享)
    pub trace_id: String,
    /// span 类型:master_decompose / swarm_worker / evolution_pass / ...
    pub span_kind: SpanKind,
    /// 开始时间(UTC ISO 8601)
    pub started_at: String,
    /// 结束时间
    pub ended_at: String,
    /// 输入内容(已脱敏)
    pub input: TracePayload,
    /// 输出内容(已脱敏)
    pub output: TracePayload,
    /// LLM 调用元数据(provider/model/token_usage/cost)
    pub llm_meta: Option<LlmMeta>,
    /// 子任务 ID 列表(如果是 DAG 节点)
    pub child_task_ids: Vec<String>,
    /// 错误信息(如果失败)
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TracePayload {
    /// 文本内容(PII 已脱敏)
    pub text: String,
    /// 结构化元数据(任务类型/技能名等)
    pub metadata: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum SpanKind {
    /// Master 拆解任务
    MasterDecompose,
    /// Master 综合结果
    MasterSynthesize,
    /// 蜂群 Worker 执行子任务
    SwarmWorker,
    /// 评审 Agent
    Reviewer,
    /// 进化 pass
    EvolutionPass,
    /// Prompt 变异
    PromptMutation,
    /// 技能执行
    SkillExec,
    /// LLM 调用(底层)
    LlmCall,
}
```

### 1.3 钩子接入点

```
master.rs::execute_loop()     → TraceCollector::start_span(MasterDecompose)
swarm/orchestrator.rs::run()  → TraceCollector::start_span(SwarmWorker)
evolution/engine.rs::pass()   → TraceCollector::start_span(EvolutionPass)
evolution/prompt_mutator.rs   → TraceCollector::start_span(PromptMutation)
skills/engine.rs::run()       → TraceCollector::start_span(SkillExec)
```

钩子是**可选的**,通过 `NEBULA_EVAL_TRACING=1` 环境变量启用,默认关闭(零开销)。

### 1.4 导出格式

JSONL(每行一个 JSON 对象),字段顺序与 `TraceSpan` 一致:

```jsonl
{"id":"01978a3b-...","parent_id":null,"trace_id":"01978a3b-...","span_kind":"MasterDecompose","started_at":"2026-07-11T10:00:00Z","ended_at":"2026-07-11T10:00:03Z","input":{"text":"...","metadata":{}},"output":{"text":"...","metadata":{}},"llm_meta":{"provider":"deepseek","model":"deepseek-chat","input_tokens":1523,"output_tokens":892,"cost_usd":0.0031},"child_task_ids":["t1","t2","t3"],"error":null}
{"id":"01978a3c-...","parent_id":"01978a3b-...","trace_id":"01978a3b-...","span_kind":"SwarmWorker",...}
```

### 1.5 存储

SQLite 两张表:

```sql
CREATE TABLE eval_traces (
    trace_id TEXT PRIMARY KEY,
    root_span_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    task_summary TEXT NOT NULL,
    span_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE eval_spans (
    id TEXT PRIMARY KEY,
    trace_id TEXT NOT NULL REFERENCES eval_traces(trace_id),
    parent_id TEXT,
    span_kind TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    input_json TEXT NOT NULL,
    output_json TEXT,
    llm_meta_json TEXT,
    child_task_ids_json TEXT,
    error TEXT
);
CREATE INDEX idx_spans_trace ON eval_spans(trace_id);
```

## 二、评测集管理 (EvalSet)

### 2.1 设计目标

管理 10-20 个代表性样本,衡量修改是否有效。支持基线对比(本次 vs 上次)。

### 2.2 评测集格式 (YAML)

```yaml
# evalsets/default.yaml
name: default
description: 默认评测集,覆盖核心场景
version: 1
created: 2026-07-11

samples:
  - id: chat-basic-001
    kind: chat
    input:
      user_message: "帮我总结一下今天的工作"
      context:
        conversation_history: []
    expected:
      # 二元标准:满足/不满足
      criteria:
        - id: c1
          description: "回答包含至少 3 个工作项"
          weight: 1
        - id: c2
          description: "回答有明确的优先级排序"
          weight: 1
        - id: c3
          description: "回答语气专业但不冷漠"
          weight: 1
      # 可选:期望输出(用于 exact match)
      expected_output: null

  - id: master-decompose-001
    kind: master_decompose
    input:
      task: "实现一个用户登录功能,包含前端表单、后端 API、数据库迁移"
    expected:
      criteria:
        - id: c1
          description: "拆解出至少 3 个子任务"
          weight: 1
        - id: c2
          description: "子任务之间有明确的依赖关系"
          weight: 1
        - id: c3
          description: "没有遗漏数据库迁移"
          weight: 2  # 重要标准,权重加倍

  - id: skill-exec-001
    kind: skill_exec
    input:
      skill_name: "loop-engineering"
      task: "审查这段代码是否符合 Loop 模式"
    expected:
      criteria:
        - id: c1
          description: "识别出至少 2 个 Loop 要素"
          weight: 1
        - id: c2
          description: "给出具体的改进建议"
          weight: 1
```

### 2.3 评测运行记录

```sql
CREATE TABLE eval_runs (
    id TEXT PRIMARY KEY,
    evalset_name TEXT NOT NULL,
    evalset_version INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    total_samples INTEGER NOT NULL,
    passed_samples INTEGER NOT NULL DEFAULT 0,
    total_score REAL NOT NULL DEFAULT 0.0,
    max_score REAL NOT NULL DEFAULT 0.0,
    judge_model TEXT NOT NULL,
    baseline_run_id TEXT,  -- 对比的基线 run
    summary_json TEXT
);
CREATE INDEX idx_runs_evalset ON eval_runs(evalset_name, evalset_version);

CREATE TABLE eval_run_samples (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES eval_runs(id),
    sample_id TEXT NOT NULL,
    trace_id TEXT,
    score REAL NOT NULL,
    max_score REAL NOT NULL,
    criteria_results_json TEXT NOT NULL,  -- [{criterion_id, passed: bool, judge_reason: string}]
    judge_raw_output TEXT,
    duration_ms INTEGER
);
CREATE INDEX idx_samples_run ON eval_run_samples(run_id);
```

## 三、LLM-as-Judge 评估器 (JudgeEngine)

### 3.1 设计目标

用本地 Ollama 作为评委,按**二元评判法**打分:对每个标准判断"满足/不满足",不使用模糊评分。

### 3.2 评判流程

```
1. 取样本 input
2. 运行 Agent(正常流程),收集 output + trace
3. 对 output 做 PII 脱敏(Scrubber)
4. 构造 judge prompt(含 input + output + criteria)
5. 调用本地 Ollama(WorkType = EvalJudge)
6. 解析 judge 输出为结构化评分卡
7. 写入 eval_run_samples 表
```

### 3.3 Judge Prompt 模板

```
你是一个严格的代码评审。你需要评估一个 AI Agent 的回答是否满足特定标准。

## 任务输入
{input}

## Agent 回答
{output}

## 评估标准
对于以下每个标准,判断 Agent 的回答是否满足。只能回答 "PASS" 或 "FAIL",并给出一句理由。

1. [标准 c1] {criteria_c1_description}
2. [标准 c2] {criteria_c2_description}
3. [标准 c3] {criteria_c3_description}

## 输出格式(严格 JSON)
```json
{
  "results": [
    {"criterion_id": "c1", "verdict": "PASS", "reason": "..."},
    {"criterion_id": "c2", "verdict": "FAIL", "reason": "..."},
    {"criterion_id": "c3", "verdict": "PASS", "reason": "..."}
  ]
}
```

注意:只能输出 JSON,不要输出其他内容。
```

### 3.4 数据主权约束

**强制本地 Ollama**:

```rust
// llm/dispatcher.rs
WorkType::EvalJudge => {
    // 强制本地,忽略任何非本地 override
    if !provider.is_local() {
        return Err(DispatchError::DataSovereigntyViolation(
            "EvalJudge must use local Ollama to protect user data"
        ));
    }
}
```

这与现有的 `Evolution` / `SoulCompile` / `Classifier` 工作类型一致,遵循 ADR-007 数据主权红线。

### 3.5 PII 脱敏 (Scrubber)

在送入 judge 之前,对 output 做 PII 脱敏:

```rust
pub struct Scrubber {
    patterns: Vec<Regex>,
}

impl Scrubber {
    pub fn new() -> Self {
        Self {
            patterns: vec![
                Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").unwrap(),
                Regex::new(r"\b\d{3}-\d{3,4}-\d{4}\b").unwrap(),  // 电话
                Regex::new(r"\b\d{15,18}\b").unwrap(),              // 身份证
                Regex::new(r"\b\d{16,19}\b").unwrap(),              // 银行卡
            ],
        }
    }

    pub fn scrub(&self, text: &str) -> String {
        let mut result = text.to_string();
        for p in &self.patterns {
            result = p.replace_all(&result, "[REDACTED]").to_string();
        }
        result
    }
}
```

## 四、节点隔离测试 (IsolationRunner)

### 4.1 设计目标

针对单个 Agent 节点(Master / Reviewer / Worker)隔离运行评测集,不启动完整蜂群,节省 LLM 调用成本。

### 4.2 工作模式

```
正常流程:
  User → Master → SwarmWorker×3 → Reviewer → Master → User
  (5+ 次 LLM 调用)

隔离测试 Master:
  EvalInput → Master(直接调用 execute_loop) → EvalOutput
  (1 次 LLM 调用)

隔离测试 Reviewer:
  EvalInput(含预制的 Worker 输出) → Reviewer → EvalOutput
  (1 次 LLM 调用)
```

### 4.3 实现方式

```rust
pub struct IsolationRunner {
    judge: JudgeEngine,
    evalset: EvalSet,
}

impl IsolationRunner {
    /// 隔离测试指定节点
    pub async fn run(
        &self,
        node: IsolatedNode,
        sample_ids: &[&str],  // 指定运行哪些样本,空则全部
    ) -> Result<EvalRunResult> {
        // ...
    }
}

pub enum IsolatedNode {
    /// 只测 Master 拆解(不执行子任务)
    MasterDecompose,
    /// 只测 Master 综合(给定预制子任务结果)
    MasterSynthesize,
    /// 只测 Reviewer(给定预制 Worker 输出)
    Reviewer,
    /// 只测单个 Worker(给定预制任务)
    SingleWorker(WorkerKind),
    /// 只测技能执行
    SkillExec(String),
}
```

## 五、EvolutionEngine 反馈接入

### 5.1 当前进化信号

```
当前: GeneMutator / PromptMutator 用"胜率"作为信号
问题: 胜率采集依赖人工标注或代理指标(用户是否点赞)
```

### 5.2 接入评估器后

```
改进: 进化 pass 完成后,自动运行评测集
      评估分数作为变异器的反馈信号
      分数提升 → 变异方向正确,继续
      分数下降 → 变异方向错误,回滚
```

```rust
// evolution/engine.rs
async fn run_pass(&mut self) -> Result<EvolutionResult> {
    let snapshot = self.snapshot().await?;

    // 1. 执行变异
    self.gene_mutator.mutate().await?;
    self.prompt_mutator.mutate().await?;
    self.skill_evolver.evolve().await?;

    // 2. 运行评估集(新增)
    let eval_result = self.eval_runner.run_default().await?;

    // 3. 对比基线
    if eval_result.score < self.baseline_score {
        // 分数下降,回滚
        self.rollback(&snapshot).await?;
        return Ok(EvolutionResult::RolledBack {
            reason: format!("eval score {} < baseline {}", eval_result.score, self.baseline_score),
        });
    }

    // 4. 分数提升,更新基线
    self.baseline_score = eval_result.score;
    self.log_result(&eval_result).await?;

    Ok(EvolutionResult::Improved { new_score: eval_result.score })
}
```

## 六、CLI 命令

```
# 运行默认评测集
nebula eval run --set default

# 运行指定样本
nebula eval run --set default --sample chat-basic-001,master-decompose-001

# 隔离测试 Master 拆解
nebula eval isolate --node master-decompose --sample 3

# 导出指定任务的 Trace
nebula eval trace --task 01978a3b-... --format jsonl --output trace.jsonl

# 对比两次评测运行
nebula eval diff --run1 <id1> --run2 <id2>
```

## 七、Feature 门控

```toml
# Cargo.toml
[features]
eval = ["dep:serde_yaml", "dep:uuid"]
```

- `eval` feature 默认 off
- 不启用时,TraceCollector 是 no-op,零开销
- 启用时,需要配置 Ollama 作为 judge 模型

## 八、技术选型

| 组件 | 选择 | 理由 |
|------|------|------|
| Trace 存储 | SQLite | 与现有架构一致,无新依赖 |
| 评测集格式 | YAML | 人类可读,易编辑 |
| Judge LLM | 本地 Ollama | 数据主权红线 |
| PII 脱敏 | Regex | 轻量,无 ML 依赖 |
| Trace 格式 | JSONL | 行级解析,便于流式处理 |
| CLI | clap | 与现有 CLI 一致 |

## 九、风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| Judge LLM 输出非 JSON | 高 | 中 | 容错解析:先尝试 JSON parse,失败则用正则提取 PASS/FAIL |
| 评测集样本不代表真实场景 | 中 | 高 | 定期更新样本,从真实 Trace 中提取 |
| 评估成本高(每个样本 1 次 LLM 调用) | 中 | 中 | 节点隔离测试 + 只跑变化的样本 |
| PII 脱敏漏网 | 低 | 高 | 多层防御:Regex + Judge prompt 声明"忽略个人信息" + 本地 Ollama |
| 进化回滚频繁导致无法改进 | 中 | 中 | 设置最小改进阈值(如 0.05),避免噪声导致回滚 |

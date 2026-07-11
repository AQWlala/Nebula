# Tasks: add-agent-eval

> **change**: add-agent-eval
> **状态**: draft
> **创建**: 2026-07-11

## 实现清单

### Phase 1: Trace 导出器 (MVP)

> 优先级: P0
> 目标: 记录 Agent 执行轨迹,可导出 JSONL

- [ ] T-EVAL-001: 创建 `src-tauri/src/eval/mod.rs` 模块骨架
  - 定义 `TraceSpan` / `TracePayload` / `SpanKind` / `LlmMeta` 结构体
  - 定义 `TraceCollector` trait
  - 添加 `eval` feature 到 `Cargo.toml`

- [ ] T-EVAL-002: 实现 `TraceCollector`
  - 文件: `src-tauri/src/eval/trace.rs`
  - `start_span()` / `end_span()` / `export_jsonl()` 方法
  - `NEBULA_EVAL_TRACING=1` 环境变量门控
  - 线程安全(`Arc<Mutex<Vec<TraceSpan>>>`)

- [ ] T-EVAL-003: SQLite 迁移 — eval_traces + eval_spans 表
  - 文件: `src/migrations/0xxx_eval_traces.sql`
  - 参考 design.md §1.5 的 schema

- [ ] T-EVAL-004: 接入 Master 钩子
  - 文件: `src-tauri/src/swarm/master.rs`
  - 在 `execute_loop()` 开头 `start_span(MasterDecompose)`
  - 在结尾 `end_span()`
  - 只在 `eval` feature + 环境变量启用时生效

- [ ] T-EVAL-005: 接入 Swarm Worker 钩子
  - 文件: `src-tauri/src/swarm/orchestrator.rs`
  - 在 `run_worker()` 前后添加 span

- [ ] T-EVAL-006: 接入 Evolution 钩子
  - 文件: `src-tauri/src/evolution/engine.rs`
  - 在 `run_pass()` 前后添加 span

- [ ] T-EVAL-007: Trace 导出命令
  - `nebula eval trace --task <id> --format jsonl --output <file>`
  - 从 SQLite 读取,写为 JSONL

- [ ] T-EVAL-008: Trace 模块单元测试
  - 测试 span 创建/结束/父子关系
  - 测试 JSONL 导出格式正确
  - 测试 feature 关闭时零开销

### Phase 2: 评测集管理 (EvalSet)

> 优先级: P0
> 目标: YAML 格式评测集,可加载和运行

- [ ] T-EVAL-009: 定义 EvalSet 数据结构
  - 文件: `src-tauri/src/eval/evalset.rs`
  - `EvalSet` / `Sample` / `Criteria` / `SampleKind` 结构体
  - 从 YAML 加载(`serde_yaml`)

- [ ] T-EVAL-010: 创建默认评测集
  - 文件: `evalsets/default.yaml`
  - 至少 10 个样本,覆盖:chat(3) / master_decompose(2) / skill_exec(2) / reviewer(2) / evolution(1)
  - 每个样本 2-3 个二元标准

- [ ] T-EVAL-011: 创建回归评测集
  - 文件: `evalsets/regression.yaml`
  - 5 个样本,用于快速回归测试

- [ ] T-EVAL-012: SQLite 迁移 — eval_runs + eval_run_samples 表
  - 文件: `src/migrations/0xxx_eval_runs.sql`
  - 参考 design.md §2.3 的 schema

- [ ] T-EVAL-013: 实现 EvalRunner
  - 文件: `src-tauri/src/eval/runner.rs`
  - `run()` 方法:加载评测集 → 逐样本运行 Agent → 调用 Judge → 记录结果
  - 支持指定样本子集运行

- [ ] T-EVAL-014: eval run CLI 命令
  - `nebula eval run --set default`
  - `nebula eval run --set default --sample id1,id2`
  - 输出评分卡到终端 + 写入 SQLite

- [ ] T-EVAL-015: EvalSet 模块单元测试
  - 测试 YAML 加载
  - 测试样本匹配
  - 测试空评测集处理

### Phase 3: LLM-as-Judge 评估器

> 优先级: P0
> 目标: 本地 Ollama 作为评委,二元评判法

- [ ] T-EVAL-016: 新增 WorkType::EvalJudge
  - 文件: `src-tauri/src/llm/mod.rs` + `dispatcher.rs`
  - `is_local_only()` 返回 true(强制本地 Ollama)
  - 更新 `specs/llm/spec.md` Delta

- [ ] T-EVAL-017: 实现 PII 脱敏器
  - 文件: `src-tauri/src/eval/scrub.rs`
  - 邮箱/电话/身份证/银行卡 Regex 脱敏
  - 单元测试覆盖每种 PII

- [ ] T-EVAL-018: 实现 JudgeEngine
  - 文件: `src-tauri/src/eval/judge.rs`
  - 构造 judge prompt(参考 design.md §3.3)
  - 调用 `UnifiedModelDispatcher::dispatch(WorkType::EvalJudge, ...)`
  - 容错 JSON 解析(失败时正则提取 PASS/FAIL)

- [ ] T-EVAL-019: 评分卡输出
  - 终端表格输出(每个样本的每个标准 PASS/FAIL)
  - 总分 / 满分 / 通过率
  - JSONL 详细输出(可选 `--detail`)

- [ ] T-EVAL-020: Judge 模块单元测试
  - 测试 judge prompt 构造
  - 测试 JSON 解析(正常 + 容错)
  - 测试 PII 脱敏

- [ ] T-EVAL-021: sovereignty-check.md 验证
  - 确认 EvalJudge 强制本地
  - 确认 PII 在送入 judge 前已脱敏
  - 确认评估结果不外发

### Phase 4: 节点隔离测试

> 优先级: P1
> 目标: 单节点隔离运行评测集,节省成本

- [ ] T-EVAL-022: 实现 IsolationRunner
  - 文件: `src-tauri/src/eval/isolation.rs`
  - `IsolatedNode` 枚举(MasterDecompose / MasterSynthesize / Reviewer / SingleWorker / SkillExec)
  - `run()` 方法:直接调用指定节点,不启动完整蜂群

- [ ] T-EVAL-023: Master 拆解隔离测试
  - 给定任务 → 调用 `MasterAgent::decompose()` → 评估拆解质量
  - 不执行子任务

- [ ] T-EVAL-024: Reviewer 隔离测试
  - 给定预制 Worker 输出 → 调用 `ReviewerAgent::review()` → 评估审查质量

- [ ] T-EVAL-025: eval isolate CLI 命令
  - `nebula eval isolate --node master-decompose --sample 3`
  - `nebula eval isolate --node reviewer --sample all`

- [ ] T-EVAL-026: 隔离测试单元测试
  - 测试各节点的隔离调用
  - 测试预制输入注入

### Phase 5: Evolution 反馈接入

> 优先级: P1
> 目标: 评估分数作为进化信号

- [ ] T-EVAL-027: EvolutionEngine 接入 EvalRunner
  - 文件: `src-tauri/src/evolution/engine.rs`
  - `run_pass()` 末尾运行评测集
  - 分数 < 基线 → 回滚

- [ ] T-EVAL-028: PromptMutator 接入评估信号
  - 文件: `src-tauri/src/evolution/prompt_mutator.rs`
  - 变异后运行评测集
  - 分数提升 → 保留变异,分数下降 → 回滚

- [ ] T-EVAL-029: eval diff CLI 命令
  - `nebula eval diff --run1 <id1> --run2 <id2>`
  - 对比两次评测运行的逐样本分数

- [ ] T-EVAL-030: 进化反馈集成测试
  - 测试分数提升时不回滚
  - 测试分数下降时回滚
  - 测试基线更新

### Phase 6: 文档与归档

> 优先级: P2

- [ ] T-EVAL-031: 更新 specs/llm/spec.md — 合并 EvalJudge Delta
- [ ] T-EVAL-032: 更新 specs/evolution/spec.md — 合并评估器反馈 Delta
- [ ] T-EVAL-033: 更新 specs/swarm/spec.md — 合并 Trace 钩子 Delta
- [ ] T-EVAL-034: 创建 specs/evaluation/spec.md — 新领域行为契约
- [ ] T-EVAL-035: 更新 ROADMAP — 添加评估体系条目
- [ ] T-EVAL-036: 归档 change 到 openspec/changes/archive/

## 依赖关系

```
Phase 1 (Trace) ──────────────┐
                               ├──► Phase 3 (Judge) ──► Phase 5 (Evolution)
Phase 2 (EvalSet) ────────────┘                              │
                                                             ▼
                                  Phase 4 (Isolation) ──► Phase 6 (Docs)
```

- Phase 1 和 Phase 2 可并行
- Phase 3 依赖 Phase 1(Trace)+ Phase 2(EvalSet)
- Phase 4 依赖 Phase 3(Judge)
- Phase 5 依赖 Phase 3 + Phase 4
- Phase 6 在所有 Phase 完成后

## 工作量估算

| Phase | 任务数 | 新增文件 | 修改文件 | 复杂度 |
|-------|--------|---------|---------|--------|
| 1 | 8 | 3 | 4 | 中 |
| 2 | 7 | 4 | 1 | 低 |
| 3 | 6 | 2 | 2 | 中 |
| 4 | 5 | 1 | 0 | 中 |
| 5 | 4 | 0 | 2 | 高 |
| 6 | 6 | 1 | 4 | 低 |
| **合计** | **36** | **11** | **13** | — |

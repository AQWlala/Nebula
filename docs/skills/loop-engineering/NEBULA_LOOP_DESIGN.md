# Nebula Loop Engineering 设计文档

> **版本**: 1.0.0  
> **日期**: 2026-07-07  
> **状态**: 设计稿（待评审）  
> **关联**: [SKILL.md](./SKILL.md) | [LOOP_PATTERNS.md](./LOOP_PATTERNS.md) | [ROADMAP_v2.2.md](../../ROADMAP_v2.2.md) | [DEVELOPMENT_PROPOSAL_v1.0.md](../../DEVELOPMENT_PROPOSAL_v1.0.md)

## 0. 文档目的

本文档说明如何将 Loop Engineering 方法论与 Nebula 现有架构（双主控 + 蜂群 worker + persona 自进化）融合，定义实施路线图，并作为后续开发的权威参考。

Loop Engineering 不是要替换 Nebula 的架构，而是为其提供一个**上层编排方法论**——把散落的 Shadow Workspace、LongTaskEngine、SwarmOrchestrator、黑洞海绵记忆等能力，用"闭环系统"的视角统一编排。

---

## 1. 架构映射分析

### 1.1 六大要素成熟度评估

```
要素                    成熟度    缺口                          实施优先级
─────────────────────────────────────────────────────────────────────────
① Automations          🟡 60%    缺 cron 调度 + LOOP.md 定义     P1
② Worktrees            🟢 95%    仅缺多 worktree 并发调度        P3
③ Skills               🟡 40%    SkillComposer 存在但无 Skill 文件系统  P1
④ Connectors/MCP       🟡 35%    有 IM webhook，缺 GitHub/Slack MCP     P2
⑤ Sub-agents           🟡 55%    有蜂群，缺 Maker-Checker 对抗模式      P1
⑥ Memory/State         🟢 85%    缺 STATE.md/LOOP.md 文件式状态         P2
```

### 1.2 Nebula 现有能力的 Loop Engineering 重新解读

#### ShadowWorkspaceEngine = Loop Engineering 的 Worktrees

**现有实现**（`src-tauri/src/shadow_workspace/engine.rs`）：
- 六态机：Creating → Running → Completed → (Merged | Aborted) / Failed
- `git worktree add -b agent/<id>` 创建隔离工作树
- `diff()` 提供全量 diff（已提交 + 未提交）
- `run_command()` 在 worktree 内执行命令
- `merge()` / `abort()` 清理
- RecordingLog 自动记录每条命令（T-E-C-09）

**Loop Engineering 视角**：这已经是 Loop Engineering 中 Worktrees 要素的完整实现。Shadow Workspace 就是 Nebula 的 Worktree 抽象——每个 Agent 任务在独立 git worktree 中隔离执行，完成后提供 diff 供审查。

**缺口**：目前 Shadow Workspace 是手动创建，缺多 worktree 并发调度器（多个 Agent 同时工作时自动分配 worktree）。

#### LongTaskEngine = Loop Engineering 的 Automations（部分）

**现有实现**（`src-tauri/src/long_task/engine.rs`）：
- 六态机：Pending → Running ⇄ Paused → Completed | Failed | Cancelled
- SQLite 持久化（`long_tasks` + `long_task_steps` 表，migration 037）
- `tokio::spawn` 后台 runner + `spawn_blocking` 调用 shadow_engine
- AtomicBool 暂停/取消信号（协同式）
- `bootstrap()` 重启恢复（Running → Paused）
- 9 个 Tauri 命令

**Loop Engineering 视角**：LongTaskEngine 已经实现了 Loop 的执行引擎，但缺两个关键能力：
1. **定时触发**（cron schedule）—— 目前只能手动 `start()`，没有"每天 09:00 自动触发"
2. **Loop 定义文件**（LOOP.md）—— 目前 goal 只是字符串，没有结构化的"观察信号 + 停止条件 + 验证方式"

#### SwarmOrchestrator = Loop Engineering 的 Sub-agents（部分）

**现有实现**（`src-tauri/src/swarm/orchestrator.rs`）：
- 2-6 generic agents 并行（`join_all`）
- 单 Agent 失败不中止整个 run
- Negotiator 协商（Voting/Cascading/Arbitration）
- DynamicAgentPool 按复杂度动态调整
- DeadlockDetector 环检测

**Loop Engineering 视角**：蜂群是 Sub-agents 要素的并行执行实现，但缺 Loop Engineering 强调的 **Maker-Checker 对抗模式**——目前所有 agent 都是"制作者"角色，没有独立的"检查者"角色用不同指令（甚至不同模型）做对抗性审查。

#### 黑洞海绵记忆 = Loop Engineering 的 Memory/State

**现有实现**：
- L0-L7 分层（L0Cache 预热 + SemanticCache + SQLite + LanceDB 向量）
- 黑洞压缩（长期记忆压缩）
- 海绵吸收（文件/剪贴板/文档吸收）
- MDRM 5 维关系图谱（T-E-B-16）

**Loop Engineering 视角**：记忆能力远超 Loop Engineering 原版的"STATE.md 文件式状态"。但 Loop Engineering 强调的"仓库记得即使模型不记得"理念，对应到 Nebula 是需要把 **任务状态也写入文件**（不只是 SQLite），让 git 跟踪、让 Agent 每次会话都能读到。

---

## 2. 目标架构：MasterAgent 的 Loop 执行模式

> ⚠️ **评审修订**（v1.1）：LoopEngine 不作为独立模块，内化为 MasterAgent 的 `Loop` 执行模式（与 `Once` 模式并列），避免"三套编排器并存"（MasterAgent + PlanEngine + LoopEngine）。

### 2.1 总体设计

```
┌─────────────────────────────────────────────────────────────┐
│              MasterAgent（现有，扩展 Loop 模式）              │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────────┐  │
│  │ Once 模式 │  │ Loop 模式 │  │ Plan 模式（现有审批流）   │  │
│  │（现有）   │  │（新增）   │  │                          │  │
│  └──────────┘  └────┬─────┘  └──────────────────────────┘  │
│                     │                                        │
│       ┌─────────────┼─────────────┐                         │
│       ▼             ▼             ▼                         │
│  ┌─────────┐  ┌──────────┐  ┌──────────────┐               │
│  │LoopDef  │  │Scheduler │  │ StateMgr     │               │
│  │(LOOP.md)│  │(扩展现有  │  │(STATE.md     │               │
│  │解析)    │  │ CronTask)│  │ 只读投影)    │               │
│  └─────────┘  └──────────┘  └──────────────┘               │
│                     │                                        │
│                     ▼                                        │
│  ┌──────────────────────────────────────────────────────┐   │
│  │           ValuesLayer（现有 L4 价值层）               │   │
│  │   Allow / Confirm / Plan / Deny 四种裁定              │   │
│  │   ← Loop 每次 Action 前必须过此门                     │   │
│  └──────────────────────────────────────────────────────┘   │
│                     │                                        │
│                     ▼                                        │
│  ┌──────────────────────────────────────────────────────┐   │
│  │    星尘群（现有蜂群）+ ReviewerAgent（升级为 Checker） │   │
│  │    Maker = 带 persona 的 GenericAgent                 │   │
│  │    Checker = 升级后的 ReviewerAgent（本地同模型 +     │   │
│  │    对抗 prompt + 独立 worktree + 只读工具集）          │   │
│  └──────────────────────────────────────────────────────┘   │
│                     │                                        │
│                     ▼                                        │
│  ┌──────────────────────────────────────────────────────┐   │
│  │    LongTaskEngine（现有）+ ShadowWorkspace（现有）     │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │  L0-L7 记忆系统  │ ◄── 复用现有
                    │  Loop 经验 → L3  │
                    │  persona 偏好 → 星魂 │
                    └─────────────────┘
```

**关键变化**（评审后）：
1. LoopEngine **不是独立模块**，是 MasterAgent 的 Loop 执行模式
2. 自主度**统一用 L0-L5**，不另建 L1-L4
3. STATE.md 是 **SQLite 只读投影**，不是独立状态源
4. Checker **强制本地同模型**，不依赖闭源 API
5. 每次 Action **必过 ValuesLayer**（L4 价值层），Maker-Checker 是 Allow 之后的追加审查
6. Loop 经验进 **L3 事实层**，不进星魂（persona 层）

### 2.2 新增文件（最小化，复用优先）

> 评审修订：不新建 `loop_engine/` 目录，而是在现有模块中扩展。

```
src-tauri/src/
├── swarm/
│   ├── master.rs           # 现有：新增 execute_loop() 方法（Loop 执行模式）
│   ├── loop_def.rs         # 新增：LOOP.md YAML 解析 → LoopDef 结构
│   └── agents/
│       └── reviewer.rs     # 现有：升级为 CheckerAgent（加 worktree 隔离 + 对抗 prompt）
├── evolution/
│   └── cron_scheduler.rs   # 现有：扩展支持完整 cron 表达式（不引入 tokio-cron-scheduler）
├── long_task/
│   └── engine.rs           # 现有：新增 state_projection() 生成 STATE.md 只读投影
└── templates/
    └── loops/              # 新增：Loop 模板目录
        ├── daily_triage.md
        ├── ci_sweeper.md
        └── memory_triage.md
```

### 2.3 Loop 定义格式（LOOP.md）

```yaml
---
name: daily-triage
description: 每日扫描 CI 失败和 Issue，写入 TODO
cadence: "0 9 * * 1-5"          # cron 表达式（扩展现有 CronTask 解析）
autonomy: L1                     # Nebula L0-L5 枚举（非 Loop 自建 L1-L4）
budget_tokens: 50000             # 单次执行 Token 预算
budget_minutes: 10               # 单次执行时间预算
---

## Intent
扫描昨日的 CI 失败和 GitHub Issue，按优先级分类

## Context
- 读取 GitHub Actions 失败记录（需 GitHub MCP 连接器）
- 读取 open issues labeled "bug"
- 加载 SKILL.md 中的项目规范

## Action
- 分类发现项（quick-win / 需深入分析 / 阻塞）
- 为 quick-win 项在 Shadow Workspace 起草修复

## Observation
- CI 失败的根因分析
- Issue 的复现路径
- quick-win 修复的测试结果

## Adjustment
- 将发现写入 STATE.md
- 若 quick-win 修复测试通过，提升自主度到 L2（写入分支，待人工 merge）
- 若无发现，归档本次运行

## Stop Condition
- STATE.md 已更新，或
- Token/时间预算耗尽

## Connectors
- github: required
- filesystem: required

## Safety
- L1: 只读 CI 日志 + 只写 STATE.md
- 不触碰源码文件
- 不开 PR
```

### 2.4 STATE.md 格式（SQLite 只读投影）

> ⚠️ **评审修订**（v1.1）：STATE.md 不是独立状态源，是 `long_tasks`/`long_task_steps` SQLite 表的**只读投影**。由 StateMgr 在每次 step 完成后单向生成，禁止 Agent 直接写。所有写操作仍走 LongTaskEngine。存储位置：`.nebula/state/STATE.md`（默认 gitignore，用户可显式 opt-in 跟踪）。

```markdown
# Nebula Loop 状态（只读投影，由 StateMgr 自动生成）

最后更新：2026-07-07 09:03 UTC（由 daily-triage Loop 自动更新）
数据源：SQLite long_tasks + long_task_steps 表

## 进行中
- [ ] fix: auth 模块 flaky test（CI Run #4821，失败 3 次）
  - 假设：并发测试 session 状态泄漏
  - 已尝试：隔离 test 数据库连接 → 无效
  - 下一步：检查 beforeEach cleanup 逻辑
  - shadow_workspace_id: a8b3c2d1
  - provenance: loop:daily-triage | checker:qwen2.5:7b | autonomy:L2

## 待处理
- [ ] 升级 axios 到 1.7.x（CVE-2026-xxxxx）
- [ ] API 文档落后于代码（PR #308 合并后）

## 已完成
- [x] 修复 billing 含单引号公司名 500 错误（PR #312，已合并）
  - provenance: loop:ci-sweeper | checker:qwen2.5:7b | autonomy:L3

## 本次运行日志
- 2026-07-07 09:00:13 [daily-triage] 启动，预算 50000 tokens / 10 min
- 2026-07-07 09:02:45 [daily-triage] 扫描到 3 个 CI 失败 + 2 个 bug Issue
- 2026-07-07 09:08:12 [daily-triage] quick-win 起草完成，写入 shadow ws a8b3c2d1
- 2026-07-07 09:09:58 [daily-triage] 完成，消耗 38200 tokens / 9m58s
```

**provenance 字段**（评审新增，数据主权要求）：每项必须含 `loop:<name> | checker:<model> | autonomy:<L?>`，让用户能追溯每行代码来源，缓解认知投降。

### 2.5 Maker-Checker 模式（升级现有 ReviewerAgent）

> ⚠️ **评审修订**（v1.1）：不新建 `maker_checker.rs`，**升级现有 `swarm/agents/reviewer.rs`** 为 CheckerAgent。reviewer.rs 已实现只读工具集 + APPROVE/REVISE/REJECT 裁决，仅需加 worktree 隔离 + 对抗 prompt。

```
Maker Agent（制作者）          Checker Agent（升级后的 ReviewerAgent）
     │                              │
     │  1. 接收任务 + Skill 注入      │
     │  2. 在 Shadow Workspace 工作   │
     │  3. 产出 diff + 测试结果       │
     │                              │
     │  ──────── 4. 提交 diff ──────►│
     │                              │  5. 独立运行测试套件（独立 worktree）
     │                              │  6. 对照 SKILL.md 检查规范
     │                              │  7. 验证用户可见行为
     │                              │  8. 输出 APPROVED 或 REJECTED
     │  ◄──────── 9. 反馈 ──────────│
     │                              │
     │  10. 若 REJECTED，根据反馈     │
     │      调整并重新提交            │
     │                              │
     │  11. 循环直到 APPROVED         │
     │      或达到最大重试次数        │
```

**关键设计**（评审修订）：
- Checker **强制本地 Ollama**，不依赖闭源 API（数据主权红线）
- 默认配置：Checker = **同模型 + 对抗 prompt + 独立 worktree**（独立性来自上下文隔离而非模型差异）
- 多模型降级为"有足够 VRAM 时可选增强"（qwen2.5:32b、deepseek-r1:14b），不作为默认依赖
- Checker 是**无 persona 的星尘群专精角色**（不升级为化身），persona 注入留 Maker 侧
- Checker 从**独立 Context 通道**加载原始任务目标 + SKILL，不读 Maker 产出的 STATE.md（防上下文投毒）
- **模型同质检测**：若 Checker 退化为同模型同实例，自动降级自主度（L4→L2）+ 写入审计日志
- 停止条件由 Checker 判断，不由 Maker 判断
- Checker 输出 REJECTED 理由写入 loop-run-log.md，人类每周抽样复核

**Maker-Checker 与星尘群的关系**（评审澄清）：
- 星尘群 = N 个同质 agent 并行 + Negotiator 选优（"哪个最好"）
- Maker-Checker = 1 Maker 串行产出 + 1 Checker 二值否决（"能不能放过"）
- 两者**正交可叠加**：星尘群产 diff → Checker 把关，而非互斥

---

## 3. 与 ROADMAP_v2.2.md 的对齐

### 3.1 现有任务复用（评审扩展）

| ROADMAP 任务 | 状态 | Loop Engineering 角色 |
|---|---|---|
| T-E-C-08 Shadow Workspace | ✅ DONE | Loop 的 Worktrees 要素 ✅ |
| T-E-C-09 任务录屏回放 | ✅ DONE | Loop 的 Observation 信号源 ✅ |
| T-E-C-10 异步长任务模式 | ✅ DONE | Loop 的 Automations 执行引擎 ✅ |
| T-E-S-01 Agent 角色专业化 | ✅ DONE | Maker-Checker 的角色基础 ✅ |
| T-E-S-04 MoA 一等公民 | ✅ DONE | Checker 的投票/仲裁机制 ✅ |
| **T-E-S-50 AutonomySlider** | ✅ DONE | **自主度 UI 复用**（L0-L5 已上线，评审新增） |
| **(现有) ReviewerAgent** | ✅ DONE | **升级为 CheckerAgent**（reviewer.rs，评审新增） |
| **(现有) CronTask 调度器** | ✅ DONE | **扩展 cron 表达式**（cron_scheduler.rs，评审新增） |
| **(现有) ValuesLayer L4 价值层** | ✅ DONE | **Loop Action 前置门**（Allow/Confirm/Plan/Deny，评审新增） |
| T-E-D-04 8 人格系统 | 待开始 | Maker 可用 persona（Checker 不用 persona） |
| T-E-D-05 Proactive Engine | 待开始 | Loop 的主动触发器（cadence） |
| T-E-S-10 WorkflowCanvas | 待开始 | Loop 的可视化编排（设计时） |
| T-E-S-11 蜂群运行时画布 | 待开始 | Loop 的运行时可视化 |
| T-E-C-18 OAuth 集成层 | 待开始 | Loop 的 Connectors（GitHub/Notion 等，pull-only） |

### 3.2 建议新增任务（评审修订版）

> 以下任务建议加入 ROADMAP_v2.2.md 的 Wave 3 或 Stage 7 贯穿层。
> ⚠️ 评审修订：T-E-L-01/02/03 从"新建独立模块"改为"扩展现有模块"。

| 建议任务 ID | 描述 | 优先级 | 复杂度 | 依赖 |
|---|---|---|---|---|
| **T-E-L-01** | **MasterAgent Loop 执行模式**：master.rs 新增 `execute_loop()` 方法 + loop_def.rs（LOOP.md YAML 解析）+ StateMgr（STATE.md 只读投影，从 SQLite 生成）+ 复用 `master_*` Tauri 命令 | P1 | L | T-E-C-10 |
| **T-E-L-02** | **CronTask 扩展**：扩展现有 `evolution/cron_scheduler.rs` 支持完整 5 字段 cron 表达式（不引入 tokio-cron-scheduler）+ Token/时间预算（AtomicU64 内存累加，异步落库）+ L0-L5 自主度 | P1 | M | T-E-L-01 |
| **T-E-L-03** | **ReviewerAgent 升级为 CheckerAgent**：升级现有 `swarm/agents/reviewer.rs`（加 worktree 隔离 + 对抗 prompt + 独立 Context 通道 + 模型同质检测 + 自动降级），不新建 maker_checker.rs | P1 | L | T-E-S-01, T-E-C-08 |
| **T-E-L-04** | **GitHub MCP 连接器（pull-only）**：读取 Actions 失败 + Issue + PR（**默认 pull-only，写操作人工触发**），为 CI Sweeper / PR Babysitter / Daily Triage 提供 Observation 信号 | P2 | L | T-E-C-18 |
| **T-E-L-05** | **Loop 模板库**：7 种 Loop 模式的 LOOP.md 模板 + 复用 TemplatesDialog（新增 automation 类别，默认只露 2 个入口） | P2 | M | T-E-L-01 |
| **T-E-L-06** | **Loop 预算管理**：loop-budget.md（拆分本地 $0 / 云端两列）+ loop-cost 估算 + 超预算自动暂停 + loop-safety-guards.md（模型同质检测 + 自动降级） | P2 | M | T-E-L-02 |
| **T-E-L-07** | **Loop 审计日志**：loop-run-log.md（人类可读 Markdown，每次运行的 cadence/token/结果 + provenance）+ 异常告警（IM webhook 通知） | P3 | S | T-E-L-01 |
| **T-E-L-08a** | **Loop 运行时阶段环**（评审拆分）：复用 SwarmView 的 AgentColumn + ToolCallCard，加五阶段高亮环（Intent→Context→Action→Observation→Adjustment），不等 WorkflowCanvas | P2 | M | T-E-S-11 |
| **T-E-L-08b** | **Loop 设计节点**（评审拆分）：WorkflowCanvas 集成 Loop 节点，依赖 T-E-S-10 | P3 | XL | T-E-S-10 |

### 3.3 优先级排序逻辑

1. **T-E-L-01 + T-E-L-02 + T-E-L-03**（P1）构成最小可用 Loop：能定义、能调度、能 Maker-Checker 验证
2. **T-E-L-04 + T-E-L-05 + T-E-L-08a**（P2）让 Loop 有真实信号源、模板和运行时可视化
3. **T-E-L-06 + T-E-L-07**（P2/P3）控制成本和可观测性
4. **T-E-L-08b**（P3）设计时编排，非阻塞

---

## 4. 实施路线图

### 4.1 阶段一：最小可用 Loop（2-3 周）

> ⚠️ 评审修订：不新建 loop_engine/ 目录，扩展现有模块。

**目标**：能在 Nebula 中定义一个 Loop，定时触发，Maker-Checker 执行，状态投影到 STATE.md。

**交付物**（评审修订版）：
- `src-tauri/src/swarm/master.rs` — 现有，新增 `execute_loop()` 方法（Loop 执行模式）
- `src-tauri/src/swarm/loop_def.rs` — 新增，LOOP.md YAML 解析
- `src-tauri/src/long_task/engine.rs` — 现有，新增 `state_projection()` 生成 STATE.md 只读投影
- `src-tauri/src/evolution/cron_scheduler.rs` — 现有，扩展完整 cron 表达式
- `src-tauri/src/swarm/agents/reviewer.rs` — 现有，升级为 CheckerAgent
- 复用 `master_*` Tauri 命令（不另起 `loop_*`）
- 集成到 LongTaskEngine（Loop 触发时创建 LongTask）
- 集成到 ShadowWorkspaceEngine（Maker 在 worktree 工作）
- 集成到 ValuesLayer（每次 Action 前必过 L4 价值层裁定）

### 4.2 阶段二：连接器与模板（1-2 周）

**目标**：Loop 能读取 GitHub 信号，有 7 种现成模板可用。

**交付物**：
- GitHub MCP 连接器（读取 Actions / Issues / PRs）
- 7 个 LOOP.md 模板文件
- Loop 模板 UI（复用 TemplatesDialog 模式）

### 4.3 阶段三：成本控制与可观测性（1 周）

**目标**：Loop 运行有预算限制，超预算自动暂停，异常通知。

**交付物**：
- loop-budget.md 解析 + 超预算暂停逻辑
- loop-run-log.md 自动追加
- IM webhook 异常通知（复用 T-E-C-17）

### 4.4 阶段四：可视化（2 周，可与 Wave 3 并行）

**目标**：WorkflowCanvas 支持 Loop 节点，运行时画布显示 Loop 阶段。

**交付物**：
- WorkflowCanvas Loop 节点类型
- 运行时 Loop 阶段高亮
- 历史回放

---

## 5. 预算管理设计（loop-budget.md）

```markdown
# Loop 预算

## 全局预算
- 月度 Token 上限: 5,000,000
- 月度美元上限: $50
- 单次执行默认: 50,000 tokens / 10 min

## 各 Loop 预算
| Loop | Cadence | Token/次 | 月度估算 | 美元估算 |
|------|---------|---------|---------|---------|
| daily-triage | 0 9 * * 1-5 | 50,000 | 1,000,000 | $10 |
| ci-sweeper | 0 */2 * * * | 30,000 | 3,600,000 | $36 |
| memory-triage | 0 3 * * 0 | 80,000 | 320,000 | $3.2 |

## 超预算行为
- 单次超预算: 暂停 + 写入 STATE.md + IM 通知
- 月度超预算: 停止所有 Loop + 需人工恢复
```

---

## 6. 风险与缓解（评审修订）

| 风险 | 严重度 | 缓解措施 |
|------|--------|---------|
| Loop 自动 merge 低质量代码 | 高 | L3 及以下不自动 merge；L4 需 ValuesLayer.Confirm + Checker + CI 双绿 + 审计日志；L5 需 ValuesLayer.Allow |
| Token 成本失控 | 中 | loop-budget.md 硬限制（拆分本地 $0 / 云端两列）+ 超预算暂停 + IM 告警 |
| 理解债积累 | 高 | 强制 STATE.md 人类可读 + provenance 字段 + 每周 Memory Triage Loop 生成"本周理解摘要" |
| Maker 和 Checker 串通 | 中 | 默认同模型 + 对抗 prompt + 独立 worktree + 独立 Context 通道（不读 Maker 的 STATE.md）+ 模型同质检测自动降级 |
| Shadow Workspace 泄漏 | 低 | worktree 在系统临时目录 + 完成后清理 + 分支名 `agent/<id>` 前缀 |
| **虚假安全感**（评审新增） | 高 | 模型同质检测 + 自动降级（L4→L2）+ 审计日志 + loop-safety-guards.md |
| **数据主权违反**（评审新增） | 高 | Checker 强制本地 Ollama；GitHub MCP pull-only；闭源模型为不可妥协红线 |

---

## 7. 与 Nebula 双主控架构的融合（评审修订）

Nebula 的"双主控 + 蜂群 worker + persona 自进化"架构与 Loop Engineering 的关系：

| Nebula 概念 | Loop Engineering 对应 | 融合方式（评审修订） |
|---|---|---|
| **主星·编排者**（双主控之一） | Loop 的外循环驱动者 | 主星新增 `execute_loop()` 方法，负责 Loop 的 Intent + Adjustment 阶段；**LoopEngine 内化为主星的 Loop 执行模式**（非独立模块） |
| **化身·灵魂分身**（双主控之二） | Loop 的 Context 注入者 | 化身负责 Skill 加载 + persona 切换（仅 Maker 侧，Checker 不带 persona） |
| **星尘群**（蜂群 worker） | Sub-agents（Maker + Checker） | 星尘群在 Shadow Workspace 中并行工作；ReviewerAgent 升级为 CheckerAgent |
| **星魂**（persona 自进化） | persona 偏好进化 | **评审修订**：星魂只吸收"用户对 Loop 自主度的偏好"这种 persona 特质；Loop 运行经验进 L3 事实层 + MDRM，不进星魂 |
| **ValuesLayer**（L4 价值层） | Loop Action 前置门 | **评审新增**：Allow/Confirm/Plan/Deny 四种裁定天然是 Loop 自主度控制；高风险任务 Deny 名单硬编码 |
| **黑洞压缩** | Memory 的压缩 | Loop 的 STATE.md 只读投影定期归档（不触发跨层压缩，黑洞压缩已自动触发） |
| **海绵吸收** | Memory 的吸收 | Loop 的 Observation 信号被海绵吸收到 L1/L2 |

**关键洞察**：Loop Engineering 的"外循环"正是 Nebula"主星·编排者"的职责——它不是执行者，而是设计"何时提示谁做什么"的系统。Loop Engineering 为 Nebula 的双主控架构提供了**可操作的方法论**。主星通过 `execute_loop()` 方法获得 Loop 能力，无需新增独立编排器。

---

## 8. 开放问题（评审后状态）

> 评审后，5 个开放问题中 3 个已决策，2 个待讨论。

1. ~~**LoopEngine 是否应该成为主星的核心能力？**~~ → ✅ **已决策**：内化为主星的 Loop 执行模式（非独立模块）
2. ~~**STATE.md 应该放在仓库根目录还是 `.nebula/` 目录下？**~~ → ✅ **已决策**：放 `.nebula/state/STATE.md`（默认 gitignore，只读投影）
3. ~~**Maker-Checker 的 Checker 是否应该用更强的闭源模型（如 Claude）？**~~ → ✅ **已决策（预先否决）**：Checker 强制本地 Ollama，闭源模型为不可妥协红线（数据主权）
4. **Loop 经验进 L3 事实层还是星魂？** → 🟡 **待讨论**：评审建议进 L3 + MDRM（事实知识），星魂只吸收 persona 偏好
5. **是否需要实现 loop-audit / loop-cost / loop-sync 命令行工具？** → 🟡 **待讨论**：倾向内置为 Tauri 命令，不另建 CLI

---

## 9. 参考资料

- [Loop Engineering 中文教程](https://www.runoob.com/ai-agent/loop-engineering.html)
- [cobusgreyling/loop-engineering GitHub](https://github.com/cobusgreyling/loop-engineering)
- Addy Osmani, "Loop Engineering" (Substack, 2026-06)
- Boris Cherny (Anthropic), "I don't prompt Claude anymore. I have loops running." (2026-06)
- Peter Steinberger (OpenClaw), "You should be designing loops that prompt your agents." (2026-06-07)
- [Nebula DEVELOPMENT_PROPOSAL_v1.0.md](../../DEVELOPMENT_PROPOSAL_v1.0.md)
- [Nebula ROADMAP_v2.2.md](../../ROADMAP_v2.2.md)
- [Nebula COMPREHENSIVE_EVOLUTION_v3.0.md](../../COMPREHENSIVE_EVOLUTION_v3.0.md)

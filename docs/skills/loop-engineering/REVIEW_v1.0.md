# Loop Engineering 架构评审报告 v1.0

> **评审日期**: 2026-07-07  
> **评审对象**: [SKILL.md](./SKILL.md) v1.0.0 / [NEBULA_LOOP_DESIGN.md](./NEBULA_LOOP_DESIGN.md) v1.0.0 / [LOOP_PATTERNS.md](./LOOP_PATTERNS.md) v1.0.0  
> **评审方式**: 7 专家并行评审（架构师 / 记忆系统 / Agent 编排 / 安全 / 性能 / UX / 数据主权）  
> **评审结论**: **需修订后通过**（平均 5.7/10，4 个致命风险需消除）

---

## 1. 评分汇总

| # | 专家维度 | 评分 | 核心发现 |
|---|---------|------|---------|
| 1 | 首席架构师 | 6/10 | LoopEngine 作为独立模块会导致"三套编排器并存" |
| 2 | 记忆系统 | 6/10 | L4→L5→L6 压缩链引用未实现的 L6；STATE.md 与 SQLite 双写 |
| 3 | Agent 编排 | 6/10 | reviewer.rs 已是 Checker 雏形却被忽略；虚假安全感陷阱 |
| 4 | 安全 | 5/10 | **L4 语义倒置**：Loop L4=全自动 vs 现有 L4=审批门 |
| 5 | 性能 | 6/10 | 异模型 Checker + 多 worktree 击穿 16GB 机器；成本估算错误 |
| 6 | 前端/UX | 5/10 | **L1-L4 与现有 AutonomySlider L0-L5 命名碰撞** |
| 7 | 数据主权 | 6/10 | Checker 闭源模型违反"0 字节上行"核心承诺 |

**平均**: 5.7/10 —— 方向正确，但与现有架构存在系统性摩擦，需修订。

---

## 2. 致命风险（4 个，必须消除）

### 风险 A：L4 语义倒置（安全 + UX 双重确认）

**问题**：
- Loop 设计的 L1-L4 自主度阶梯中，**L4 = "Maker+Checker 双 Agent，CI 通过后自动 merge"**（全自动）
- Nebula 现有的 L0-L5 自主度中，**L4 = "ApprovalGate 价值层门"**（需人工裁定的确认层）
- 同一标识符 L4 在两套体系里语义**完全相反**——一个是"自动合并"，一个是"人工审批"
- 现有 `AutonomySlider.tsx`（T-E-S-50）已上线，L0-L5 六档已部署在 App.tsx:307

**影响**：用户在 UI 看到 L4 会联想到现有体系的"审批层"，误以为"已审批"而放行 auto-merge，在 L4 价值层尚未裁定的情况下让 Loop 执行本应人工确认的高风险合并。

**修订决策**：
- **废弃 Loop 自建 L1-L4 阶梯**
- Loop 自主度字段改用现有 L0-L5 枚举，映射关系：
  - Loop "L1 report"（只读）→ 现有 **L1**（定向编辑，只读模式）
  - Loop "L2 assisted"（草稿）→ 现有 **L2-L3**（对话 + Plan 模式）
  - Loop "L3 unattended"（Draft PR）→ 现有 **L3 + L4 ApprovalGate(Confirm)**
  - Loop "L4 fully-auto"（auto-merge）→ 现有 **L5 后台例外 + ValuesLayer.Allow**
- LOOP.md 的 `autonomy:` 字段直接使用 L0-L5 枚举值

### 风险 B：三套编排器并存（架构师确认）

**问题**：
- 现有 `master.rs`（MasterAgent DAG 编排）+ `plan_mode.rs`（PlanEngine 审批流）已覆盖编排职责
- NEBULA_LOOP_DESIGN §2.1 又提出独立的 LoopEngine（持有 Scheduler/LoopDef/StateMgr/Maker-Checker）
- 三套状态机 + 三套事件 + 三套 Tauri 命令并存 → 认知与运维复杂度爆炸

**修订决策**：
- **LoopEngine 不作为独立模块**，内化为 MasterAgent 的 `Loop` 执行模式（与 `Once` 模式并列）
- 新增文件仅 2 个：`loop_def.rs`（LOOP.md 解析）+ `scheduler.rs`（cron 触发，复用现有 cron_scheduler.rs）
- Tauri 命令复用 `master_*` 系列，不另起 `loop_*`
- Maker-Checker 不新建 `maker_checker.rs`，**升级现有 `swarm/agents/reviewer.rs`** 为 CheckerAgent

### 风险 C：Checker 闭源模型违反数据主权（数据主权 + 性能 + 编排三重确认）

**问题**：
- NEBULA_LOOP_DESIGN §8 开放问题 3 暗示 Checker 可用 Claude 等闭源模型
- 代码 diff 发给云端 = "把你的代码变成别人的养料"，直接违反"0 字节上行是默认态"
- 性能问题：本地 Ollama 切换 7b→14b 模型耗时 30-90s，16GB RAM 常触发 OOM
- 编排问题：VRAM 不足会静默退化为"同模型同实例"，Maker-Checker 沦为"自我审查"，比单 agent 更危险（产出"已通过独立审查"的虚假信号）

**修订决策**：
- **Checker 强制本地 Ollama**，"不同模型"通过本地不同参数/persona 实现（qwen2.5:32b、deepseek-r1:14b）
- 开放问题 3 从"待讨论"改为**预先否决**，移入风险表标注"不可妥协"
- 默认配置：Checker = 同模型 + 对抗 prompt + 独立 worktree（独立性来自上下文隔离而非模型差异）
- 多模型降级为"有足够 VRAM 时可选增强"，不作为默认依赖
- 新增 `loop-safety-guards.md`：模型同质检测 + 自动降级（L4→L2）+ 审计日志

### 风险 D：STATE.md 双写状态漂移（架构师 + 记忆系统双重确认）

**问题**：
- `long_tasks` + `long_task_steps` SQLite 表已是任务状态权威源
- STATE.md 若可写，必产生 git 提交的 STATE.md 与 SQLite 不一致
- Agent 加载时不知以谁为准

**修订决策**：
- **STATE.md 定义为 SQLite 的只读投影**（由 StateMgr 单向生成，禁止 Agent 直接写）
- 所有写操作仍走 LongTaskEngine
- 存储位置：`.nebula/state/STATE.md`（默认 gitignore，用户可显式 opt-in 跟踪）

---

## 3. 共识问题（多位专家独立提出）

### 共识 1：现有代码复用缺失（4 位专家独立指出）

| 现有能力 | 文件位置 | 文档遗漏 |
|---------|---------|---------|
| ReviewerAgent（Checker 雏形） | `swarm/agents/reviewer.rs` | 编排专家指出：已实现只读工具集 + APPROVE/REVISE/REJECT 裁决 |
| CronTask 调度器 | `evolution/cron_scheduler.rs` | 性能专家指出：已有 interval 轮询架构，仅缺完整 cron 表达式 |
| L4 价值层（ValuesLayer） | `memory/values.rs` | 安全专家指出：Allow/Confirm/Plan/Deny 四种裁定天然是 Loop 自主度控制 |
| AutonomySlider | `src/components/AutonomySlider.tsx` | UX 专家指出：L0-L5 六档已上线 |

**修订决策**：所有 T-E-L-01~08 任务必须先评估"现有能力能否复用/扩展"，禁止重复造轮子。

### 共识 2：记忆层级纠正

**问题**：评审中我对记忆层级的理解有误。
- `src-tauri/src/memory/mod.rs:6` 明确写 "eight MemoryLayers (L0..L7)"
- L0 缓存 / L1 消息 / L2 经验 / L3 事实 / L4 知识 / L5 教训 / **L6 原则（未实现）** / **L7 奇点核心价值**
- WHITEPAPER_v2.0 的 L0-L5 是营销简化版
- SKILL.md 的"L0-L7"引用**正确**，但 LOOP_PATTERNS 的"L4→L5→L6 压缩链"是**错误**的（L6 未实现，且黑洞压缩不做跨层晋升，只做同层密度压缩）

**修订决策**：
- SKILL.md 保留 L0-L7 引用，注明"L6/L7 未实现，v2.5+ 计划"
- LOOP_PATTERNS 模式 7（Memory Triage）删除"L4→L5→L6 压缩链"，改为"触发 ForgettingEngine.tick + blackhole.run_pass_archived + MDRM 图谱刷新"

### 共识 3：Loop 经验的层级归属

**问题**：NEBULA_LOOP_DESIGN §7 把 Loop 运行经验塞进星魂（persona 层）。但 Loop 经验是"事实知识"（哪种修复模式有效、哪个 CI 根因），非"人格特质"。

**修订决策**：
- Loop 经验进 **L3 事实层 + MDRM 关系图谱**
- 星魂只吸收"用户对 Loop 自主度的偏好"这种 persona 特质
- 混层会污染 persona 自进化

### 共识 4：Loop 产出强制 provenance

**问题**：Nebula"可追溯"原则要求每条记忆有来源，但 Loop 产出未要求携带 provenance。

**修订决策**：
- Shadow Workspace commit message + STATE.md 已完成项必须含 `loop:<name> | checker:<model> | autonomy:<L?>` 字段
- 让用户能追溯每行代码来源，缓解认知投降

### 共识 5：GitHub MCP 连接器的读写边界

**问题**：T-E-L-04 提出 GitHub MCP 连接器，但 L3"开 Draft PR"是把本地代码上行云端，违反 0 字节上行。

**修订决策**：
- 连接器默认 **pull-only**（读取 Issue/PR/Actions 不违反主权）
- 写操作（push/comment/PR）必须人工触发，Loop 不得自动执行
- L3 自主度仅限"写入本地 Shadow Workspace 分支"

---

## 4. 修订动作清单

### SKILL.md 修订
- [ ] §3 六大要素表：记忆列改"L0-L7（L6/L7 未实现）"
- [ ] §5 自主度阶梯：废弃 L1-L4，改用 L0-L5 映射表
- [ ] §6 故障模式：新增"虚假安全感"（Checker 退化为同模型）
- [ ] §9 关键文件：新增 `loop-safety-guards.md`
- [ ] 新增"现有能力复用"约束章节

### NEBULA_LOOP_DESIGN.md 修订
- [ ] §2.1 架构图：LoopEngine 从独立模块改为 MasterAgent 的 Loop 模式
- [ ] §2.2 目录结构：删除 maker_checker.rs，改为"升级 reviewer.rs"
- [ ] §2.4 STATE.md：改为"SQLite 只读投影"，存储位置 `.nebula/state/`
- [ ] §2.5 Maker-Checker：Checker 强制本地，默认同模型 + 对抗 prompt
- [ ] §3.1 现有任务复用：新增 reviewer.rs / cron_scheduler.rs / ValuesLayer
- [ ] §3.2 新增任务：T-E-L-01 改为"MasterAgent Loop 模式"；T-E-L-02 改为"扩展现有 CronTask"；T-E-L-03 改为"升级 ReviewerAgent"
- [ ] §5 预算管理：拆分本地（$0）/云端两列
- [ ] §7 与双主控融合：Loop 经验进 L3 而非星魂
- [ ] §8 开放问题：问题 3（闭源 Checker）从"待讨论"改为"预先否决"

### LOOP_PATTERNS.md 修订
- [ ] 所有模式 `autonomy:` 字段从 L1-L4 改为 L0-L5
- [ ] 模式 7（Memory Triage）：删除"L4→L5→L6 压缩链"
- [ ] 成本估算：拆分本地 $0 / 云端两列，修正月度总额
- [ ] 自主度提升路径：改名消除 L 碰撞

---

## 5. 修订后的核心架构决策

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
│  │解析     │  │ CronTask)│  │ 只读投影)    │               │
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
                    │  L0-L7 记忆系统  │
                    │  Loop 经验 → L3  │
                    │  persona 偏好 → 星魂 │
                    └─────────────────┘
```

**关键变化**：
1. LoopEngine **不是独立模块**，是 MasterAgent 的 Loop 执行模式
2. 自主度**统一用 L0-L5**，不另建 L1-L4
3. STATE.md 是 **SQLite 只读投影**，不是独立状态源
4. Checker **强制本地同模型**，不依赖闭源 API
5. 每次 Action **必过 ValuesLayer**（L4 价值层），Maker-Checker 是 Allow 之后的追加审查
6. Loop 经验进 **L3 事实层**，不进星魂（persona 层）

---

## 6. 下一步建议

1. **立即修订**三个设计文档（SKILL.md / NEBULA_LOOP_DESIGN.md / LOOP_PATTERNS.md）以消除 4 个致命风险
2. 修订后**重新提交 7 专家快速复审**（仅针对修订点，不全面评审）
3. 复审通过后，将 T-E-L-01~08 任务（修订版）加入 ROADMAP_v2.2.md
4. 启动 T-E-L-01（MasterAgent Loop 模式）实现

---

## 附录：7 专家完整评审意见

### A1. 首席架构师（6/10）
- Q1: LoopEngine 定位自相矛盾，沦为"第四套编排器"
- Q2: 五阶段映射有断层，Adjustment 信号路径不通（主星不直接干活，看不到 Observation）
- Q3: STATE.md 与 LongTaskEngine SQLite 双写，状态来源不明
- Q4: Skill 文件系统与 SOUL/AGENTS/TOOLS 三件套关系未定义
- 致命风险：三套编排器并存导致认知与运维复杂度爆炸

### A2. 记忆系统专家（6/10）
- Q1: 评审前提纠正——实际是 L0-L7（8层），非 L0-L5；但 L6 未实现
- Q2: STATE.md 与 SQLite 的边界未划定（建议 STATE.md 为只读投影）
- Q3: loop-run-log.md 与 RecordingLog 重复（RecordingLog 是纯内存，应先持久化）
- Q4: Memory Triage Loop 部分解决不存在的问题（黑洞压缩已自动触发）
- Q5: STATE.md 存储位置应放 `.nebula/state/`
- 致命风险：L4→L5→L6 压缩链向不存在的 L6 写入

### A3. Agent 编排专家（6/10）
- Q1: reviewer.rs 已存在但文档未提及（应升级而非新建）
- Q2: Maker-Checker 与星尘群是正交关系，非特例（星尘群=优选，Maker-Checker=放行）
- Q3: Checker 应是无 persona 星尘，不应升级为化身
- Q4: 两套停止条件未分层协调
- Q5: 多模型策略与本地优先硬冲突
- 致命风险：虚假安全感陷阱（Checker 退化为同模型自我审查）

### A4. 安全专家（5/10）
- Q1: 自主度阶梯语义冲突（L4 语义倒置）
- Q2: L4 价值层未被复用（Allow/Confirm/Plan/Deny 天然是 Loop 自主度控制）
- Q3: 高风险任务约束存在绕过路径（应下沉到 ValuesLayer 硬编码 Deny 名单）
- Q4: 自主度滑块与 ApprovalGate 关系未定义（滑块只设上限，不绕过下级门）
- Q5: Checker 可被绕过的三个路径（Skill 注入污染 / 共享 STATE.md / 共享上下文投毒）
- 致命风险：L4 语义倒置导致用户误放行 auto-merge

### A5. 性能专家（6/10）
- Q1: Cron 调度应扩展现有 cron_scheduler.rs，不引入 tokio-cron-scheduler
- Q2: 预算检查频率未定义，存在首响风险（用 AtomicU64 内存累加，异步落库）
- Q3: 多 Loop 并发的 git worktree 磁盘与锁竞争（设全局上限 3，共享 target/node_modules）
- Q4: Maker-Checker 模型切换开销击穿"快速循环"（默认同模型不同 persona）
- Q5: 成本估算与"本地 Ollama 0 成本"前提矛盾（拆分本地/云端两列）
- 致命风险：异模型 + 多 worktree 在 16GB 机器触发 RAM 抖荡 + Ollama 反复加载

### A6. 前端/UX 专家（5/10）
- Q1: L1-L4 与现有 AutonomySlider L0-L5 命名碰撞（Loop 改名 T1-T4 或映射 L0-L5）
- Q2: 自主度滑块放哪——文档未决（全局滑块保持现状，Loop 卡片放微型 Tier 选择器）
- Q3: STATE.md 给谁看、怎么看——无 UI 设计（复用 Markdown 视图 + Loop 状态侧栏）
- Q4: 7 种模式用户心智过载（复用 TemplatesDialog，默认只露 2 个入口）
- Q5: Loop 可视化不应等 WorkflowCanvas（拆为 08a 运行时阶段环 P2 + 08b 设计节点 P3）
- 致命风险：L1-L4 与 L0-L5 命名碰撞导致用户崩溃

### A7. 数据主权专家（6/10）
- Q1: Checker 闭源模型悖论（强制本地，"不同模型"用本地不同参数/persona）
- Q2: GitHub MCP 读写边界模糊（默认 pull-only，写操作人工触发）
- Q3: loop-run-log / loop-budget 可读性缺失（强制人类可读 Markdown）
- Q4: Loop 产出缺强制 provenance（commit message + STATE.md 含 loop/checker/autonomy 字段）
- Q5: Loop 经验层级错配（经验进 L3 事实层，星魂只吸收 persona 偏好）
- 致命风险：开放问题 3 若被采纳，Nebula "数据主权"核心承诺破产

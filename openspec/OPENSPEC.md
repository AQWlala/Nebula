# OpenSpec 使用指南 — Nebula 定制版

> **版本**: v1.0
> **日期**: 2026-07-11
> **状态**: 生效中
> **基础**: [OpenSpec](https://github.com/Fission-AI/OpenSpec) by Fission-AI
> **定制方**: Nebula 项目

---

## 一、设计哲学

OpenSpec 在 Nebula 中的落地遵循四条核心哲学。任何对该工作流的偏离都应当能用其中一条解释。

### 1.1 Fluid, not Rigid（流动而非僵化）

Spec 不是合同条款，而是"当前系统的最佳描述"。当现实与 spec 冲突时，优先更新 spec，而不是扭曲实现去迁就过时的文档。

- spec.md 描述的是**当前**行为，不是"理想"行为
- 当代码与 spec 不一致时，二者必有一个是错的——修那个错的
- Delta spec 只描述"什么变了"，不重述未变的部分

### 1.2 Iterative, not Waterfall（迭代而非瀑布）

变更提案不是一次性设计文档，而是随着实现推进逐步细化的活文档。允许 proposal → design → tasks 之间出现轻微的回填和修订。

- proposal.md 可以先写"为什么 + 大致做什么"，design.md 后补技术细节
- tasks.md 在实现过程中持续勾选，允许新增子任务
- Delta spec 在实现稳定后才最终定稿，归档前可调整

### 1.3 Easy, not Complex（简单而非复杂）

不为小改动强加全套工件。一个仅修复拼写错误的 change 不需要 design.md 和记忆影响评估。工件按需创建，由 schema 中的依赖规则约束最小集合。

- 纯文档修正：只需 proposal.md + tasks.md
- 小型 bug 修复：proposal.md + tasks.md + delta.md
- 影响记忆/数据流的变更：追加 memory-impact.md / sovereignty-check.md
- 架构级变更：全套工件

### 1.4 Brownfield-First（棕地优先）

Nebula 已是一个有 v2.3.0 历史、10+ 领域、300+ 模块的成熟项目。OpenSpec 不是"从零设计"，而是"为既有系统建立可追溯的契约"。因此：

- 初始 `specs/` 通过**回填**现有代码行为生成，而非凭空设计
- 早期归档（如 v2.0.0、v2.3.0）以**摘要形式**记录已发生的历史变更
- 与现有 `docs/` 共存：旧文档标记为"参考"，新 spec 为"权威"

---

## 二、目录结构详解

```
openspec/
├── specs/                          # 系统当前行为的"真实来源"
│   ├── memory/spec.md              # 记忆系统行为契约
│   ├── chat/spec.md                # 对话系统行为契约
│   ├── swarm/spec.md               # 蜂群协作行为契约
│   ├── skills/spec.md              # 技能系统行为契约
│   ├── security/spec.md            # 安全防护行为契约
│   ├── llm/spec.md                 # LLM 网关行为契约
│   ├── evolution/spec.md           # 进化系统行为契约
│   ├── os/spec.md                  # OS 控制行为契约
│   ├── sync/spec.md                # 多设备同步行为契约
│   ├── ui/spec.md                  # 前端 UI 行为契约
│   └── project/spec.md             # 项目级约束(数据主权/本地优先)
│
├── changes/                        # 进行中的修改
│   ├── <change-name>/              # 每个修改一个文件夹
│   │   ├── proposal.md
│   │   ├── design.md
│   │   ├── tasks.md
│   │   ├── memory-impact.md        # (Nebula 专用,按需)
│   │   ├── sovereignty-check.md    # (Nebula 专用,按需)
│   │   └── specs/<domain>/delta.md
│   │
│   └── archive/                    # 已归档的修改
│       ├── v2.0.0-feature-completion/
│       └── v2.3.0-macos-redesign/
│
├── versions/                       # 版本迭代文件(双轨制)
│   ├── major/                      # 大版本 (功能/架构/里程碑)
│   └── minor/                      # 小版本 (Bug/测试/文档/配置)
│
├── schema/
│   └── nebula.schema.md            # Nebula 专用工作流 schema
│
├── OPENSPEC.md                     # 本文件
└── README.md                       # 目录说明
```

### 2.1 specs/ — 真实来源

每个领域一个 `spec.md`，描述该领域**当前**对外承诺的行为。它是契约，不是教程。读者默认具备 Nebula 基础知识。

- 修改 specs/ 的唯一合法途径：通过 change 归档时合并 delta
- 禁止直接编辑 specs/ —— 这会破坏可追溯性
- spec.md 顶部的"最后更新"字段指向最近一次合并的 change

### 2.2 changes/ — 变更提案

进行中的修改存放于此。命名规则：`<动词>-<对象>` 或 `<版本>-<主题>`。

- 进行中的 change：`changes/add-l6-distillation/`
- 已归档的 change：`changes/archive/v2.3.0-macos-redesign/`

### 2.3 versions/ — 版本迭代

版本文件是"一次发布的汇总"，不是"又一次设计文档"。它引用归档的 changes，列出新增/修改/破坏性变更，附验收清单。

### 2.4 schema/ — 工作流 schema

定义工件的种类、依赖关系、验证规则。详见 `schema/nebula.schema.md`。

---

## 三、Spec 文件格式说明

### 3.1 行为契约 spec.md

```markdown
# [领域名] 行为契约

> **领域**: memory
> **状态**: 当前系统行为
> **最后更新**: 2026-07-11 (merged from change: add-l5-reflection)

## 概述
[一段话描述该领域的核心职责]

## Requirements

### Requirement: 记忆层级
The system MUST support L0-L5 memory layers.
- L0: 原始上下文 (短期)
- L1: 对话摘要 (中期)
- ...

#### Scenario: L0→L1 自动压缩
- **WHEN** 对话超过 20 轮
- **THEN** 系统自动生成 L1 摘要
- **AND** 清理 L0 中的已压缩内容
```

**关键约定**：

- `## Requirements` 下每个 `### Requirement: <名称>` 是一个独立契约
- 使用 RFC 2119 关键词：MUST / SHALL / SHOULD / MAY
- 每个 Requirement 至少配一个 `#### Scenario`，用 WHEN/THEN/AND 描述可验证的行为
- Scenario 必须是**可测试的**——能映射到一个测试用例

### 3.2 Spec 编写要点

| 要点 | 正确示范 | 错误示范 |
|------|---------|---------|
| 描述行为，不描述实现 | "系统 SHALL 在对话超 20 轮时压缩" | "系统用 summarize() 函数压缩" |
| 可验证 | "首响时间 SHOULD < 500ms" | "系统应该尽量快" |
| 单一职责 | 一个 Requirement 一件事 | "系统处理记忆、压缩和检索" |
| 不重复 | L0-L5 只在"记忆层级"定义一次 | 每个 Scenario 都重述层级定义 |

---

## 四、Delta Spec 格式说明

Delta spec 描述"相对于当前 specs/，本次变更改了什么"。它**不重述**未变的部分。

```markdown
# Delta for Memory

## ADDED Requirements

### Requirement: L6 蒸馏层
The system SHALL distill L4 facts into compressed semantic capsules.
- 蒸馏触发条件: L3 容量超过 80%
- 蒸馏输出: 语义胶囊(<=512 tokens)

#### Scenario: L3 容量超限触发蒸馏
- **WHEN** L3 存储容量超过 80%
- **THEN** 系统启动 L6 蒸馏流程
- **AND** 生成语义胶囊写入 L6

## MODIFIED Requirements

### Requirement: 记忆层级
The system MUST support L0-L6 layers. (Previously: L0-L5)
- L0-L5: 不变
- L6: 蒸馏层(新增)

## REMOVED Requirements

(none)
```

### 4.1 三种操作

| 操作 | 含义 | 合并到 specs/ 时的行为 |
|------|------|----------------------|
| `ADDED` | 新增 Requirement | 追加到对应领域的 `## Requirements` 末尾 |
| `MODIFIED` | 修改已有 Requirement | 替换同名 Requirement（保留名称，更新内容） |
| `REMOVED` | 删除 Requirement | 从 specs/ 中删除同名 Requirement |

### 4.2 Delta 编写要点

- **MODIFIED 必须注明 (Previously: ...)**，让审阅者一眼看出变化
- 如果只是给 Requirement 新增一个 Scenario，用 MODIFIED 并保留原有 Scenario
- 如果一个 Requirement 被彻底重写，仍然用 MODIFIED（名称不变）而非 REMOVED+ADDED
- `## REMOVED Requirements` 下如果无内容，写 `(none)` 而非留空

### 4.3 Delta 的领域归属

Delta 文件放在 `changes/<name>/specs/<domain>/delta.md`。一个 change 可以同时修改多个领域：

```
changes/add-l6-distillation/specs/
├── memory/delta.md      # 新增 L6 层
└── llm/delta.md         # 调整 dispatcher 以支持蒸馏任务
```

---

## 五、五步工作流详解

```
nebula:explore ──► nebula:propose ──► nebula:apply ──► nebula:verify ──► nebula:archive
   (探索)           (提案+规划)         (实现)            (验证)            (归档)
```

### Step 1: Explore（探索）

**目标**：理解现状，明确"要改什么、为什么改"。

**动作**：
- 读取 `openspec/specs/<domain>/spec.md` 了解当前行为
- 如果该领域尚无 spec，阅读相关源码和文档，准备回填
- 识别要修改的 Requirement 或要新增的能力
- **不创建任何文件**

**产出**：一份口头/笔记形式的"变更意图"陈述。

**退出条件**：能清晰回答"我要改哪个领域的哪些 Requirement"。

### Step 2: Propose（提案）

**目标**：把变更意图固化为可审阅的工件。

**动作**：在 `openspec/changes/<change-name>/` 创建：

| 工件 | 必需性 | 内容 |
|------|--------|------|
| `proposal.md` | 必需 | 为什么 + 做什么（不含技术细节） |
| `design.md` | 大变更必需 | 怎么做（技术方案、数据流、模块划分） |
| `tasks.md` | 必需 | 实现清单（T-XX 编号，可勾选） |
| `specs/<domain>/delta.md` | 必需 | ADDED/MODIFIED/REMOVED |
| `memory-impact.md` | 影响记忆时必需 | 见第六节 |
| `sovereignty-check.md` | 影响数据流时必需 | 见第六节 |

**命名规则**：`<动词>-<对象>`（如 `add-l6-distillation`）或 `<版本>-<主题>`（历史回填用，如 `v2.3.0-macos-redesign`）。

**退出条件**：所有必需工件创建完成，delta.md 通过 schema 验证。

### Step 3: Apply（实现）

**目标**：按 tasks.md 实现代码。

**动作**：
- 按 `tasks.md` 中的任务顺序实现
- 每完成一个任务，将其 `[ ]` 改为 `[x]`
- 如果实现中发现新任务，追加到 tasks.md（编号递增）
- 如果 design.md 与实现出现偏差，回填 design.md

**退出条件**：tasks.md 中所有任务标记为 `[x]`。

### Step 4: Verify（验证）

**目标**：确认实现符合 spec 契约。

**动作**：
- 运行 `cargo test --lib`
- 运行 `npm test`
- 运行 `tsc --noEmit`
- 确认 CI 全绿
- 人工核对 delta.md 中每个 Scenario 都有对应测试覆盖

**退出条件**：所有测试通过，CI 全绿。

### Step 5: Archive（归档）

**目标**：合并变更到主 specs，记录版本。

**动作**：
1. 将 `changes/<change-name>/` 移到 `changes/archive/<change-name>/`
2. 创建 `archived-at.md`，记录归档时间和合并到的版本
3. 将 Delta specs 合并到主 specs：
   - `ADDED` → 追加到 `specs/<domain>/spec.md` 的 `## Requirements`
   - `MODIFIED` → 替换同名 Requirement
   - `REMOVED` → 删除同名 Requirement
4. 更新 `specs/<domain>/spec.md` 顶部的"最后更新"字段
5. 在 `versions/major/` 或 `versions/minor/` 创建版本迭代文件（见第七节）

**退出条件**：归档完成，specs/ 已更新，版本文件已创建。

### 并行处理

多个 change 可并行进行，前提是它们修改**不同的领域**：

- `add-l6-distillation` 修改 `memory/` + `llm/`
- `fix-swarm-deadlock` 修改 `swarm/`
- 两者不冲突，可同时处于进行中

如果两个 change 都要修改同一领域的同一 Requirement，后提出者必须等待前者归档后再基于新 specs/ 重新探索。

---

## 六、Nebula 专用 Schema

在标准 OpenSpec 工件基础上，Nebula 加入两个专用工件，用于守护项目的两条红线：**记忆完整性**和**数据主权**。

### 6.1 记忆影响评估 — `memory-impact.md`

**触发条件**（满足任一即需创建）：
- 变更触及 `src-tauri/src/memory/` 下任何代码
- 变更修改记忆相关的 Tauri 命令或前端 store
- 变更影响 L0-L7 任一层级的读写行为
- 变更修改记忆的存储格式、加密方式、压缩策略

**格式**：

```markdown
# 记忆影响评估

> **变更**: add-l6-distillation
> **评估日期**: 2026-07-11

## 影响的层级
- [ ] L0 (原始上下文)
- [ ] L1 (对话摘要)
- [ ] L2 (知识抽取)
- [x] L3 (事实记忆) — 蒸馏读取 L3 作为输入
- [ ] L4 (价值观记忆)
- [ ] L5 (反思记忆)
- [x] L6 (蒸馏层) — 新增层,写入语义胶囊

## 影响描述
L3: 蒸馏引擎以只读方式访问 L3 事实,不修改 L3 内容...
L6: 新增层级,存储格式为...

## 回滚策略
若蒸馏导致记忆退化:
1. 禁用 L6 蒸馏引擎(config: memory.distillation.enabled = false)
2. L6 已生成的语义胶囊保留但不再参与检索
3. 通过 version_control 回滚 specs/memory/spec.md
```

### 6.2 数据主权检查 — `sovereignty-check.md`

**触发条件**（满足任一即需创建）：
- 变更引入新的网络请求或第三方服务依赖
- 变更修改数据存储位置（本地/云端）
- 变更触及 E2EE 加密链路
- 变更涉及用户数据导出/同步
- 变更引入新的 AI 模型或 Checker

**格式**：

```markdown
# 数据主权检查

> **变更**: add-cloud-sync-provider
> **检查日期**: 2026-07-11

## 检查项
- [x] 变更不会将用户数据发送到第三方服务器
      说明: 新增的 S3 同步仅在用户显式配置后启用,数据经 E2EE 加密
- [x] 变更不会引入新的云端依赖(除非用户显式配置)
      说明: S3 为可选 provider,默认禁用
- [x] 变更不会绕过现有的 E2EE 加密
      说明: 同步前强制经过 sync/e2ee.rs 加密链路
- [x] 变更不会降低本地优先架构的保证
      说明: 本地 SQLite 仍为权威存储,云端仅为副本
- [x] 变更不会引入闭源 Checker 模型
      说明: 未引入任何 Checker

## 红线确认
本变更**不违反** Nebula 数据主权红线。
```

### 6.3 工件依赖关系

工件的创建有先后依赖。详见 `schema/nebula.schema.md`。简言之：

```
proposal.md ──► design.md ──► tasks.md
                    │
                    ├─► specs/<domain>/delta.md
                    │
                    ├─► memory-impact.md     (如触及记忆)
                    └─► sovereignty-check.md (如触及数据流)
```

---

## 七、版本迭代双轨制说明

Nebula 采用 SemVer 的 `MAJOR.MINOR.PATCH`，但 OpenSpec 只跟踪后两段：

```
MAJOR.MINOR.PATCH
  │      │      │
  │      │      └── 小版本: Bug/测试/文档/配置 → versions/minor/
  │      └───────── 大版本: 功能/架构/里程碑 → versions/major/
  └────────────────── 主版本: 破坏性变更(手动管理,极少触发)
```

### 7.1 大版本 (Major) — `versions/major/`

**触发条件**（满足任一）：
- 新功能模块（如新增 L6 蒸馏层）
- 架构级变更（如 MasterOrchestrator 重构）
- 破坏性 API 变更
- 重大 UI 重设计
- 里程碑达成

**文件格式**：

```markdown
# v2.4.0 — 知识蒸馏 + 记忆深化

> **发布日期**: 2026-07-XX
> **类型**: 大版本
> **前一版本**: v2.3.1
> **变更范围**: memory/, llm/

## 变更摘要
[一段话]

## 包含的 Changes
- [add-l6-distillation](../changes/archive/add-l6-distillation/)

## 新增功能
1. L6 蒸馏层 — 自动压缩 L3 事实为语义胶囊

## 修改的功能
1. 记忆层级从 L0-L5 扩展为 L0-L6

## 破坏性变更
(none)

## 验收标准
- [x] cargo test --lib 全通过
- [x] npm test 全通过
- [x] CI 全绿
```

### 7.2 小版本 (Minor) — `versions/minor/`

**触发条件**（满足任一）：
- Bug 修复（不影响 API）
- 测试修复
- 文档修正
- 依赖更新
- 配置调整
- CI 修复

**文件格式**：

```markdown
# v2.3.1 — 测试修复 + Dependabot lru 忽略

> **发布日期**: 2026-07-11
> **类型**: 小版本
> **前一版本**: v2.3.0
> **变更范围**: 测试文件 + .github/dependabot.yml

## 变更摘要
修复 v2.3.0 macOS 重设计后 5 个前端测试失败。

## 修复列表
1. ChatPanel.test.tsx: streaming 测试改用 .chat-send-btn 选择器
2. ...

## 关联
- CI Run: 29132407863
- 提交: c124f67
```

### 7.3 版本号规则

- **大版本示例**: 1.0→1.1, 1.2, 2.0, 2.1, 2.2, 2.3, 2.4
- **小版本示例**: 1.0.1, 1.0.2, 2.0.1, 2.3.1, 2.3.2
- 小版本总是依附于某个大版本：`v2.3.1` 依附于 `v2.3.0`
- 一个大版本周期内可以有 0 个或多个小版本

### 7.4 版本迭代文件的触发条件

版本文件**不是**每次归档 change 都创建。规则如下：

| 情况 | 动作 |
|------|------|
| 单个 change 归档，且属于大版本范畴 | 创建/更新一个 `versions/major/vX.Y.0.md` |
| 单个 change 归档，且属于小版本范畴 | 创建/更新一个 `versions/minor/vX.Y.Z.md` |
| 多个 change 在同一发布周期归档 | 合并到同一个版本文件，`## 包含的 Changes` 列多条 |
| change 归档但未发布 | 仅归档，不创建版本文件；待发布时再创建 |

---

## 八、与现有文档的关系

Nebula 已有大量 `docs/` 文档。OpenSpec **不取代**它们，而是建立**权威契约层**。

| 现有文档 | OpenSpec 对应 | 关系 |
|---------|--------------|------|
| `ROADMAP_v3.1.md` | `specs/`（各领域） | ROADMAP 的功能描述提炼为行为契约 |
| `WHITEPAPER_v3.2.md` | `specs/project/` | 白皮书的项目级约束提取为 project spec |
| `CHANGELOG.md` | `versions/major/` + `versions/minor/` | 变更日志拆分为结构化版本文件 |
| `ARCHITECTURE.md` | `specs/`（各领域） | 架构契约分散到各领域 spec |
| `ADR-001~004` | `changes/archive/` | ADR 归档为已完成的 change |
| `DEVELOPMENT_PROPOSAL_v3.0.md` | `changes/`（进行中） | 开发提案转为进行中的 change |
| `SPEC_DRIVEN_DESIGN.md` | `OPENSPEC.md`（本文件） | 本文件是其落地实现 |

**原则**：
- 现有文档**不删除**
- 新 spec 为**权威来源**
- 旧文档逐步在顶部标注 "> ⚠️ 参考文档，权威契约见 `openspec/specs/<domain>/spec.md`"
- 当二者冲突时，以 `openspec/` 为准

---

## 九、FAQ

### Q1: 我只想改一个拼写错误，也要走五步工作流吗？

A: 不需要全套。最小集合是 `proposal.md` + `tasks.md`，归档时创建一个小版本文件。design.md、delta.md、memory-impact.md 等按需创建（见第三节"Easy, not Complex"）。

### Q2: specs/ 里没有我想改的领域怎么办？

A: 说明该领域尚未回填。你可以：
1. 先基于现有代码和文档，创建该领域的初始 `specs/<domain>/spec.md`（作为一个独立的"回填 change"）
2. 然后基于新 spec 提出你的变更 change

### Q3: Delta spec 里的 Scenario 必须有对应测试吗？

A: 是的。Scenario 的本质是"可验证的行为"。归档前的 Verify 步骤会人工核对每个 Scenario 是否有测试覆盖。如果暂未实现测试，应在 tasks.md 中补一个测试任务。

### Q4: 多个 change 修改同一领域怎么办？

A: 串行处理。后提出者必须等前者归档后，基于更新后的 `specs/` 重新探索。这是为了保证 delta 的基线一致。

### Q5: 归档后发现 delta 有错误怎么办？

A: 不要直接改 archive 里的文件。新开一个 change（如 `fix-xxx`），其 delta 描述对前次变更的修正。这保留了完整的追溯链。

### Q6: memory-impact.md 和 sovereignty-check.md 什么时候必须创建？

A: 见第六节的触发条件。简言之：触及 `src-tauri/src/memory/` 或记忆读写 → memory-impact.md；引入网络请求/第三方依赖/修改加密链路 → sovereignty-check.md。不确定时，宁可多创建。

### Q7: 历史归档（如 v2.0.0、v2.3.0）为什么内容是摘要形式？

A: 因为它们是**回填**的——变更早已发生，无法还原完整的设计讨论。因此以"事后总结"形式记录，重点是保留可追溯性，而非重现决策过程。未来的 change 会是完整的实时记录。

### Q8: OpenSpec 和 Git commit 有什么区别？

A: Git commit 记录"代码怎么变的"，OpenSpec 记录"行为契约怎么变的"。一个重构可能产生 50 个 commit 但只有 1 个 delta（行为不变）。反之，一个 1 行的 commit 可能对应一个重要 delta（改变了对外承诺）。

### Q9: 我可以直接编辑 specs/ 吗？

A: 不可以。specs/ 的唯一合法修改途径是通过 change 归档时合并 delta。直接编辑会破坏可追溯性，使"最后更新"字段失效。

### Q10: 版本文件和 CHANGELOG.md 重复吗？

A: 不重复。CHANGELOG.md 是面向用户的"有什么新功能"的通俗列表；版本文件是面向开发者的"哪些 spec 契约变了、包含哪些 change"的结构化记录。前者是营销，后者是工程。

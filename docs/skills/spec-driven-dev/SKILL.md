---
name: spec-driven-dev
version: 1.0.0
description: |
  Spec-driven development 技能——在写代码前先对齐"要建什么",通过行为契约和
  Delta Specs 管理变更。当涉及需求对齐、行为契约编写、变更提案、Delta spec、
  版本迭代文件、openspec/ 目录操作时加载此技能。本技能把 Nebula 从"版本号驱动
  + 散乱文档"升级为"规范驱动"的开发模式,通过五步工作流(explore → propose →
  apply → verify → archive)实现"先写规范,再写代码"的工程方法论。
language: markdown
author: Nebula Project
source: |
  - https://github.com/Fission-AI/OpenSpec
  - Nebula 内部文档: docs/SPEC_DRIVEN_DESIGN.md
status: stable
commands:
  - nebula:explore
  - nebula:propose
  - nebula:apply
  - nebula:verify
  - nebula:archive
capabilities: ["filesystem:read", "filesystem:write", "exec:shell"]
transport: local
dependencies: []
eligibility:
  bins: []
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Spec-Driven Development 技能（Nebula 内化版）

## 1. 概述

Spec-driven development 是一种"先写规范,再写代码"的开发方法论。在修改代码前,
先在 `openspec/` 目录中描述要改什么、为什么改、怎么改,然后按规范实现。

> **核心理念**: 与其让 Agent 在代码里猜测意图,不如先在规范里把意图说清楚。
> 规范是"要建什么"的契约,代码是"建出来什么"的实现。

```
传统开发:       想法 → 直接写代码 → 测试 → 文档(可选)
Spec-driven:    想法 → 写规范 → 评审规范 → 实现代码 → 验证 → 归档
                       ↑契约对齐                    ↑验证契约
```

本技能把 OpenSpec 的方法论集成到 Nebula 的技能系统中,通过五个 CLI 命令
驱动整个工作流,并加入 Nebula 专用的记忆影响评估和数据主权检查工件。

## 2. 核心概念

| 概念 | 说明 | 存放位置 |
|------|------|---------|
| **Spec (行为契约)** | 描述系统当前应该做什么的结构化文档,是单一真实来源(source of truth) | `openspec/specs/<domain>/spec.md` |
| **Change (变更提案)** | 描述要改什么的一个完整提案,包含 why/what/how/tasks/delta | `openspec/changes/<change-name>/` |
| **Delta Spec** | 只描述"什么变了"(ADDED/MODIFIED/REMOVED),不重写整个文档 | `openspec/changes/<change-name>/specs/<domain>/delta.md` |
| **版本迭代文件** | 记录每个版本包含的变更内容,双轨制(大版本 + 小版本) | `openspec/versions/major/` 或 `openspec/versions/minor/` |
| **记忆影响评估** | Nebula 专用: 评估变更对 L0-L6 记忆层的影响 | `openspec/changes/<change-name>/memory-impact.md` |
| **数据主权检查** | Nebula 专用: 确认变更不违反数据主权红线 | `openspec/changes/<change-name>/sovereignty-check.md` |

### 2.1 openspec/ 目录结构

```
openspec/
├── specs/                          # 系统当前行为的"真实来源"
│   ├── memory/
│   │   └── spec.md                 # 记忆系统行为契约
│   ├── chat/
│   │   └── spec.md                 # 对话系统行为契约
│   ├── swarm/
│   │   └── spec.md                 # 蜂群协作行为契约
│   ├── skills/
│   │   └── spec.md                 # 技能系统行为契约
│   ├── security/
│   │   └── spec.md                 # 安全防护行为契约
│   ├── llm/
│   │   └── spec.md                 # LLM 网关行为契约
│   ├── evolution/
│   │   └── spec.md                 # 进化系统行为契约
│   ├── os/
│   │   └── spec.md                 # OS 控制行为契约
│   ├── sync/
│   │   └── spec.md                 # 多设备同步行为契约
│   ├── ui/
│   │   └── spec.md                 # 前端 UI 行为契约
│   └── project/
│       └── spec.md                 # 项目级约束(数据主权/本地优先等)
│
├── changes/                        # 提议中的修改(每个修改一个文件夹)
│   ├── <change-name>/              # 进行中的 change
│   │   ├── proposal.md
│   │   ├── design.md
│   │   ├── tasks.md
│   │   ├── memory-impact.md        # Nebula 专用
│   │   ├── sovereignty-check.md    # Nebula 专用
│   │   └── specs/
│   │       └── <domain>/
│   │           └── delta.md
│   │
│   └── archive/                    # 已完成的修改归档
│       └── <change-name>/
│           ├── proposal.md
│           ├── design.md
│           ├── tasks.md
│           ├── specs/
│           │   └── <domain>/
│           │       └── delta.md
│           └── archived-at.md      # 归档时间 + 合并到的版本
│
├── versions/                       # 版本迭代文件(双轨制)
│   ├── major/                      # 大版本 (新功能/架构变更/里程碑)
│   │   ├── v1.0.0.md
│   │   ├── v2.0.0.md
│   │   └── v2.4.0.md
│   └── minor/                      # 小版本 (Bug修复/测试/文档/配置)
│       ├── v2.3.1.md
│       └── README.md
│
├── schema/
│   └── nebula.schema.md            # Nebula 专用工作流 schema
│
├── OPENSPEC.md                     # OpenSpec 使用指南(Nebula 定制版)
└── README.md                       # openspec/ 目录说明
```

### 2.2 领域划分

Nebula 划分 10 个领域,每个领域一个 `spec.md`:

| 领域 | 职责 | 典型 Requirement 示例 |
|------|------|----------------------|
| `memory` | 记忆系统(L0-L6 分层、黑洞压缩、海绵吸收) | "记忆层级"、"压缩引擎"、"遗忘策略" |
| `chat` | 对话系统(消息流、流式响应、多模型切换) | "消息持久化"、"流式输出"、"上下文窗口" |
| `swarm` | 蜂群协作(Maker-Checker、SwarmOrchestrator) | "并行编排"、"Negotiator 仲裁"、"死锁检测" |
| `skills` | 技能系统(SkillComposer、加载、注入) | "技能加载"、"上下文注入"、"技能组合" |
| `security` | 安全防护(ValuesLayer、ApprovalGate、红线) | "价值层裁定"、"审批门"、"数据主权红线" |
| `llm` | LLM 网关(多 provider、模型路由、token 计量) | "provider 切换"、"模型同质检测"、"预算守护" |
| `evolution` | 进化系统(persona 自进化、CronScheduler) | "定时任务"、"persona 演进"、"自反思" |
| `os` | OS 控制(Tauri 命令、文件系统、进程) | "命令执行"、"文件操作"、"权限隔离" |
| `sync` | 多设备同步(E2EE、CRDT、冲突解决) | "端到端加密"、"冲突合并"、"离线同步" |
| `ui` | 前端 UI(React 组件、状态管理、样式) | "组件规范"、"状态流"、"主题系统" |
| `project` | 项目级约束(数据主权、本地优先、开闭源边界) | "本地优先"、"0 字节上行"、"开源边界" |

## 3. 五步工作流

```
nebula:explore ──► nebula:propose ──► nebula:apply ──► nebula:verify ──► nebula:archive
   (探索)           (提案+规划)         (实现)            (验证)            (归档)
```

详细说明见 [WORKFLOW.md](./WORKFLOW.md)。

### 3.1 命令速查

| 命令 | 输入 | 输出 | 副作用 |
|------|------|------|--------|
| `nebula:explore` | 领域名(可选) | 当前 spec 内容 | 无(只读) |
| `nebula:propose` | change 名称 + 领域 + 描述 | 创建 change 文件夹和初始文件 | 创建 `openspec/changes/<name>/` |
| `nebula:apply` | change 名称 | 按 tasks.md 实现,更新完成状态 | 修改代码 + 更新 tasks.md |
| `nebula:verify` | change 名称(可选) | 测试结果报告 | 无(只读,仅运行测试) |
| `nebula:archive` | change 名称 + 版本号 + 大/小版本 | 归档 + 合并 delta + 创建版本文件 | 移动文件夹 + 修改 specs + 创建版本文件 |

## 4. Delta Spec 编写指南

Delta spec 只描述"什么变了",使用三种语义标记:

- **ADDED** — 新增的 requirement
- **MODIFIED** — 修改现有 requirement 的行为
- **REMOVED** — 删除的 requirement

详细编写规范见 [DELTA_SPEC_GUIDE.md](./DELTA_SPEC_GUIDE.md)。

## 5. Nebula 专用工件

在标准 OpenSpec 工件基础上,Nebula 加入两个专用工件,用于守护 Nebula 的核心价值:

### 5.1 记忆影响评估 (`memory-impact.md`)

评估变更对 L0-L6 记忆层的影响。任何涉及记忆系统的 change 必须填写此工件。

| 层级 | 名称 | 评估重点 |
|------|------|---------|
| L0 | 原始上下文 | 是否影响短期上下文窗口管理 |
| L1 | 对话摘要 | 是否改变摘要生成策略 |
| L2 | 知识抽取 | 是否修改知识抽取流程 |
| L3 | 事实记忆 | 是否影响持久化事实存储 |
| L4 | 价值观记忆 | 是否触及宪法层价值观 |
| L5 | 反思记忆 | 是否修改元认知流程 |
| L6 | 蒸馏层 | 是否影响语义胶囊蒸馏(如存在) |

模板见 [templates/memory-impact.md](./templates/memory-impact.md)。

### 5.2 数据主权检查 (`sovereignty-check.md`)

确认变更不违反数据主权红线。**任何 change 必须填写此工件**——不仅限于涉及数据的 change,
因为即便看似无关的变更(如新增依赖)也可能引入隐蔽数据流。

检查项(全部必须为 ✅,否则 change 不得归档):

- 变更不会将用户数据发送到第三方服务器
- 变更不会引入新的云端依赖(除非用户显式配置)
- 变更不会绕过现有的 E2EE 加密
- 变更不会降低本地优先架构的保证
- 变更不会引入闭源 Checker 模型

模板见 [templates/sovereignty-check.md](./templates/sovereignty-check.md)。

> ⚠️ **红线依据**: 数据主权是 Nebula 的核心承诺——"0 字节上行是默认态"。
> Checker 处理代码 diff,发给云端 = "把你的代码变成别人的养料"。
> 即便 change 看似与数据无关,新增依赖也可能引入隐蔽数据流,因此强制全量检查。

## 6. 版本迭代双轨制

| 类型 | 触发条件 | 存放位置 | 版本号变化 |
|------|---------|---------|-----------|
| **大版本 (Major)** | 新功能模块 / 架构级变更 / 破坏性 API 变更 / 重大 UI 重设计 / 里程碑达成 | `versions/major/` | SemVer 的 minor 位 +1 |
| **小版本 (Minor)** | Bug 修复 / 测试修复 / 文档修正 / 依赖更新 / 配置调整 / CI 修复 | `versions/minor/` | SemVer 的 patch 位 +1 |

```
MAJOR.MINOR.PATCH
  │      │      │
  │      │      └── 小版本: versions/minor/
  │      └───────── 大版本: versions/major/
  └────────────────── 主版本: 破坏性变更(手动管理,极少触发)
```

- 大版本示例: 1.0→1.1, 1.2, 2.0, 2.1, 2.2, 2.3, 2.4
- 小版本示例: 1.0.1, 1.0.2, 2.0.1, 2.3.1, 2.3.2

## 7. 使用示例

### 7.1 场景: 添加 L6 蒸馏层

**背景**: 当前记忆系统支持 L0-L5,需要新增 L6 蒸馏层,自动压缩 L3 事实为语义胶囊。

**Step 1: 探索**

```bash
nebula:explore --domain memory
```

读取 `openspec/specs/memory/spec.md`,确认当前 "记忆层级" requirement 描述的是 L0-L5。

**Step 2: 提案**

```bash
nebula:propose --name add-l6-distillation --domain memory \
  --description "添加 L6 蒸馏层,自动压缩 L3 事实为语义胶囊"
```

生成 `openspec/changes/add-l6-distillation/` 目录,包含:
- `proposal.md` — 阐述为什么需要 L6(解决 L3 膨胀问题)+ 做什么(新增蒸馏层)
- `design.md` — 技术方案:BlackholeEngine 扩展、蒸馏触发条件、胶囊格式
- `tasks.md` — 实现清单:8 个任务(后端 trait + 实现 + 测试 + 文档)
- `specs/memory/delta.md` — Delta spec:ADDED "L6 蒸馏层"、MODIFIED "记忆层级"
- `memory-impact.md` — 勾选 L3、L6,描述影响
- `sovereignty-check.md` — 全部 ✅(蒸馏在本地完成)

**Step 3: 实现**

```bash
nebula:apply --name add-l6-distillation
```

按 `tasks.md` 逐项实现,完成后勾选 `- [x]`。

**Step 4: 验证**

```bash
nebula:verify --name add-l6-distillation
```

运行 `cargo test --lib`、`npm test`、`tsc --noEmit`,确认全绿。

**Step 5: 归档**

```bash
nebula:archive --name add-l6-distillation --version 2.4.0 --major
```

- 移动 `changes/add-l6-distillation/` → `changes/archive/add-l6-distillation/`
- 创建 `archived-at.md` 记录归档时间和合并到的版本
- 合并 Delta specs 到 `openspec/specs/memory/spec.md`(ADDED 追加 / MODIFIED 替换)
- 创建 `openspec/versions/major/v2.4.0.md` 版本迭代文件

### 7.2 并行处理

多个 change 可以并行进行,只要它们修改不同的领域:

```
add-l6-distillation   → 修改 memory/      ✅ 可并行
fix-swarm-deadlock    → 修改 swarm/       ✅ 可并行
optimize-llm-cache    → 修改 llm/         ✅ 可并行
update-ui-theme       → 修改 ui/          ✅ 可并行
```

若两个 change 修改同一领域,后归档的 change 需重新 `explore` 确认 spec 已更新,再调整自己的 delta。

## 8. 加载时机

本技能应在以下场景自动加载:

- 用户提到"规范驱动"、"spec-driven"、"行为契约"、"Delta spec"
- 用户要在 `openspec/` 目录创建或修改文件
- 用户要求"先写规范再写代码"、"对齐需求后再实现"
- 用户讨论变更管理、版本迭代、change 提案
- 涉及记忆影响评估或数据主权检查
- 用户调用 `nebula:explore` / `nebula:propose` / `nebula:apply` / `nebula:verify` / `nebula:archive` 命令

## 9. 关键文件结构

```
docs/skills/spec-driven-dev/
├── SKILL.md                # 本文件(Agent 加载入口)
├── WORKFLOW.md             # 五步工作流详细说明(输入/输出/检查点)
├── DELTA_SPEC_GUIDE.md     # Delta spec 编写指南(ADDED/MODIFIED/REMOVED 语义 + 命名规范)
└── templates/
    ├── proposal.md         # 提案模板(为什么 + 做什么)
    ├── design.md           # 设计模板(技术方案)
    ├── tasks.md            # 任务清单模板(可勾选)
    ├── delta.md            # Delta spec 模板
    ├── memory-impact.md    # 记忆影响评估模板(Nebula 专用)
    └── sovereignty-check.md # 数据主权检查模板(Nebula 专用)
```

## 10. 与现有文档的关系

| 现有文档 | openspec/ 对应 | 关系 |
|---------|---------------|------|
| `docs/SPEC_DRIVEN_DESIGN.md` | 整个 openspec/ | 设计方案文档,openspec/ 是其落地实现 |
| `docs/ROADMAP_v3.1.md` | `openspec/specs/`(各领域) | ROADMAP 功能描述提炼为行为契约 |
| `docs/WHITEPAPER_v3.2.md` | `openspec/specs/project/` | 白皮书项目级约束提取为 project spec |
| `CHANGELOG.md` | `versions/major/` + `versions/minor/` | 变更日志拆分为版本迭代文件 |
| `docs/ARCHITECTURE.md` | `openspec/specs/`(各领域) | 架构文档契约分散到各领域 spec |
| `docs/skills/loop-engineering/` | `openspec/specs/swarm/` + `openspec/specs/evolution/` | Loop Engineering 的行为契约归入对应领域 |

**现有文档不删除**,openspec/ 作为权威来源,旧文档逐步标记为"参考"。

---

*本技能内化自 OpenSpec 方法论,结合 Nebula 双主控 + 蜂群 worker + persona 自进化架构适配。版本 1.0.0,2026-07-11。*

# Nebula Spec-Driven Development 改造方案

> **版本**: v1.0
> **日期**: 2026-07-11
> **状态**: 设计中
> **参考**: [OpenSpec](https://github.com/Fission-AI/OpenSpec) by Fission-AI

---

## 一、改造目标

将 Nebula 从**版本号驱动 + 散乱文档**的模式，升级为 **spec-driven（规范驱动）** 的开发模式：

1. **建立单一真实来源** — `openspec/specs/` 存放系统当前行为的结构化契约
2. **引入 Delta Specs** — 修改不再重写整个文档，只描述"什么变了"
3. **双轨版本迭代** — 大版本(major) + 小版本(minor) 双轨制
4. **集成到 Nebula 代码** — 通过技能系统和 CLI 命令，让 Agent 能执行 spec 工作流

---

## 二、openspec/ 目录结构

```
openspec/
├── specs/                          # 系统当前行为的"真实来源"(source of truth)
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
│   ├── add-l6-distillation/        # 示例: 添加 L6 蒸馏层
│   │   ├── proposal.md             # 为什么 + 做什么
│   │   ├── design.md               # 怎么做(技术方案)
│   │   ├── tasks.md                # 实现清单
│   │   └── specs/                  # Delta specs(描述什么改变了)
│   │       └── memory/
│   │           └── delta.md        # ADDED/MODIFIED/REMOVED
│   │
│   └── archive/                    # 已完成的修改归档
│       └── v2.3.0-macos-redesign/  # 示例: v2.3 macOS 重设计
│           ├── proposal.md
│           ├── design.md
│           ├── tasks.md
│           ├── specs/
│           │   └── ui/
│           │       └── delta.md
│           └── archived-at.md      # 归档时间 + 合并到的版本
│
├── versions/                       # 版本迭代文件(双轨制)
│   ├── major/                      # 大版本更迭 (1.0→1.1, 1.2, 2.0, 2.1)
│   │   ├── v1.0.0.md               # 首发版 MVP
│   │   ├── v1.1.0.md               # 安全加固 + CI 强化
│   │   ├── v2.0.0.md               # 功能补齐 + 架构升级
│   │   ├── v2.1.0.md               # 可见性 + 生态增强
│   │   ├── v2.2.0.md               # 前端优化
│   │   ├── v2.3.0.md               # macOS 风格重设计
│   │   └── v2.4.0.md               # 下一大版本(规划中)
│   │
│   └── minor/                      # 小版本更迭 (1.0.1, 1.0.2, 2.0.1, 2.0.2)
│       ├── v1.0.1.md               # 签名密钥轮换
│       ├── v1.1.1.md               # MCP wiring 修复
│       ├── v1.1.2.md               # gRPC trait 冲突修复
│       ├── v1.1.3.md               # migrations 路径修复
│       ├── v1.1.4.md               # autostart panic 修复
│       ├── v2.0.1.md               # (如有)
│       ├── v2.3.1.md               # 测试修复 + Dependabot lru 忽略
│       └── README.md               # 小版本更迭规范说明
│
├── schema/                         # 可自定义的 schema
│   └── nebula.schema.md            # Nebula 专用工作流 schema
│                                   # (加入"记忆影响评估"和"数据主权检查"工件)
│
├── OPENSPEC.md                     # OpenSpec 使用指南(Nebula 定制版)
└── README.md                       # openspec/ 目录说明
```

---

## 三、Spec 文件格式

### 3.1 行为契约 spec.md 格式

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
- L2: 知识抽取 (长期)
- L3: 事实记忆 (持久)
- L4: 价值观记忆 (宪法)
- L5: 反思记忆 (元认知)

#### Scenario: L0→L1 自动压缩
- **WHEN** 对话超过 20 轮
- **THEN** 系统自动生成 L1 摘要
- **AND** 清理 L0 中的已压缩内容

### Requirement: 黑洞压缩引擎
The system SHALL compress L3 facts into semantic capsules using the BlackholeEngine.
...
```

### 3.2 Delta spec 格式

```markdown
# Delta for Memory

## ADDED Requirements

### Requirement: L6 蒸馏层
The system SHALL distill L4 facts into compressed semantic capsules.
- 蒸馏触发条件: L3 容量超过 80%
- 蒸馏输出: 语义胶囊(<=512 tokens)
- 蒸馏频率: 每日 03:00

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

---

## 四、版本迭代双轨制

### 4.1 大版本 (Major) — `versions/major/`

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
> **类型**: 大版本 (Minor version bump in SemVer major.minor)
> **前一版本**: v2.3.1
> **变更范围**: memory/, llm/

## 变更摘要
[一段话]

## 包含的 Changes
- [add-l6-distillation](../changes/archive/add-l6-distillation/)
- [optimize-embedding-cache](../changes/archive/optimize-embedding-cache/)

## 新增功能
1. L6 蒸馏层 — 自动压缩 L3 事实为语义胶囊
2. ...

## 修改的功能
1. 记忆层级从 L0-L5 扩展为 L0-L6
2. ...

## 破坏性变更
(none)

## 技术债务清理
- T-D-B-12: 修复 embedding 缓存击穿问题

## 验收标准
- [x] cargo test --lib 全通过
- [x] npm test 全通过
- [x] tsc --noEmit 无错误
- [x] CI 全绿
```

### 4.2 小版本 (Minor) — `versions/minor/`

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
> **类型**: 小版本 (Patch version bump in SemVer)
> **前一版本**: v2.3.0
> **变更范围**: 测试文件 + .github/dependabot.yml

## 变更摘要
修复 v2.3.0 macOS 风格重设计后 5 个前端测试失败，忽略 Dependabot lru 更新。

## 修复列表
1. ChatPanel.test.tsx: OllamaStatusBanner v2.2 已改为空壳,banner 测试改为验证不渲染
2. ChatPanel.test.tsx: streaming 测试改用 .chat-send-btn 选择器
3. MemoryMap.test.tsx: header 选择器从 .flex.items-center.justify-between 改为 .page-header
4. ShadowWorkspacePanel: merge/abort 按钮加 data-testid
5. dependabot.yml: 忽略 lru 更新(PR #6 已关闭)

## 关联
- CI Run: 29132407863
- 提交: c124f67
```

### 4.3 版本号规则

```
MAJOR.MINOR.PATCH
  │      │      │
  │      │      └── 小版本: Bug修复/测试/文档/配置 (versions/minor/)
  │      └───────── 大版本: 新功能/架构变更/里程碑 (versions/major/)
  └────────────────── 主版本: 破坏性变更 (手动管理,极少触发)
```

**大版本示例**: 1.0→1.1, 1.2, 2.0, 2.1, 2.2, 2.3, 2.4
**小版本示例**: 1.0.1, 1.0.2, 2.0.1, 2.3.1, 2.3.2

---

## 五、工作流

### 5.1 五步工作流

```
nebula:explore ──► nebula:propose ──► nebula:apply ──► nebula:verify ──► nebula:archive
   (探索)           (提案+规划)         (实现)            (验证)            (归档)
```

#### Step 1: Explore (探索)
- 读取 `openspec/specs/` 了解当前系统行为
- 识别要修改的领域和 requirements
- 不创建任何文件

#### Step 2: Propose (提案)
- 在 `openspec/changes/<change-name>/` 创建:
  - `proposal.md` — 为什么 + 做什么
  - `design.md` — 怎么做(技术方案)
  - `tasks.md` — 实现清单(可勾选)
  - `specs/<domain>/delta.md` — Delta spec(ADDED/MODIFIED/REMOVED)

#### Step 3: Apply (实现)
- 按 `tasks.md` 实现代码
- 更新 tasks.md 中的完成状态

#### Step 4: Verify (验证)
- 运行 `cargo test --lib`
- 运行 `npm test`
- 运行 `tsc --noEmit`
- 确认 CI 全绿

#### Step 5: Archive (归档)
- 将 `changes/<change-name>/` 移到 `changes/archive/<change-name>/`
- 创建 `archived-at.md` 记录归档时间和合并到的版本
- 将 Delta specs 合并到主 specs（ADDED 追加 / MODIFIED 替换 / REMOVED 删除）
- 在 `versions/major/` 或 `versions/minor/` 创建版本迭代文件

### 5.2 并行处理

多个 change 可以并行进行，只要它们修改不同的领域：
- `add-l6-distillation` 修改 memory/
- `fix-swarm-deadlock` 修改 swarm/
- 两者互不冲突，可同时进行

---

## 六、Nebula 专用 Schema

### 6.1 额外工件

在标准 OpenSpec 工件基础上，Nebula 加入两个专用工件：

| 工件 | 文件 | 用途 |
|------|------|------|
| **记忆影响评估** | `memory-impact.md` | 评估变更对记忆系统各层的影响 |
| **数据主权检查** | `sovereignty-check.md` | 确认变更不违反数据主权红线 |

### 6.2 记忆影响评估格式

```markdown
# 记忆影响评估

## 影响的层级
- [ ] L0 (原始上下文)
- [ ] L1 (对话摘要)
- [ ] L2 (知识抽取)
- [ ] L3 (事实记忆)
- [ ] L4 (价值观记忆)
- [ ] L5 (反思记忆)
- [ ] L6 (蒸馏层 — 如果存在)

## 影响描述
[描述变更如何影响每个勾选的层级]

## 回滚策略
[如果变更导致记忆系统退化,如何回滚]
```

### 6.3 数据主权检查格式

```markdown
# 数据主权检查

## 检查项
- [x] 变更不会将用户数据发送到第三方服务器
- [x] 变更不会引入新的云端依赖(除非用户显式配置)
- [x] 变更不会绕过现有的 E2EE 加密
- [x] 变更不会降低本地优先架构的保证
- [x] 变更不会引入闭源 Checker 模型

## 说明
[对每个勾选项的说明]
```

---

## 七、集成到 Nebula 代码

### 7.1 技能系统集成

在 `docs/skills/spec-driven-dev/` 创建新技能：

```
docs/skills/spec-driven-dev/
├── SKILL.md              # 技能定义(YAML frontmatter + Markdown)
├── WORKFLOW.md           # 五步工作流详细说明
├── DELTA_SPEC_GUIDE.md   # Delta spec 编写指南
└── templates/
    ├── proposal.md       # 提案模板
    ├── design.md         # 设计模板
    ├── tasks.md          # 任务清单模板
    └── delta.md          # Delta spec 模板
```

### 7.2 CLI 命令集成

在 Nebula 的技能引擎中注册以下命令：

| 命令 | 功能 |
|------|------|
| `nebula:explore` | 读取 specs/ 并返回指定领域的当前行为 |
| `nebula:propose` | 创建新的 change 文件夹和初始文件 |
| `nebula:apply` | 按 tasks.md 实现,更新完成状态 |
| `nebula:verify` | 运行测试套件,报告结果 |
| `nebula:archive` | 归档 change,合并 delta specs,创建版本文件 |

### 7.3 Tauri 命令集成

在后端添加以下 Tauri 命令：

```rust
#[tauri::command]
async fn spec_list_domains() -> Result<Vec<String>, String>;

#[tauri::command]
async fn spec_read_domain(domain: String) -> Result<String, String>;

#[tauri::command]
async fn spec_create_change(name: String, domain: String) -> Result<String, String>;

#[tauri::command]
async fn spec_archive_change(name: String, version: String, is_major: bool) -> Result<(), String>;

#[tauri::command]
async fn spec_list_changes() -> Result<Vec<ChangeInfo>, String>;

#[tauri::command]
async fn spec_list_versions(major_only: bool) -> Result<Vec<VersionInfo>, String>;
```

---

## 八、迁移计划

### Phase 1: 建立目录结构 (本次)
- [ ] 创建 `openspec/` 目录结构
- [ ] 创建 10 个领域的初始 `spec.md`
- [ ] 创建 `OPENSPEC.md` 使用指南
- [ ] 创建 `README.md`

### Phase 2: 回填历史版本
- [ ] 从 CHANGELOG.md 提取 v0.1.0→v2.3.0 的版本记录
- [ ] 在 `versions/major/` 创建大版本文件
- [ ] 在 `versions/minor/` 创建小版本文件
- [ ] 将已完成的主要变更归档到 `changes/archive/`

### Phase 3: 技能系统集成
- [ ] 创建 `docs/skills/spec-driven-dev/SKILL.md`
- [ ] 创建工作流和模板文件
- [ ] 在技能引擎中注册命令

### Phase 4: 后端集成
- [ ] 添加 Tauri 命令
- [ ] 添加前端 API 绑定
- [ ] 添加 spec 管理面板

---

## 九、与现有文档的关系

| 现有文档 | 新文档 | 关系 |
|---------|--------|------|
| ROADMAP_v3.1.md | openspec/specs/ (各领域) | ROADMAP 中的功能描述提炼为行为契约 |
| WHITEPAPER_v3.2.md | openspec/specs/project/ | 白皮书中的项目级约束提取为 project spec |
| CHANGELOG.md | versions/major/ + versions/minor/ | 变更日志拆分为版本迭代文件 |
| ARCHITECTURE.md | openspec/specs/ (各领域) | 架构文档中的契约分散到各领域 spec |
| ADR-001~004 | openspec/changes/archive/ | ADR 归档为已完成的 change |
| DEVELOPMENT_PROPOSAL_v3.0.md | openspec/changes/ (进行中) | 开发提案转为进行中的 change |

**现有文档不删除**，新文档作为权威来源，旧文档逐步标记为"参考"。

---

## 十、验收标准

- [ ] `openspec/` 目录结构完整
- [ ] 10 个领域的 spec.md 内容准确反映当前系统行为
- [ ] 版本迭代文件覆盖 v0.1.0→v2.3.0
- [ ] 技能系统 SKILL.md 可被 Agent 调用
- [ ] CLI 命令可执行五步工作流
- [ ] CI 全绿

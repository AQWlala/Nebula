# Nebula 专用工作流 Schema

> **版本**: v1.0
> **日期**: 2026-07-11
> **定义方**: Nebula 项目
> **用途**: 定义 OpenSpec 在 Nebula 中的工件种类、依赖关系、验证规则和版本迭代触发条件

本 schema 是 `OPENSPEC.md` 的配套规范，提供可机器校验的工件契约。所有 change 在归档前必须通过本 schema 的验证。

---

## 一、标准工件

标准工件继承自 [OpenSpec](https://github.com/Fission-AI/OpenSpec) 上游规范，Nebula 未做语义修改，仅约束命名和最小必需集合。

### 1.1 proposal.md — 提案

**用途**：说明"为什么做"和"做什么"（不含技术细节）。

**必需性**：所有 change 必需。

**最小结构**：

```markdown
# <change-name>

> **状态**: proposed | in-progress | archived
> **领域**: <domain1>, <domain2>
> **提出日期**: YYYY-MM-DD

## 为什么
[动机、痛点、目标]

## 做什么
[变更范围的高层描述,不含技术实现]

## 不做什么
[明确排除的范围,避免范围蔓延]
```

**验证规则**：
- 必须包含 `## 为什么` 和 `## 做什么` 两节
- `> **领域**:` 列出的每个领域必须在 `specs/` 中存在对应目录（回填型 change 除外）
- `> **状态**:` 必须是 `proposed` / `in-progress` / `archived` 之一

### 1.2 design.md — 技术方案

**用途**：说明"怎么做"——技术方案、数据流、模块划分、关键决策。

**必需性**：满足任一条件时必需：
- 变更跨越 2 个及以上领域
- 变更修改数据存储格式或加密链路
- 变更新增/删除/重构模块
- tasks.md 预计超过 10 个任务

**最小结构**：

```markdown
# 技术方案 — <change-name>

> **领域**: <domain>
> **设计日期**: YYYY-MM-DD

## 现状
[当前系统如何工作,引用 specs/<domain>/spec.md 中的相关 Requirement]

## 方案
[技术实现方案,含数据流图/模块图(可用 Mermaid)]

## 关键决策
| 决策 | 选项 | 选择 | 理由 |
|------|------|------|------|
| ...  | A/B  | A    | ...  |

## 风险与缓解
- 风险1: ... → 缓解: ...
```

**验证规则**：
- 必须包含 `## 现状` 和 `## 方案` 两节
- `## 现状` 必须引用至少一个 spec 中的 Requirement 名称

### 1.3 tasks.md — 实现清单

**用途**：可勾选的任务列表，驱动 Apply 阶段。

**必需性**：所有 change 必需。

**最小结构**：

```markdown
# 实现清单 — <change-name>

> **总任务数**: N
> **已完成**: 0

## T-01: <任务标题>
- [ ] 子任务1
- [ ] 子任务2

## T-02: <任务标题>
- [ ] ...
```

**编号规则**：
- Nebula 使用 `T-<序号>` 作为主任务编号（如 `T-01`、`T-02`）
- 历史回填型 change 使用领域前缀：`T-E-*`（feature 补齐）、`T-B-*`（bug 修复）、`T-D-*`（技术债务）
- 子任务用 `- [ ]` 复选框，不单独编号

**验证规则**：
- 每个主任务必须有标题
- 归档时所有 `- [ ]` 必须变为 `- [x]`
- `> **已完成**:` 字段在归档时必须等于 `> **总任务数**:`

### 1.4 delta.md — Delta Spec

**用途**：描述相对于当前 `specs/` 的变更（ADDED/MODIFIED/REMOVED）。

**必需性**：所有 change 必需（即使没有修改，也要写 `(none)`）。

**位置**：`changes/<name>/specs/<domain>/delta.md`，每个受影响领域一个文件。

**最小结构**：

```markdown
# Delta for <Domain>

## ADDED Requirements

### Requirement: <名称>
[完整 Requirement 定义,含 Scenario]

## MODIFIED Requirements

### Requirement: <名称>
[更新后的 Requirement 定义]
(Previously: [原定义摘要])

## REMOVED Requirements

### Requirement: <名称>
[被删除的 Requirement,保留名称供追溯]
```

**验证规则**：
- 必须包含 `## ADDED Requirements`、`## MODIFIED Requirements`、`## REMOVED Requirements` 三节（即使为空写 `(none)`）
- 每个 Requirement 必须有 `### Requirement: <名称>` 标题
- 每个 Requirement 至少包含一个 `#### Scenario`
- MODIFIED 必须注明 `(Previously: ...)`
- REMOVED 下的 Requirement 名称必须能在当前 `specs/<domain>/spec.md` 中找到

---

## 二、Nebula 专用工件

Nebula 在标准工件基础上新增两个专用工件，用于守护项目的两条红线。

### 2.1 memory-impact.md — 记忆影响评估

**用途**：评估变更对记忆系统各层（L0-L7）的影响，并定义回滚策略。

**必需性**：满足任一条件时必需（由 `nebula:propose` 自动检测）：
- 变更触及 `src-tauri/src/memory/` 下任何文件
- 变更修改记忆相关的 Tauri 命令（`memory.rs`、`reflect.rs`、`evolution.rs` 等）
- 变更修改前端记忆相关组件（`MemoryMap.tsx`、`MemoryInspector.tsx` 等）或 store
- 变更影响 L0-L7 任一层级的读写行为
- 变更修改记忆的存储格式、加密方式、压缩策略、检索逻辑
- Delta spec 中 `specs/memory/delta.md` 非空

**最小结构**：

```markdown
# 记忆影响评估

> **变更**: <change-name>
> **评估日期**: YYYY-MM-DD
> **评估人**: <name/agent>

## 影响的层级
- [ ] L0 (原始上下文)
- [ ] L1 (对话摘要)
- [ ] L2 (知识抽取)
- [ ] L3 (事实记忆)
- [ ] L4 (价值观记忆)
- [ ] L5 (反思记忆)
- [ ] L6 (蒸馏层 — 如果存在)
- [ ] L7 (元认知 — 如果存在)

## 影响描述
[对每个勾选层级的具体影响说明]

## 兼容性
- [ ] 变更不破坏现有记忆的读取
- [ ] 变更不需要数据迁移
- [ ] 变更不需要用户手动操作

## 回滚策略
[若变更导致记忆退化,如何安全回滚]
```

**验证规则**：
- 必须包含 `## 影响的层级` 和 `## 回滚策略` 两节
- `## 回滚策略` 不得为空或写"无"——即使是"禁用 feature flag"也要写明
- 如果勾选了任何层级，`## 影响描述` 必须对该层级有说明

### 2.2 sovereignty-check.md — 数据主权检查

**用途**：确认变更不违反 Nebula 的数据主权红线（本地优先、E2EE、无强制云端）。

**必需性**：满足任一条件时必需：
- 变更引入新的网络请求或第三方服务依赖
- 变更修改数据存储位置（本地/云端/混合）
- 变更触及 E2EE 加密链路（`src-tauri/src/sync/e2ee.rs`）
- 变更涉及用户数据导出、同步、备份
- 变更引入新的 AI 模型或 Checker
- 变更修改 OAuth/身份认证流程
- Delta spec 中 `specs/security/`、`specs/sync/`、`specs/project/` 任一非空

**最小结构**：

```markdown
# 数据主权检查

> **变更**: <change-name>
> **检查日期**: YYYY-MM-DD
> **检查人**: <name/agent>

## 检查项
- [ ] 变更不会将用户数据发送到第三方服务器
      说明: [...]
- [ ] 变更不会引入新的云端依赖(除非用户显式配置)
      说明: [...]
- [ ] 变更不会绕过现有的 E2EE 加密
      说明: [...]
- [ ] 变更不会降低本地优先架构的保证
      说明: [...]
- [ ] 变更不会引入闭源 Checker 模型
      说明: [...]

## 红线确认
本变更 [不违反 / 违反] Nebula 数据主权红线。
[如违反,说明豁免理由和缓解措施]
```

**验证规则**：
- 5 个检查项必须全部列出，不得省略
- 每个勾选项必须附 `说明:`
- `## 红线确认` 必须明确写"不违反"或"违反"（违反时需附豁免理由）
- 任何一项未勾选（`[ ]`），必须在说明中解释原因和缓解措施

---

## 三、工件之间的依赖关系

工件的创建有先后依赖。下图描述完整的依赖图：

```
proposal.md
    │
    ├──► design.md           (大变更时必需)
    │        │
    │        ├──► tasks.md   (实现清单)
    │        │
    │        ├──► specs/<domain>/delta.md   (每个受影响领域一个)
    │        │
    │        ├──► memory-impact.md          (触及记忆时)
    │        │
    │        └──► sovereignty-check.md      (触及数据流时)
    │
    └──► tasks.md             (小变更可直接从 proposal 生成)
```

### 3.1 依赖规则表

| 工件 | 前置依赖 | 后续触发 |
|------|---------|---------|
| `proposal.md` | 无 | design.md, tasks.md, delta.md |
| `design.md` | proposal.md | tasks.md, delta.md, memory-impact.md, sovereignty-check.md |
| `tasks.md` | proposal.md（或 design.md） | 无 |
| `delta.md` | design.md（或 proposal.md） | memory-impact.md（若触及 memory/）, sovereignty-check.md（若触及 security/sync/project/） |
| `memory-impact.md` | delta.md（且 specs/memory/delta.md 非空） | 无 |
| `sovereignty-check.md` | delta.md（且触及数据流领域） | 无 |

### 3.2 最小必需集合

根据变更规模，最小必需工件集合不同：

| 变更类型 | proposal | design | tasks | delta | memory-impact | sovereignty-check |
|---------|:--------:|:------:|:-----:|:-----:|:-------------:|:-----------------:|
| 纯文档修正 | ✓ | — | ✓ | ✓(写 none) | — | — |
| 小型 bug 修复 | ✓ | — | ✓ | ✓ | 按需 | 按需 |
| 功能新增 | ✓ | ✓ | ✓ | ✓ | 按需 | 按需 |
| 架构重构 | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| 记忆系统变更 | ✓ | ✓ | ✓ | ✓ | **✓** | 按需 |
| 数据流/同步变更 | ✓ | ✓ | ✓ | ✓ | 按需 | **✓** |

"按需"指根据第二节中的触发条件判断。

---

## 四、验证规则

### 4.1 提案阶段验证（nebula:propose）

在创建 change 文件夹后立即执行：

1. `proposal.md` 存在且包含必需章节
2. `> **领域**:` 列出的领域在 `specs/` 中存在（回填型除外）
3. `> **状态**:` 为 `proposed`
4. 至少创建了一个 `specs/<domain>/delta.md`（即使是 `(none)`）
5. 根据触发条件，`memory-impact.md` / `sovereignty-check.md` 如需创建则已创建

### 4.2 实现阶段验证（nebula:apply）

每次更新 tasks.md 后执行：

1. 新增任务的编号未与现有任务冲突
2. 勾选状态与实际代码改动一致（人工核对）

### 4.3 归档前验证（nebula:archive）

归档前执行完整验证：

1. **工件完整性**：根据第三节最小必需集合，所有应创建的工件均存在
2. **tasks.md**：所有 `- [ ]` 已变为 `- [x]`；`> **已完成**:` 等于 `> **总任务数**:`
3. **delta.md 验证**：
   - 三节（ADDED/MODIFIED/REMOVED）齐全
   - 每个 Requirement 有至少一个 Scenario
   - MODIFIED 注明了 `(Previously: ...)`
   - REMOVED 的 Requirement 能在当前 specs/ 中找到
4. **memory-impact.md 验证**（如存在）：
   - `## 回滚策略` 非空
   - 勾选层级与 `## 影响描述` 一致
5. **sovereignty-check.md 验证**（如存在）：
   - 5 个检查项齐全
   - `## 红线确认` 明确写"不违反"或附豁免
6. **测试验证**：
   - `cargo test --lib` 全通过
   - `npm test` 全通过
   - `tsc --noEmit` 无错误
   - CI 全绿
7. **Scenario 覆盖**：delta.md 中每个 Scenario 有对应测试（人工核对）

### 4.4 合并验证

归档时合并 delta 到 specs/ 后执行：

1. 合并后的 `specs/<domain>/spec.md` 语法正确（章节层级完整）
2. 同名 Requirement 未重复出现
3. `> **最后更新**:` 字段已更新为本次 change 名称
4. 被 REMOVED 的 Requirement 确实已从 specs/ 中删除

---

## 五、版本迭代文件的触发条件

版本文件不是每次归档都创建。本节定义何时创建版本文件。

### 5.1 大版本文件 — `versions/major/vX.Y.0.md`

**触发条件**（满足任一）：

- 归档的 change 包含 `ADDED Requirements`（新增功能契约）
- 归档的 change 包含 `MODIFIED Requirements` 且修改了对外 API 行为
- 归档的 change 涉及架构级重构（design.md 中标注为"架构变更"）
- 归档的 change 包含 UI 重设计
- 里程碑达成（如 Phase 0-3 功能补齐完成）

**不触发的情况**：
- 纯 bug 修复（即使有 delta，但只修正实现与 spec 的偏差，未改变契约）
- 纯文档修正
- 纯测试补充

**文件命名**：`v<MAJOR>.<MINOR>.0.md`，如 `v2.4.0.md`

**内容要求**：
- `## 包含的 Changes` 列出本次发布合并的所有归档 change
- `## 新增功能` / `## 修改的功能` / `## 破坏性变更` 三节齐全
- `## 验收标准` 全部勾选

### 5.2 小版本文件 — `versions/minor/vX.Y.Z.md`

**触发条件**（满足任一）：

- Bug 修复（不影响 spec 契约）
- 测试修复
- 文档修正
- 依赖更新
- 配置调整
- CI 修复
- 性能优化（不改 API）

**不触发的情况**：
- 任何改变 spec 契约的变更（应走大版本）

**文件命名**：`v<MAJOR>.<MINOR>.<PATCH>.md`，如 `v2.3.1.md`

**内容要求**：
- `## 修复列表` 或 `## 变更列表` 列出具体改动
- `## 关联` 记录 CI Run ID 和关键 commit hash

### 5.3 合并发布

同一发布周期内归档的多个 change 合并到一个版本文件：

- `versions/major/v2.4.0.md` 的 `## 包含的 Changes` 列多条
- 每条指向 `../changes/archive/<change-name>/`
- 如果周期内既有大版本范畴又有小版本范畴的 change：
  - 大版本文件先创建（`v2.4.0.md`）
  - 小版本依附于该大版本（`v2.4.1.md` 等）

### 5.4 不发布的情况

change 归档但未发布时：
- 仅执行归档（移到 `changes/archive/`，合并 delta 到 specs/）
- 不创建版本文件
- 待正式发布时，将该周期内所有归档 change 汇总到一个版本文件

这允许"持续归档、定期发布"的工作模式。

---

## 六、领域清单

当前 Nebula 定义的领域（与 `specs/` 目录一一对应）：

| 领域 | 目录 | 职责 | 源码位置 |
|------|------|------|---------|
| memory | `specs/memory/` | L0-L7 记忆层级、压缩引擎、检索 | `src-tauri/src/memory/` |
| chat | `specs/chat/` | 对话流、流式响应、上下文管理 | `src-tauri/src/commands/chat.rs` |
| swarm | `specs/swarm/` | 蜂群协作、Master-Orchestrator、DAG | `src-tauri/src/swarm/` |
| skills | `specs/skills/` | 技能引擎、市场、沙箱、发布 | `src-tauri/src/skills/` |
| security | `specs/security/` | 注入防护、SSRF 守卫、密钥链、沙箱 | `src-tauri/src/security/` |
| llm | `specs/llm/` | 模型调度、网关、成本追踪、健康检查 | `src-tauri/src/llm/` |
| evolution | `specs/evolution/` | 进化引擎、Cron、基因突变、反思 | `src-tauri/src/evolution/` |
| os | `specs/os/` | OS 控制、剪贴板、通知、托盘 | `src-tauri/src/os/` |
| sync | `specs/sync/` | E2EE、CRDT、多设备、配对 | `src-tauri/src/sync/` |
| ui | `specs/ui/` | 前端组件、布局、主题、i18n | `src/components/` |
| project | `specs/project/` | 项目级约束：数据主权、本地优先、哲学 | 跨领域 |

新增领域需通过一个专门的 change（如 `add-domain-voice`）提议，经评审后加入本清单。

---

## 七、变更日志

| 日期 | 版本 | 变更 |
|------|------|------|
| 2026-07-11 | v1.0 | 初始版本，定义 6 种工件、依赖关系、验证规则、版本触发条件 |

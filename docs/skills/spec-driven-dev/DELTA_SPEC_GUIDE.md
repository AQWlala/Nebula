# Delta Spec 编写指南

> **版本**: 1.0.0
> **日期**: 2026-07-11
> **关联**: [SKILL.md](./SKILL.md) | [WORKFLOW.md](./WORKFLOW.md) | [templates/delta.md](./templates/delta.md)
> **设计依据**: [docs/SPEC_DRIVEN_DESIGN.md](../../SPEC_DRIVEN_DESIGN.md) §三

本文档是 Delta Spec 的编写指南,定义 ADDED/MODIFIED/REMOVED 的语义、Requirement 命名规范、
Scenario 编写规范,以及 Delta 合并到主 spec 的规则。

---

## 1. Delta Spec 是什么

Delta Spec 只描述"什么变了",不重写整个文档。它是 change 提案的核心工件之一,
存放在 `openspec/changes/<change-name>/specs/<domain>/delta.md`。

**为什么不直接改 spec.md?**
- spec.md 是"当前系统行为"的真实来源,直接改会丢失变更历史
- Delta spec 让 reviewer 一眼看出"这个 change 改了什么",无需 diff 整个 spec
- 归档时才合并 delta 到主 spec,保证 spec 的演进有迹可循

```
spec.md (当前行为)  +  delta.md (要改什么)  ──archive──►  spec.md (新行为)
```

---

## 2. 三种变更语义

### 2.1 ADDED — 新增

**语义**: 向系统中新增一个之前不存在的 requirement。

**使用场景**:
- 新增功能模块(如添加 L6 蒸馏层)
- 为现有功能补充之前未文档化的行为契约
- 新增约束(如新增一条数据主权检查规则)

**格式**:
```markdown
## ADDED Requirements

### Requirement: <Requirement 名称>
<行为描述,使用 MUST/SHALL/SHOULD/MAY 等RFC 2119 关键词>

- <细节点 1>
- <细节点 2>

#### Scenario: <场景名称>
- **WHEN** <触发条件>
- **THEN** <预期行为>
- **AND** <附加约束>
```

**示例**:
```markdown
## ADDED Requirements

### Requirement: L6 蒸馏层
The system SHALL distill L3 facts into compressed semantic capsules.

- 蒸馏触发条件: L3 容量超过 80%
- 蒸馏输出: 语义胶囊(<=512 tokens)
- 蒸馏频率: 每日 03:00

#### Scenario: L3 容量超限触发蒸馏
- **WHEN** L3 存储容量超过 80%
- **THEN** 系统启动 L6 蒸馏流程
- **AND** 生成语义胶囊写入 L6
- **AND** 在 STATE.md 记录蒸馏事件

#### Scenario: 定时蒸馏
- **WHEN** 每日 03:00
- **THEN** 系统执行 L6 蒸馏流程
- **AND** 仅蒸馏 L3 中 30 天未访问的事实
```

### 2.2 MODIFIED — 修改

**语义**: 修改现有 requirement 的行为。被修改的 requirement **必须在当前 spec.md 中存在**。

**使用场景**:
- 扩展现有功能(如记忆层级从 L0-L5 扩展为 L0-L6)
- 修改行为参数(如压缩阈值从 80% 改为 70%)
- 增加新 scenario 到现有 requirement
- 修改 scenario 的预期行为

**格式**:
```markdown
## MODIFIED Requirements

### Requirement: <现有 Requirement 名称>
<新的行为描述>

- <变更说明: 之前是 X,现在是 Y>

#### Scenario: <场景名称>
- **WHEN** <触发条件>
- **THEN** <新的预期行为>
```

**关键规则**:
- MODIFIED 必须写出**完整的**新 requirement(不是 diff),归档时会整体替换
- 必须在描述中说明"之前是什么,现在是什么"
- Scenario 可以新增、修改或删除(在 MODIFIED 节内说明)

**示例**:
```markdown
## MODIFIED Requirements

### Requirement: 记忆层级
The system MUST support L0-L6 memory layers. (Previously: L0-L5)

- L0: 原始上下文 (短期) — 不变
- L1: 对话摘要 (中期) — 不变
- L2: 知识抽取 (长期) — 不变
- L3: 事实记忆 (持久) — 不变
- L4: 价值观记忆 (宪法) — 不变
- L5: 反思记忆 (元认知) — 不变
- L6: 蒸馏层 (新增) — 压缩 L3 事实为语义胶囊

#### Scenario: L0→L1 自动压缩
- **WHEN** 对话超过 20 轮
- **THEN** 系统自动生成 L1 摘要
- **AND** 清理 L0 中的已压缩内容

#### Scenario: L3→L6 蒸馏 (新增)
- **WHEN** L3 容量超过 80%
- **THEN** 系统启动 L6 蒸馏
- **AND** 压缩 L3 事实为语义胶囊
```

### 2.3 REMOVED — 删除

**语义**: 从系统中删除一个现有 requirement。被删除的 requirement **必须在当前 spec.md 中存在**。

**使用场景**:
- 移除废弃功能(如删除旧的压缩引擎)
- 合并两个 requirement 为一个
- 删除不再需要的约束

**格式**:
```markdown
## REMOVED Requirements

### Requirement: <现有 Requirement 名称>
<删除原因和影响说明>

**迁移说明**: <如果有替代方案,说明如何迁移>
```

**示例**:
```markdown
## REMOVED Requirements

### Requirement: 旧版 Embedding 缓存
The legacy embedding cache using Redis is removed.

**删除原因**: 已被 BlackholeEngine 的语义胶囊缓存替代,v2.4.0 起不再维护。
**迁移说明**: 使用 BlackholeEngine::cached_embedding() 替代旧 API。
**影响**: 
- 删除 `src-tauri/src/memory/legacy_cache.rs`
- 迁移 `legacy_cache` 表数据到 `semantic_capsules` 表
- 配置项 `NEBULA_LEGACY_CACHE_PATH` 废弃
```

---

## 3. Requirement 命名规范

### 3.1 命名规则

| 规则 | 说明 | 示例 |
|------|------|------|
| 使用中文名词短语 | Nebula 项目约定使用中文 requirement 名 | ✅ "记忆层级"、❌ "Memory Hierarchy" |
| 简洁明确 | 名称应一眼看出 requirement 管的是什么 | ✅ "流式输出"、❌ "关于消息流式传输的行为" |
| 唯一性 | 同一领域内 requirement 名称唯一 | 不可有两个 "记忆层级" |
| 不含动词 | Requirement 是"名词"(描述是什么),不是"动词"(描述做什么) | ✅ "消息持久化"、❌ "持久化消息" |
| 不含实现细节 | 名称描述能力,不描述实现 | ✅ "压缩引擎"、❌ "BlackholeEngine 压缩" |

### 3.2 好命名 vs 坏命名

| ✅ 好命名 | ❌ 坏命名 | 原因 |
|---------|---------|------|
| 记忆层级 | L0到L6的记忆层支持 | 简洁,无实现细节 |
| 流式输出 | SSE流式响应实现 | 不含实现细节(SSE) |
| 模型同质检测 | detect_model_homogeneity方法 | 不含方法名 |
| 数据主权红线 | 关于数据不能发到云端的规则 | 简洁明确 |
| 审批门 | ApprovalGate审批流程 | 不含类名(ApprovalGate) |

### 3.3 跨领域命名

若一个 requirement 横跨多个领域,放在主要领域的 spec 中,其他领域用引用:

```markdown
### Requirement: 蜂群记忆共享
(本 requirement 的完整定义在 openspec/specs/memory/spec.md 中)
```

---

## 4. Scenario 编写规范

### 4.1 Scenario 是什么

Scenario 是 requirement 的可验证示例,描述"在什么条件下,系统应该怎么表现"。
每个 requirement 至少有一个 scenario,scenario 应该是可测试的。

### 4.2 Scenario 格式

使用 WHEN/THEN/AND 结构(Given-When-Then 的简化版):

```markdown
#### Scenario: <场景名称>
- **WHEN** <触发条件>
- **THEN** <预期行为>
- **AND** <附加约束或副作用>
- **AND NOT** <不应发生的行为>(可选)
```

### 4.3 Scenario 命名规则

| 规则 | 说明 | 示例 |
|------|------|------|
| 描述场景,不是测试用例名 | ✅ "L3 容量超限触发蒸馏"、❌ "test_l6_distill" |
| 包含关键条件 | ✅ "对话超过 20 轮触发压缩"、❌ "压缩场景" |
| 唯一性 | 同一 requirement 内 scenario 名称唯一 | — |

### 4.4 好 Scenario vs 坏 Scenario

**✅ 好 Scenario**:
```markdown
#### Scenario: L3 容量超限触发蒸馏
- **WHEN** L3 存储容量超过 80%
- **THEN** 系统启动 L6 蒸馏流程
- **AND** 生成语义胶囊写入 L6
- **AND** 在 STATE.md 记录蒸馏事件
- **AND NOT** 删除 L3 中的原始事实(仅压缩,不删除)
```

**❌ 坏 Scenario**(太模糊,不可测试):
```markdown
#### Scenario: 蒸馏
- **WHEN** 系统需要蒸馏
- **THEN** 系统执行蒸馏
```

### 4.5 Scenario 必须可测试

每个 scenario 应该能转化为至少一个测试用例。编写时自问:
- 我能用 `assert` 验证 THEN 吗?
- 我能构造 WHEN 的条件吗?
- AND 的副作用能观测到吗?

若不能,scenario 需要更具体。

---

## 5. RFC 2119 关键词

Requirement 描述中使用以下关键词表达强制程度(对齐 RFC 2119):

| 关键词 | 含义 | 使用场景 | 示例 |
|--------|------|---------|------|
| **MUST** | 绝对必须 | 红线、安全、数据主权 | "系统 MUST NOT 将用户数据发送到第三方" |
| **SHALL** | 必须(契约性) | 核心行为契约 | "系统 SHALL 支持 L0-L6 记忆层级" |
| **SHOULD** | 强烈建议 | 最佳实践,有例外 | "系统 SHOULD 在 03:00 执行蒸馏" |
| **MAY** | 可选 | 可选能力 | "系统 MAY 提供手动触发蒸馏的 API" |

**使用建议**:
- 安全/主权相关 → 用 `MUST` / `MUST NOT`
- 核心功能契约 → 用 `SHALL` / `SHALL NOT`
- 优化建议 → 用 `SHOULD`
- 可选功能 → 用 `MAY`
- 避免用"应该"(中文歧义),用 `SHALL`(必须)或 `SHOULD`(建议)

---

## 6. Delta 合并规则

归档(`nebula:archive`)时,delta.md 会合并到主 spec.md。合并规则:

### 6.1 ADDED 合并

**规则**: 将 ADDED 的 requirement 追加到 spec.md 的 Requirements 节末尾。

```
spec.md (before):
  ## Requirements
  ### Requirement: A
  ### Requirement: B

delta.md:
  ## ADDED Requirements
  ### Requirement: C

spec.md (after):
  ## Requirements
  ### Requirement: A
  ### Requirement: B
  ### Requirement: C    ← 追加
```

**冲突检测**: 若 ADDED 的 requirement 名称已在 spec 中存在,合并失败,需人工裁决。

### 6.2 MODIFIED 合并

**规则**: 用 delta 中的新版本**整体替换** spec 中的同名 requirement。

```
spec.md (before):
  ### Requirement: 记忆层级
  The system MUST support L0-L5 layers.   ← 旧版本
  #### Scenario: L0→L1 压缩

delta.md:
  ## MODIFIED Requirements
  ### Requirement: 记忆层级
  The system MUST support L0-L6 layers.   ← 新版本
  #### Scenario: L0→L1 压缩
  #### Scenario: L3→L6 蒸馏 (新增)

spec.md (after):
  ### Requirement: 记忆层级
  The system MUST support L0-L6 layers.   ← 被替换
  #### Scenario: L0→L1 压缩
  #### Scenario: L3→L6 蒸馏 (新增)        ← 新 scenario 一并替换
```

**关键**: MODIFIED 是整体替换,不是 patch。delta 中必须写出完整的 requirement(含所有 scenario)。

### 6.3 REMOVED 合并

**规则**: 从 spec.md 中删除同名的 requirement 及其所有 scenario。

```
spec.md (before):
  ### Requirement: 旧版 Embedding 缓存
  ... (含 3 个 scenario)

delta.md:
  ## REMOVED Requirements
  ### Requirement: 旧版 Embedding 缓存
  (删除原因说明)

spec.md (after):
  (requirement 及其 scenario 全部删除)
```

### 6.4 合并顺序

同一 delta 中若有多类变更,按 **REMOVED → MODIFIED → ADDED** 顺序合并:
1. 先执行 REMOVED(删除旧的)
2. 再执行 MODIFIED(替换现有的)
3. 最后执行 ADDED(追加新的)

这样避免名称冲突(如先删 "X" 再加新的 "X")。

### 6.5 跨 change 合并冲突

若两个 change 修改同一 requirement:
- 先归档者的 MODIFIED 已合并到 spec
- 后归档者的 MODIFIED 引用的是旧版本 → 合并时检测到 requirement 已变更
- **处置**: 阻断后归档者,要求其重新 explore 并更新 delta

---

## 7. 完整 Delta Spec 示例

以下是一个完整的 delta.md 示例(change: `add-l6-distillation`):

```markdown
# Delta for Memory

> **Change**: add-l6-distillation
> **Domain**: memory
> **Created**: 2026-07-11

## ADDED Requirements

### Requirement: L6 蒸馏层
The system SHALL distill L3 facts into compressed semantic capsules using the BlackholeEngine.

- 蒸馏触发条件: L3 容量超过 80% 或每日 03:00 定时触发
- 蒸馏输入: L3 中超过 30 天未访问的事实
- 蒸馏输出: 语义胶囊(<=512 tokens),存入 L6
- 蒸馏后: 原始 L3 事实标记为"已蒸馏"(不删除,降级为冷存储)

#### Scenario: L3 容量超限触发蒸馏
- **WHEN** L3 存储容量超过 80%
- **THEN** 系统启动 L6 蒸馏流程
- **AND** 生成语义胶囊写入 L6
- **AND** 在 STATE.md 记录蒸馏事件
- **AND NOT** 删除 L3 中的原始事实

#### Scenario: 定时蒸馏
- **WHEN** 每日 03:00(本地时区)
- **THEN** 系统执行 L6 蒸馏流程
- **AND** 仅蒸馏 L3 中 30 天未访问的事实
- **AND** 蒸馏完成后更新 STATE.md 的"最后蒸馏时间"

#### Scenario: 手动触发蒸馏
- **WHEN** 用户调用 `memory:distill` 命令
- **THEN** 系统立即执行 L6 蒸馏流程
- **AND** 返回蒸馏的胶囊数量和节省的存储空间

## MODIFIED Requirements

### Requirement: 记忆层级
The system MUST support L0-L6 memory layers. (Previously: L0-L5)

- L0: 原始上下文 (短期) — 不变
- L1: 对话摘要 (中期) — 不变
- L2: 知识抽取 (长期) — 不变
- L3: 事实记忆 (持久) — 不变,但新增"已蒸馏"标记
- L4: 价值观记忆 (宪法) — 不变
- L5: 反思记忆 (元认知) — 不变
- L6: 蒸馏层 (新增) — 压缩 L3 事实为语义胶囊

#### Scenario: L0→L1 自动压缩
- **WHEN** 对话超过 20 轮
- **THEN** 系统自动生成 L1 摘要
- **AND** 清理 L0 中的已压缩内容

#### Scenario: L3→L6 蒸馏 (新增)
- **WHEN** L3 容量超过 80% 或每日 03:00
- **THEN** 系统启动 L6 蒸馏
- **AND** 压缩 L3 事实为语义胶囊
- **AND** 原始事实标记为"已蒸馏"

## REMOVED Requirements

(none)
```

---

## 8. 常见错误

| 错误 | 说明 | 正确做法 |
|------|------|---------|
| MODIFIED 只写 diff | "把 L0-L5 改成 L0-L6" | 写出完整的新 requirement |
| Scenario 不可测试 | "系统需要时蒸馏" | 写明触发条件和预期行为 |
| Requirement 含实现细节 | "BlackholeEngine 压缩" | 名称描述能力,描述中可含实现 |
| ADDED 引用已存在 requirement | 试图 ADDED "记忆层级"(已存在) | 用 MODIFIED 修改现有 |
| REMOVED 无迁移说明 | 删除功能但不说怎么迁移 | 提供替代方案 |
| 用"应该"而非 SHALL | "系统应该支持 L6" | "系统 SHALL 支持 L6" |
| Scenario 缺少 AND NOT | 只写应该发生什么 | 补充不应该发生什么(防过拟合) |

---

## 9. Checklist: Delta Spec 编写完成度

编写 delta.md 时,逐项检查:

- [ ] 每个 ADDED requirement 是全新的(不在当前 spec 中)
- [ ] 每个 MODIFIED requirement 在当前 spec 中存在
- [ ] 每个 REMOVED requirement 在当前 spec 中存在
- [ ] 每个 requirement 至少有一个 scenario
- [ ] 每个 scenario 有 WHEN + THEN
- [ ] Scenario 可测试(能转化为 assert)
- [ ] MODIFIED requirement 写出了完整新版本(不是 diff)
- [ ] REMOVED requirement 有删除原因和迁移说明
- [ ] 使用 RFC 2119 关键词(MUST/SHALL/SHOULD/MAY)
- [ ] Requirement 名称符合命名规范(中文名词短语,不含实现细节)
- [ ] Scenario 名称在 requirement 内唯一

---

*本指南是 spec-driven-dev 技能的组成部分,版本 1.0.0,2026-07-11。*

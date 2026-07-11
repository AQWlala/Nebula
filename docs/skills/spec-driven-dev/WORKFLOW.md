# Spec-Driven Development 五步工作流详解

> **版本**: 1.0.0
> **日期**: 2026-07-11
> **关联**: [SKILL.md](./SKILL.md) | [DELTA_SPEC_GUIDE.md](./DELTA_SPEC_GUIDE.md) | [templates/](./templates/)
> **设计依据**: [docs/SPEC_DRIVEN_DESIGN.md](../../SPEC_DRIVEN_DESIGN.md) §五

本文档详细描述 spec-driven development 的五步工作流,每一步包括:输入、输出、检查点、
常见错误和示例。所有命令遵循 `nebula:<step>` 命名约定。

---

## 工作流总览

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│ 1. explore  │───►│ 2. propose  │───►│ 3. apply    │───►│ 4. verify   │───►│ 5. archive  │
│   (探索)    │    │  (提案)     │    │   (实现)    │    │   (验证)    │    │   (归档)    │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
   只读               创建文件          修改代码           运行测试           合并 + 归档
   无副作用           无代码修改        更新 tasks.md      无副作用           修改 specs/
```

**核心原则**:
- 每一步有明确的入口和出口检查点,不满足检查点不得进入下一步
- explore 和 verify 是只读操作,不产生副作用
- propose 创建文件但不修改代码,apply 修改代码但不创建 spec 文件
- archive 是唯一会修改 `openspec/specs/` 的步骤

---

## Step 1: nebula:explore (探索)

### 目的

读取 `openspec/specs/` 了解当前系统行为,识别要修改的领域和 requirements。
**不创建任何文件,不修改任何代码**。

### 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--domain` | string | 否 | 领域名(如 `memory`、`chat`、`swarm`)。省略则列出所有领域 |
| `--requirement` | string | 否 | 特定 requirement 名称,只读取该 requirement 的详情 |

### 输出

- 指定领域的 `spec.md` 内容(或所有领域的列表)
- 标注当前 spec 的最后更新时间和来源 change
- 识别出的"可修改点"建议(哪些 requirement 看起来需要扩展)

### 检查点

| 检查项 | 通过条件 | 失败处置 |
|--------|---------|---------|
| ✅ 目录存在 | `openspec/specs/<domain>/spec.md` 存在 | 提示用户该领域尚未建立 spec,需先初始化 |
| ✅ Spec 可读 | 文件内容非空且符合 spec.md 格式 | 提示 spec 格式有误,需修复 |
| ✅ 领域有效 | `--domain` 参数在 10 个已知领域中 | 列出有效领域供用户选择 |

### 执行步骤

1. 解析 `--domain` 参数,若为空则列出 `openspec/specs/` 下所有目录
2. 读取 `openspec/specs/<domain>/spec.md`
3. 解析 spec 的 frontmatter 和 requirements 列表
4. 若指定 `--requirement`,定位到该 requirement 并展示其 scenarios
5. 输出结构化摘要

### 示例

```bash
# 列出所有领域
nebula:explore

# 输出:
# Available domains:
#   - memory     (12 requirements, last updated: 2026-07-10)
#   - chat       (8 requirements, last updated: 2026-07-08)
#   - swarm      (15 requirements, last updated: 2026-07-11)
#   ...

# 读取 memory 领域的 spec
nebula:explore --domain memory

# 读取特定 requirement
nebula:explore --domain memory --requirement "记忆层级"
```

### 常见错误

| 错误 | 原因 | 解决 |
|------|------|------|
| "Domain not found" | 领域名拼写错误或不存在 | 检查 `openspec/specs/` 目录,使用正确的领域名 |
| "Spec file is empty" | spec.md 为空 | 需要先为该领域创建初始 spec |
| "Requirement not found" | `--requirement` 名称不匹配 | 使用 explore 不带 `--requirement` 先查看所有 requirement 名称 |

---

## Step 2: nebula:propose (提案)

### 目的

在 `openspec/changes/<change-name>/` 创建完整的变更提案,包含:
- `proposal.md` — 为什么 + 做什么
- `design.md` — 怎么做(技术方案)
- `tasks.md` — 实现清单(可勾选)
- `specs/<domain>/delta.md` — Delta spec(ADDED/MODIFIED/REMOVED)
- `memory-impact.md` — 记忆影响评估(Nebula 专用)
- `sovereignty-check.md` — 数据主权检查(Nebula 专用)

**不修改任何代码,不修改 `openspec/specs/`**。

### 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--name` | string | 是 | change 名称,kebab-case(如 `add-l6-distillation`) |
| `--domain` | string | 是 | 受影响的领域(如 `memory`) |
| `--description` | string | 是 | 一句话描述变更内容 |
| `--template` | string | 否 | 使用自定义模板路径,默认用技能内置模板 |

### 输出

- 创建 `openspec/changes/<change-name>/` 目录及全部初始文件
- 每个文件从 [templates/](./templates/) 加载,填入 change 元信息
- 输出创建的文件清单和下一步建议

### 检查点

| 检查项 | 通过条件 | 失败处置 |
|--------|---------|---------|
| ✅ 名称合法 | kebab-case,不以数字开头,长度 ≤ 64 | 提示重命名 |
| ✅ 名称唯一 | `openspec/changes/<name>/` 不存在 | 提示已有同名 change,选择其他名称或恢复已有 change |
| ✅ 领域有效 | `--domain` 在 10 个已知领域中 | 列出有效领域 |
| ✅ Spec 已建立 | `openspec/specs/<domain>/spec.md` 存在 | 提示先运行 explore 建立 spec |
| ✅ Delta 引用有效 | delta.md 中的 MODIFIED/REMOVED requirement 在当前 spec 中存在 | 提示先 explore 确认 requirement 名称 |

### 执行步骤

1. 验证 `--name`、`--domain`、`--description` 参数
2. 检查名称唯一性和领域有效性
3. 读取 `openspec/specs/<domain>/spec.md` 获取当前 requirement 列表(用于 delta 引用校验)
4. 创建 `openspec/changes/<name>/` 目录
5. 从 templates 加载并填充:
   - `proposal.md` ← `templates/proposal.md` + 元信息
   - `design.md` ← `templates/design.md` + 元信息
   - `tasks.md` ← `templates/tasks.md` + 元信息
   - `specs/<domain>/delta.md` ← `templates/delta.md` + 元信息
   - `memory-impact.md` ← `templates/memory-impact.md`
   - `sovereignty-check.md` ← `templates/sovereignty-check.md`
6. 输出创建的文件清单

### 示例

```bash
nebula:propose --name add-l6-distillation --domain memory \
  --description "添加 L6 蒸馏层,自动压缩 L3 事实为语义胶囊"

# 输出:
# Created change: add-l6-distillation
# Files:
#   openspec/changes/add-l6-distillation/proposal.md
#   openspec/changes/add-l6-distillation/design.md
#   openspec/changes/add-l6-distillation/tasks.md
#   openspec/changes/add-l6-distillation/specs/memory/delta.md
#   openspec/changes/add-l6-distillation/memory-impact.md
#   openspec/changes/add-l6-distillation/sovereignty-check.md
#
# Next steps:
#   1. Edit proposal.md to describe WHY and WHAT
#   2. Edit design.md to describe HOW
#   3. Fill tasks.md with implementation checklist
#   4. Write delta.md with ADDED/MODIFIED/REMOVED requirements
#   5. Complete memory-impact.md and sovereignty-check.md
#   6. Run: nebula:apply --name add-l6-distillation
```

### 常见错误

| 错误 | 原因 | 解决 |
|------|------|------|
| "Change already exists" | 同名 change 已存在 | 选择新名称,或删除/恢复已有 change |
| "Delta references unknown requirement" | delta.md 的 MODIFIED 引用了不存在的 requirement | 先 explore 确认 requirement 名称 |
| "Sovereignty check not filled" | sovereignty-check.md 仍为模板默认值 | 必须逐项填写,不得保留模板占位符 |

---

## Step 3: nebula:apply (实现)

### 目的

按 `tasks.md` 实现代码,逐项勾选完成状态。**不修改 spec 文件,不创建版本文件**。

### 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--name` | string | 是 | change 名称 |
| `--task` | string | 否 | 只执行指定 task ID(如 `T-02`),省略则按顺序执行所有未完成 task |
| `--dry-run` | bool | 否 | 只展示将要执行的修改,不实际写文件 |

### 输出

- 按 tasks.md 逐项实现代码修改
- 更新 `tasks.md` 中对应 task 的状态为 `- [x]`
- 输出实现摘要:完成的 task 数 / 总 task 数

### 检查点

| 检查项 | 通过条件 | 失败处置 |
|--------|---------|---------|
| ✅ Change 存在 | `openspec/changes/<name>/` 存在 | 提示先运行 propose |
| ✅ tasks.md 已填写 | tasks.md 不含模板占位符 | 提示先填写 tasks.md |
| ✅ Delta 已填写 | delta.md 不含模板占位符 | 提示先填写 delta.md |
| ✅ Sovereignty check 已通过 | sovereignty-check.md 所有检查项为 ✅ | 阻断实现,提示先通过主权检查 |
| ⚠️ Memory impact 已评估 | memory-impact.md 已填写(若涉及记忆系统) | 警告但不阻断(非记忆领域可跳过) |

### 执行步骤

1. 读取 `openspec/changes/<name>/tasks.md`,解析 task 列表
2. 读取 `openspec/changes/<name>/specs/<domain>/delta.md`,理解要实现的行为变更
3. 读取 `openspec/changes/<name>/design.md`,理解技术方案
4. 对每个未完成 task:
   a. 定位要修改的文件
   b. 按 design.md 描述的方案修改代码
   c. 运行该 task 的单元测试(如有)
   d. 勾选 `tasks.md` 中该 task 为 `- [x]`
5. 输出实现摘要

### 示例

```bash
# 执行所有 task
nebula:apply --name add-l6-distillation

# 只执行特定 task
nebula:apply --name add-l6-distillation --task T-02

# 干跑模式
nebula:apply --name add-l6-distillation --dry-run
```

### 常见错误

| 错误 | 原因 | 解决 |
|------|------|------|
| "Task T-XX failed" | 代码修改导致编译错误 | 检查 design.md 方案,修复后重试该 task |
| "Sovereignty check blocked" | sovereignty-check.md 有未通过项 | 先解决主权检查问题 |
| "Delta spec outdated" | explore 后 spec 已被其他 change 更新 | 重新 explore,更新 delta.md |

---

## Step 4: nebula:verify (验证)

### 目的

运行测试套件,确认实现符合 spec。**只读操作,不修改任何文件**(除测试报告输出)。

### 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--name` | string | 否 | change 名称。省略则运行全量测试 |
| `--suite` | string | 否 | 指定测试套件: `rust` / `frontend` / `all`(默认) |
| `--report` | string | 否 | 测试报告输出路径,默认输出到终端 |

### 输出

- 测试结果报告(pass / fail / skip 统计)
- 若指定 `--name`,额外检查: delta.md 中每个 scenario 是否有对应测试覆盖
- CI 状态检查(若已 push)

### 检查点

| 检查项 | 通过条件 | 失败处置 |
|--------|---------|---------|
| ✅ Rust 测试通过 | `cargo test --lib` 全绿 | 列出失败的测试,返回 apply 修复 |
| ✅ 前端测试通过 | `npm test` 全绿 | 列出失败的测试,返回 apply 修复 |
| ✅ 类型检查通过 | `tsc --noEmit` 无错误 | 列出类型错误,返回 apply 修复 |
| ✅ Lint 通过 | `cargo clippy` + `eslint` 无警告 | 列出 lint 问题 |
| ⚠️ Scenario 覆盖 | delta.md 每个 scenario 有对应测试 | 警告(不阻断),建议补充测试 |

### 执行步骤

1. 运行 `rustup run stable-x86_64-pc-windows-msvc cargo test --lib`(Nebula 约定)
2. 运行 `npm test`(前端测试)
3. 运行 `tsc --noEmit`(类型检查)
4. 运行 `cargo clippy`(Rust lint)
5. 若指定 `--name`,解析 delta.md 的 scenarios,检查测试覆盖
6. 输出结构化报告

### Nebula 测试约定

> ⚠️ Nebula 在 Windows 上开发,测试必须用 MSVC 工具链:
> ```bash
> rustup run stable-x86_64-pc-windows-msvc cargo test
> ```
> gnu 工具链编译的二进制无法执行。

### 示例

```bash
# 验证特定 change
nebula:verify --name add-l6-distillation

# 只跑 Rust 测试
nebula:verify --suite rust

# 全量验证
nebula:verify
```

### 常见错误

| 错误 | 原因 | 解决 |
|------|------|------|
| "cargo test failed" | Rust 测试失败 | 检查失败的测试,返回 apply 修复代码 |
| "npm test failed" | 前端测试失败 | 检查失败的测试,返回 apply 修复代码 |
| "tsc --noEmit failed" | 类型错误 | 修复类型问题 |
| "Scenario not covered" | delta.md 的 scenario 缺少测试 | 补充对应测试(不阻断 verify,但 archive 前需补全) |

---

## Step 5: nebula:archive (归档)

### 目的

将已验证的 change 归档,合并 Delta specs 到主 specs,创建版本迭代文件。
**这是唯一会修改 `openspec/specs/` 的步骤**。

### 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `--name` | string | 是 | change 名称 |
| `--version` | string | 是 | 目标版本号(如 `2.4.0`、`2.3.1`) |
| `--major` | bool | 是(二选一) | 标记为大版本(存入 `versions/major/`) |
| `--minor` | bool | 是(二选一) | 标记为小版本(存入 `versions/minor/`) |
| `--merge` | bool | 否 | 是否合并 delta 到主 spec,默认 true |

### 输出

- 移动 `changes/<name>/` → `changes/archive/<name>/`
- 创建 `changes/archive/<name>/archived-at.md` 记录归档时间和合并到的版本
- 合并 Delta specs 到 `openspec/specs/<domain>/spec.md`:
  - ADDED → 追加到 spec 末尾
  - MODIFIED → 替换对应 requirement
  - REMOVED → 删除对应 requirement
- 更新 spec.md 的 "最后更新" 字段
- 创建 `versions/major/v<version>.md` 或 `versions/minor/v<version>.md`

### 检查点

| 检查项 | 通过条件 | 失败处置 |
|--------|---------|---------|
| ✅ Change 存在 | `openspec/changes/<name>/` 存在 | 提示名称错误 |
| ✅ 所有 task 完成 | tasks.md 中所有 task 为 `- [x]` | 阻断,提示先完成所有 task |
| ✅ Verify 已通过 | 最近一次 verify 结果为全绿 | 阻断,提示先运行 verify |
| ✅ Sovereignty check 全通过 | sovereignty-check.md 所有项为 ✅ | 阻断,主权红线不可绕过 |
| ✅ 版本号合法 | 符合 SemVer,且大于当前最大版本号 | 提示版本号问题 |
| ✅ 版本号唯一 | `versions/major/v<version>.md` 或 `versions/minor/v<version>.md` 不存在 | 提示版本号已使用 |
| ⚠️ Memory impact 已填写 | memory-impact.md 不含模板占位符(若涉及记忆系统) | 警告但不阻断 |

### 执行步骤

1. 验证所有检查点
2. 创建 `changes/archive/<name>/` 目录(若不存在)
3. 移动 `changes/<name>/*` → `changes/archive/<name>/*`
4. 创建 `changes/archive/<name>/archived-at.md`:
   ```markdown
   # Archived: <change-name>
   - **Archived at**: 2026-07-11T10:30:00+08:00
   - **Merged to version**: v2.4.0 (major)
   - **Affected domain**: memory
   ```
5. 若 `--merge`(默认):
   a. 读取 `changes/archive/<name>/specs/<domain>/delta.md`
   b. 读取 `openspec/specs/<domain>/spec.md`
   c. 应用 delta:
      - ADDED requirements → 追加到 spec 的 Requirements 节
      - MODIFIED requirements → 替换 spec 中同名 requirement
      - REMOVED requirements → 从 spec 中删除
   d. 更新 spec.md frontmatter 的 "最后更新" 字段
6. 创建版本迭代文件:
   - 大版本: `versions/major/v<version>.md`
   - 小版本: `versions/minor/v<version>.md`
7. 输出归档摘要

### 版本迭代文件格式

#### 大版本 (`versions/major/v<version>.md`)

```markdown
# v2.4.0 — <版本标题>

> **发布日期**: 2026-07-XX
> **类型**: 大版本 (Minor version bump in SemVer major.minor)
> **前一版本**: v2.3.1
> **变更范围**: <受影响的领域>

## 变更摘要
[一段话]

## 包含的 Changes
- [change-name-1](../changes/archive/change-name-1/)
- [change-name-2](../changes/archive/change-name-2/)

## 新增功能
1. ...

## 修改的功能
1. ...

## 破坏性变更
(none)

## 技术债务清理
- T-D-B-XX: ...

## 验收标准
- [x] cargo test --lib 全通过
- [x] npm test 全通过
- [x] tsc --noEmit 无错误
- [x] CI 全绿
```

#### 小版本 (`versions/minor/v<version>.md`)

```markdown
# v2.3.1 — <版本标题>

> **发布日期**: 2026-07-11
> **类型**: 小版本 (Patch version bump in SemVer)
> **前一版本**: v2.3.0
> **变更范围**: <受影响的文件/模块>

## 变更摘要
[一段话]

## 修复列表
1. ...

## 关联
- CI Run: <run-id>
- 提交: <commit-hash>
```

### 示例

```bash
# 归档为大版本
nebula:archive --name add-l6-distillation --version 2.4.0 --major

# 归档为小版本
nebula:archive --name fix-memory-leak --version 2.3.2 --minor

# 归档但不合并 delta(手动合并)
nebula:archive --name experimental-feature --version 2.4.0 --major --merge false
```

### 常见错误

| 错误 | 原因 | 解决 |
|------|------|------|
| "Tasks not all completed" | tasks.md 有未勾选的 task | 先完成所有 task |
| "Verify not passed" | 最近 verify 未全绿 | 先修复测试,重新 verify |
| "Sovereignty check failed" | sovereignty-check.md 有 ❌ 项 | 主权红线不可绕过,必须修复 |
| "Version already exists" | 版本号已使用 | 选择新版本号 |
| "Version is not greater than current" | 版本号 ≤ 当前最大版本 | 递增版本号 |

---

## 并行处理规则

### 可并行的 change

多个 change 可以并行进行,只要它们修改**不同的领域**:

```
add-l6-distillation   → memory/      ┐
fix-swarm-deadlock    → swarm/       ├── 可并行
optimize-llm-cache    → llm/         │
update-ui-theme       → ui/          ┘
```

### 同领域冲突处理

若两个 change 修改同一领域,按以下规则处理:

1. **先归档者优先**: 先 archive 的 change 的 delta 合并到主 spec
2. **后归档者需重新 explore**: 后归档的 change 在 archive 前必须重新 `nebula:explore`,
   确认 spec 已更新,再调整自己的 delta.md
3. **若 delta 冲突**: 后归档者的 MODIFIED/REMOVED 引用的 requirement 已被前一个 change
   修改/删除,需人工裁决

### 并行工作流示例

```
时间 ──►

Team A:  explore(memory) ──► propose ──► apply ──► verify ──► archive(v2.4.0)
Team B:                           explore(swarm) ──► propose ──► apply ──► verify ──► archive(v2.4.0)
Team C:                                                    explore(llm) ──► propose ──► ...
                                                                    ↑ 不同领域,可并行
```

---

## 工作流检查点总览

| 步骤 | 输入检查 | 输出检查 |
|------|---------|---------|
| explore | 领域存在 + spec 可读 | 返回当前 spec 内容 |
| propose | 名称唯一 + 领域有效 + delta 引用有效 | 创建 6 个文件,无代码修改 |
| apply | tasks 已填写 + delta 已填写 + sovereignty 已通过 | 代码修改 + tasks.md 全勾选 |
| verify | 代码已实现 | 测试全绿 + scenario 有覆盖 |
| archive | tasks 全完成 + verify 全绿 + sovereignty 全通过 + 版本号合法 | 归档 + 合并 delta + 创建版本文件 |

---

*本工作流文档是 spec-driven-dev 技能的组成部分,版本 1.0.0,2026-07-11。*

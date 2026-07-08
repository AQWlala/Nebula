# Loop 安全防护规则

> **版本**: 1.0.0
> **日期**: 2026-07-08
> **状态**: 设计稿（T-E-L-06 Task 1 产出）
> **关联**: [SKILL.md](./SKILL.md) | [NEBULA_LOOP_DESIGN.md](./NEBULA_LOOP_DESIGN.md) | [REVIEW_v1.0.md](./REVIEW_v1.0.md) | [LOOP_PATTERNS.md](./LOOP_PATTERNS.md)
> **实现依赖**: `src-tauri/src/swarm/agents/reviewer.rs`（T-E-L-03 已实现同质检测 + 降级策略）+ `src-tauri/src/autonomy/mod.rs`（L0-L5 枚举）

本文档定义 Loop 执行时的安全防护规则，是 7 专家评审共识（REVIEW_v1.0.md 风险 C）的落地规格。所有规则在 `MasterOrchestrator::execute_loop`（T-E-L-06 Task 6 集成）中强制执行，不可被 LOOP.md 声明绕过。

---

## 1. 模型同质检测口径

### 1.1 「同质」定义

Maker 与 Checker 使用**相同的 `ModelDescriptor`**——即 `provider` 与 `model_name` 两个分量均精确匹配。

| 分量 | 来源 | 示例值 |
|------|------|--------|
| `provider` | `LlmGateway::provider()` | `"ollama"` / `"deepseek"` / `"anthropic"` / `"openai-compat"` |
| `model_name` | `LlmGateway::default_model()` | `"qwen2.5:7b"` / `"deepseek-chat"` / `"claude-3-5-haiku-20241022"` |

> ⚠️ 同 provider 不同 model_name（如 Maker 用 `qwen2.5:32b`、Checker 用 `deepseek-r1:14b`，均 `provider="ollama"`）**不视为同质**，这是本地多模型降级路径的设计前提。

### 1.2 检测方法

由 `ReviewerAgent::detect_model_homogeneity()`（T-E-L-03 Commit 1 已实现）执行，返回 `Option<HomogeneityWarning>`：
- `None`：未检测到同质（Maker 与 Checker 模型不同，或 `maker_model` 未注入——旧调用方）
- `Some(warning)`：检测到同质，`warning.reason` 含人类可读说明

### 1.3 触发条件

**仅当 Loop `autonomy = L4`（蜂群模式，Maker + Checker 双 Agent）时检测**。

| 自主度 | 是否检测 | 原因 |
|--------|---------|------|
| L0-L3 | 否 | 单 Agent 模式，无 Checker 独立审查，同质检测无意义 |
| **L4** | **是** | 蜂群 + ApprovalGate，Maker-Checker 双 Agent 是核心防线 |
| L5 | 否 | 后台自动化有更严格的 Checker 要求（强 Checker + 审计日志），不在本规则范围 |

---

## 2. 自动降级触发条件

### 2.1 降级规则表

| 触发条件 | 降级目标 | 设计依据 |
|---------|---------|---------|
| **L4 + 检测到模型同质** | **L2**（对话模式，人类介入裁决） | L3 仍是单 Agent Plan 模式，无 Checker 独立审查；L2 让人类直接介入，是最安全的降级目标（7 专家评审一致同意） |
| L5 + 异常（如 CronScheduler 故障 / budget_used 读取失败） | L4 | L5 要求严格定时 + 健康的预算守护，故障时降级让 ApprovalGate 介入 |
| 连续 3 小时超 80% 月度预算 | 降级一级（L5→L4→L3，**但不低于 L3**） | 见 §5 预算阈值行为表；L3 是 Plan 模式审批底，再降会失去自动执行能力 |
| L4 + ValuesLayer 裁定 Deny | 不降级，直接阻断 | Deny 是硬性阻断，不属于自主度调整 |

### 2.2 降级目标是固定的

L4 → L2 的降级目标**固定为 L2**，不可配置为 L3 或 L1。理由：
- L3 Plan 模式仍是单 Agent，无独立 Checker 审查，模型同质问题依然存在
- L2 对话模式让人类直接裁决，是同质检测触发后唯一能恢复"人类审查"的档位
- 7 专家评审一致同意：**固定 L2，不开放配置**

### 2.3 降级是幂等的

`ReviewerAgent::enforce_homogeneity_policy(current_level)` 是**纯函数**（T-E-L-03 Commit 4 已实现），返回 `HomogeneityPolicy` 枚举：

```
HomogeneityPolicy::Enforced { original_level: L4, downgraded_to: L2, warning }
HomogeneityPolicy::NoHomogeneity { level: L4 }
HomogeneityPolicy::NotCheckerMode { level: <非 L4> }
```

- 可重复调用：Loop 每次 Action 前调用一次，结果一致
- 无副作用：不直接修改 Loop 状态，降级动作由调用方（`execute_loop`）执行
- 可测试：无需 LLM / git 即可单测（T-E-L-03 已覆盖）

---

## 3. 数据主权红线

### 3.1 Checker 模型红线

| 角色 | 允许的 provider | 禁止的 provider |
|------|----------------|----------------|
| **Checker** | `ollama`（本地，**唯一允许**） | `anthropic` / `openai-compat` / `deepseek` 等所有闭源或云端 provider |
| Maker | 任意（含闭源） | 无限制 |

> **红线依据**（REVIEW_v1.0.md 风险 C）：Checker 处理的是代码 diff，发给云端 = "把你的代码变成别人的养料"，直接违反"0 字节上行是默认态"的核心承诺。本地 Ollama 是唯一合规的 Checker provider。

### 3.2 GitHub MCP 红线

| 操作 | 允许 |
|------|------|
| `pull` / `list` / `read` / `comment` | ✅ |
| `push` / `merge` / `delete branch` / `force push` | ❌ 禁止 |

GitHub MCP 必须配置为 **pull-only**（只读）。所有写操作必须经 Shadow Workspace + 人工 PR 审查路径，不可由 Loop 直接执行 push/merge。

### 3.3 红线违反时的处置

- **立即拒绝执行**：Loop 当前 Action 中止，返回 `HomogeneityPolicy::NotCheckerMode` 或同等阻断信号
- **告警**：通过 IM webhook（飞书/企微/钉钉）推送告警，写入 loop-run-log.md（见 §6）
- **不可被 LOOP.md 绕过**：即便 LOOP.md 声明 `checker: claude-3-5-sonnet`，执行时仍强制阻断
- **不可被用户配置覆盖**：红线写在 `enforce_homogeneity_policy` 与 `execute_loop` 的硬编码逻辑中，非配置项

---

## 4. 异常突增检测

### 4.1 检测规则

单个 Loop 本小时消耗 > 月度预算的 **20%** → 立即暂停该 Loop。

| 触发阈值 | 行为 | 恢复方式 |
|---------|------|---------|
| 单 Loop 本小时消耗 ≤ 月度 20% | 正常运行 | — |
| **单 Loop 本小时消耗 > 月度 20%** | **立即暂停该 Loop** | **需用户手动恢复**（不可自动恢复） |

### 4.2 检测频率与执行者

- **检测频率**：每小时一次
- **执行者**：`budget-guardian` Loop（cadence `0 * * * *`，见 `templates/budget-guardian.md`）
- **数据源**：`CronScheduler::budget_used`（AtomicU64 内存累加值）+ STATE.md 月度累计

### 4.3 暂停后的恢复

- 暂停后**必须用户手动恢复**，不可自动恢复——异常突增通常是 Loop 陷入空转（Thrashing）或上下文漂移的信号，需人工诊断根因
- 用户通过 STATE.md 顶部「暂停的 Loop」区域手动 reset，重置该 Loop 的 `budget_used`
- 恢复前建议用户审查 loop-run-log.md 中该 Loop 最近的降级与暂停事件

---

## 5. 预算阈值行为表

| 阈值 | 行为 | 恢复方式 | 实现位置 |
|------|------|---------|---------|
| **80% 月度预算** | emit `loop_budget_warning` 事件，**不暂停** | — | CronScheduler 事件总线 + IM webhook 告警 |
| **100% 月度预算** | `LongTaskEngine::pause_all()` + emit `loop_budget_exceeded` | **需手动 reset**（用户审查后重置月度计数） | LongTaskEngine + STATE.md |
| **单 Loop > 月度 20% / 小时** | 立即暂停该 Loop | 需用户手动恢复（见 §4.3） | budget-guardian Loop |
| **连续 3 小时 > 80%** | 降级一级（L5→L4→L3，**但不低于 L3**） | 用户可手动调回（审查预算后） | budget-guardian Loop + CronScheduler |
| **云端占比 > 70%** | 优先暂停云端 Loop（保留本地 $0 Loop） | 用户可手动恢复 | budget-guardian Loop |

### 5.1 降级链边界

```
L5 → L4 → L3（停止）
```

- 降级链在 L3 处停止，不再降到 L2——L3 是 Plan 模式审批底，再降会失去自动执行能力，与 Loop 的"持续运行"目标冲突
- L3 以下（L2/L1/L0）需用户显式手动调整，不属于自动降级范围

### 5.2 云端占比优先级

当云端消耗占比 > 70% 时：
- **优先暂停云端 Loop**（provider 非 `ollama` 的 Loop）
- **保留本地 $0 Loop**（provider = `ollama` 的 Loop）
- 依据：本地 Ollama 不消耗月度预算（电费不计入 token 预算），保留本地 Loop 可维持基础自动化能力

---

## 6. 审计日志要求（与 T-E-L-07 衔接）

### 6.1 降级事件必须记录

所有降级事件（§2 降级规则表中的所有触发条件）**必须**记录到 `loop-run-log.md`（人类可读 Markdown，与 `loop-budget.md` 并列的 Loop 持久化文件）。

### 6.2 记录字段

每条降级日志必须包含以下字段：

| 字段 | 示例 | 说明 |
|------|------|------|
| 时间戳 | `2026-07-08 14:32:15 +08:00` | ISO 8601 带时区 |
| Loop 名称 | `pr-babysitter` | 来自 LOOP.md 的 `name` 字段 |
| 原自主度 | `L4` | 降级前的 AutonomyLevel |
| 降级后自主度 | `L2` | 降级后的 AutonomyLevel |
| 触发原因 | `model_homogeneity` / `budget_3h_overrun` / `cron_failure` | 枚举值 |
| 同质检测详情（若适用） | `maker=anthropic/claude-3-5-haiku, checker=anthropic/claude-3-5-haiku, reason="..."` | 仅当触发原因为 `model_homogeneity` 时填写 |

### 6.3 记录格式示例

```markdown
## 2026-07-08 14:32:15 +08:00 — Loop pr-babysitter 降级

- Loop 名称: pr-babysitter
- 原自主度: L4
- 降级后自主度: L2
- 触发原因: model_homogeneity
- 同质检测详情:
  - maker: anthropic / claude-3-5-haiku-20241022
  - checker: anthropic / claude-3-5-haiku-20241022
  - reason: "Maker 与 Checker 使用相同闭源模型，confirmation bias 风险，自动降级到 L2 由人类裁决"
```

### 6.4 STATE.md provenance 强制

STATE.md 中的 Shadow Workspace commit message + 已完成项必须包含 `loop:<name> | checker:<model> | autonomy:<L?>` 三段 provenance 字段：

```
loop:pr-babysitter | checker:ollama/qwen2.5:32b | autonomy:L4
```

- `loop:<name>`：执行该 Action 的 Loop 名称
- `checker:<model>`：Checker 使用的模型描述符（provider/model_name 格式）
- `autonomy:<L?>`：实际执行的自主度（降级后的值，非 LOOP.md 声明值）

> **设计依据**：provenance 字段让事后审计能追溯"这个 commit 是哪个 Loop、用哪个 Checker 模型、在什么自主度下产出的"——是 T-E-L-07 审计日志体系的基础数据。

### 6.5 与 T-E-L-07 的衔接

- T-E-L-07 将实现完整的 loop-run-log.md 解析与查询能力（按 Loop 名称 / 时间范围 / 降级原因过滤）
- 本文档只定义**记录格式与字段**，不定义查询接口
- T-E-L-07 实现时必须遵守本文档的记录格式，不可自行变更字段名

---

*本文档由 T-E-L-06 Task 1 产出，是 7 专家评审共识（REVIEW_v1.0.md 风险 C）的落地规格。本文件定义的规则在 `execute_loop`（T-E-L-06 Task 6 集成）中必须复用 T-E-L-03 已实现的 `enforce_homogeneity_policy`，禁止重复实现。后续 T-E-L-07（审计日志）必须遵守本文档定义的记录格式。版本 1.0.0，2026-07-08。*

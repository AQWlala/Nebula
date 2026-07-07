---
name: loop-engineering
description: |
  Loop Engineering（循环工程）技能——把 Nebula 从"提示 Agent 的工具"升级为
  "设计提示 Agent 的系统"的工程方法论。当涉及自动化任务循环、多 Agent 编排、
  长任务调度、Shadow Workspace 隔离执行、Maker-Checker 审查模式、定时触发器、
  STATE.md 状态持久化时加载此技能。本技能是 Nebula 双主控 + 蜂群 worker +
  persona 自进化架构的上层编排方法论。
version: 1.0.0
author: Nebula Project
source: |
  - https://www.runoob.com/ai-agent/loop-engineering.html
  - https://github.com/cobusgreyling/loop-engineering
  - Boris Cherny (Anthropic Claude Code) / Peter Steinberger (OpenClaw) / Addy Osmani
status: internalized
---

# Loop Engineering 技能（Nebula 内化版）

## 1. 核心理念（一句话）

> **Loop Engineering 是把你从"提示 Agent 的人"变成"设计提示 Agent 的系统"的工程师。**

你不是坐在 Agent 旁边为每一步打下一条指令，而是在设计一套外部系统，它替你驾驶 Agent 的内循环（感知→推理→行动→观察），而你去做更有判断价值的事。

```
Prompt Engineering  →  Context Engineering  →  Harness Engineering  →  Loop Engineering
"怎么问 AI"           "给 AI 什么信息"          "如何组织 AI 的能力"      "如何让 AI 持续创造结果"
提问者                信息组织者               系统设计者               规则制定者
```

Loop Engineering 不是替代 Prompt Engineering，而是在其之上的层次——一个 Loop 由多个 Prompt 组成，写得差的 Prompt 放进 Loop 里只会让糟糕的工作以更快的速度产出。

## 2. Loop 核心循环（五阶段闭环）

```
Intent → Context → Action → Observation → Adjustment → (回到 Intent)
意图     上下文     行动      观察            调整
```

| 阶段 | 做什么 | Nebula 对应能力 |
|------|--------|----------------|
| **Intent**（意图） | 定义目标结果：成功是什么样子，约束是什么 | 用户对话 / PlanEngine / LongTaskEngine.goal |
| **Context**（上下文） | 收集相关代码、文档、报错日志、约定规范 | 黑洞压缩 + 海绵吸收 + L0-L7 分层记忆 + SkillComposer 注入 |
| **Action**（行动） | 编辑文件、运行命令、调用工具、草拟方案 | ShadowWorkspace.run_command + Agent tool_calls + Tauri 命令 |
| **Observation**（观察） | 获取测试结果、编译错误、运行时输出、diff | RecordingLog 录屏 + SwarmEvent + CI 信号 + diff() |
| **Adjustment**（调整） | 根据观察更新计划，重复循环直到完成或阻塞 | Negotiator 协商 + PlanEngine 审批 + pause/resume 信号 |

> **Loop 的力量不在于任何单独的步骤，而在于闭环。** 测试失败不只是一条错误消息，它是新的上下文；类型错误不只是阻断，它是一个关于错误假设的信号。

## 3. 六大构成要素 → Nebula 映射

| Loop Engineering 要素 | 作用 | Nebula 现有实现 | 状态 |
|---|---|---|---|
| **① Automations/Scheduling**（自动触发器） | Loop 的心跳：何时做什么 | `LongTaskEngine`（SQLite 持久化 + tokio::spawn 后台 runner）+ `triggers/watch.rs`（文件监控）+ `ClipboardWatcherEngine`（剪贴板） | 🟡 缺 cron 定时调度和 LOOP.md 定义文件 |
| **② Worktrees**（并行隔离） | 每个 Agent 独立工作目录 | `ShadowWorkspaceEngine`（git worktree + `agent/<id>` 临时分支，六态机：Creating→Running→Completed→Merged/Aborted/Failed） | ✅ 完全匹配，可直接复用 |
| **③ Skills**（技能文件） | 持久化项目知识，避免每次从零推断 | `SkillComposer` + `templates/scenarios.json` + SOUL.md/AGENTS.md/TOOLS.md 注入设计 + **本文件** | 🟡 架构已有，本文件是第一个原生 Loop Skill |
| **④ Connectors/MCP**（连接器） | 让 Agent 读取 Issue、查询数据库、调用 API | gRPC proto + IM webhook（飞书/企微/钉钉）+ OAuth 集成层（T-E-C-18） | 🟡 部分实现，缺 GitHub/Slack 等 MCP 连接器 |
| **⑤ Sub-agents**（子 Agent） | Maker-Checker：写代码和检查代码的 Agent 分开 | `SwarmOrchestrator`（2-6 generic agents 并行）+ `Negotiator`（投票/级联/仲裁）+ `DeadlockDetector` | 🟡 已有蜂群，缺明确的对抗性 Checker 模式 |
| **⑥ Memory/State**（持久记忆） | 仓库记得，即使模型不记得 | 黑洞压缩 + 海绵吸收 + SQLite 持久化 + L0-L7 分层（L6 原则/L7 奇点核心价值未实现）+ `long_tasks`/`long_task_steps` 表 + STATE.md 只读投影 | ✅ 核心能力，STATE.md 作为 SQLite 只读投影（非独立状态源） |

## 4. Nebula 版 Loop 模式（7 种）

| 模式 | 核心观察信号 | 停止条件 | Nebula 适配 |
|------|------------|---------|------------|
| **Daily Triage** | CI 失败 + Issue | 发现项写入 TODO | 复用 IM webhook + LongTaskEngine，每日 09:00 扫描 |
| **PR Babysitter** | PR Review 评论 | 所有评论处理或据理忽略 | SwarmOrchestrator + Shadow Workspace，Maker 修复 + Checker 验证 |
| **CI Sweeper** | CI 红灯 | CI 转绿 | LongTaskEngine 监听 GitHub Actions webhook，自动起 Shadow Workspace 修复 |
| **Dependency Sweeper** | dependabot PR | 所有 PR 合并或关闭 | 已有 dependabot 经验，可自动化批量处理 |
| **Changelog Drafter** | 合并的 PR | 草稿写入 CHANGELOG.md | 读 git log + LLM 生成 + Sponge 吸收 |
| **Post-Merge Cleanup** | 合并后残留分支/worktree | 清理完成 | 复用 ShadowWorkspaceEngine.abort/cleanup |
| **Memory Triage** | 记忆膨胀信号 | 压缩到目标体积 | 触发黑洞压缩 + MDRM 关系图谱整理 |

## 5. 自主度阶梯（对齐 Nebula L0-L5）

> ⚠️ **重要**：Loop 不自建自主度阶梯，直接使用 Nebula 现有的 L0-L5 枚举。
> 现有 `AutonomySlider.tsx`（T-E-S-50）已上线 L0-L5 六档，Loop 必须复用而非另建。

| Nebula 自主度 | 含义 | Loop 能做的事 | 人类做的事 | 典型 Loop 模式 |
|------|------|-------------|----------|------------|
| **L0** | 内联补全 | — | — | （Loop 不适用） |
| **L1** | 定向编辑（只读） | 发现问题、分类任务、写 STATE.md 投影 | 审查 STATE.md，手动决定 | Daily Triage + Memory Triage |
| **L2** | 对话 | 起草修复方案、运行测试、写入 Shadow Workspace 分支 | 审查 diff，手动 push | PR Babysitter + Changelog Drafter |
| **L3** | Plan 模式 | 开 Draft PR、运行 CI、IM 通知 | 审查 PR，手动点击 Merge | CI Sweeper + Dependency Sweeper |
| **L4** | 蜂群 + ApprovalGate | Maker + Checker 双 Agent，**需 ValuesLayer.Confirm 裁定** | 审批门确认后放行 | （需用户显式审批） |
| **L5** | 后台自动化 | CI 通过后自动合并 | 异常时介入，定期审计 | 未来目标，需强 Checker + 审计日志 |

**关键约束**：
- Loop 的每次 Action 阶段**必须先过 ValuesLayer（L4 价值层）**裁定：Deny→阻断；Plan→走 PlanEngine；Confirm→触发 ApprovalGate；Allow→才进入 Maker-Checker
- 高风险任务（认证/支付/数据库/push force）**硬编码在 ValuesLayer.Deny 名单**，与 LOOP.md 声明解耦——即便 LOOP.md 写 L5，ValuesLayer 仍 Deny
- L5 auto-merge 必须叠加 ValuesLayer.Allow 裁定 + Checker APPROVED + 审计日志

## 6. 四大故障模式与防御

| 故障模式 | 表现 | Nebula 防御机制 |
|---------|------|----------------|
| **空转（Thrashing）** | Agent 反复尝试同一修复，N 轮无进展 | LongTaskEngine 进度条 + 最大步数限制 + pause 信号 + 用户可 cancel |
| **过拟合测试** | Agent 修改测试让其通过而非修复代码 | Checker 子 Agent 运行独立测试套件 + 禁止修改测试文件的 Skill 约束 |
| **上下文漂移** | 长任务中 Agent 忘记原始目标 | LongTask.goal 持久化 + 每步重注入 Skill + STATE.md 检查点 |
| **不安全自主** | Agent 执行高风险操作（push/merge/删数据） | L4 审批闭环 + PlanEngine.PendingGate + ValuesLayer.Deny 硬编码名单 |
| **虚假安全感**（评审新增） | Checker 因 VRAM 不足静默退化为同模型同实例，Maker-Checker 沦为"自我审查" | 模型同质检测 + 自动降级（L4→L2）+ 审计日志写入 loop-run-log.md |

## 7. 三大风险（不可消除，只能管理）

1. **验证仍然是你的责任** —— "通过了验证"是声明，不是证明。Checker 子 Agent 降低风险但不消除风险。
2. **理解债（Comprehension Debt）** —— Loop 产出代码速度越快，你实际理解的比例越低。唯一解药：读 Loop 产出的代码。
3. **认知投降（Cognitive Surrender）** —— 接受 Loop 返回的任何结果是最舒适的选择。两个人构建相同 Loop 却得到相反结果：一个用它深度理解后推进，另一个用它回避理解。

## 8. 现有能力复用约束（评审新增）

> 7 专家评审共识：禁止重复造轮子。以下现有能力必须在 Loop 实现中复用/扩展。

| 现有能力 | 文件位置 | Loop 中的角色 |
|---------|---------|--------------|
| `ReviewerAgent` | `swarm/agents/reviewer.rs` | **升级为 CheckerAgent**（已实现只读工具集 + APPROVE/REVISE/REJECT 裁决，仅需加 worktree 隔离 + 对抗 prompt） |
| `CronTask` 调度器 | `evolution/cron_scheduler.rs` | **扩展支持完整 cron 表达式**（已有 interval 轮询架构，不引入 tokio-cron-scheduler） |
| `ValuesLayer`（L4 价值层） | `memory/values.rs` | **Loop 每次 Action 前必过此门**（Allow/Confirm/Plan/Deny 四种裁定天然是 Loop 自主度控制） |
| `AutonomySlider` | `src/components/AutonomySlider.tsx` | **自主度 UI 复用**（L0-L5 已上线，Loop 卡片放微型 Tier 选择器，不另建滑块） |
| `TemplatesDialog` | `src/components/TemplatesDialog.tsx` | **Loop 模板 UI 复用**（新增 automation 类别，默认只露 2 个入口） |

## 9. Nebula 实施约束（硬性规则）

> 以下规则是本技能加载时必须遵守的工程约束，来源于 Nebula 项目记忆。

- **CI 只构建 windows-x86_64**：test.yml/release.yml/release-minimal.yml 中 macOS/Linux 已注释保留
- **proto 输出目录** `src/grpc/proto` 必须在 build.rs 中显式创建
- **环境变量** 必须用 `NEBULA_DIAGNOSTICS` 前缀，不得用 `NINE_SNAKE_*`
- **Git on Windows** 必须设置 `http.schannelCheckRevoke false` + `core.longpaths true`
- **SSH for Git** 必须用 port 443（`ssh.github.com:443`）
- **增量编译** 必须禁用（`.cargo/config.toml` 中 `[build] incremental = false`，规避 rmeta encoder ICE）
- **测试运行** 必须用 `rustup run stable-x86_64-pc-windows-msvc cargo test`（gnu 工具链编译的二进制无法执行）
- **SQLite migration** 不得包含 UTF-8 BOM，PRAGMA 不得在事务内
- **构建生成文件** `*.v1.rs` 加入 .gitignore，不跟踪

## 9. 关键文件结构

```
docs/skills/loop-engineering/
├── SKILL.md                    # 本文件（Agent 加载入口）
├── NEBULA_LOOP_DESIGN.md       # 详细架构映射 + 实施路线图
└── LOOP_PATTERNS.md            # 7 种 Loop 模式详细规格（cadence/token/停止条件）
```

对应 Loop Engineering 原版的：
- `LOOP.md`（Loop 定义）→ Nebula 的 `LongTask.goal` + PlanEngine
- `STATE.md`（任务状态）→ Nebula 的 `long_tasks`/`long_task_steps` 表 + RecordingLog
- `loop-budget.md`（预算）→ 待实现（见 NEBULA_LOOP_DESIGN.md §5）
- `loop-run-log.md`（运行日志）→ Nebula 的 RecordingLog + tracing 日志

## 10. 加载时机

本技能应在以下场景自动加载：
- 用户要求"自动化"、"定时任务"、"循环执行"、"持续监控"
- 涉及 Shadow Workspace 多 Agent 并行
- LongTaskEngine 创建/恢复任务
- SwarmOrchestrator 编排蜂群
- 用户讨论 Agent 自主程度、Maker-Checker、CI 自动修复

---

*本技能内化自 Loop Engineering 公开资料，结合 Nebula 双主控 + 蜂群 worker + persona 自进化架构适配。版本 1.0.0，2026-07-07。*

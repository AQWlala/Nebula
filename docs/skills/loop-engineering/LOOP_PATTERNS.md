# Nebula Loop 模式目录

> **版本**: 1.0.0  
> **日期**: 2026-07-07  
> **关联**: [SKILL.md](./SKILL.md) | [NEBULA_LOOP_DESIGN.md](./NEBULA_LOOP_DESIGN.md)

本文档定义 7 种生产级 Loop 模式的详细规格，供 `loop_engine/templates/` 实现参考。每个模式包含：cadence、token 预算、停止条件、自主度阶梯、Nebula 适配路径。

---

## 模式 1：Daily Triage（每日分类）

**用途**：每日扫描 CI 失败和 Issue，按优先级分类，写入 STATE.md。

```yaml
---
name: daily-triage
description: 每日扫描 CI 失败和 Issue，分类并起草 quick-win 修复
cadence: "0 9 * * 1-5"
autonomy: L1
budget_tokens: 50000
budget_minutes: 10
connectors: [github, filesystem]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 工作日 09:00 |
| 单次 Token | ~50,000 |
| 月度 Token | ~1,000,000（20 个工作日） |
| 停止条件 | STATE.md 已更新 |
| 自主度 | L1（只读 + 只写 STATE.md） |

**Nebula 适配**：
- 复用 IM webhook 读取通知（若已绑定 GitHub webhook）
- 复用 LongTaskEngine 执行分类逻辑
- 复用 SpongeEngine 吸收 CI 日志
- 输出到 `.nebula/STATE.md`（或仓库根目录 STATE.md）

**L1→L2 提升路径**：若 quick-win 项可在 5 分钟内修复，自动在 Shadow Workspace 起草，写入分支但不 push。

---

## 模式 2：PR Babysitter（PR 看护）

**用途**：监听 PR Review 评论，自动处理机械性修改请求。

```yaml
---
name: pr-babysitter
description: 监听 PR Review 评论，起草修复，Maker-Checker 验证
cadence: "on-webhook"        # 事件驱动，非 cron
autonomy: L2
budget_tokens: 80000
budget_minutes: 15
connectors: [github]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 事件驱动（PR comment webhook） |
| 单次 Token | ~80,000 |
| 停止条件 | 所有评论处理或据理忽略 |
| 自主度 | L2（写入 Shadow Workspace 分支，人工 push） |

**Nebula 适配**：
- GitHub webhook → IM webhook 复用（T-E-C-17）
- SwarmOrchestrator 起 Maker Agent 在 Shadow Workspace 修复
- Checker Agent（不同模型）对抗性审查 diff
- 修复写入 `agent/pr-<n>` 分支，待人工 push

**Maker-Checker 流程**：
1. Maker 读取评论 + 加载 Skill + 在 Shadow Workspace 修改
2. Maker 运行 `cargo test` + `cargo clippy`
3. 提交 diff 给 Checker
4. Checker 在独立 worktree 运行完整测试套件 + 对照 Skill 检查
5. APPROVED → 写入分支；REJECTED → Maker 根据反馈调整

---

## 模式 3：CI Sweeper（CI 清扫）

**用途**：CI 红灯时自动起 Shadow Workspace 修复。

```yaml
---
name: ci-sweeper
description: CI 失败时自动起草修复，Maker-Checker 验证
cadence: "on-webhook"        # GitHub Actions webhook
autonomy: L3
budget_tokens: 100000
budget_minutes: 20
connectors: [github, filesystem]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 事件驱动（CI failed webhook） |
| 单次 Token | ~100,000 |
| 停止条件 | CI 转绿 或 预算耗尽 |
| 自主度 | L3（开 Draft PR，CI 通过后人工 merge） |

**Nebula 适配**：
- GitHub Actions webhook 触发
- LongTaskEngine 创建任务，关联 Shadow Workspace
- Maker 读取 CI 日志 → 诊断 → 修复 → 测试
- Checker 验证 → 开 Draft PR → 关联 Issue
- IM webhook 通知用户

**安全约束**：
- 不修改测试文件（Skill 约束）
- 不自动 merge（L3 上限）
- 同一 CI 失败最多重试 3 次（防 Thrashing）

---

## 模式 4：Dependency Sweeper（依赖清扫）

**用途**：批量处理 dependabot PR。

```yaml
---
name: dependency-sweeper
description: 批量处理 dependabot PR，升级依赖并验证
cadence: "0 10 * * 1"        # 每周一 10:00
autonomy: L3
budget_tokens: 200000
budget_minutes: 60
connectors: [github, filesystem]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 每周一 10:00 |
| 单次 Token | ~200,000 |
| 停止条件 | 所有 PR 合并/关闭 |
| 自主度 | L3（批量验证 + Draft PR） |

**Nebula 适配**：
- 复用本项目 dependabot 处理经验（批次 A/B/C/D 策略）
- 自动分批：actions → Rust crates → 前端 deps
- 每批在 Shadow Workspace 验证（cargo check + cargo test + npm test）
- Checker 检查版本冲突（如 sha2 0.10→0.11 的 digest 冲突）
- 通过的开 Draft PR，失败的写入 STATE.md 待人工处理

---

## 模式 5：Changelog Drafter（变更日志起草）

**用途**：从合并的 PR 生成 CHANGELOG.md 草稿。

```yaml
---
name: changelog-drafter
description: 从本周合并的 PR 生成 CHANGELOG 草稿
cadence: "0 18 * * 5"        # 每周五 18:00
autonomy: L1
budget_tokens: 30000
budget_minutes: 5
connectors: [github, filesystem]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 每周五 18:00 |
| 单次 Token | ~30,000 |
| 停止条件 | CHANGELOG-draft.md 已更新 |
| 自主度 | L1（只读 git log + 只写草稿文件） |

**Nebula 适配**：
- `git log --since="last friday" --merges` 读取本周合并
- LLM 生成 conventional-changelog 格式
- SpongeEngine 吸收为 L1 记忆
- 写入 `CHANGELOG-draft.md`（不覆盖正式 CHANGELOG.md）

---

## 模式 6：Post-Merge Cleanup（合并后清理）

**用途**：合并后清理残留分支、Shadow Workspace、临时文件。

```yaml
---
name: post-merge-cleanup
description: 合并后清理残留 worktree、分支、临时文件
cadence: "on-webhook"        # PR merged webhook
autonomy: L3
budget_tokens: 10000
budget_minutes: 2
connectors: [github, filesystem]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 事件驱动（PR merged webhook） |
| 单次 Token | ~10,000 |
| 停止条件 | 清理完成 |
| 自主度 | L3（自动清理，无破坏性操作） |

**Nebula 适配**：
- 复用 `ShadowWorkspaceEngine.cleanup()` / `abort()`
- 清理 `agent/<id>` 分支
- 清理临时目录 `nebula-shadow-ws/<id>`
- 清理 `.nebula/tmp/` 超过 7 天的文件
- 记录到 loop-run-log.md

---

## 模式 7：Memory Triage（记忆分类）

**用途**：定期整理记忆，压缩膨胀，更新 MDRM 关系图谱。

```yaml
---
name: memory-triage
description: 定期触发黑洞压缩 + MDRM 整理 + 生成理解摘要
cadence: "0 3 * * 0"         # 每周日 03:00
autonomy: L2
budget_tokens: 120000
budget_minutes: 30
connectors: [filesystem]
---
```

| 属性 | 值 |
|------|-----|
| Cadence | 每周日 03:00 |
| 单次 Token | ~120,000 |
| 停止条件 | 记忆体积低于阈值 或 预算耗尽 |
| 自主度 | L2（压缩记忆 + 写入理解摘要，人工确认） |

**Nebula 适配**（评审修订）：
- 触发 ForgettingEngine.tick + blackhole.run_pass_archived（同层密度压缩，非跨层晋升；L6 原则层未实现）
- 更新 MDRM 5 维关系图谱（T-E-B-16）
- 检测记忆膨胀信号（L0 cache miss rate > 30%）
- 生成"本周理解摘要"写入 STATE.md（缓解理解债）
- SpongeEngine 吸收本周新文件

**理解债缓解**：
- 扫描本周 Loop 产出的所有 diff
- LLM 生成"这些代码做了什么"的摘要
- 写入 `.nebula/comprehension/<week>.md`
- 用户周一早上阅读，保持理解比例

---

## 模式选择指南

```
你的需求是什么？
│
├─ "每天早上知道今天该做什么"        → Daily Triage (L1)
├─ "PR Review 评论有人跟"            → PR Babysitter (L2)
├─ "CI 红灯自动修"                   → CI Sweeper (L3)
├─ "dependabot PR 别堆着"            → Dependency Sweeper (L3)
├─ "CHANGELOG 别手写"                → Changelog Drafter (L1)
├─ "合并后残留分支别堆"               → Post-Merge Cleanup (L3)
└─ "记忆别膨胀，理解别欠债"           → Memory Triage (L2)
```

## 自主度提升路径

```
L1 (只读 + 写状态文件)
  │  验证 1 周，无异常
  ▼
L2 (写 Shadow Workspace 分支，人工 push)
  │  验证 2 周，Checker 通过率 > 90%
  ▼
L3 (开 Draft PR，CI 通过后人工 merge)
  │  验证 1 月，无生产事故
  ▼
L4 (Maker + Checker 双 Agent，CI 通过后自动 merge)
  ※ 仅对低风险任务开放（如依赖升级、文档更新）
  ※ 高风险任务（如认证、支付、数据库）永远 L3 上限
```

---

## 成本估算总表（评审修订：拆分本地/云端）

> 评审修订：本地 Ollama 的 token 应 $0。L1/L2 只读/草稿类 Loop 默认本地（$0），仅 L3 复杂修复走云端 API。

| 模式 | 自主度 | 本地/云端 | 月度触发 | 月度 Token | 月度美元 |
|------|--------|----------|---------|-----------|---------|
| Daily Triage | L1 | 本地 | 20 | 1,000,000 | $0 |
| PR Babysitter | L2 | 本地 | 30 | 2,400,000 | $0 |
| CI Sweeper | L3 | 云端 | 15 | 1,500,000 | $15 |
| Dependency Sweeper | L3 | 云端 | 4 | 800,000 | $8 |
| Changelog Drafter | L1 | 本地 | 4 | 120,000 | $0 |
| Post-Merge Cleanup | L3 | 本地 | 20 | 200,000 | $0 |
| Memory Triage | L2 | 本地 | 4 | 480,000 | $0 |
| **合计** | — | — | **97** | **6,500,000** | **$23** |

**评审修订后**：月度 $23（仅 CI Sweeper + Dependency Sweeper 走云端），在 $50 预算内，无需降级。

> 若全云端运行（禁用本地 Ollama），则合计 $65，需按原建议降级。但 Nebula 默认本地优先，$23 是常态。

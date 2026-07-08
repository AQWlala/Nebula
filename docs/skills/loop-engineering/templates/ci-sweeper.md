---
name: ci-sweeper
description: 每小时扫描 CI 失败，自动修复简单问题（L4 自主度）
cadence: "0 * * * *"
autonomy: L4
budget_tokens: 100000
budget_minutes: 20
---

## Intent
自动发现并修复 CI 失败，减少人工干预，保持主干分支始终可构建。

## Context
- 读取最近 1 小时的 GitHub Actions 失败记录
- 加载项目根目录的 SKILL.md / AGENTS.md 规范
- 读取 STATE.md 了解上一轮 Loop 的处理进度

## Action
- 调用 GitHub API 获取最近 1 小时失败的 workflow runs
- 拉取失败日志，分类失败原因（编译错误 / 测试失败 / 格式问题 / 依赖缺失）
- 对可自动修复的简单问题（rustfmt / clippy / import / 依赖版本）生成 patch
- 在 Shadow Workspace 中运行 `cargo test` + `cargo clippy` 验证 patch
- 验证通过后由 Checker Agent 对抗性审查 diff
- Checker APPROVED 后提交 Draft PR 并关联失败 Issue

## Observation
- CI 失败次数和类型分布
- 自动修复成功率（patch 通过测试的比例）
- Checker 拒绝率（衡量 patch 质量的信号）
- 单次 Loop 消耗的 Token / 时间

## Adjustment
- 连续 3 次修复同一失败仍不通过 → 升级到人工处理，写入 STATE.md
- 修复成功率 < 50% → 降低自主度到 L3（不再自动开 PR，只起草草稿）
- Checker 拒绝率 > 30% → 加强 Maker 的 Skill 上下文注入
- Token 预算耗尽 → 停止本轮，剩余失败留待下一轮

## Stop Condition
- 最近 1 小时所有 CI 失败已处理（修复或升级），或
- Token / 时间预算耗尽

## Connectors
- github-actions: required
- filesystem: required

## Safety
- 不修改 .github/workflows/ 目录（CI 配置只读）
- 不修改测试文件（Skill 约束，防止通过删测试让 CI 转绿）
- 不执行 force push
- 同一 CI 失败最多重试 3 次（防 Thrashing）
- L4 上限：CI 通过后自动合并仅限低风险 patch，认证 / 支付 / 数据库相关失败永远停在 Draft PR

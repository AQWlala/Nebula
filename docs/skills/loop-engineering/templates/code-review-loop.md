---
name: code-review-loop
description: PR 变更触发，Checker 审查 + 反馈（L4 自主度）
cadence: "on-webhook"
autonomy: L4
budget_tokens: 80000
budget_minutes: 15
---

## Intent
PR 产生新 commit 时自动触发 Checker Agent 对抗性审查，提供高质量反馈，减少人工 review 负担并守住代码质量底线。

## Context
- 监听 PR push webhook 事件
- 加载 PR 的 diff + 关联的 SKILL.md 规范
- 读取项目根目录的 AGENTS.md 了解架构约束
- 加载历史 review 记录（避免重复指出已评论的问题）

## Action
- 接收 PR push webhook，拉取最新 diff
- Maker Agent 对照 SKILL.md / AGENTS.md 生成审查清单
- Checker Agent 在独立 worktree 运行 `cargo test` + `cargo clippy` + 对照审查清单逐项检查
- Checker 输出 APPROVED / REQUEST_CHANGES + 具体反馈
- REQUEST_CHANGES → 在 PR 上评论反馈 + 建议修复 patch
- 对低风险 PR（文档 / 格式 / 依赖）且 Checker APPROVED → 自动 approve PR

## Observation
- Checker 拒绝率（衡量 PR 质量）
- 反馈被作者采纳的比例（衡量反馈质量）
- Checker 与作者意见分歧的比例（衡量 Checker 准确性）
- 单次 review 消耗的 Token / 时间

## Adjustment
- Checker 拒绝率 > 50% → 通知作者加强 Skill 上下文学习
- 反馈采纳率 < 40% → 调整 Checker prompt，减少误报
- Checker 与作者分歧率 > 30% → 触发"Checker 校准"，人工抽检
- Token 预算耗尽 → 大 PR 降级为只审查关键文件

## Stop Condition
- PR 的所有 commit 已审查并给出反馈，或
- Token / 时间预算耗尽

## Connectors
- github: required
- filesystem: required

## Safety
- 不自动 merge PR（L4 仅自动 approve，merge 留人工或 L5）
- Checker 必须使用与 Maker 不同模型（模型同质检测，防盲从）
- 不修改 PR 源码（只评论 + 建议 patch，不直接提交）
- 认证 / 支付 / 数据库相关 PR 永远降级到 L3（只反馈，不自动 approve）
- 审查反馈经 ValuesLayer 红线扫描（不含凭证 / 内部信息）

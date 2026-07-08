---
name: pr-babysitter
description: PR 创建后监控 CI，失败时通知（L3 自主度）
cadence: "*/10 * * * *"
autonomy: L3
budget_tokens: 50000
budget_minutes: 10
---

## Intent
PR 创建后持续监控 CI 状态，CI 失败时及时通知作者并起草修复建议，缩短 PR 反馈周期。

## Context
- 读取当前所有 open PR 列表
- 查询每个 PR 关联的最新 CI 运行状态
- 加载 PR 作者的偏好（通知渠道 / 是否允许自动起草修复）

## Action
- 每 10 分钟轮询 open PR 的 CI 状态
- 检测到 CI failed → 拉取失败日志分类原因
- 在 PR 上评论失败摘要 + 修复建议（不直接改代码）
- 对作者启用"自动起草"偏好且属低风险的 PR：在 Shadow Workspace 起草修复分支
- 通过 IM webhook 通知 PR 作者（若已绑定）

## Observation
- PR CI 失败到通知的延迟（分钟）
- 作者响应通知的中位时间
- 自动起草修复被作者采纳的比例

## Adjustment
- 通知延迟 > 30 分钟 → 缩短轮询间隔到 5 分钟
- 作者连续 3 次忽略通知 → 降低通知频率，汇总到每日摘要
- 自动起草采纳率 < 20% → 停止自动起草，仅发通知
- 同一 PR 重复失败 → 标记为"阻塞"，不再重复通知

## Stop Condition
- 所有 open PR 的 CI 状态为 success / 无 CI，或
- Token / 时间预算耗尽

## Connectors
- github: required
- im-webhook: optional

## Safety
- 不直接修改 PR 源码（L3 上限：只评论 + 起草草稿分支）
- 不自动 merge PR
- 通知内容不包含凭证 / 密钥（ValuesLayer 红线扫描）
- 同一 PR 同一失败最多通知 2 次（避免骚扰）

---
name: daily-triage
description: 每日 09:00 整理 Issue/PR，生成摘要（L3 自主度）
cadence: "0 9 * * 1-5"
autonomy: L3
budget_tokens: 50000
budget_minutes: 15
---

## Intent
每个工作日 09:00 扫描 Issue / PR，按优先级分类并生成当日工作摘要，让用户一眼看清今天该处理什么。

## Context
- 读取最近 24 小时新增 / 更新的 Issue 和 PR
- 加载项目标签体系（bug / enhancement / question / blocked）
- 读取 STATE.md 上一轮 Loop 的遗留项

## Action
- 拉取最近 24 小时的 Issue / PR 变更
- 按优先级分类：紧急（阻塞 / CI 红）/ 重要（bug 高频）/ 一般（enhancement）/ 低（question）
- 为紧急 Issue 在 Shadow Workspace 起草 quick-win 修复草稿
- 生成"今日工作摘要"写入 STATE.md 的 `## Daily Triage` 章节
- 通过 IM webhook 推送摘要给用户（若已绑定）

## Observation
- 新增 Issue / PR 数量趋势
- 紧急项占比（衡量项目健康度）
- quick-win 修复草稿被采纳的比例
- 摘要阅读率（用户是否打开 STATE.md）

## Adjustment
- 紧急项占比 > 40% → 触发"项目健康告警"写入 STATE.md 顶部
- quick-win 采纳率 < 30% → 停止自动起草，仅做分类
- 连续 3 天摘要未读 → 降低推送频率到每周一次
- 分类置信度低 → 标记为"待人工分类"，不强行归类

## Stop Condition
- 今日摘要已写入 STATE.md，或
- Token / 时间预算耗尽

## Connectors
- github: required
- filesystem: required
- im-webhook: optional

## Safety
- L3 上限：quick-win 修复只写 Shadow Workspace 草稿，不开 PR
- 不修改 Issue 标签（避免误判影响项目治理）
- 摘要不含敏感信息（凭证 / 内部讨论）
- 读取的 Issue 内容经 ValuesLayer 红线扫描后再写入 STATE.md

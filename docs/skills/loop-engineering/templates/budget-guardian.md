---
name: budget-guardian
description: 每小时检查 Token 预算，超限暂停（L5 自主度）
cadence: "0 * * * *"
autonomy: L5
budget_tokens: 20000
budget_minutes: 5
---

## Intent
每小时检查全局 Token / 费用预算消耗，超限时自动暂停所有运行中的 Loop，防止成本失控，守住月度预算红线。

## Context
- 读取 CronScheduler 的 budget_used（AtomicU64 内存累加值）
- 加载月度预算配置（loop-budget.md 的本地 / 云端两列）
- 读取所有运行中 LongTask 的状态
- 读取 STATE.md 了解本月累计消耗

## Action
- 汇总所有 Loop 本小时 Token 消耗（本地 + 云端分开统计）
- 对比月度预算阈值：达 80% → 告警，达 100% → 暂停
- 达 80% → 通过 IM webhook 推送告警，写入 STATE.md 顶部
- 达 100% → 调用 LongTaskEngine.pause_all() 暂停所有运行中 Loop
- 生成"本小时预算消耗报告"写入 loop-run-log.md
- 检测异常消耗突增（单 Loop 本小时 > 月度预算 20%）→ 立即暂停该 Loop

## Observation
- 本小时 Token 消耗（本地 / 云端分开）
- 月度累计消耗 vs 预算阈值
- 被暂停的 Loop 数量
- 异常消耗突增事件次数

## Adjustment
- 连续 3 小时超 80% 阈值 → 降低所有 Loop 自主度一级（L5→L4→L3）
- 云端消耗占比 > 70% → 优先暂停云端 Loop，保留本地 $0 Loop
- 异常突增频发 → 缩小单 Loop 单次预算上限
- 暂停后用户手动恢复 → 重置该 Loop 的 budget_used

## Stop Condition
- 本小时预算检查完成且报告已写入，或
- Token / 时间预算耗尽（本 Loop 自身的预算）

## Connectors
- filesystem: required
- im-webhook: optional

## Safety
- L5 自主度：达 100% 阈值自动暂停，无需人工确认（成本红线硬约束）
- 暂停操作可逆：用户可通过 STATE.md 手动恢复
- 不删除 Loop 定义（只暂停运行，配置保留）
- 本 Loop 自身消耗计入预算（防"看门人"自身失控）
- 暂停操作记录完整 provenance（loop:budget-guardian | autonomy:L5）
- 若 CronScheduler 异常无法读取 budget_used → 降级到 L2，通知人工介入

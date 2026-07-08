---
name: memory-consolidation
description: 每日 03:00 L1→L2 记忆合并（L5 自主度）
cadence: "0 3 * * *"
autonomy: L5
budget_tokens: 120000
budget_minutes: 30
---

## Intent
每日凌晨 03:00 自动执行记忆合并：将 L1 事实层的高价值记忆压缩合并到 L2 摘要层，更新 MDRM 关系图谱，生成"昨日理解摘要"缓解理解债。

## Context
- 读取 L1 事实层最近 24 小时新增的记忆条目
- 加载 MDRM 5 维关系图谱（T-E-B-16）
- 读取 STATE.md 了解昨日 Loop 产出的 diff 列表
- 检测 L0 cache miss rate（记忆膨胀信号）

## Action
- 扫描 L1 新增记忆，按重要性 + 访问频率排序
- 对高价值 L1 记忆调用 ForgettingEngine.tick 进行同层密度压缩
- 调用 blackhole.run_pass_archived 执行 L1→L2 合并（非跨层晋升）
- 更新 MDRM 5 维关系图谱（时间 / 实体 / 层级 / 相似 / 因果）
- 扫描昨日 Loop 产出的所有 diff，LLM 生成"这些代码做了什么"摘要
- 将理解摘要写入 .nebula/comprehension/<date>.md
- SpongeEngine 吸收昨日新文件

## Observation
- L1 记忆体积变化（压缩前 / 后）
- L0 cache miss rate（衡量合并后检索效率）
- MDRM 关系图谱节点 / 边增量
- 理解摘要覆盖的 diff 比例

## Adjustment
- L0 cache miss rate > 30% → 加强压缩力度，合并更多低频 L1 记忆
- L1 体积未下降 → 检查 ForgettingEngine 是否异常，写入 STATE.md 告警
- 理解摘要覆盖 < 80% → 提高预算或降低 diff 摘要粒度
- 合并冲突 → 回滚到合并前状态，记录到 loop-run-log.md

## Stop Condition
- L1 记忆体积低于阈值 且 理解摘要已生成，或
- Token / 时间预算耗尽

## Connectors
- filesystem: required

## Safety
- L5 自主度：合并后 CI 通过即自动落库，无需人工确认
- 合并前自动备份 L1 记忆到 .nebula/backup/<timestamp>/
- 不删除原始 L1 记忆（只标记为 archived，保留可追溯）
- 不触碰星魂（persona 层），Loop 经验只进 L3 事实层
- 合并操作记录完整 provenance（loop:memory-consolidation | autonomy:L5）
- 若检测到合并异常 → 自动降级到 L2，暂停后续合并，通知人工

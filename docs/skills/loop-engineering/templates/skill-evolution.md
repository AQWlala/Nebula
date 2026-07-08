---
name: skill-evolution
description: 每周评估技能使用率，归档低效技能（L3 自主度）
cadence: "0 10 * * 1"
autonomy: L3
budget_tokens: 60000
budget_minutes: 20
---

## Intent
每周一 10:00 评估所有 Skill 的使用率和效果，归档低效技能，确保 Skill 库保持精炼高效，避免技能膨胀拖慢检索。

## Context
- 读取 Skill 注册表（skill_list）
- 加载最近 7 天的 Skill 调用日志（skill_audit_list）
- 读取 MDRM 中 Skill 的关系图谱（被哪些任务调用）
- 加载项目根目录的 SKILL.md 了解技能规范

## Action
- 统计每个 Skill 的 7 天调用次数 / 成功率 / 平均耗时
- 计算技能健康度得分（调用频率 × 成功率 ÷ 平均耗时）
- 对健康度 < 阈值的 Skill 生成归档建议（含归档理由）
- 在 Shadow Workspace 起草归档操作（标记为 archived，不删除）
- 生成"本周技能健康报告"写入 STATE.md 的 `## Skill Evolution` 章节
- 对高频但低成功率的 Skill 生成改进建议

## Observation
- Skill 总数变化趋势（防膨胀）
- 平均健康度得分（衡量 Skill 库整体质量）
- 归档后检索延迟变化（应下降）
- 低成功率 Skill 的改进建议被采纳比例

## Adjustment
- Skill 总数 > 100 → 提高归档阈值，加速精简
- 归档后检索延迟未下降 → 回滚归档，重新评估
- 改进建议采纳率 < 20% → 停止生成建议，只做归档
- 健康度计算异常（除零 / NaN）→ 跳过该 Skill，记录告警

## Stop Condition
- 本周健康报告已写入 STATE.md，或
- Token / 时间预算耗尽

## Connectors
- filesystem: required

## Safety
- L3 上限：归档操作只起草草稿，不直接执行（人工确认后落库）
- 不删除 Skill（只标记为 archived，保留可恢复）
- 不修改 Skill 内容（只调整状态字段）
- 核心技能（标记为 protected）不参与归档评估
- 归档建议经 ValuesLayer 红线扫描（不含凭证 / 内部信息）

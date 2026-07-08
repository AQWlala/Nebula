---
monthly_tokens: 5000000
monthly_usd: 50.0
default_per_run_tokens: 50000
default_per_run_minutes: 10
cloud_ratio_threshold: 0.7
---

# Loop 预算

> T-E-L-06 配置文件。定义全局月度预算上限、各 Loop 估算、超预算行为。
> 由 `swarm::loop_budget::LoopBudgetConfig::from_markdown` 解析。
> 数据主权:云端消耗占比 > 70% 时优先暂停云端 Loop,保留本地 $0 Loop。

## 全局预算

- 月度 Token 上限: 5,000,000(0 = 不限制)
- 月度美元上限: $50.0(0.0 = 不限制)
- 单次执行默认: 50,000 tokens / 10 min
- 云端占比阈值: 70%(超过则优先暂停云端 Loop)

### 本地($0) / 云端两列拆分

数据主权原则:本地 Ollama 执行 = $0 成本;云端执行消耗美元预算。
budget-guardian Loop 监控云端占比,超 70% 优先暂停云端 Loop。

## 各 Loop 预算

| Loop | Cadence | Token/次 | 月度估算 Token | 月度估算 USD | 本地 |
|------|---------|----------|---------------|-------------|------|
| daily-triage | 0 9 * * 1-5 | 50,000 | 1,100,000 | $0.0 | true |
| ci-sweeper | 0 * * * * | 20,000 | 800,000 | $8.0 | false |
| code-review-loop | on-webhook | 80,000 | 640,000 | $6.4 | false |
| pr-babysitter | */10 * * * * | 10,000 | 400,000 | $0.0 | true |
| memory-consolidation | 0 3 * * * | 120,000 | 960,000 | $0.0 | true |
| skill-evolution | 0 10 * * 1 | 60,000 | 240,000 | $0.0 | true |
| budget-guardian | 0 * * * * | 1,000 | 720,000 | $0.0 | true |

> 月度估算合计:4,860,000 Token / $14.4(本地 3,420,000 + 云端 1,440,000 Token)
> 云端占比:1,440,000 / 4,860,000 ≈ 29.6%(低于 70% 阈值)

## 超预算行为

### 单次超预算

- 触发条件:单次 Loop 执行 Token 或时间超 `default_per_run_tokens` / `default_per_run_minutes`
- 行为:暂停该 Loop + 写入 STATE.md + IM 通知用户
- 恢复:用户手动确认后恢复

### 月度超预算

- 触发条件:月度累计 Token ≥ `monthly_tokens` 或 USD ≥ `monthly_usd`
- 行为:停止所有运行中 Loop(budget-guardian 调用 `pause_all()`)
- 恢复:需人工通过 STATE.md 手动恢复,重置月度计数

### 异常突增

- 触发条件:单个 Loop 本小时消耗 > 月度预算的 20%
- 行为:立即暂停该 Loop + IM 告警
- 恢复:用户审查后手动恢复

### 云端占比超阈值

- 触发条件:云端消耗占比 > `cloud_ratio_threshold`(默认 70%)
- 行为:优先暂停云端 Loop,保留本地 $0 Loop
- 恢复:云端占比回落到阈值以下后自动恢复云端 Loop

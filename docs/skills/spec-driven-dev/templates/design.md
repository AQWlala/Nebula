---
change: {{CHANGE_NAME}}
domain: {{DOMAIN}}
status: draft
created: {{DATE}}
---

# Design: {{CHANGE_NAME}}

> **领域**: {{DOMAIN}}
> **关联提案**: [proposal.md](./proposal.md)
> **状态**: draft

## 设计目标

<!-- 这个 change 的技术设计要达成什么目标? -->

[描述设计目标,与 proposal.md 的"做什么"对齐。]

```
例:
1. 在 BlackholeEngine 中实现 L3→L6 蒸馏流程
2. 蒸馏触发条件: L3 容量 > 80% 或定时 03:00
3. 蒸馏输出: 语义胶囊(<=512 tokens)
4. 原始 L3 事实标记为"已蒸馏",不删除
5. UI 展示 L6 层和蒸馏状态
```

## 现状分析

<!-- 当前系统是怎么实现的?要改的部分现在长什么样? -->

### 当前架构

[描述与本 change 相关的当前架构。引用代码位置。]

```
例:
当前记忆层级(src-tauri/src/memory/mod.rs):
  MemoryLayer { L0, L1, L2, L3, L4, L5 }

BlackholeEngine(src-tauri/src/memory/blackhole.rs):
  - compress(L3_facts) -> L4_values  (L3→L4 压缩)
  - 缺少 L3→L6 蒸馏能力

存储(src-tauri/src/memory/store.rs):
  - memory_facts 表: 存 L3 事实
  - 无 L6 胶囊表
```

### 为什么现有方案不够

[说明现有架构为什么无法直接满足需求。]

```
例:
- BlackholeEngine 只有 L3→L4 压缩,无 L3→L6 蒸馏
- L4 是价值观层(不可变),L6 是蒸馏层(可压缩),语义不同
- 现有存储无 L6 表,需新增 migration
```

## 技术方案

### 方案概述

[一段话描述技术方案。]

```
例:
扩展 BlackholeEngine 新增 distill() 方法,将 L3 事实压缩为语义胶囊。
新增 l6_capsules 表存储胶囊。L3 事实新增 is_distilled 字段标记蒸馏状态。
CronScheduler 注册 03:00 定时任务触发蒸馏。
```

### 详细设计

#### 1. [组件/模块 1]

[详细描述每个组件的设计。]

```rust
// 例: BlackholeEngine 扩展
pub struct BlackholeEngine {
    // 现有字段...
    distill_threshold: f64,  // 蒸馏阈值,默认 0.8
}

impl BlackholeEngine {
    /// 将 L3 事实蒸馏为 L6 语义胶囊
    pub async fn distill(&self, facts: Vec<L3Fact>) -> Result<Vec<L6Capsule>> {
        // 1. 筛选 30 天未访问的事实
        // 2. 按 embedding 聚类
        // 3. 每类生成 <=512 token 的胶囊
        // 4. 标记原始事实为 is_distilled = true
        // 5. 返回胶囊列表
    }
}
```

#### 2. [组件/模块 2]

[继续描述其他组件。]

### 数据模型

[描述新增/修改的数据结构、数据库表。]

```sql
-- 例: 新增 L6 胶囊表
CREATE TABLE l6_capsules (
    id TEXT PRIMARY KEY,
    domain TEXT NOT NULL,           -- 来源领域
    source_fact_ids TEXT NOT NULL,  -- JSON array of L3 fact IDs
    capsule_text TEXT NOT NULL,     -- 胶囊内容(<=512 tokens)
    embedding BLOB,                 -- 胶囊 embedding
    created_at INTEGER NOT NULL,
    last_accessed_at INTEGER NOT NULL
);

-- 修改 L3 事实表
ALTER TABLE memory_facts ADD COLUMN is_distilled BOOLEAN DEFAULT 0;
ALTER TABLE memory_facts ADD COLUMN distilled_at INTEGER;
```

### API 设计

[描述新增/修改的 API 接口。]

```rust
// 例: Tauri 命令
#[tauri::command]
async fn memory_distill(manual: bool) -> Result<DistillReport, String>;

#[tauri::command]
async fn memory_get_l6_capsules(domain: String) -> Result<Vec<L6Capsule>, String>;
```

### 关键流程

[描述核心流程,可用流程图。]

```
蒸馏流程:
  Trigger (容量>80% 或 03:00 定时)
    │
    ▼
  查询 L3 中 30 天未访问且未蒸馏的事实
    │
    ▼
  按 embedding 聚类(k-means, k = facts/10)
    │
    ▼
  每类生成语义胶囊(LLM 压缩到 <=512 tokens)
    │
    ▼
  写入 l6_capsules 表
    │
    ▼
  标记原始 L3 事实 is_distilled = true
    │
    ▼
  更新 STATE.md "最后蒸馏时间"
    │
    ▼
  发送 memory:distilled 事件(IM webhook 可选通知)
```

## 替代方案

<!-- 列出考虑过的替代方案,以及为什么没选。 -->

### 方案 B: [替代方案名称]

[描述替代方案。]

**未选择原因**: [为什么没选这个方案。]

```
例:
方案 B: 直接删除 L3 旧事实而非蒸馏
未选择原因:
- 违反"记忆不丢失"的设计承诺
- 蒸馏保留语义信息,删除则永久丢失
- 用户可能需要回溯原始事实
```

### 方案 C: [替代方案名称]

[描述另一个替代方案。]

**未选择原因**: [为什么没选。]

## 风险与权衡

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| [风险描述] | [影响程度] | [缓解措施] |

```
例:
| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| 蒸馏 LLM 调用消耗 token | 月度预算 +5% | 使用本地 Ollama 模型蒸馏,不消耗云端预算 |
| 聚类算法对小数据集不稳定 | 胶囊质量差 | 事实数 < 100 时不蒸馏,等积累足够数据 |
| 蒸馏后检索精度下降 | 用户感知记忆"变模糊" | 保留原始 L3 事实(标记 is_distilled),必要时可回溯 |
```

## 向后兼容性

[描述这个变更是否破坏向后兼容,如何处理迁移。]

```
例:
- 向后兼容: L0-L5 行为不变,L6 是纯新增
- 数据库迁移: 新增 l6_capsules 表 + memory_facts 加列,不破坏现有数据
- API 兼容: 现有 memory_* 命令不变,新增 memory_distill 命令
- 配置兼容: 无新强制配置,蒸馏阈值有默认值 0.8
```

## 性能影响

[描述对性能的影响。]

```
例:
- 蒸馏流程: 异步执行,不阻塞主线程
- 蒸馏耗时: 100 条事实约 30s(本地 Ollama)
- 存储影响: L6 胶囊约为原始事实的 10%(512 tokens vs 5KB)
- 检索影响: L6 检索 < 50ms(embedding 索引)
```

## 测试策略

[描述如何测试这个变更。]

```
例:
单元测试:
- test_distill_basic: 基本蒸馏流程
- test_distill_threshold: 容量阈值触发
- test_distill_skip_recent: 跳过 30 天内事实
- test_distill_idempotent: 重复蒸馏幂等性

集成测试:
- test_distill_full_pipeline: Trigger → 蒸馏 → 存储 → STATE.md 更新
- test_distill_manual_trigger: 手动触发命令

E2E 测试:
- test_memory_map_shows_l6: UI 展示 L6
```

## 实现顺序建议

[建议 tasks.md 的实现顺序。]

```
例:
1. 先做数据模型(migration + 表)
2. 再做 BlackholeEngine::distill() 核心逻辑
3. 然后做触发机制(阈值 + 定时)
4. 最后做 UI 和文档
```

---

*本文件是 change `{{CHANGE_NAME}}` 的技术方案。提案见 [proposal.md](./proposal.md),
实现清单见 [tasks.md](./tasks.md)。*

---
change: {{CHANGE_NAME}}
domain: {{DOMAIN}}
description: {{DESCRIPTION}}
status: draft
created: {{DATE}}
---

# Proposal: {{CHANGE_NAME}}

> **领域**: {{DOMAIN}}
> **一句话描述**: {{DESCRIPTION}}
> **状态**: draft

## 为什么 (Why)

<!-- 阐述变更的动机。回答以下问题:
1. 当前系统存在什么问题?
2. 不解决这个问题会怎样?
3. 为什么现在解决?
-->

### 当前问题

[描述当前系统的痛点或缺失。引用 spec.md 中的相关 requirement 说明现状。]

```
例:
当前记忆系统(specs/memory/spec.md "记忆层级" requirement)支持 L0-L5,
但 L3 事实记忆会随时间无限增长。根据 STATE.md 统计,v2.3.0 用户平均 L3
容量已达 65%,预计 3 个月内突破 80%,导致:
- 检索延迟从 200ms 升至 1.2s
- SQLite 存储膨胀至 800MB+
- 上下文注入时 token 消耗翻倍
```

### 不解决的后果

[说明如果不做这个变更,系统会怎样。量化影响最佳。]

```
例:
若不引入蒸馏层:
- L3 容量将在 3 个月内突破 80%,检索延迟 > 2s
- 用户将被迫手动清理 L3,违反"自动记忆管理"的设计承诺
- 与 ROADMAP_v3.1.md 的"记忆自进化"目标冲突
```

### 为什么现在

[说明时机选择。]

```
例:
- v2.4.0 规划了"记忆深化"主题,是引入蒸馏层的窗口
- BlackholeEngine 在 v2.3.0 已稳定,可扩展支持蒸馏
- 竞品(如 mem0)已实现类似功能,需尽快跟进
```

## 做什么 (What)

<!-- 用非技术语言描述要做什么。技术细节放在 design.md 中。 -->

### 变更摘要

[一段话描述要做什么。]

```
例:
新增 L6 蒸馏层,在 L3 容量超过 80% 或每日 03:00 时,自动将 L3 中
30 天未访问的事实压缩为语义胶囊(<=512 tokens),存入 L6。原始 L3
事实不删除,标记为"已蒸馏"并降级为冷存储。
```

### 受影响的 Requirement

| Requirement | 变更类型 | 说明 |
|------------|---------|------|
| [requirement 名称] | ADDED / MODIFIED / REMOVED | [一句话说明] |

```
例:
| Requirement | 变更类型 | 说明 |
|------------|---------|------|
| L6 蒸馏层 | ADDED | 新增蒸馏层 requirement |
| 记忆层级 | MODIFIED | 从 L0-L5 扩展为 L0-L6 |
```

### 不做什么 (Out of Scope)

[明确说明这个 change 不做什么,避免范围蔓延。]

```
例:
本 change 不涉及:
- L7 奇点核心价值(单独 change 处理)
- 蒸馏胶囊的跨设备同步(由 sync/ 领域独立处理)
- 蒸馏算法的模型选择(使用现有 BlackholeEngine,不引入新模型)
```

## 影响评估

### 受影响的文件

[列出预计会修改的文件/模块。]

```
例:
- src-tauri/src/memory/mod.rs           — 新增 L6 层定义
- src-tauri/src/memory/blackhole.rs     — 扩展蒸馏功能
- src-tauri/src/memory/store.rs         — 新增 L6 存储
- src/migrations/xxx_add_l6.sql         — 数据库迁移
- src/components/MemoryMap.tsx          — UI 展示 L6
- docs/skills/loop-engineering/         — 更新记忆层级引用
```

### 受影响的领域

[列出受影响的 spec 领域。]

```
例:
- memory/ (主要) — 新增 L6 requirement,修改记忆层级
- skills/ (次要) — loop-engineering 技能需更新 L0-L5 引用为 L0-L6
```

## 验收标准

[这个 change 完成后,如何判断"做完了"?]

- [ ] Delta spec 中的所有 scenario 有对应测试
- [ ] `cargo test --lib` 全通过
- [ ] `npm test` 全通过
- [ ] `tsc --noEmit` 无错误
- [ ] CI 全绿
- [ ] sovereignty-check.md 所有项通过
- [ ] memory-impact.md 已评估

## 关联

- **关联 change**: [其他相关 change 名称,或"无"]
- **关联 ADR**: [ADR 编号,或"无"]
- **关联 issue**: [issue 编号,或"无"]
- **关联 ROADMAP 项**: [ROADMAP 中的条目,或"无"]

---

*本文件是 change `{{CHANGE_NAME}}` 的提案。技术方案见 [design.md](./design.md),
实现清单见 [tasks.md](./tasks.md),Delta spec 见 [specs/{{DOMAIN}}/delta.md](./specs/{{DOMAIN}}/delta.md)。*

---
change: {{CHANGE_NAME}}
domain: {{DOMAIN}}
status: draft
created: {{DATE}}
total_tasks: 0
completed_tasks: 0
---

# Tasks: {{CHANGE_NAME}}

> **领域**: {{DOMAIN}}
> **关联提案**: [proposal.md](./proposal.md)
> **关联设计**: [design.md](./design.md)
> **状态**: draft

## 实现清单

<!-- 按 design.md 的实现顺序建议排列 task。
     每个 task 格式:
     - [ ] T-XX: [task 名称] — [简述] (`[涉及文件]`)

     完成后勾选 - [x]。
     阻塞时标记 - [~] 并在下方说明原因。
-->

### 阶段 1: 数据模型

- [ ] T-01: 新增数据库迁移文件 — 创建 L6 相关表和字段 (`src/migrations/xxx_add_l6.sql`)
- [ ] T-02: 定义 L6 数据结构 — L6Capsule struct + is_distilled 字段 (`src-tauri/src/memory/types.rs`)

### 阶段 2: 核心逻辑

- [ ] T-03: 实现 BlackholeEngine::distill() — L3→L6 蒸馏核心方法 (`src-tauri/src/memory/blackhole.rs`)
- [ ] T-04: 实现蒸馏聚类算法 — 按 embedding 聚类 L3 事实 (`src-tauri/src/memory/cluster.rs`)
- [ ] T-05: 实现胶囊生成 — LLM 压缩为 <=512 tokens 胶囊 (`src-tauri/src/memory/capsule.rs`)

### 阶段 3: 触发机制

- [ ] T-06: 实现容量阈值触发 — L3 容量 > 80% 时触发蒸馏 (`src-tauri/src/memory/trigger.rs`)
- [ ] T-07: 注册定时任务 — CronScheduler 03:00 定时触发 (`src-tauri/src/evolution/cron_scheduler.rs`)

### 阶段 4: API 与存储

- [ ] T-08: 实现 L6 存储层 — l6_capsules 表 CRUD (`src-tauri/src/memory/store.rs`)
- [ ] T-09: 新增 Tauri 命令 — memory_distill + memory_get_l6_capsules (`src-tauri/src/commands/memory.rs`)
- [ ] T-10: 更新 MemoryLayer 枚举 — 加入 L6 变体 (`src-tauri/src/memory/mod.rs`)

### 阶段 5: 前端 UI

- [ ] T-11: 更新 MemoryMap 组件 — 展示 L6 层 (`src/components/MemoryMap.tsx`)
- [ ] T-12: 新增蒸馏状态指示器 — 显示最后蒸馏时间 (`src/components/DistillStatus.tsx`)

### 阶段 6: 测试

- [ ] T-13: 编写单元测试 — distill/cluster/capsule 核心逻辑 (`src-tauri/src/memory/blackhole.rs#tests`)
- [ ] T-14: 编写集成测试 — 完整蒸馏 pipeline (`src-tauri/tests/memory_distill.rs`)
- [ ] T-15: 编写前端测试 — MemoryMap L6 展示 (`src/components/__tests__/MemoryMap.test.tsx`)

### 阶段 7: 文档与收尾

- [ ] T-16: 更新 loop-engineering 技能 — L0-L5 引用改为 L0-L6 (`docs/skills/loop-engineering/SKILL.md`)
- [ ] T-17: 更新 STATE.md 模板 — 新增"最后蒸馏时间"字段 (`docs/templates/STATE.md`)
- [ ] T-18: 更新 CHANGELOG — 记录 L6 蒸馏层新增 (`CHANGELOG.md`)

## 阻塞记录

<!-- 若某 task 被阻塞,在此记录。
     格式:
     ### T-XX 阻塞
     - **阻塞原因**: [原因]
     - **阻塞时间**: [时间]
     - **解除条件**: [条件]
-->

(无阻塞)

## 完成标准

每个 task 完成需满足:
1. 代码实现完成且能编译通过
2. 对应的单元测试通过(若有)
3. 不引入新的 clippy/eslint 警告
4. 不违反 sovereignty-check.md 的任何检查项

## 依赖关系

<!-- 若 task 之间有依赖,在此标注。
     格式: T-XX depends on T-YY
-->

```
T-03 depends on T-01, T-02
T-06 depends on T-03
T-07 depends on T-03
T-08 depends on T-01, T-02
T-09 depends on T-03, T-08
T-11 depends on T-09
T-13 depends on T-03
T-14 depends on T-06, T-09
T-15 depends on T-11
```

## 进度统计

- 总 task 数: 18
- 已完成: 0
- 进行中: 0
- 阻塞: 0
- 未开始: 18

---

*本文件是 change `{{CHANGE_NAME}}` 的实现清单。完成所有 task 后,
运行 `nebula:verify --name {{CHANGE_NAME}}` 验证。*

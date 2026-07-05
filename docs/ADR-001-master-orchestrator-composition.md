# ADR-001: MasterOrchestrator 组合模式

> **状态**: 已接受  
> **日期**: 2026-07-05  
> **决策者**: 架构组  
> **关联**: SWARM_EVOLUTION_DESIGN_v2.md §1.1, §8.1  
> **解决**: P0-6 — MasterAgent 与 SwarmOrchestrator 的 fan-out 职责矛盾

---

## Context（背景）

### 问题

蜂群进化设计 v2.0 在两处对 MasterOrchestrator 与 SwarmOrchestrator 的关系给出了**互斥**描述：

- **§1.1**："MasterOrchestrator 组合 SwarmOrchestrator，复用 fan-out" → SwarmOrchestrator 负责 fan-out
- **§8.1**：`MasterAgent::dispatch_round()` 内部直接 `self.worker_pool.acquire()` + `tokio::spawn` → MasterAgent 自己 fan-out，绕过 SwarmOrchestrator

### 现有代码

`SwarmOrchestrator::execute()`（orchestrator.rs，350+ 行）是一个复杂方法，集成了：

| 子系统 | 说明 |
|--------|------|
| DynamicAgentPool | 动态 Worker 池管理 |
| TeamContextPool | 团队上下文缓存 |
| LeaderElector | 加权随机选 Leader |
| AgentBus | P2P 消息总线 |
| Negotiator | 结果协商 + 仲裁 |
| RAG (SpongeEngine + LanceDB) | 记忆检索增强 |
| ValuesLayer | L4 价值层评估 |
| PlanEngine | Plan 模式门禁 |
| SwarmCrdtSync | CRDT 跨设备同步 |
| CancellationToken | 任务取消 |
| Retry + Exponential Backoff | 容错重试 |

重写或绕过这些能力意味着 MasterAgent 要重新实现全部子系统，工作量翻倍。

### 约束

1. **bus factor=1**：单人开发，不能维护两套 fan-out 逻辑
2. **向后兼容**：现有 `swarm_execute` 命令必须继续工作
3. **BypassMode**：MasterAgent 需要"快速直通"模式，跳过 Negotiator 协商

---

## Decision（决策）

**采用方案 A：MasterOrchestrator 完全委托 SwarmOrchestrator 做 fan-out。**

### 架构

```
用户任务
  │
  ▼
MasterOrchestrator
  │
  ├─ 1. TaskDecomposer: 拆解 DAG（LLM 调用）
  │     → 输出: TaskDag { nodes: Vec<SubTask>, edges: Vec<DependencyEdge> }
  │
  ├─ 2. DAG Executor: 拓扑排序 + 按依赖执行
  │     │
  │     ▼ (每个 SubTask)
  │     SwarmOrchestrator::execute(swarm_task)  ← 复用现有 fan-out
  │       ├─ DynamicAgentPool.acquire()
  │       ├─ RAG 注入
  │       ├─ 2-6 Worker 并行
  │       ├─ Negotiator 协商 / BypassMode 直通
  │       └─ 返回 OrchestrationReport
  │
  ├─ 3. SubTaskResultMap: 收集结果 + placeholder 替换
  │
  └─ 4. Synthesizer: 综合最终输出（LLM 调用）
        → 输出: 最终结果
```

### 关键设计

1. **MasterOrchestrator 持有 `Arc<SwarmOrchestrator>`**，不持有 Worker 池
2. **SubTask → SwarmTask 适配层**：MasterAgent 把 `SubTask` 转换为 `SwarmTask` 传给 `SwarmOrchestrator::execute()`
3. **MasterAgent 不直接 acquire Worker**——删除 v2.0 §8.1 的 `dispatch_round()` 伪代码
4. **BypassMode 通过 `ExecuteMode` 参数传入** `SwarmOrchestrator::execute_with_mode(task, mode)`

```rust
/// 执行模式
pub enum ExecuteMode {
    /// 标准模式：完整 RAG + Leader + Negotiator 协商
    Standard,
    /// Bypass 模式：跳过 Negotiator，选最高置信度结果
    Bypass,
    /// Plan 模式：L4 门禁预检
    Plan,
}

impl SwarmOrchestrator {
    /// 统一执行入口（新增）
    pub async fn execute_with_mode(
        &self,
        task: SwarmTask,
        mode: ExecuteMode,
    ) -> Result<OrchestrationReport> {
        match mode {
            ExecuteMode::Standard | ExecuteMode::Plan => self.execute(task).await,
            ExecuteMode::Bypass => self.execute_bypass(task).await,
        }
    }

    /// Bypass 模式：跳过 Negotiator，直接选最高置信度
    async fn execute_bypass(&self, task: SwarmTask) -> Result<OrchestrationReport> {
        // ... fan-out 不变，只改 synthesize 阶段
    }
}
```

### SubTask → SwarmTask 适配

```rust
impl MasterOrchestrator {
    /// 将 DAG 子任务转换为 SwarmTask
    fn sub_task_to_swarm_task(&self, sub: &SubTask) -> SwarmTask {
        SwarmTask {
            description: sub.prompt.clone(),
            agent_count: sub.worker_count.unwrap_or(3),
            max_retries: sub.max_retries.unwrap_or(1),
            agents: sub.agent_kinds.clone(),
        }
    }
}
```

---

## Consequences（后果）

### 正面

- **零重复**：MasterAgent 不重新实现 fan-out/RAG/Leader/CRDT 等子系统
- **向后兼容**：`swarm_execute` 命令继续走 `execute()`，不受影响
- **BypassMode 清晰**：通过 `ExecuteMode` 枚举传入，不侵入 execute() 签名
- **Worker 池统一管理**：DynamicAgentPool 只在 SwarmOrchestrator 中存在一份

### 负面

- **SwarmTask 结构限制**：SubTask 的 `capabilities`/`work_type_hint` 等字段无法直接传给 SwarmTask（SwarmTask 只有 description/agent_count/agents）
- **execute() 签名需扩展**：可能需要新增 `execute_with_mode()` 或在 SwarmTask 中加 `mode` 字段
- **SwarmOrchestrator 改动量标为 M→L**：BypassMode 需要改 execute() 内部 Negotiator 逻辑

### 缓解

- SubTask → SwarmTask 适配层中，把 `capabilities` 编码到 `description` 或 `agents` 字段
- 新增 `execute_with_mode()` 统一入口，不破坏现有 `execute()` 签名
- BypassMode 仅影响 Negotiator 阶段，RAG/Leader/CRDT 等保持不变

---

## Alternatives（备选方案）

### 方案 B：MasterAgent 直接 fan-out（已拒绝）

MasterAgent 持有自己的 `worker_pool: DynamicAgentPool`，直接 `acquire()` + `tokio::spawn`。

**拒绝原因**：
- 需要重新实现 RAG/Leader/CRDT/Negotiator/ValuesLayer 等 10+ 子系统
- bus factor=1 下无法维护两套 fan-out
- 工时翻倍（M3 从 16-22 天涨到 30+ 天）

### 方案 C：完全合并（已拒绝）

MasterOrchestrator 吸收 SwarmOrchestrator，成为单一编排器。

**拒绝原因**：
- 破坏向后兼容（现有 `swarm_execute` 命令的调用路径全改）
- 单个 struct 过大（SwarmOrchestrator 已有 15+ 字段）

---

## 修订项

v2.0 设计文档需做以下修订：
1. §8.1 删除 `dispatch_round()` 伪代码，改为调用 `self.swarm.execute_with_mode(swarm_task, mode).await`
2. §1.1 复用清单中 SwarmOrchestrator 改动量从 "M" 改为 "L"（新增 execute_with_mode + BypassMode）
3. §10 组件清单中 MasterOrchestrator 的字段列表删除 `worker_pool`，增加 `swarm: Arc<SwarmOrchestrator>`

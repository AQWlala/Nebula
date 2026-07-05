# ADR-004: Feature Flag 策略

> **状态**: 已接受  
> **日期**: 2026-07-05  
> **决策者**: 架构组  
> **关联**: SWARM_EVOLUTION_DESIGN_v2.md §11, ADR-003 §7  
> **解决**: P0-11 — 缺少 feature flag + PR 拆分策略

---

## Context（背景）

蜂群进化设计 v2.0 引入 6 个新子系统（Soul / Evolution / DAG / MemoryDomain / MasterOrchestrator / WorkerL4），ADR-003 改造所有 LLM 调用路径。合计 2000+ 行新代码。

**无 feature flag 的风险**：
- 任一里程碑合并到 main 后，所有用户立即启用新代码路径
- 无法灰度发布和独立回滚
- 单个 PR 过大（> 1000 行），评审不可控
- bus factor=1 下，未启用代码也参与编译破坏 `cargo check`

**现有 feature flag 模式**：项目已有 `feature = "self-evolution"`（现有 evolution/ 模块）+ 运行时 `EVOLUTION_ENABLED` 环境变量双层 gate。

---

## Decision（决策）

### 1. Cargo.toml 新增 4 个 feature flag

```toml
[features]
# 现有 feature 保持不变
default = ["vector-store"]
self-evolution = []
# ... 其他现有 feature ...

# === 蜂群进化 v2.0 新增 ===

# Soul 系统（M1-M2）：SOUL.md + SoulCompiler + 注入扫描
soul-system = []

# MasterOrchestrator + DAG（M3）：MasterAgent + TaskDag + petgraph
master-orchestrator = ["dep:petgraph"]

# EvolutionEngine（M4-M5）：4 Phase 进化管线
# 与现有 self-evolution 合并：开启 self-evolution 时自动包含 evolution-engine
evolution-engine = ["self-evolution"]

# UnifiedModelDispatcher（ADR-003）：统一模型调度层
unified-dispatcher = []
```

### 2. 运行时 master switch

每个 feature flag 除了编译期 gate，还有运行时开关（沿用现有 `EVOLUTION_ENABLED` 模式）：

```rust
/// 运行时 feature flag 检查
pub struct FeatureFlags {
    soul_system_enabled: bool,
    master_orchestrator_enabled: bool,
    evolution_engine_enabled: bool,
    unified_dispatcher_enabled: bool,
}

impl FeatureFlags {
    /// 从环境变量 + Settings 初始化
    pub fn load() -> Self {
        Self {
            // 编译期 gate：feature 未启用时运行时永远 false
            soul_system_enabled: cfg!(feature = "soul-system")
                && env::var("SOUL_SYSTEM_ENABLED").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
            master_orchestrator_enabled: cfg!(feature = "master-orchestrator")
                && env::var("MASTER_ORCHESTRATOR_ENABLED").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
            evolution_engine_enabled: cfg!(feature = "evolution-engine")
                && env::var("EVOLUTION_ENABLED").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
            unified_dispatcher_enabled: cfg!(feature = "unified-dispatcher")
                && env::var("UNIFIED_DISPATCHER_ENABLED").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
        }
    }
}
```

### 3. 代码中的使用方式

```rust
// 编译期 gate：feature 未启用时不编译
#[cfg(feature = "soul-system")]
mod soul;

// 运行时 gate：feature 启用但运行时关闭时不执行
async fn handle_task(task: &Task) -> Result<()> {
    if feature_flags.soul_system_enabled {
        // 走 Soul 系统路径
        soul::handle(task).await
    } else {
        // 走旧路径（PersonaConfig）
        legacy::handle(task).await
    }
}
```

### 4. Feature flag 与里程碑映射

| Feature Flag | 编译期 Cargo feature | 运行时环境变量 | 对应里程碑 | 默认值 |
|--------------|---------------------|---------------|-----------|--------|
| Soul 系统 | `soul-system` | `SOUL_SYSTEM_ENABLED` | M1-M2 | off |
| MasterOrchestrator | `master-orchestrator` | `MASTER_ORCHESTRATOR_ENABLED` | M3 | off |
| EvolutionEngine | `evolution-engine` | `EVOLUTION_ENABLED` | M4-M5 | off |
| UnifiedDispatcher | `unified-dispatcher` | `UNIFIED_DISPATCHER_ENABLED` | M0c-M7a | off |

### 5. PR 拆分策略

| PR | 内容 | 最大行数 | Feature Flag | 前置 PR |
|----|------|---------|--------------|---------|
| PR-1 | petgraph 引入 + CI 烟囱测试 | 50 | — | — |
| PR-2 | Feature flag 框架 + Cargo.toml | 100 | — | PR-1 |
| PR-3 | UnifiedModelDispatcher 骨架 | 500 | `unified-dispatcher` | PR-2 |
| PR-4 | Soul 系统 + SoulCompiler | 600 | `soul-system` | PR-3 |
| PR-5 | domain schema migration | 300 | — | PR-4 |
| PR-6 | ACL 重写 + PrincipalDomainMap | 500 | `soul-system` | PR-5 |
| PR-7 | TaskDag + petgraph DAG | 600 | `master-orchestrator` | PR-6 |
| PR-8 | MasterOrchestrator + BypassMode | 600 | `master-orchestrator` | PR-7 |
| PR-9 | ModelRouter 迁移 | 300 | `unified-dispatcher` | PR-8 |
| PR-10 | SwarmWorker 迁移 | 400 | `unified-dispatcher` | PR-9 |
| PR-11 | EvolutionEngine 4 Phase | 800 | `evolution-engine` | PR-10 |
| PR-12 | L4 审批 + Worker L4 | 500 | `master-orchestrator` | PR-11 |
| PR-13 | 前端 Soul 编辑器 + 进化日志 | 600 | — | PR-12 |
| PR-14 | chat 命令迁移 | 200 | `unified-dispatcher` | PR-13 |
| PR-15 | 集成测试 + 回归 | 400 | — | PR-14 |

每个 PR 必须：
- < 600 行（文档除外）
- 包含对应单元测试
- 更新 `docs/CHANGELOG.md`
- CI 全绿（`cargo check` + `cargo test` + `npx tsc --noEmit`）

### 6. 回滚策略

| 场景 | 回滚方式 |
|------|---------|
| 某 feature 有 bug | 运行时关闭环境变量（无需重新编译） |
| 需要完全回退 | `git revert` 对应 PR + feature flag 保持 off |
| 数据库 migration 回退 | 执行 down migration SQL + 恢复 SQLite 备份 |
| models.json v2→v1 | 加载时检测 version=1，回退到旧逻辑 |

---

## Consequences（后果）

### 正面

- **灰度发布**：feature flag off 时旧路径不受影响
- **独立回滚**：每个 feature 可独立关闭，不影响其他
- **PR 可评审**：每个 PR < 600 行
- **编译隔离**：feature off 时不编译新代码，`cargo check` 不受影响

### 负面

- **代码膨胀**：`#[cfg(feature = "...")]` 散布在代码中
- **测试矩阵**：需要测试 feature on/off 两种组合
- **迁移完成后清理**：所有里程碑完成并稳定后，需删除 feature flag 和旧路径代码

---

## 清理计划

当所有里程碑（M0-M7）完成并稳定运行 1 个版本周期后：
1. 删除所有 `#[cfg(feature = "...")]` gate（新代码成为唯一路径）
2. 删除旧路径代码（PersonaConfig fallback / ModelRouter 直连等）
3. feature flag 变为 no-op（保留 Cargo.toml 条目但标记 deprecated）
4. 统一清理 `#[deprecated]` 标记

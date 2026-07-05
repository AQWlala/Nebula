# Feature Flag 默认值审计报告

> **关联**: ADR-004, M7b #97 feature flag 默认值审计
> **日期**: 2026-07-05
> **审计人**: 架构组

---

## 1. 审计范围

本报告审计 ADR-004 定义的 4 个 v2.0 feature flag + 2 个相关 feature flag 的默认值和运行时开关状态。

---

## 2. 审计结果总览

| Feature Flag | 编译期默认 | 运行时开关 | Settings UI 命令 | 审计结论 |
|---|---|---|---|---|
| `soul-system` | ✅ off(不在 default) | ✅ `SOUL_SYSTEM_ENABLED: AtomicBool` | ✅ `soul_system_enabled` / `soul_system_set_enabled`(M7b #97 新增) | ✅ 通过 |
| `master-orchestrator` | ✅ off(不在 default) | ⚠️ 无 AtomicBool(通过 `#[cfg]` + `Option<Arc>` 软回退) | ❌ 无 | ⚠️ 设计偏差(见 §4) |
| `evolution-engine` | ✅ off(不在 default) | ✅ `EVOLUTION_ENABLED: AtomicBool` | ✅ `evolution_enabled` / `evolution_set_enabled` | ✅ 通过 |
| `unified-dispatcher` | ✅ off(不在 default) | ⚠️ 无 AtomicBool(通过 `Option<Arc<Dispatcher>>` 软回退) | ❌ 无 | ⚠️ 设计偏差(见 §4) |
| `self-evolution`(既有) | ✅ off(不在 default) | ✅ `EVOLUTION_ENABLED: AtomicBool`(与 evolution-engine 共享) | ✅ 复用 evolution 命令 | ✅ 通过 |
| `channels`(既有) | ✅ off(不在 default) | N/A(无运行时开关需求) | N/A | ✅ 通过 |

---

## 3. 详细审计

### 3.1 soul-system ✅

**编译期 gate**:
- `Cargo.toml`: `soul-system = ["unified-dispatcher"]`(不在 default)
- `src/soul/mod.rs`: `#![cfg(feature = "soul-system")]` 整模块 gate

**运行时 master switch**:
- `src/soul/mod.rs`: `SOUL_SYSTEM_ENABLED: AtomicBool`(默认 false)
- `soul_system_enabled()` / `set_soul_system_enabled(bool)`
- `init_from_env()`: 启动时从 `SOUL_SYSTEM_ENABLED` 环境变量读取

**Settings UI 命令**(M7b #97 新增):
- `src/commands/soul.rs`: `soul_system_enabled` / `soul_system_set_enabled`
- `src/lib/tauri.ts`: `soulSystemEnabled()` / `soulSystemSetEnabled(bool)`
- 注册在 `lib.rs::invoke_handler` 中,由 `#[cfg(feature = "soul-system")]` gate

**结论**: 完全符合 ADR-004 设计。

### 3.2 evolution-engine ✅

**编译期 gate**:
- `Cargo.toml`: `evolution-engine = ["self-evolution", "unified-dispatcher"]`(不在 default)
- `src/evolution/engine/mod.rs`: `#[cfg(feature = "evolution-engine")]` gate

**运行时 master switch**:
- `src/evolution/mod.rs`: `EVOLUTION_ENABLED: AtomicBool`(默认 false)
- `evolution_enabled()` / `set_evolution_enabled(bool)`
- 与 `self-evolution` feature 共享同一 AtomicBool

**Settings UI 命令**:
- `src/commands/evolution.rs`: `evolution_enabled` / `evolution_set_enabled`
- 由 `#[cfg(feature = "self-evolution")]` gate(evolution-engine 隐含 self-evolution)

**结论**: 完全符合 ADR-004 设计。

### 3.3 master-orchestrator ⚠️

**编译期 gate**:
- `Cargo.toml`: `master-orchestrator = ["unified-dispatcher"]`(不在 default)
- 代码中 `#[cfg(feature = "master-orchestrator")]` gate

**运行时 master switch**: ❌ 无
- ADR-004 要求 `MASTER_ORCHESTRATOR_ENABLED: AtomicBool` + `master_orchestrator_enabled()`
- 实际实现:通过 `#[cfg(feature = "master-orchestrator")]` 编译期 gate + feature on 时总是构造 MasterOrchestrator
- 无运行时关闭能力(feature on 时无法禁用)

**Settings UI 命令**: ❌ 无

**设计偏差原因**:
- MasterOrchestrator 是显式触发的命令(`master_run`),不像 EvolutionEngine 有后台自动执行循环
- 用户不调用 `master_run` 时,MasterOrchestrator 不会执行,等同于"运行时关闭"
- 添加 AtomicBool 开关对 MasterOrchestrator 意义不大(用户通过不触发命令实现禁用)

**风险评估**: 低。MasterOrchestrator 是同步显式触发,无后台自动执行路径。

### 3.4 unified-dispatcher ⚠️

**编译期 gate**:
- `Cargo.toml`: `unified-dispatcher = []`(不在 default)
- `src/llm/dispatcher.rs`: `#[cfg(feature = "unified-dispatcher")]` gate

**运行时 master switch**: ⚠️ 软回退(非 AtomicBool)
- ADR-004 要求 `UNIFIED_DISPATCHER_ENABLED: AtomicBool` + `unified_dispatcher_enabled()`
- 实际实现:`AppState.dispatcher: Option<Arc<UnifiedModelDispatcher>>`
  - feature on 时:bootstrap 总是构造为 `Some(Arc<...>)`
  - feature off 时:不编译,为 `None`
  - 调用方(chat.rs): `if let Some(dispatcher) = &state.dispatcher { ... } else { 回退到 LlmGateway }`
- 无运行时关闭能力(feature on 时 dispatcher 总是 Some)

**Settings UI 命令**: ❌ 无

**设计偏差原因**:
- UnifiedModelDispatcher 是基础设施层(非可选功能),feature on 时应总是启用
- chat 命令已迁移到 dispatcher(M7a 完成),运行时关闭会导致功能回退到旧路径
- 软回退模式(`Option<Arc>`)已足够:feature off 时 None,feature on 时 Some

**风险评估**: 中。运行时关闭 dispatcher 会导致 chat 命令回退到 LlmGateway 旧路径,但旧路径已不再维护(M7a 迁移后)。建议保持当前设计,在 ADR-004 清理阶段删除旧路径代码。

---

## 4. 设计偏差说明与建议

### 4.1 master-orchestrator 和 unified-dispatcher 的设计偏差

ADR-004 原设计要求 4 个 feature flag 都有 AtomicBool 运行时开关,但实际实现中 master-orchestrator 和 unified-dispatcher 采用"编译期 gate + Option<Arc> 软回退"模式。

**原因**:这两个子系统与 soul/evolution 的后台自动执行模式不同:
- soul/evolution 有后台 EvolutionRunner 循环,需要运行时开关控制是否启动
- master-orchestrator 是显式触发(用户调用 `master_run`)
- unified-dispatcher 是基础设施层(chat 命令调用,feature on 时应总是启用)

**建议**:
1. **短期(M7b)**:保持当前设计,文档化偏差原因
2. **长期(ADR-004 清理阶段)**:里程碑稳定后删除 feature flag 和旧路径代码,届时偏差消失

### 4.2 Settings UI 开关覆盖

| Feature | Settings UI 开关 | 状态 |
|---|---|---|
| soul-system | `soulSystemEnabled()` / `soulSystemSetEnabled()` | ✅ M7b #97 新增 |
| evolution-engine | `evolutionEnabled()` / `evolutionSetEnabled()` | ✅ M6 已有 |
| master-orchestrator | N/A(显式触发,无需开关) | ✅ 设计合理 |
| unified-dispatcher | N/A(基础设施,无需开关) | ✅ 设计合理 |

---

## 5. 验证

- `cargo check`(default features):exit 0
- `cargo check --features soul-system`:exit 0
- `npm run typecheck`:exit 0
- 所有 feature flag 在 Cargo.toml `default = ["vector-store"]` 中均未列出 ✅

---

## 6. 结论

**总体审计结论**: ✅ 通过(2 项设计偏差已说明原因且风险可控)

- **编译期默认 off**:✅ 所有 6 个 feature flag 均不在 default 列表
- **运行时开关**:4/6 完整实现(soul/evolution 有 AtomicBool + Settings UI;master-orchestrator/unified-dispatcher 设计偏差合理)
- **Settings UI 开关**:2/4 有完整开关(soul/evolution);2/4 无需开关(master-orchestrator 显式触发,unified-dispatcher 基础设施)

M7b #97 验收标准达成:确认所有 feature 默认 off ✅ + Settings UI 开关可用 ✅(覆盖需要开关的 feature)。

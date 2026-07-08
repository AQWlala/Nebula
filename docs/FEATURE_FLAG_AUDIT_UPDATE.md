# Feature Flag 默认开启决策审计 — T-D-C-06

> **任务**: T-D-C-06 关键功能开关默认关闭 → 决策开启
> **日期**: 2026-07-08
> **参考**: ADR-004, `docs/FEATURE_FLAG_AUDIT.md`, `src-tauri/Cargo.toml` [features] 段, `src-tauri/src/app_config.rs`
> **约束**: 仅输出决策文档，不修改 Cargo.toml（等主会话审核后统一修改）

---

## 1. 当前状态

Cargo.toml `default = ["vector-store", "channels"]`，其余 feature 全部默认关闭。

| Feature Flag | 当前默认 | 运行时开关 | 运行时默认 | 说明 |
|---|---|---|---|---|
| `vector-store` | **ON** | N/A | N/A | LanceDB 向量存储，核心依赖 |
| `channels` | **ON** | N/A | N/A | Telegram/Discord 渠道适配器 |
| `grpc` | OFF | `NEBULA_GRPC` env var | **true** | gRPC 服务器 |
| `rest-api` | OFF | `NEBULA_REST` env var | false | REST API 服务器 |
| `unified-dispatcher` | OFF | `Option<Arc>` 软回退 | feature on = Some | 统一模型调度层 |
| `master-orchestrator` | OFF | 无 AtomicBool | N/A | 主编排器 |
| `self-evolution` | OFF | `EVOLUTION_ENABLED` A.Bool | false | 自我进化模块 |
| `soul-system` | OFF | `SOUL_SYSTEM_ENABLED` A.Bool | false | Soul 系统 |
| `evolution-engine` | OFF | `EVOLUTION_ENABLED` A.Bool | false | 进化引擎(impl self-evolution) |
| `perf-telemetry` | OFF | env var | N/A | 性能遥测 |
| `wasm-sandbox` | OFF | N/A | N/A | WASM 沙箱 |
| `mcp` | OFF | N/A | N/A | MCP 协议客户端 |
| `otel` | OFF | N/A | N/A | OpenTelemetry |
| `sqlcipher` | OFF | env var | false | DB 加密 |
| `vision` | OFF | N/A | N/A | ScreenReader |
| `openapi` | OFF | N/A | N/A | OpenAPI 工具服务器 |
| `qdrant` / `chroma` | OFF | env var | N/A | 替代向量后端 |
| `storage-s3` / `storage-webdav` | OFF | env var | N/A | 替代存储后端 |
| `json-framing` | OFF | N/A | N/A | gRPC JSON 回退 |
| `headless` | OFF | N/A | N/A | Docker 无头模式(impl grpc+rest-api) |
| `custom-protocol` | OFF | N/A | N/A | Tauri 自定义协议 |

---

## 2. 逐项评估

### 2.1 `grpc` → ✅ 推荐默认开启

**理由**: 
- 运行时开关 `grpc_enabled` 已默认 `true`（`app_config.rs:218-221`），编译期 feature 应与运行期一致
- gRPC 是 sidecar 通信的基础 IPC，headless 模式也依赖 gRPC
- 开启后增加 tonic/prost/hyper 等依赖，但仅 `sidecar` 二进制需要（`required-features = ["grpc"]`），主二进制无额外编译开销

**风险**: 低。运行时仍可通过 `NEBULA_GRPC=0` 关闭。

### 2.2 `unified-dispatcher` → ✅ 推荐默认开启

**理由**:
- ADR-003/M7a 已完成 chat 全量迁移到 Dispatcher，旧 LlmGateway 路径不再维护
- 当前 feature off 时 `AppState.dispatcher = None`，chat 回退到旧路径（无 configuration refresh、无 cost tracker 统一统计）
- 基础设施层，feature on 时应总是启用（当前已有 `Option<Arc>` 软回退模式）
- `soul-system`/`master-orchestrator`/`evolution-engine` 均隐含 `unified-dispatcher`

**风险**: 中低。开启后旧 LlmGateway 路径不再被测试覆盖，需在后续清理旧路径代码。

### 2.3 `master-orchestrator` → ⚠️ 暂缓默认开启，先补运行时开关

**理由**:
- 当前无运行时 AtomicBool 开关（ADR-004 设计偏差，见 `FEATURE_FLAG_AUDIT.md §3.3`）
- ROADMAP v3.1 §6.2 已有 T-D-C-08 任务计划为其新增 AtomicBool
- 建议 T-D-C-08 完成后随该 PR 一起默认开启

**风险**: 中低。MasterAgent 是显式触发命令，不调用等同于关闭。但无运行时开关意味着 feature on 时无法在 runtime 禁用。

### 2.4 `perf-telemetry` → ✅ 推荐默认开启

**理由**:
- 轻量级 in-process 计数器（无外部依赖），已在 bootstrap 中默认初始化
- MetricsSnapshot 已在前端 Dashboard 使用
- 编译期 gate 无新增依赖（Cargo.toml `perf-telemetry = []`）

**风险**: 几乎为零。仅增加微量内存开销（几个 AtomicU64）。

### 2.5 `self-evolution` → ❌ 保持默认关闭

**理由**:
- 有后台 EvolutionRunner 循环，运行时 AtomicBool 默认 false
- 进化引擎依赖进化算法和 LLM 调用，默认关闭避免用户无感知消耗资源
- 用户需显式通过 Setting UI 或 env var 开启

**风险**: 保持现状无风险。

### 2.6 `soul-system` → ❌ 保持默认关闭

**理由**:
- 隐含 `unified-dispatcher`（已在 2.2 推荐默认开启），但 soul-system 自身有额外复杂度（SoulCompiler + injection scan）
- 运行时 AtomicBool 默认 false，SOUL.md 需用户手动编写
- feature on 时额外编译 SoulCompiler 模块

**风险**: 低。默认 off 不影响核心功能。

### 2.7 `evolution-engine` → ❌ 保持默认关闭

**理由**:
- 隐含 `self-evolution` + `unified-dispatcher`
- 后台 4-Phase 自动执行，运行时 AtomicBool 默认 false
- 功能足够复杂，应保持 opt-in

**风险**: 低。

### 2.8 其余 Feature → ❌ 保持默认关闭

| Feature | 理由 |
|---------|------|
| `rest-api` | 运行时 `rest_enabled` 默认 false，feature on 增加 hyper/http-body |
| `wasm-sandbox` | wasmtime 24.x 是 heavy dep，且当前有 6 个已知 advisory |
| `mcp` | 协议尚在演进，默认编译增加维护负担 |
| `otel` | OTel exporter 需外部 collector，默认开启无意义 |
| `sqlcipher` | DB 加密需 migration，默认 off |
| `vision` | screenshots + image 依赖，默认 off |
| `openapi` | 专用场景（OpenAPI 工具服务器） |
| `qdrant` / `chroma` | 替换默认后端，用户按需开启 |
| `storage-s3` / `storage-webdav` | 替换默认存储，用户按需开启 |
| `json-framing` | 仅 grpc 启用时有效 |
| `headless` | Docker 专用构建模式 |
| `custom-protocol` | Tauri 深层定制 |
| `rest-api` | 运行时默认 off |

---

## 3. 推荐汇总

| Feature Flag | 当前默认 | 推荐默认 | 优先级 | 依赖 |
|---|---|---|---|---|
| `grpc` | OFF | **ON** | P0 | 无 |
| `unified-dispatcher` | OFF | **ON** | P0 | 无 |
| `master-orchestrator` | OFF | **ON**(先补 T-D-C-08) | P1 | T-D-C-08 |
| `perf-telemetry` | OFF | **ON** | P1 | 无 |
| `self-evolution` | OFF | OFF | — | — |
| `soul-system` | OFF | OFF | — | unified-dispatcher(已推荐 ON) |
| `evolution-engine` | OFF | OFF | — | self-evolution |
| 其余 | OFF | OFF | — | — |

**新的 default 列表（建议）**:
```
default = ["vector-store", "channels", "grpc", "unified-dispatcher", "perf-telemetry"]
```

**后续步骤**:
1. 主会话审核后，修改 `Cargo.toml` default 列表
2. T-D-C-08 为 `master-orchestrator` 新增 AtomicBool 开关后，加入 default
3. 清理 `unified-dispatcher` 旧路径回退代码（Long-term ADR-004 清理阶段）

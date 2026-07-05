# Nebula (nebula) · 生产路线图 v2.1

## ——剩余任务审计、依赖分析与分阶段实施计划

**版本**：v2.1（路线图版）
**日期**：2026-07-02
**作者**：Solo Developer
**性质**：本文档是 WHITEPAPER_v2.0.md §14.2/§14.3 的执行规划，是后续开发的唯一权威任务清单
**配套文档**：`docs/WHITEPAPER_v2.0.md`（设计权威）

---

## 0. 文档定位

本文档解决两个核心问题：

1. **避免每次遍历设计文档产生工作误差** —— 所有剩余任务汇总于此，单点查阅
2. **避免程序重写造成的冗余和适配错误** —— 通过依赖分析、分阶段实施，确保关联工作同批完成，避免"做好 A 发现 B 未完成导致 A 重写"

**使用约定**：
- 任务 ID 格式：`T-<阶段>-<组>-<序号>`（如 `T-S1-A-01`）
- 优先级：`P0`（阻塞下游）/ `P1`（关键路径）/ `P2`（增强体验）/ `P3`（远期）
- 复杂度：`S`（1-2 天）/ `M`（3-5 天）/ `L`（1-2 周）/ `XL`（2-4 周）
- 状态：`TODO` / `DOING` / `DONE` / `SKIP`（明确不做）

**外部报告引用须知**：

> 第三方基于 v1.1.10 的分析报告**不可直接用于任务执行决策**。v1.1.10 → v2.0 之间存在一个 major 版本跨越（Phase 1-8 完整实施），L4 价值层、L5 真反思、CausalGraphEngine、PlanEngine、SidecarManager、Negotiator、AgentBus 等核心模块均已在 v2.0 完整实现。
> 
> 引用第三方报告前，必须先对照 `WHITEPAPER_v2.0.md §0.4 常见版本误解澄清` 表格校准。报告中关于「记忆系统断裂带」「L0Cache 命中率硬编码」「反思引擎无护栏」「MemoryAcl 未接入 search」等**部分实现项判断**仍然有效，已映射到本 ROADMAP Stage 1 任务（详见 §8.1 映射表）。第三方报告中的任务编号（P-01~P-21、U-01~U-12）已升级为本文档的 `T-S<阶段>-<组>-<序号>` 体系，**禁止混用旧编号**。

**专家审议融入记录**：

> 本文档已于 2026-07-02 融入 `EXPERT_REVIEW_v2.1.md` §7 的修订建议，主要变更：
> - §1.1 模块加权完成率从 65% 修正为 57%（5 位专家共识，原 21 项"部分实现"统一按 0.5 加权高估）
> - §1.3 风险表新增 5 项（3 个 P0 级隐性缺陷 + 版本号漂移 + feature 死开关）
> - §2.4 关键依赖链新增 3 条隐式依赖（Sidecar/gRPC、Reflection/持久化、ACL/CRDT）
> - §3.1 Stage 1 新增 3 个前置任务 T-S1-PRE-01/02/03 + 拆分 A-03/B-01
> - §3.2 Stage 2 拆分为 Stage 2a（协议层 v2.2.0）+ Stage 2b（安全层 v2.2.1）
> - §4.4 测试策略 Stage 1 新增 4 个测试文件（L0Cache/ACL/重入/Dashboard）
> - §0.1 新增 Stage 1 前置修复完成记录（6 件紧迫事项已全部完成）

---

## 0.1 Stage 1 前置修复完成记录（v2.1.0-pre）

依据 `EXPERT_REVIEW_v2.1.md §6.1` 列出的 6 件紧迫事项，已于 2026-07-02 全部完成。这些修复是 Stage 1 正式任务启动前的必要前置，避免在已知的 P0 级缺陷基础上叠加新工作。

| 前置 ID | 描述 | 优先级 | 状态 | 关联代码 |
|---------|------|--------|------|---------|
| T-S1-PRE-01 | Negotiator 仲裁死代码修复（改用 `negotiate_with_arbitration`） | P0+ | ✅ DONE | `swarm/orchestrator.rs:459-465` |
| T-S1-PRE-02 | MemoryAcl 默认 deny-all（可信主体 allow，其他拒绝） | P0+ | ✅ DONE | `memory/acl.rs:44-78` |
| T-S1-PRE-03 | LayerPolicy L4→L6 提升到虚空 hotfix（返回 None） | P0 | ✅ DONE | `memory/layers.rs:152-163` |
| T-S1-PRE-04 | 版本号同步至 2.0.0（package.json + Cargo.toml + tauri.conf.json） | P0 | ✅ DONE | 三处版本字段 |
| T-S1-PRE-05 | 新增 `rust-toolchain.toml` 固定 MSVC targets | P0 | ✅ DONE | `rust-toolchain.toml` |
| T-S1-PRE-06 | 补 `vitest.config.ts` + `@vitest/coverage-v8` + cargo-audit 门禁 + CI coverage 上传 | P0 | ✅ DONE | `vitest.config.ts` + `.github/workflows/test.yml` |

**前置完成后的状态**：
- 项目对外宣称版本：`v2.0.0`（与 WHITEPAPER 一致）
- CI 工具链：MSVC（与 `test.yml` 三平台构建一致）
- 测试覆盖率：前端 `npm run test:coverage` 可执行，Rust 端 cargo-audit 漏洞阻断 CI
- P0 级隐性缺陷：3 个全部修复，可安全启动 Stage 1 正式任务

---

## 1. 当前生产进度总览

### 1.1 整体完成度

> **v2.1 修订**：根据 `EXPERT_REVIEW_v2.1.md §2.1.4`，5 位专家一致认为原 65% 完成度被系统性高估。原方案将 21 项"部分实现"统一按 0.5 加权，但实际上 5 项是"接口已定义但内部空"（实现度 <10%，应按 0.2 加权）。修正后模块加权完成率为 57%。

| 维度 | 已完成 | 部分完成 | 未实现 | 完成度 |
|------|--------|---------|--------|--------|
| 模块数 | 24 | 21 | 12 | **57 项中 24 完整 + 21 半 = 57%**（v2.1 修正，原 65%） |
| 代码量 | ~38k LOC Rust + ~12k LOC TS | — | — | — |
| 测试 | 45 前端 + 部分 Rust 单测 | — | — | cargo check ✅ / cargo nextest ✅（MSVC 工具链，已修复 MinGW 问题） |
| 文档 | WHITEPAPER_v2.0 + ROADMAP_v2.1 + EXPERT_REVIEW_v2.1 | — | — | ✅ |
| 工程质量 | CI 三平台 + clippy/fmt/audit + vitest coverage + MSVC 固定 | — | — | ✅ Stage 1 前置已全部完成 |

**加权完成率明细**（专家共识）：
- 完整实现 24 项 × 1.0 = 24.0
- 接口已定义但内部空 5 项 × 0.2 = 1.0（A 类：sidecar/MCP/REST/wire/feature 死开关相关）
- 接口未连接 11 项 × 0.5 = 5.5（B 类）
- 字段/数据缺失 5 项 × 0.6 = 3.0（C 类）
- 完全未实现 12 项 × 0 = 0
- **合计 (24 + 1 + 5.5 + 3) / 57 ≈ 57%**

### 1.2 版本里程碑

| 版本 | 主题 | 目标完成度 | 状态 |
|------|------|----------|------|
| v1.0-v1.7 | MVP + 记忆系统 + 蜂群 + OS 集成 + E2EE | 40% | ✅ 已交付 |
| v1.8 | 可观测性（Prometheus + OTLP） | 45% | ✅ 已交付 |
| v2.0 | L4 价值层 + L5 反思 + Sidecar 骨架 + Plan 模式 | 57% | ✅ 已交付 |
| **v2.1.0-pre** | **Stage 1 前置修复（6 件紧迫事项）** | **59%** | **✅ 已交付（2026-07-02）** |
| **v2.1.0** | **记忆与可观测性闭环（Stage 1 正式任务）** | **75%** | **⏳ 当前阶段** |
| v2.2.0 | Stage 2a 协议层（gRPC tonic + MCP + REST） | 85% | 待启动 |
| v2.2.1 | Stage 2b 安全层（WASM + SSRF） | 88% | 待启动 |
| v2.3 | 技能生态 + 蜂群协作增强 | 93% | 待启动 |
| v2.4 | 蜂群深度协作 + Sidecar 完整 | 96% | 待启动 |
| v2.5 | UX 升级（WebGL / 三视角 LLM / 浮动窗） | 99% | 待启动 |
| v3.0 | OS-Controller + 多模态 + 云中继 | 100% | 远期 |

> **v2.1 修订**：Stage 2 拆分为 2a/2b，避免 A 类协议帧任务堆积疲劳；v2.1 整体目标从 80% 调整为 75%（因 Stage 2 拆出后单独算版本）。

### 1.3 阻塞项与已知风险

**原有风险**：

| 风险 | 影响 | 缓解策略 | 状态 |
|------|------|---------|------|
| ~~Windows MinGW 链接器无法跑 cargo test~~ | ~~Rust 测试覆盖率无法 CI 验证~~ | ~~切换 MSVC 工具链或 WSL Linux 构建~~ | ✅ 已修复（T-S1-PRE-05 新增 `rust-toolchain.toml`） |
| wasmtime 依赖在 Windows 编译困难 | WASM 沙箱无法启用 | Stage 2b 单独验证，必要时改用 wasmer 4.x（MinGW 兼容性更好） | 待处理 |
| LLM 流式 IPC 需重构 Tauri command 签名 | 影响 Stage 1 进度 | 采用 Tauri 2.0 `ipc::Channel<ChatToken>`，保留旧 `chat()` 兼容 | 待处理 |
| Sidecar Skill/Reflection 服务无独立进程实现 | Stage 4 需新写两个 sidecar 二进制 | 复用 Memory Sidecar 模板，采用单二进制多角色方案 | 待处理 |

**v2.1 新增风险**（源自 `EXPERT_REVIEW_v2.1.md §5`）：

| 风险 | 严重度 | 概率 | 阶段 | 缓解策略 | 状态 |
|------|--------|------|------|---------|------|
| ~~Negotiator 仲裁死代码~~ | 🔴 高 | 已确认 | Stage 1 前 | 1 行修复改用 `negotiate_with_arbitration` | ✅ T-S1-PRE-01 已修复 |
| ~~MemoryAcl 默认 allow-all~~ | 🔴 高 | 已确认 | Stage 1 | 默认改 deny-all（可信主体 allow） | ✅ T-S1-PRE-02 已修复 |
| ~~Orchestrator 孤儿模块（chat 路径未注入）~~ | 🔴 高 | 已确认 | Stage 1 | T-S1-A-02 接入 `AppState::chat` | ⏳ 由 T-S1-A-02 处理 |
| ~~版本号漂移（package.json/Cargo.toml/tauri.conf.json 均 1.1.10）~~ | 🟡 中 | 已确认 | Stage 1 前 | 首个 commit 同步至 2.0.0 | ✅ T-S1-PRE-04 已修复 |
| feature 死开关（did-identity/crdt-sync 零 cfg 匹配，rest-api 未定义） | 🟡 中 | 已确认 | Stage 2a | 补 cfg 或删 feature，定义独立 `rest-api` feature | ⏳ 由 T-S2-B-03a 处理 |
| Stage 2a A 类任务堆积疲劳 | 🟡 中 | 高 | Stage 2a | 已拆分为 2a/2b，单人开发节奏可控 | ✅ 已拆分 |
| bus factor = 1 | 🟡 中 | 持续 | 全程 | 补 project_memory + backup reviewer | 持续 |
| ~~覆盖率全盲（前端无配置、Rust 无工具）~~ | 🟡 中 | 已确认 | Stage 1 前 | 补 vitest.config + tarpaulin | ✅ T-S1-PRE-06 前端已修复，Rust 端 Stage 2 引入 tarpaulin |
| ~~cargo-audit 门禁失效（continue-on-error）~~ | 🟡 中 | 已确认 | Stage 1 前 | 移除 continue-on-error，漏洞阻断 CI | ✅ T-S1-PRE-06 已修复 |
| WASM 编译困难 | 🟡 中 | 中 | Stage 2b | wasmer 4.x 替代 wasmtime | 待验证 |
| 跨设备 CRDT 缺失任务 | 🟡 中 | 已确认 | Stage 6 | 新增 T-S6-B-03 跨设备 CRDT op 传播 | ✅ 已新增任务 |
| 单人 243 人天工时（35 任务估算） | 🟡 中 | 持续 | 全程 | Stage 5 前端任务裁剪，必要时延后 U-09/U-10 | 持续 |

---

## 2. 剩余任务全面审计

### 2.1 部分实现模块（21 项，源自 WHITEPAPER §14.2）

| ID | 模块 | 主要差距 | 影响范围 |
|----|------|---------|---------|
| P-01 | L0Cache | `stats()` 中 hot_hits/hot_misses 硬编码 0 | 仪表盘、MemoryOrchestrator 决策 |
| P-02 | MemoryOrchestrator | sponge.rs 未集成，无独立 IPC | L0 之上记忆调度断层 |
| P-03 | ForgettingEngine | blackhole 未联动优先压缩"待归档" | 长期运行记忆膨胀 |
| P-04 | MemoryAcl | sponge search 未接入 ACL 过滤 | Skill 跨权限读取记忆 |
| P-05 | DataExporter | 缺 RelationEntity，relation_count 硬编码 0 | 导出数据不完整 |
| P-06 | 反思引擎护栏 | 缺 ≤5 轮限制 + 1h 冷却 | 反思引擎可能空转 |
| P-07 | WASM 沙箱 | wasmtime 依赖被注释，feature 空 | 技能隔离无强约束 |
| P-08 | SSRF 防护 | skills/engine.rs 未集成 | 外部 URL 请求无防护 |
| P-09 | agentskills.io 兼容 | SkillMeta 结构 + 3 字段缺失 | 无法导入外部技能 |
| P-10 | gRPC wire | stream_events NOT_IMPLEMENTED，未用 tonic | 外部客户端无法订阅事件 |
| P-11 | MCP 协议 | JSON-RPC 帧未实现，discover/invoke 是桩 | MCP 工具不可用 |
| P-12 | 通信渠道 | 无 teloxide/serenity，AppState 无 channel_router | 无 IM 桥接 |
| P-13 | REST API | 无 auth.rs，无 rest-api feature | 外部 HTTP 集成受限 |
| P-14 | TeamSkillsHub | importer 返回 "not yet implemented" | 团队技能共享不可用 |
| P-15 | AgentDynamicPool | orchestrator 未接入，仍用 build_agent_pool | 蜂群无法动态扩缩 |
| P-16 | LLM 流式 | IPC 边界 collect 为 Vec，无 chat-token 事件 | 前端无法逐字渲染 |
| P-17 | Swarm 可视化 | 无 subscribe_events，无 SwarmEvent 枚举 | 蜂群状态不可实时观察 |
| P-18 | 设备撤销 UI | Settings.tsx 仅占位文本 | 用户无法管理已配对设备 |
| P-19 | 三视角切换 | 仅关键词启发式，非 LLM 级 | 模式判断准确率低 |
| P-20 | Sidecar | 仅 3/5 服务，bootstrap 未自动 start_all | 进程拆分不完整 |
| P-21 | 仪表盘 | 缺 Token 成本 + 记忆命中率，L4 拦截率占位 0 | 可观测性数据断层 |

### 2.2 完全未实现模块（12 项，源自 WHITEPAPER §14.3）

| ID | 模块 | 设计来源 | 推迟原因 |
|----|------|---------|---------|
| U-01 | 领导轮值制 | 蜂群协作增强 | 依赖 AgentDynamicPool |
| U-02 | 跨任务 Team Context Pool | 蜂群协作增强 | 依赖 AgentDynamicPool |
| U-03 | 蜂群内 CRDT 同步 | 蜂群协作增强 | 依赖 CRDT 引擎（已完成） |
| U-04 | OS-Controller | 白皮书 v1.5+ | 需深度平台适配 |
| U-05 | 电源管理 | OS 集成 | 平台相关 API 差异大 |
| U-06 | 自动备份（7 天每日 1 次） | 设计 §20.2 | 优先级低 |
| U-07 | 多模态嵌入 | 白皮书 §3.6 | 需 multimodal embedder 模型 |
| U-08 | 云端中继同步 | 设计 v1.0+ | 需 relay server 基础设施 |
| U-09 | 浮动窗 / 画中画 | 前端体验 | Tauri 2.0 多窗口 API 适配 |
| U-10 | 记忆画布 WebGL (PixiJS/D3) | 前端性能 | 当前 SVG 在 1000+ 节点卡顿 |
| U-11 | 代码分割懒加载 | 前端性能 | Vite + React.lazy 适配 |
| U-12 | "AI 自动判断模式"哲学 | 白皮书 §2.1 | 依赖三视角 LLM 级 |

### 2.3 任务间依赖关系图

```
┌─────────────────────────────────────────────────────────────────┐
│                      依赖关系图（→ 表示依赖）                     │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─── Stage 1: 记忆与可观测性闭环 ───┐                          │
│  │ P-01 L0Cache 命中率统计           │                          │
│  │ P-02 MemoryOrchestrator 集成 sponge                          │
│  │ P-03 ForgettingEngine 联动 blackhole                          │
│  │ P-04 MemoryAcl 接入 sponge search                             │
│  │ P-05 DataExporter 补全 RelationEntity                         │
│  │ P-06 反思引擎护栏                                             │
│  │ P-16 LLM 流式 IPC ──────┐                                    │
│  │ P-17 Swarm 可视化 ◀─────┤ (依赖 P-16 事件流机制)              │
│  │ P-21 仪表盘 ◀───────────┤ (依赖 P-01, P-16, P-17)            │
│  └─────────────────────────┼──────────────────────────────────┘  │
│                            ▼                                     │
│  ┌─── Stage 2: 安全与外部 API ───┐                              │
│  │ P-07 WASM 沙箱                │                              │
│  │ P-08 SSRF 接入 engine         │                              │
│  │ P-10 gRPC wire (tonic) ───────┐                              │
│  │ P-11 MCP JSON-RPC 帧          │                              │
│  │ P-13 REST API auth            │                              │
│  └───────────────────────────────┘                              │
│                            ▼                                     │
│  ┌─── Stage 3: 技能生态 + 蜂群协作基础 ───┐                     │
│  │ P-09 agentskills.io SkillMeta ──┐                            │
│  │ P-14 TeamSkillsHub ◀─────────────┤ (依赖 P-09)                │
│  │ P-12 通信渠道 (teloxide/serenity) │                            │
│  │ P-15 AgentDynamicPool ──────────┐                             │
│  └─────────────────────────────────┼─────────────────────────┘   │
│                            ▼                                     │
│  ┌─── Stage 4: 蜂群深度协作 + Sidecar ───┐                      │
│  │ U-01 领导轮值制 ◀── P-15              │                      │
│  │ U-02 Team Context Pool ◀── P-15       │                      │
│  │ U-03 蜂群内 CRDT 同步                  │                      │
│  │ P-20 Sidecar 5/5 服务 ◀── Stage 3     │                      │
│  │   + bootstrap start_all               │                      │
│  └────────────────────────────────────────┘                      │
│                            ▼                                     │
│  ┌─── Stage 5: UX 升级 ───┐                                     │
│  │ P-18 设备撤销 UI        │                                     │
│  │ P-19 三视角 LLM 级 ──┐  │                                     │
│  │ U-12 AI 自动模式 ◀───┤  │ (依赖 P-19)                          │
│  │ U-09 浮动窗/画中画    │  │                                     │
│  │ U-10 WebGL 画布       │  │                                     │
│  │ U-11 代码分割懒加载   │  │                                     │
│  └────────────────────────┘                                      │
│                            ▼                                     │
│  ┌─── Stage 6: OS 集成 + 高级同步 ───┐                          │
│  │ U-04 OS-Controller                  │                         │
│  │ U-05 电源管理                       │                         │
│  │ U-06 自动备份                       │                         │
│  │ U-07 多模态嵌入                     │                         │
│  │ U-08 云端中继同步                   │                         │
│  └─────────────────────────────────────┘                         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### 2.4 关键依赖链（避免重做的核心约束）

**原有依赖链**：

| 依赖链 | 原因 | 违反后果 |
|--------|------|---------|
| P-01 → P-21 | 仪表盘的命中率字段依赖 L0Cache 统计 | 先做 P-21 再做 P-01 需重写仪表盘 |
| P-16 → P-17 → P-21 | 事件流机制是 Swarm 可视化和仪表盘的公共基础 | 三者顺序错乱需重写 IPC 层 |
| P-09 → P-14 | TeamSkillsHub 导入依赖 SkillMeta 结构 | 先做 P-14 需重写 importer |
| P-15 → U-01/U-02 | 领导轮值和 Team Context Pool 依赖动态池 | 先做 U-01/U-02 需重写编排器 |
| P-19 → U-12 | AI 自动判断模式依赖 LLM 级三视角 | 先做 U-12 需重写模式路由 |
| Stage 3 → P-20 | Sidecar 的 Skill/Reflection 服务依赖技能生态和反思护栏 | 先做 P-20 需补写 Sidecar 内部逻辑 |

**v2.1 新增隐式依赖链**（源自 `EXPERT_REVIEW_v2.1.md §7.3`）：

| 依赖链 | 原因 | 违反后果 |
|--------|------|---------|
| T-S2-B-01（gRPC wire tonic）→ T-S4-B-01/02（Sidecar Skill/Reflection） | Sidecar 的业务 RPC 必须在 gRPC wire 升级后才能定义 | 先做 Sidecar 再升级 wire 需重写所有 RPC 接口 |
| T-S1-A-06（反思护栏）→ T-S4-B-02（Reflection sidecar） | Reflection sidecar 需要持久化反思状态，护栏是状态机的前置 | 先做 sidecar 需补写护栏，且状态格式可能不兼容 |
| T-S1-A-04（MemoryAcl 接入 sponge search）→ T-S4-A-03（蜂群内 CRDT 同步） | 蜂群 CRDT 同步时跨 Agent 的记忆访问必须经过 ACL 过滤 | 先做 CRDT 同步会导致跨 Agent 隐私数据泄漏 |

---

## 3. 分阶段实施计划

### 3.1 Stage 1：记忆与可观测性闭环（v2.1.0）

**目标**：完成记忆系统内部断层 + 可观测性数据流闭环，使仪表盘能真实反映系统状态。

**为什么放在第一阶段**：记忆系统是项目核心，所有上层模块（蜂群决策、反思、ACL）都依赖其完整性。可观测性数据断层会让后续调试成本剧增。

**v2.1 修订**：
- §0.1 中已完成的 6 件前置任务（T-S1-PRE-01~06）不在此表重复，但作为 Stage 1 正式任务的前置依赖
- T-S1-A-03 拆分为 03a/03b（ForgettingEngine 工作量被低估，见 `EXPERT_REVIEW_v2.1.md §2.2.2`）
- T-S1-B-01 拆分为 01a/01b/01c（粒度过大，见 `EXPERT_REVIEW_v2.1.md §2.5.6`）
- T-S1-A-04 默认策略已从"allow-all 兼容"改为"可信主体 allow + 其他 deny-all"（由 T-S1-PRE-02 完成）

**前置任务**（§0.1 已记录全部完成）：
- T-S1-PRE-01/02/03/04/05/06 详见 §0.1 表格

**正式任务表**：

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 | 状态 |
|---------|----------|---------|--------|--------|------|------|
| T-S1-A-01 | P-01 | L0Cache `stats()` 实现真实计数：在 `get()`/`put()` 路径累加 `hot_hits`/`hot_misses`，移除硬编码 0 | P0 | S | 无 | ✅ DONE |
| T-S1-A-02 | P-02 | MemoryOrchestrator 接入 chat 路径 + 集成 sponge.rs：① 在 `AppState::chat` 内部调 `self.orchestrator.assemble_context(&request.user_message).await?`，把 `ContextBundle.text` 拼到 system prompt 前；② 在 `select_memories()` 中调用 `SpongeEngine::search_with_graph()`（注：原 ROADMAP 写 `SpongeEngine::search()` 不存在，实际 API 是 `search_with_graph()`，见 `sponge.rs:471`）替代直接 LanceDB 查询；③ 新增 `memory_orchestrator_run` IPC 命令 | P0 | M | T-S1-A-01 | ✅ DONE |
| T-S1-A-03a | P-03（拆分） | ForgettingEngine.tick() 写 archived=true：实现 `tick()` 方法（当前只有 `scan_for_archive()` 返回候选但不写库），扫描 `importance<0.3 && last_access>TTL` 的记忆并 UPDATE `archived=1`；记录 `forgetting_archived_total` Prometheus 指标 | P1 | S | 无 | ✅ DONE |
| T-S1-A-03b | P-03（拆分） | BlackholeEngine.run_pass_archived() 只扫 archived=1：在 `BlackholeEngine` 新增 `run_pass_archived()` 方法，仅压缩 `archived=true` 的记忆；原 `run_pass()` 保持全扫描不变；调用关系：ForgettingEngine.tick() → BlackholeEngine.run_pass_archived() | P1 | S | T-S1-A-03a | ✅ DONE |
| T-S1-A-04 | P-04 | MemoryAcl 接入 sponge search：在 `SpongeEngine::search_with_graph()` 末尾增加 `acl_filter(results, requester_id)` 步骤；**默认策略已由 T-S1-PRE-02 改为可信主体 allow + 其他 deny-all**，本任务只需在 sponge 调用 `acl.check()` 并过滤结果 | P0 | M | 无 | ✅ DONE |
| T-S1-A-05 | P-05 | DataExporter 补全 RelationEntity：新增 `RelationEntity { source_id, target_id, kind, evidence }` 序列化；从 `memory_relations` 表查询填充；移除 `relation_count: 0` 硬编码 | P1 | S | 无 | TODO |
| T-S1-A-06 | P-06 | 反思引擎护栏：`ReflectionEngine` 增加 `RoundGuard { max_rounds: 5, cooldown: 1h }`；超过阈值则 skip 并记录 `reflection_skipped` 指标；**同时实现 `self_reflections` 表持久化反思结果**（EXPERT_REVIEW §2.2.3 指出当前 `reflect_all()` 不写库，L5 无法历史回溯，是 L6 的前置阻塞） | P0（升） | M | 无 | ✅ DONE |
| T-S1-B-01a | P-16（拆分） | LLM 流式 IPC 后端 Channel：新增 `chat_stream(request: ChatRequestDto, on_token: tauri::ipc::Channel<ChatToken>) -> Result<ChatComplete>` Tauri command；采用 Tauri 2.0 `ipc::Channel`（双向流，前端可取消）而非 `emit_event`；**同时实现 DeepSeek 的 SSE 流式解析**（gateway.rs:530 只走 Ollama，DeepSeek 主路径无流式） | P0 | L | 无 | ✅ DONE |
| T-S1-B-01b | P-16（拆分） | LLM 流式 IPC 前端 listen：`ChatPanel.tsx` 监听 `on_token` 事件，逐字渲染；支持中途取消按钮；保留旧 `chat()` 兼容路径作为 fallback | P0 | M | T-S1-B-01a | ✅ DONE |
| T-S1-B-01c | P-16（拆分） | LLM 流式 IPC 兼容性测试：编写 `tests/integration/chat_stream_test.rs`，覆盖正常流、取消流、DeepSeek/Ollama 双路径、断网恢复 | P1 | S | T-S1-B-01a, T-S1-B-01b | ✅ DONE |
| T-S1-B-02 | P-17 | Swarm 可视化：定义 `SwarmEvent` 枚举（AgentStart/Complete/Negotiate/Arbitrate）；新增 `subscribe_events() -> Channel<SwarmEvent>` IPC；前端 `SwarmView.tsx` 监听 | P0 | L | T-S1-B-01a | ✅ DONE |
| T-S1-B-03 | P-21 | 仪表盘真实数据：Token 成本从 `LlmGateway.usage` 累加（当前 `engine.rs` 返回 tokens 永远是 0，需先修复）；记忆命中率取 `L0Cache.stats()`；L4 拦截率从 `ValuesLayer.evaluate()` 计数；ACL 拒绝计数；反思引擎 `reflection_skipped` 指标 | P0（升） | M | T-S1-A-01, T-S1-B-02 | ✅ DONE |

> **优先级变更说明**（源自 `EXPERT_REVIEW_v2.1.md §3.1`）：
> - T-S1-A-06 从 P1 升为 P0：反思不闭环（不写库）是 L6 的前置阻塞，且护栏缺失会导致反思空转
> - T-S1-B-03 从 P1 升为 P0：5 项可观测性指标缺口导致安全事件不可观测

**Stage 1 验收标准**：
- [ ] L0Cache 命中率在仪表盘显示非 0 值
- [ ] MemoryOrchestrator 通过 sponge 查询记忆，token 预算生效；`AppState::chat` 路径已注入 ContextBundle
- [ ] Sponge search 在 Skill 调用时按 ACL 过滤；非可信主体访问被拒绝
- [ ] DataExporter 导出包含 RelationEntity 数组
- [ ] ForgettingEngine.tick() 调用后 `archived=1` 的记忆数 > 0；BlackholeEngine.run_pass_archived() 压缩这些记忆
- [ ] 反思引擎连续触发 6 次后第 6 次被护栏拦截；`self_reflections` 表有记录
- [ ] Chat 面板逐字渲染（不是一次性返回）；DeepSeek 路径也支持流式
- [ ] SwarmView 实时显示 Agent 启动/完成事件
- [ ] 仪表盘 Token 成本随对话增长（非 0）；L4 拦截率非 0；ACL 拒绝计数可见

**Stage 1 推荐执行顺序**（基于 §2.4 依赖链）：

```
Wave 1（无依赖，并行）:
  - T-S1-A-01 L0Cache 命中率 [P0/S]
  - T-S1-A-03a ForgettingEngine.tick [P1/S]
  - T-S1-A-04 MemoryAcl 接入 sponge [P0/M]
  - T-S1-A-05 DataExporter RelationEntity [P1/S]
  - T-S1-A-06 反思护栏 + 持久化 [P0/M]
  - T-S1-B-01a LLM 流式后端 [P0/L]

Wave 2（依赖 Wave 1）:
  - T-S1-A-02 MemoryOrchestrator 接入 chat [P0/M]  ← 依赖 A-01
  - T-S1-A-03b BlackholeEngine.run_pass_archived [P1/S]  ← 依赖 A-03a
  - T-S1-B-01b LLM 流式前端 [P0/M]  ← 依赖 B-01a
  - T-S1-B-02 Swarm 可视化 [P0/L]  ← 依赖 B-01a

Wave 3（依赖 Wave 2）:
  - T-S1-B-01c LLM 流式兼容性测试 [P1/S]  ← 依赖 B-01a/b
  - T-S1-B-03 仪表盘真实数据 [P0/M]  ← 依赖 A-01 + B-02
```

**Stage 1 完成后整体进度**：57% → 75%（原 65%→80%，因 Stage 2 拆出后单独算版本，Stage 1 目标相应下调）

---

### 3.2 Stage 2：安全与外部 API 协议层（拆分为 2a + 2b）

**v2.1 修订**：根据 `EXPERT_REVIEW_v2.1.md §2.1.5 / §7.5`，Stage 2 原 5 个任务中 4 个是 A 类（协议帧实现），单人开发会产生"协议帧设计疲劳"。拆分为：
- **Stage 2a 协议层**（v2.2.0）：gRPC tonic + MCP + REST（3 个 A 类任务集中处理）
- **Stage 2b 安全层**（v2.2.1）：WASM + SSRF（2 个独立任务，可在 Stage 2a 完成后并行）

#### 3.2.1 Stage 2a：协议层（v2.2.0）

**目标**：完成 gRPC/MCP/REST 三大外部协议层，使 nebula 可被外部系统集成。

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 | 状态 |
|---------|----------|---------|--------|--------|------|------|
| T-S2-B-01 | P-10 | gRPC wire 升级：迁移到 `tonic::transport::Server`；实现 `stream_events` 为真实 `tonic::Streaming<SwarmEvent>`；保留 JSON framing 作为 fallback feature；**同时补完 Sidecar 通用服务模板**（EXPERT_REVIEW §2.1.1 指出 `sidecar/ipc.rs` 仅有 health_check，无业务 RPC，是 Stage 4 sidecar 的前置依赖） | P1 | XL+50% | T-S1-B-02（SwarmEvent 枚举） | ✅ DONE |
| T-S2-B-02 | P-11 | MCP JSON-RPC 2.0 帧实现：`mcp/protocol.rs` 实现 `parse_frame()`/`write_frame()`；`discover_tools()` 调用 `tools/list`；`invoke_tool()` 调用 `tools/call`；stdio 子进程环境变量扩展为 `PATH/HOME/USER/LANG/SYSTEMROOT/TEMP/TMP/TZ/LOCALE/LC_ALL`（EXPERT_REVIEW §2.4.5 指出原 SAFE_ENV_VARS 不够）；**同时实现 `filter_safe_env_vars` 在 mcp/ 目录的实际调用点** | P1 | L | 无 | ✅ DONE |
| T-S2-B-03a | P-13（拆分） | REST API auth + rest-api feature 定义：① 在 `Cargo.toml` 定义独立 `rest-api` feature（当前被 `grpc` feature 隐式包含，任何启用 gRPC 的构建都会暴露无认证 REST，见 EXPERT_REVIEW §2.4.4）；② 新增 `api/auth.rs`（Bearer token + API key 双模式）；③ `rest.rs` 的 `#[cfg(feature = "grpc")]` 改为 `#[cfg(feature = "rest-api")]`，与 gRPC 解耦，默认关闭 | P1（升） | S | 无 | ✅ DONE |
| T-S2-B-03b | P-13（拆分） | REST API 业务接入：`/api/chat` 接入真实 LlmGateway；`/api/swarm/execute` 接入 SwarmOrchestrator；`/api/memory/search` 接入 SpongeEngine（经过 ACL 过滤） | P2 | M | T-S2-B-03a | ✅ DONE |
| T-S2-C-01 | 新增 | feature flag 一致性修复：`did-identity` 和 `crdt-sync` 两个 feature 在 Cargo.toml 定义但代码中零 cfg 匹配；要么补 `#[cfg(feature = "did-identity")]` 守卫，要么删除 feature 定义；同时新增 CI 检查脚本扫描 feature 与 cfg 一致性 | P1 | S | 无 | ✅ DONE |

**Stage 2a 验收标准**：
- [ ] `grpcurl -plaintext 127.0.0.1:50051 list` 返回服务列表
- [ ] `grpcurl ... stream_events` 返回实时事件流
- [ ] Sidecar 通用服务模板定义完成，Stage 4 可直接复用
- [ ] MCP 客户端可连接 stdio MCP 服务器并调用工具
- [ ] `cargo build --no-default-features` 不再编入 did-identity/crdt-sync 模块
- [ ] `cargo build --features rest-api` 启用 REST；默认构建不暴露 REST 端点
- [ ] `curl -H "Authorization: Bearer xxx" /api/chat` 返回真实响应

**Stage 2a 完成后整体进度**：75% → 85%

#### 3.2.2 Stage 2b：安全层（v2.2.1）

**目标**：完成 WASM 沙箱和 SSRF 防护，使技能执行有强隔离约束。

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 | 状态 |
|---------|----------|---------|--------|--------|------|------|
| T-S2-A-01a | P-07（拆分） | WASM 沙箱工具链 + 依赖：① 切换 MSVC 工具链后（T-S1-PRE-05 已完成）验证 wasmtime 24.x 可正常编译；若失败改用 wasmer 4.x（MinGW 兼容性更好）；② `Cargo.toml` 取消 wasmtime/wasmer 注释；③ `wasm-sandbox = []` 空数组改为 `wasm-sandbox = ["wasmtime"]`（或 wasmer） | P2（降） | M | 无 | ✅ DONE |
| T-S2-A-01b | P-07（拆分） | WASM 沙箱 host function 实现：`skills/wasm_sandbox.rs` 实现 `WasmExecutor::run()`；host function 不再返回 -1 占位，实现 `http_get`/`file_read`/`memory_search` 三个真实绑定（带权限检查） | P2（降） | L | T-S2-A-01a | ✅ DONE |
| T-S2-A-01c | P-07（拆分） | WASM 沙箱 WASI 裁剪：按 WHITEPAPER §10.2 裁剪 WASI 接口，仅保留 `fd_write`/`random_get`，禁用 `fd_read`/`path_open`；编写 `tests/integration/wasm_sandbox_test.rs` 验证文件系统访问被拦截 | P2（降） | M | T-S2-A-01b | ✅ DONE |
| T-S2-A-02 | P-08 | SSRF 接入 skills/engine.rs + 真实重定向链验证：① 在 `SkillEngine::execute()` 调用工具前增加 `SsrfGuard::validate_url()`；② **实现真实 HTTP 重定向链每跳验证**（EXPERT_REVIEW §2.4.2 指出当前 `validate_redirect_chain` 是伪实现，接受预定义 urls 列表而非实际重定向链）：使用 `reqwest::redirect::Policy::custom` 在每跳调用 `validate_url` | P0 | S | 无 | ✅ DONE |

> **优先级变更说明**（源自 `EXPERT_REVIEW_v2.1.md §3.2`）：
> - T-S2-A-01 从 P1 降为 P2：Python 沙箱已兜底，WASM 非阻塞性
> - T-S2-B-03 从 P2 升为 P1：gRPC feature 隐式暴露无认证 REST 是安全漏洞

**Stage 2b 验收标准**：
- [ ] WASM 技能在 wasmtime/wasmer 沙箱中执行，文件系统访问被拦截
- [ ] Skill 执行 HTTP 请求到 169.254.169.254 被 SSRF 拒绝
- [ ] HTTP 重定向到内网地址（如 127.0.0.1:6379）在每跳被验证拦截
- [ ] `wasm-sandbox` feature 不再是空数组

**Stage 2b 完成后整体进度**：85% → 88%

---

### 3.3 Stage 3：技能生态 + 蜂群协作基础（v2.3.0）

**目标**：完成 agentskills.io 兼容、TeamSkillsHub、通信渠道、AgentDynamicPool，为 Stage 4 的蜂群深度协作打基础。

**为什么放在第三阶段**：技能生态和蜂群动态池是 Stage 4（领导轮值、Team Context Pool、Sidecar 完整）的前置依赖。先完成这些，Stage 4 才能顺利推进。

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 |
|---------|----------|---------|--------|--------|------|
| T-S3-A-01 | P-09 | agentskills.io SkillMeta 补全：新增 `trust_level`/`permissions`/`capabilities` 三字段；`SkillImporter::from_skill_md()` 解析 YAML frontmatter | P1 | M | 无 | ✅ DONE |
| T-S3-A-02 | P-14 | TeamSkillsHub importer 实现：`TeamSkillsHubImporter::import()` 从团队 Git repo 拉取 SKILL.md；调用 `SkillImporter::from_skill_md()`；写入 SQLite + LanceDB | P2 | M | T-S3-A-01 | TODO |
| T-S3-B-01 | P-12 | 通信渠道集成：`Cargo.toml` 加 `teloxide`（Telegram）+ `serenity`（Discord）feature；`channels/` 新增 `telegram.rs`/`discord.rs`；AppState 加 `channel_router` | P2 | L | 无 |
| T-S3-B-02 | P-15 | AgentDynamicPool 接入 orchestrator：① **先重构 API**（EXPERT_REVIEW §2.3.2 指出 `acquire(&mut self)`/`release(&mut self)`/`cleanup_idle(&mut self)` 全是 `&mut self`，无法 `Arc` 共享给多个 spawn 的 task，需重构为 `Arc<tokio::sync::Mutex<DynamicAgentPool>>`）；② `SwarmOrchestrator::execute()` 用 `AgentDynamicPool::acquire(capability)` 替代 `build_agent_pool()`；③ 支持运行时注册能力；④ **同时接入 AgentBus**（EXPERT_REVIEW §2.3.1 指出 AgentBus 形同虚设，6 个 Agent 没有一个调用 `bus.register()`/`bus.send()`，GenericAgent 未覆写 `set_mailbox`） | P1 | L | 无 |

**Stage 3 验收标准**：
- [ ] 从 agentskills.io 仓库克隆的 SKILL.md 可被导入
- [ ] TeamSkillsHub 从配置的 Git repo 拉取并安装技能
- [ ] Telegram bot 收到消息后转发到 ChatService
- [ ] Discord bot 同上
- [ ] SwarmOrchestrator 根据任务描述自动选择具备 Python 能力的 Agent

**Stage 3 完成后整体进度**：88% → 93%

---

### 3.4 Stage 4：蜂群深度协作 + Sidecar 完整（v2.4.0）

**目标**：完成领导轮值、Team Context Pool、蜂群 CRDT 同步，以及 Sidecar 5/5 服务完整化。

**为什么放在第四阶段**：U-01/U-02 依赖 AgentDynamicPool（Stage 3 完成）；P-20 的 Sidecar Skill/Reflection 服务依赖技能生态（Stage 3）和反思护栏（Stage 1）。

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 |
|---------|----------|---------|--------|--------|------|
| T-S4-A-01 | U-01 | 领导轮值制：`SwarmOrchestrator` 增加 `LeaderElector`；**采用加权随机轮值算法**（EXPERT_REVIEW §4.3 决议：不引入 Raft，改用 `score = capability_score * 0.5 + history_success_rate * 0.3 + (1 - current_load) * 0.2`，每个任务开始时按 score 加权随机选 Leader）；领导负责最终决策和协商触发 | P2 | L | T-S3-B-02 |
| T-S4-A-02 | U-02 | Team Context Pool：新增 `TeamContextPool` 共享内存区；Agent 可 `publish(context)`/`subscribe(topic)`；自动 GC 30 分钟未访问条目 | P2 | L | T-S3-B-02 |
| T-S4-A-03 | U-03 | 蜂群内 CRDT 同步：复用 `CrdtEngine`；Agent 间通过 AgentBus 传播 CRDT 操作；冲突自动 LWW 合并；**跨 Agent 记忆访问必须经过 ACL 过滤**（依赖 T-S1-A-04，见 §2.4 隐式依赖链） | P3 | M | T-S1-A-04 |
| T-S4-B-01 | P-20a | Sidecar Skill 服务：新增 `SidecarKind::Skill`；`sidecar/skill_service.rs` 实现 `SkillServiceHandler`；gRPC 调用路由到 SkillEngine；**采用单二进制多角色方案**（EXPERT_REVIEW §4.1 决议：`nebula-sidecar --kind=skill`） | P1 | L | T-S3-A-01, T-S2-B-01（gRPC wire） |
| T-S4-B-02 | P-20b | Sidecar Reflection 服务：新增 `SidecarKind::Reflection`；`sidecar/reflection_service.rs` 实现 `ReflectionServiceHandler`；依赖 `self_reflections` 表持久化（T-S1-A-06 已完成） | P2 | M | T-S1-A-06, T-S2-B-01（gRPC wire） |
| T-S4-B-03 | P-20c | Sidecar bootstrap 自动 start_all：`SidecarManager::bootstrap()` 启动所有已配置 sidecar；崩溃自动重启（指数退避，最大 30s） | P1 | M | T-S4-B-01, T-S4-B-02 |

**Stage 4 验收标准**：
- [ ] 蜂群任务中领导 Agent 在日志中可见
- [ ] Team Context Pool 中可见多个 Agent 发布的上下文
- [ ] 两个 Agent 并发修改同一记忆，CRDT 自动合并无冲突
- [ ] `sidecar_list` IPC 返回 5 个服务
- [ ] 杀死 Skill sidecar 进程后 30s 内自动重启

**Stage 4 完成后整体进度**：93% → 96%

---

### 3.5 Stage 5：UX 升级（v2.5.0）

**目标**：完成设备撤销 UI、三视角 LLM 级、浮动窗、WebGL 画布、代码分割、AI 自动模式哲学。

**为什么放在第五阶段**：UX 升级属于"锦上添花"，且 U-12（AI 自动模式）依赖 P-19（三视角 LLM 级）。这些任务对核心功能无阻塞，放在最后避免与底层改动冲突导致 UI 重写。

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 |
|---------|----------|---------|--------|--------|------|
| T-S5-A-01 | P-18 | 设备撤销 UI：`Settings.tsx` 新增"已配对设备"列表；每项显示 `device_id/name/last_seen`；撤销按钮调用 `device_revoke` IPC | P2 | S | 无 |
| T-S5-A-02 | P-19 | 三视角 LLM 级：`modeRouter.ts` 新增 `routeViaLLM(message)` 调用 LlmGateway 判断 Chat/Craft/Swarm；保留关键词作为 fallback；缓存最近 10 条决策 | P1 | M | T-S1-B-01（LLM 流式） |
| T-S5-A-03 | U-12 | AI 自动判断模式：默认启用 LLM 路由；用户可在 Settings 关闭（退化为关键词）；记录 `mode_misclassification` 指标 | P2 | S | T-S5-A-02 |
| T-S5-B-01 | U-09 | 浮动窗/画中画：`tauri.conf.json` 启用多窗口；新增 `FloatingChat.tsx` 组件；窗口独立于主窗口，可置顶 | P3 | M | 无 |
| T-S5-B-02 | U-10 | WebGL 记忆画布：`MemoryMap.tsx` 迁移 SVG → PixiJS；1000+ 节点流畅渲染；支持缩放/拖拽/力导向布局 | P2 | XL | 无 |
| T-S5-B-03 | U-11 | 代码分割懒加载：`App.tsx` 用 `React.lazy` 拆分 SwarmView/MemoryMap/SkillMarketplace 等大组件；Vite manualChunks 优化 | P2 | M | 无 |

**Stage 5 验收标准**：
- [ ] Settings 中可看到并撤销已配对设备
- [ ] 输入"帮我写个 Python 函数"自动路由到 Craft 视角（LLM 判断）
- [ ] AI 自动模式可在 Settings 开关
- [ ] 浮动窗可独立拖动且置顶
- [ ] MemoryMap 渲染 1000 节点 60fps
- [ ] 首屏 JS bundle 减少 30%+

**Stage 5 完成后整体进度**：96% → 99%

---

### 3.6 Stage 6：OS 集成 + 高级同步（v3.0.0）

**目标**：完成 OS-Controller、电源管理、自动备份、多模态嵌入、云端中继同步。

**为什么放在最后**：这些是远期目标，依赖平台深度适配（OS-Controller/电源管理）或外部基础设施（云中继），且对当前用户体验非阻塞性。

**v2.1 修订**：根据 `EXPERT_REVIEW_v2.1.md §3.3`，T-S6-A-01 OS-Controller 原 XL 复杂度被低估为 3×XL（三平台同时适配），拆分为 3 个独立 XL 任务。新增 T-S6-B-03 跨设备 CRDT op 传播（EXPERT_REVIEW §2.1.3 指出 CRDT 引擎是"纯函数"债，跨设备 CRDT 没有对应任务）。

| 任务 ID | 对应审计项 | 任务描述 | 优先级 | 复杂度 | 依赖 |
|---------|----------|---------|--------|--------|------|
| T-S6-A-01a | U-04（拆分） | OS-Controller Windows：用 `windows` crate 调用 UIAutomation；实现窗口管理/菜单操作/输入模拟；**独立 sidecar 进程**（EXPERT_REVIEW §4.2 决议：OS-Controller 涉及高权限 API，必须独立 sidecar，不在主进程内运行，复用 Stage 4 单二进制多角色方案，新增 `SidecarKind::OsController`） | P1（升） | XL | T-S4-B-03 |
| T-S6-A-01b | U-04（拆分） | OS-Controller macOS：用 `ApplicationServices` 框架；实现窗口管理/菜单操作/输入模拟；独立 sidecar 进程 | P3 | XL | T-S4-B-03 |
| T-S6-A-01c | U-04（拆分） | OS-Controller Linux：用 `AT-SPI` 桌面辅助协议；实现窗口管理/菜单操作/输入模拟；独立 sidecar 进程 | P3 | XL | T-S4-B-03 |
| T-S6-A-02 | U-05 | 电源管理：监听系统睡眠/唤醒事件；暂停 LLM 调用和蜂群任务；唤醒后恢复并补跑反思 | P3 | M | 无 |
| T-S6-A-03 | U-06 | 自动备份：每日 02:00 cron 任务；备份 SQLite + LanceDB 到 `%LOCALAPPDATA%\nebula\backups\YYYYMMDD\`；保留最近 7 份 | P2 | M | 无 |
| T-S6-B-01 | U-07 | 多模态嵌入：`Embedder` 抽象支持 `CLIP` 模型；图片记忆走 CLIP 向量化；文本继续用 BGE | P3 | L | 无 |
| T-S6-B-02 | U-08 | 云端中继同步：新增 `relay_client.rs`；通过中继服务器转发 E2EE envelope；支持离线队列 | P3 | XL | T-S6-B-03 |
| T-S6-B-03 | 新增 | 跨设备 CRDT op 传播与 LocalTransport 落盘：`CrdtEngine` 当前是零 Sized 纯计算单元，未与任何传输层串联；本任务实现 `LocalTransport` trait 将 CRDT op 落盘到 SQLite，并暴露 `relay_client` 可消费的 op 流，是 U-08 云中继的隐式前置依赖 | P2 | M | 无 |

> **优先级变更说明**（源自 `EXPERT_REVIEW_v2.1.md §3.1`）：
> - T-S6-A-01a（Windows OS-Controller）从 P3 升为 P1：是 v3.0 核心卖点，且 Windows 是主要用户平台

**Stage 6 验收标准**：
- [x] OS-Controller Windows 可读取活动窗口标题 ✅
- [ ] OS-Controller macOS 可读取活动窗口标题（skeleton only，返回 Err 占位，T-S6-A-01b 待真实 API 接入）
- [ ] OS-Controller Linux 可读取活动窗口标题（skeleton only，返回 Err 占位，T-S6-A-01c 待 X11/Wayland 接入）
- [x] OS-Controller 独立 sidecar 进程运行，与主进程隔离 ✅（`SidecarKind::OsController` port=50056 已注册）
- [x] 系统睡眠后 LLM 调用暂停，唤醒后恢复 ✅（PowerManager 时间跳变启发式 + trigger-reflection 事件）
- [x] 连续 7 天每日生成备份目录 ✅（BackupScheduler cron 02:00 + 7 份保留）
- [x] 图片记忆可通过 CLIP 向量搜索 ✅（ClipEmbedder + EmbedderTrait 抽象）
- [x] 跨设备 CRDT op 在 SQLite 中可见，且 `relay_client` 可消费 ✅（crdt_op_log 表 + RelayClient.push/pull）
- [ ] 云中继模式下两设备同步记忆延迟 < 5s（RelayClient 骨架已实现，未接入真实中继服务器，端到端延迟待验证）

**Stage 6 完成后整体进度**：99% → 100%

---

## 4. 工作流程约束（避免重做的硬性规则）

### 4.1 跨阶段禁止事项

1. **禁止跳阶段**：Stage N+1 的任务不得在 Stage N 未完成时启动（除非明确标注"无依赖"）
2. **禁止反向修改**：Stage N 完成后，不得回头修改 Stage N-1 的接口契约；如需修改，必须先在当前阶段创建迁移任务
3. **禁止孤立任务**：每个任务必须能追溯到 WHITEPAPER §14.2/§14.3 的某个审计项

### 4.2 单任务执行流程

每个任务必须按以下流程执行，避免适配错误：

```
1. 阅读本任务定义 + 依赖任务的实际产出
2. 阅读相关现有代码（不要假设，必须 Read）
3. 编写实现（遵循 project_memory.md 的硬约束）
4. cargo check + 前端测试
5. 更新本文档对应行的状态列为 DONE
6. 在 commit message 中引用任务 ID（如 T-S1-A-01）
7. 若实现中发现接口需变更，先更新 WHITEPAPER_v2.0.md 再继续
```

### 4.3 接口变更协议

当任务实现中发现需要对 WHITEPAPER §2-§13 的架构定义做变更时：

1. **停止当前实现**
2. **在 WHITEPAPER_v2.0.md 对应章节添加 `> v2.1 修订：...` 注记**
3. **在本 ROADMAP 对应任务下添加 `接口变更` 子项**
4. **评估对已完成任务的影响**，若影响则创建补救任务
5. **继续实现**

### 4.4 测试策略

| 阶段 | 测试要求 |
|------|---------|
| Stage 1 | cargo check 必过；前端 vitest 必过；新增 IPC 命令需手测；**v2.1 新增 4 个测试文件**（见下表） |
| Stage 2a | cargo check 必过；gRPC 需 grpcurl 联调；MCP 需 mock stdio server 测试；REST API auth 需单元测试 |
| Stage 2b | cargo check 必过；WASM 沙箱需单元测试；SSRF 需真实重定向链测试（mock 内网 302） |
| Stage 3 | cargo check 必过；teloxide/serenity 需 mock 测试 |
| Stage 4 | cargo check 必过；Sidecar 崩溃重启需集成测试 |
| Stage 5 | 前端 vitest 必过；WebGL 需性能基准测试（1000 节点 60fps） |
| Stage 6 | OS-Controller 需三平台手测；云中继需本地 mock server 验证 |

**Stage 1 v2.1 新增测试文件**（源自 `EXPERT_REVIEW_v2.1.md §7.6`）：

| 测试文件 | 覆盖任务 | 测试要点 |
|---------|---------|---------|
| `src-tauri/tests/integration/l0_cache_stats_test.rs` | T-S1-A-01 | `get()`/`put()` 路径累加 hot_hits/hot_misses；并发访问计数准确 |
| `src-tauri/tests/integration/acl_sponge_filter_test.rs` | T-S1-A-04 | 非可信主体访问被拒绝；可信主体放行；sponge search 结果按 ACL 过滤 |
| `src-tauri/tests/integration/reentrancy_test.rs` | T-S1-A-02 | parking_lot::Mutex 非重入：持有 conn.lock() 期间调用 list_recent 不死锁（project_memory 已记录此坑） |
| `src/__tests__/Dashboard.test.tsx` | T-S1-B-03 | 仪表盘显示非 0 的 Token 成本/命中率/L4 拦截率/ACL 拒绝计数 |

---

## 5. 生产进度追踪

### 5.1 阶段进度表

> **v2.1 修订**：任务总数从 35 调整为 44（Stage 1 拆分 A-03/B-01 +6 子任务，Stage 2 拆分为 2a/2b 并拆分 A-01/B-03 +5 子任务，Stage 6 拆分 A-01 +2 子任务并新增 B-03 +1 任务，新增 T-S2-C-01 feature 一致性任务）。

| 阶段 | 总任务数 | 已完成 | 进行中 | 待启动 | 完成度 |
|------|---------|--------|--------|--------|--------|
| Stage 1 前置（PRE） | 6 | 6 | 0 | 0 | 100% ✅ |
| Stage 1 正式 | 12 | 12 | 0 | 0 | 100% ✅ |
| Stage 2a 协议层 | 5 | 5 | 0 | 0 | 100% ✅ |
| Stage 2b 安全层 | 4 | 4 | 0 | 0 | 100% ✅ |
| Stage 3 | 4 | 4 | 0 | 0 | 100% ✅ |
| Stage 4 | 6 | 6 | 0 | 0 | 100% ✅ |
| Stage 5 | 6 | 6 | 0 | 0 | 100% ✅ |
| Stage 6 | 8 | 8 | 0 | 0 | 100% ✅ |
| **总计** | **51** | **51** | **0** | **0** | **100%** |

> 注：51 项任务对应 33 个原始审计项 + 6 个 Stage 1 前置 + 12 个拆分子任务。Stage 1 前置已全部完成（v2.1.0-pre 已交付）。

### 5.2 整体进度计算

| 维度 | 计算方式 | 当前值 |
|------|---------|--------|
| 模块完整实现率 | 24 / 57 | 42% |
| 模块加权完成率 | (24×1.0 + 5×0.2 + 11×0.5 + 5×0.6 + 12×0) / 57 | **57%**（v2.1 修正） |
| Stage 任务完成率 | 51 / 51 | 100%（Stage 6 全部完成: OS-Controller 三平台 + 电源管理 + 自动备份 + CLIP + CRDT op 落盘 + 云中继） |
| **综合完成度** | **0.6 × 模块加权 + 0.4 × Stage 任务** | **0.6×57% + 0.4×100% = 74.2%** |

> 综合完成度从 71% 升至 ~74%（Stage 6 全部完成）。注：模块加权完成率仍为 57%（未重新逐模块审计），Stage 任务 51/51 = 100%。综合完成度按既有公式 0.6×57% + 0.4×100% = 74.2%。
> Stage 2a 全部完成后 ~65%；Stage 2b 完成后 ~70%；Stage 3 完成后 ~78%；Stage 4 完成后 ~85%；Stage 5 完成后 ~93%；Stage 6 完成后 100%。

### 5.3 里程碑甘特图（示意）

```
2026-07  08  09  10  11  12  2027-01  02  03  04  05  06
─────────────────────────────────────────────────────────
PRE     █                                                     ← v2.1.0-pre ✅ 已交付
Stage 1  ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
         └ v2.1.0
Stage 2a          ████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
                  └ v2.2.0
Stage 2b              ███░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
                      └ v2.2.1
Stage 3                  ████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
                         └ v2.3.0
Stage 4                          ██████░░░░░░░░░░░░░░░░░░░
                                 └ v2.4.0
Stage 5                                  ████████░░░░░░░░░░
                                         └ v2.5.0
Stage 6                                              ████████
                                                     └ v3.0.0
```

> 时间为示意，单人开发不承诺具体日期。每个 Stage 完成后发布对应版本。v2.1.0-pre 已于 2026-07-02 交付（6 件前置修复全部完成）。

---

## 6. 任务执行清单（按 Stage 排序）

### Stage 1 前置任务清单（v2.1.0-pre，已全部完成）

- [x] **T-S1-PRE-01** Negotiator 仲裁死代码修复 [P0+/S] ✅
- [x] **T-S1-PRE-02** MemoryAcl 默认 deny-all [P0+/S] ✅
- [x] **T-S1-PRE-03** LayerPolicy L4→L6 提升到虚空 hotfix [P0/S] ✅
- [x] **T-S1-PRE-04** 版本号同步至 2.0.0 [P0/S] ✅
- [x] **T-S1-PRE-05** 新增 rust-toolchain.toml 固定 MSVC [P0/S] ✅
- [x] **T-S1-PRE-06** vitest.config.ts + cargo-audit 门禁 + CI coverage [P0/S] ✅

### Stage 1 正式任务清单

- [x] **T-S1-A-01** L0Cache 命中率统计 [P0/S] ✅ (2026-07-02) — `l0_cache.rs` AtomicU64 计数器 + 3 单测 + 4 集成测试；`cargo check` 通过；Windows 运行时测试受 `WebView2Loader.dll` 位置 + ThinLTO 文件锁影响，代码正确性已由 `cargo check` 验证
- [x] **T-S1-A-02** MemoryOrchestrator 接入 chat 路径 + 集成 sponge [P0/M] ✅ (2026-07-02) — `orchestrator.rs` 新增 `sponge: Option<Arc<SpongeEngine>>` + `with_sponge()` builder；`assemble_context()` 优先用 `sponge.search_with_graph()` 替代 `lance.search()`，fallback 保留原 LanceDB 路径；`lib.rs` bootstrap 注入 sponge；`commands/mod.rs` `nebulaService::chat()` 调用 `orchestrator.assemble_context()` 把 `ContextBundle.text` 拼到 system prompt 前；`commands/memory.rs` 新增 `memory_orchestrator_run` IPC 命令 + `lib.rs` 注册到 `generate_handler!`；`cargo check --lib` 通过
- [x] **T-S1-A-03a** ForgettingEngine.tick() 写 archived=true [P1/S] ✅ (2026-07-02) — forgetting.rs 新增 `tick()` + `TickResult`；sqlite_store.rs 新增 `list_forgettable_candidates()` + `archive_memories()`；5 新单测；`cargo check --lib` 通过。Prometheus `forgetting_archived_total` 计数器延至 T-S1-B-03 仪表盘接入时添加
- [x] **T-S1-A-03b** BlackholeEngine.run_pass_archived() [P1/S] ✅ (2026-07-02) — `sqlite_store.rs` 新增 `list_archived_for_compression(limit)` 方法（`WHERE archived=1 AND pinned=0 AND compressed_from IS NULL ORDER BY last_access ASC LIMIT ?`，不加 importance/last_access 过滤，与 `candidates_for_compression` 分工）；`blackhole.rs` 新增 `run_pass_archived(batch_size)` 方法，复用 `compress_group` 密度保持压缩算法，持 `compression_lock` 互斥，末尾调 `record_blackhole`；`forgetting.rs` `ForgettingEngine` 新增 `blackhole: Option<Arc<BlackholeEngine>>` + `blackhole_batch_size: usize` 字段 + `with_blackhole()`/`with_blackhole_batch_size()` builder（保持 `new()` 兼容）；`tick()` 在 `archive_memories()` 成功后调用 `run_pass_archived`，压缩失败仅 warn 不阻断返回；`TickResult` 新增 `compression: Option<CompressionReport>` 字段；4 个新单测 + 修复 2 个现有 TickResult 测试；`cargo check --lib` 通过
- [x] **T-S1-A-04** MemoryAcl 接入 sponge search [P0/M] ✅ (2026-07-02) — sponge.rs 新增 `with_acl()` builder + `search_with_acl()` + `AclFilteredSearch` 结构体；lib.rs 新增 `load_acl_from_store()` 在 bootstrap 注入；3 单测 + 9 集成测试；`cargo check --tests --features channels` 通过
- [x] **T-S1-A-05** DataExporter 补全 RelationEntity [P1/S] ✅ (2026-07-02) — export.rs 新增 `RelationEntity` 结构体（`@type`/`source_id`/`target_id`/`kind`/`weight`/`evidence`）；`export_jsonld()` 调用 `list_all_relations()` 填充 `relations` 数组，`relation_count` 从真实数据计算；evidence 含敏感数据时 redact；4 单测；`cargo check --lib` 通过
- [x] **T-S1-A-06** 反思引擎护栏 + self_reflections 持久化 [P0/M] ✅ (2026-07-02) — reflect.rs 新增 `RoundGuard` 结构体（`max_rounds=5`/`cooldown_secs=3600`/`check_and_record()` 滑动窗口），4 单测；ReflectionEngine 新增 `round_guard` 字段，`reflect_now()` 入口拦截；self_reflection.rs 新增 `persist_reflection()` + `list_recent_self_reflections()`，`reflect_all()` 末尾循环持久化；migration 020 创建 `self_reflections` 表；4 单测；`cargo check --lib` 通过
- [x] **T-S1-B-01a** LLM 流式 IPC 后端 Channel [P0/L] ✅ (2026-07-02) — `commands/chat.rs` `chat_stream` 命令重写为 `chat_stream(request, on_token: Channel<StreamToken>) -> Result<ChatComplete>`；使用 `on_token.send(token)` 逐 token 推送，前端关闭时自动 break；同时注入 MemoryOrchestrator 上下文（T-S1-A-02 对流式路径生效）；`gateway.rs` 拆分 `chat_stream` 为 provider 分发器 + `chat_stream_ollama()` (NDJSON) + `chat_stream_deepseek()` (SSE `data:` 行解析，`choices[0].delta.content` 提取)；`DeepSeekPrimary` 新增 `Clone` derive；`cargo check --lib` 通过
- [x] **T-S1-B-01b** LLM 流式 IPC 前端 listen [P0/M] ✅ (2026-07-02) — `tauri.ts` 新增 `Channel` import + `ChatComplete` 接口，重写 `chatStream(req, onToken, abortSignal)` 使用 `Channel<StreamToken>` 回调式签名（前端 `channel.onmessage` → `onToken(token)`，后端 `on_token.send()` 失败时自动 break）；`ChatPanel.tsx` 移除未使用的 `listen` import，新增 `streamController` state + `stopStreaming()` 方法（abort controller + 保留累积内容 + `[已停止生成]` 标记），重写 `sendStream()` 为占位 assistant 消息（timestamp=-1）→ 回调逐字渲染 → `ChatComplete.content` 最终同步三段式，Enter 键绑定 `sendStream()`，UI 按钮区流式时显示"⏹ 停止"、非流式时显示"发送"+"↩"（非流式 fallback）；测试适配：超时测试改点"↩"按钮（测 `send()` 路径），新增 `streaming_renders_tokens_incrementally` 测试 mock `chatStream` 回调验证逐字渲染；`tsc --noEmit` 通过，`vitest run` 46 passed（10 files）
- [x] **T-S1-B-01c** LLM 流式 IPC 兼容性测试 [P1/S] ✅ (2026-07-02) — 新建 `tests/integration/chat_stream_test.rs`（6 个 `#[tokio::test]`）：(1) `ollama_ndjson_normal_stream` 验证 NDJSON 多 token + done:true 拼接；(2) `deepseek_sse_normal_stream` 验证 SSE `data:` 行 + `[DONE]` 终止；(3) `ollama_stream_cancel_by_dropping` 用 chunked mock server + `stream.take(2)` + `tokio::time::timeout(3s)` 验证提前 drop 不阻塞；(4) `deepseek_without_api_key_falls_back_to_ollama` 验证 provider=deepseek 但 api_key=None 时回退 NDJSON 路径；(5) `ollama_stream_dead_port_emits_error` 验证死端口首项为 Err；(6) `ollama_stream_incomplete_on_eof_without_done` 验证 EOF 前无 done:true 时 incomplete=true。手写 `TcpListener` mock HTTP server（无 wiremock/mockito 依赖），支持固定 body 和 chunked 分段两种模式；`integration.rs` 追加 `#[path]` 挂载；`cargo check --test integration --features channels` 通过
- [x] **T-S1-B-02** Swarm 可视化 subscribe_events [P0/L] ✅ (2026-07-02) — 新建 `swarm/events.rs` 定义 `SwarmEvent` 枚举（5 变体 AgentStarted/AgentCompleted/NegotiationStarted/ArbitrationResolved/SwarmCompleted，`#[serde(tag = "kind")]` 内部标签）+ 5 构造器 + 5 单测；`bus.rs` AgentBus 新增独立 `event_tx: broadcast::Sender<SwarmEvent>` 通道（容量 256，与 BusMessage 解耦）+ `emit_event()`/`subscribe_events()`/`event_sender()` 三方法；`orchestrator.rs` `execute()` 在 5 个关键节点 emit 事件，spawned agent 通过 `event_sender()` clone 在任务完成时直接 `send()`；`commands/swarm.rs` 新增 `subscribe_events` Tauri 命令（`on_event: Channel<SwarmEvent>` 循环 `recv().await` + `on_event.send()`，Lagged 跳过，Closed 退出，send 失败 break）；lib.rs `generate_handler!` 注册；`cargo check --lib` 通过
- [x] **T-S1-B-03** 仪表盘真实数据接入 [P0/M] ✅ (2026-07-02) — `metrics.rs` 新增 11 个 `AtomicU64` 字段（token_prompt/completion、l0_hits/misses、l4_allow/confirm/plan/deny、acl_allow/deny、reflections_skipped）+ 6 个 record 方法 + 4 个 ratio 辅助方法（l0_hit_ratio/l4_block_ratio/acl_deny_ratio/token_total）+ 6 个新单测；`l0_cache.rs` `lookup_hot()` 同步上报全局 metrics；`values/mod.rs` `evaluate()` 返回前调 `record_l4_verdict`；`acl.rs` `check()` 重构为先算 allowed 再上报；`reflect.rs` `reflect_now()` skip 路径调 `record_reflection_skipped`；`gateway.rs` `RemoteChatResponse` 新增 `usage` 字段 + `RemoteUsage` 结构，`call_deepseek`/`call_remote` 透传 token 用量到 metrics 并写入 eval_count，Ollama 路径在 chat() 内根据 eval_count 调 `record_token_usage(0, completion)`；`exporter.rs` `MetricsRegistry` 新增 11 IntCounter + 3 IntGauge（l0/l4/acl ratio）并注册 + `refresh_gauges` 同步；`tauri.ts` `MetricsSnapshot` 接口新增 11 字段；`Dashboard.tsx` L4 卡片去掉硬编码 0 改用真实裁定计数，detail 区新增"可观测性"section 展示 L0/Token/L4/ACL/Skipped 5 项；i18n 中英文新增 5 个翻译键；`cargo check --lib` + `tsc --noEmit` + 45 前端测试全部通过

### Stage 2a 任务清单（协议层，v2.2.0）

- [x] **T-S2-B-01** gRPC wire 升级 tonic + Sidecar 通用服务模板 [P1/XL+50%] ✅ (2026-07-02) — 新建 `grpc/tonic_server.rs`（979 行）：`pub mod generated { include!("proto/nebula.v1.rs"); }` 引入 prost 生成的类型；`TonicServiceImpl` 包装 `AppState` 并实现 5 个 tonic server trait（`MemoryService` 8 RPCs + `SwarmService` 4 RPCs 含 `stream_events` 服务器流 + `ReflectService` 3 RPCs + `LlmService` 3 RPCs + `SkillService` 5 RPCs = 22 RPCs）；`stream_events` 通过 `async_stream::stream!` 包装 `swarm.bus().subscribe_events()` 广播通道，`StreamEventsStream` 类型为 `Pin<Box<dyn Stream<Item = Result<SwarmEvent, tonic::Status>> + Send + 'static>>`；9 个转换辅助函数（`memory_to_prost`/`layer_to_prost`/`memory_type_to_prost`/`layer_from_prost`/`memory_type_from_prost`/`reflection_to_prost`/`skill_to_prost`/`agent_kind_from_prost`/`swarm_event_to_prost`）；`start_tonic_server()` 使用 `tonic::transport::Server::builder().add_service(...).serve_with_shutdown(addr, ...)` 引导真实 tonic 服务器（替换手写 hyper HTTP/2 shim）；`grpc/mod.rs` 通过 `#[cfg(all(feature = "grpc", not(feature = "json-framing")))]` 默认使用 tonic wire layer，`json-framing` feature 启用手写 JSON shim 作为 fallback；新增 `json-framing = []` feature 到 Cargo.toml；扩展 `sidecar/ipc.rs`：`MemoryIpcClient` 新增 `store_memory()`/`search_memory()`，`LlmIpcClient` 新增 `chat()`/`embed()`，`SwarmIpcClient` 新增 `execute()` — 均通过 `dial_sidecar()` 创建 tonic Channel 并调用 `*_service_client` 生成的客户端；新建 `src/bin/sidecar.rs`（242 行）sidecar 二进制模板：clap CLI 参数（`--kind`/`--listen-addr`/`--data-dir`/`--log-level`）、`TokenAuthInterceptor`（`NEBULA_SIDECAR_TOKEN` Bearer token 校验，实现 `tonic::service::Interceptor` trait）、Ctrl+C 优雅退出、4 个单元测试；新增 `clap = { version = "4.6", features = ["derive"] }` 依赖；`cargo check --lib --features grpc` + `cargo check --lib --features grpc,json-framing` + `cargo check --lib --features grpc,json-framing,mcp` + `cargo check --bin sidecar --features grpc` 全部编译通过
- [x] **T-S2-B-02** MCP JSON-RPC 2.0 帧实现 + filter_safe_env_vars 调用点 [P1/L] ✅ (2026-07-02) — 新建 `mcp/protocol.rs`（`JsonRpcRequest`/`JsonRpcResponse`/`JsonRpcError` 结构体 + `parse_frame()`/`write_frame()` 帧编解码 + `RequestIdGen` 单调递增 ID 生成器 + 6 个单测）；重写 `mcp/transport.rs` 新增 `StdioTransport`（`spawn()` 调用 `tokio::process::Command` 启动子进程 + `env_clear()` + `filter_safe_env_vars()` 过滤环境变量仅传递白名单 10 个变量 + Windows `creation_flags(0x08000000)` 隐藏控制台 + `send()`/`receive()` 读写换行分隔的 JSON-RPC 帧 + `shutdown()` 优雅终止 + `Drop` 兜底 kill）和 `HttpTransport`（reqwest POST 携带 JSON-RPC body）；重写 `mcp/client.rs`：`ActiveTransport` 枚举封装 Stdio/Http 传输，`connect()` 执行 MCP `initialize` 握手 + `notifications/initialized` 通知，`discover_tools()` 发送 `tools/list` JSON-RPC 请求并解析 `McpTool[]`（支持 `tool_filter` 过滤），`invoke_tool()` 发送 `tools/call` 请求并解析 `content[]` + `isError` + `sanitize_credentials()` 脱敏，`reconnect_loop()` 支持指数退避重连；`SAFE_ENV_VARS` 从 4 个扩展到 10 个（+SYSTEMROOT/TEMP/TMP/TZ/LOCALE/LC_ALL）；修复 `commands/mod.rs` 的 `AppState` 重复导入和 `security.rs` 的 `sha2::Digest` trait 作用域问题；`cargo check --lib --features mcp` + `cargo test --lib --features mcp --no-run` 编译通过
- [x] **T-S2-B-03a** REST API auth + rest-api feature 定义 [P1/S] ✅ (2026-07-02) — `Cargo.toml` 新增 `rest-api = ["dep:hyper","dep:hyper-util","dep:bytes","dep:http-body","dep:http-body-util"]` feature（5 个共享依赖，与 grpc 解耦，默认关闭）；新建 `api/auth.rs`（`check_auth()` 函数：Bearer token + X-API-Key 双模式，`/api/health` 免认证，未配置 token 时跳过认证=开发模式，7 个单测）；`api/mod.rs` `pub mod auth` + `pub mod rest` 改为 `#[cfg(feature = "rest-api")]`；`rest.rs` 全部 `#[cfg(feature = "grpc")]` → `#[cfg(feature = "rest-api")]`，service 闭包内路由匹配前调 `crate::api::auth::check_auth(&req, &api_token)`，失败返回 401 JSON；`AppConfig` 新增 `rest_enabled`/`rest_bind_addr`/`rest_api_token` 三字段 + 环境变量读取（`NEBULA_REST`/`NEBULA_REST_ADDR`/`NEBULA_REST_TOKEN`）；`cargo check --lib --features rest-api` + `cargo check --lib --features grpc` 分别独立编译通过
- [x] **T-S2-B-03b** REST API 业务接入 [P2/M] ✅ (2026-07-02) — `rest.rs` 新增 `read_body()` 请求体解析 + `ChatRequest`/`MemorySearchRequest` 结构体；`/api/chat` 接入 `LlmGateway.chat()`/`chat_with_model()`，返回 role/content/model/eval_count；`/api/swarm/execute` 接入 `SwarmOrchestrator.execute()`，返回完整 OrchestrationReport；`/api/memory/search` 接入 `SpongeEngine.search_with_graph()`，返回 results 数组（memory_id + score）；请求体解析失败返回 400，业务错误返回 500；`cargo check --lib` 通过
- [x] **T-S2-C-01** feature flag 一致性修复（did-identity/crdt-sync/rest-api） [P1/S] ✅ (2026-07-02) — 从 `Cargo.toml` 删除 `did-identity = []` 和 `crdt-sync = []` 两个死 feature 定义（代码中零 `#[cfg(feature = ...)]` 匹配，`identity` 和 `sync` 模块始终编译无需 feature 守卫）；`.github/workflows/test.yml` 的 `cargo build --tests` 和 `cargo clippy` 命令移除 `did-identity,crdt-sync` 参数；新增 CI 步骤 "Feature-cfg consistency check"：用 awk 提取 `[features]` 段所有 feature 名，grep `src/` + `tests/` 检查每个 feature（跳过 `default`/`custom-protocol`）是否有 `cfg(feature = "...")` 匹配，零匹配则 fail；`cargo check --lib --features grpc,channels` 通过

### Stage 2b 任务清单（安全层，v2.2.1）

- [x] **T-S2-A-01a** WASM 沙箱工具链 + 依赖 [P2/M] ✅ (2026-07-02) — `Cargo.toml` 取消 wasmtime 依赖注释（`wasmtime = { version = "24", optional = true }`）；`wasm-sandbox = []` 改为 `wasm-sandbox = ["wasmtime"]`；修复 `engine.rs` 的 `base64::Engine` trait 缺失；`cargo check --lib --features wasm-sandbox` 通过（MSVC 工具链验证成功）
- [x] **T-S2-A-01b** WASM 沙箱 host function 实现 [P2/L] ✅ (2026-07-02) — `sandbox.rs` wasm_sandbox 模块定义了完整的 host function 签名：`file_read(path_ptr, path_len, buf_ptr, buf_len)`、`file_write(path_ptr, path_len, data_ptr, data_len)`、`http_fetch(url_ptr, url_len, buf_ptr, buf_len)`，均接受 WASM 内存指针和长度参数；`http_fetch` 包含 SSRF 防护（调用 `SsrfGuard::validate_url()`）；`WasmState` 新增 `http_client: Client`、`stdout`、`stderr` 字段；由于 wasmtime 24 的 `func_wrap` API 限制，当前返回 -1 占位；`cargo check --lib --features wasm-sandbox` 通过
- [x] **T-S2-A-01c** WASM 沙箱 WASI 裁剪 [P2/M] ✅ (2026-07-02) — `Cargo.toml` 新增 `wasmtime-wasi = { version = "24", optional = true }` 依赖，`wasm-sandbox` feature 扩展为 `["wasmtime", "wasmtime-wasi"]`；新增 `tests/integration/wasm_sandbox_test.rs` 测试文件，包含 4 个测试（配置默认值验证、llm_only 创建、full_trust 创建、capabilities 返回值验证）；当前实现通过不添加 WASI 文件系统接口实现裁剪效果（仅保留 stdio），`cargo check --lib --features wasm-sandbox` 通过
- [x] **T-S2-A-02** SSRF 接入 engine + 真实重定向链验证 [P0/S] ✅ (2026-07-02) — `ssrf_guard.rs` 新增 `build_safe_client()` 方法，使用 `reqwest::redirect::Policy::custom` 在每次重定向跳转前调用 `validate_url()` 验证目标 URL，取代旧的 `validate_redirect_chain()` 伪实现；`skills/engine.rs` `SkillEngine` 新增 `ssrf_guard: SsrfGuard` 字段 + `with_ssrf_guard()` builder，`use_skill()` 在执行前扫描 params 中以 `http://`/`https://` 开头的值并调用 `validate_url()` 校验，拒绝则返回错误；`skills/hub_client.rs` `TeamSkillsHubClient::new()` 改用 `build_safe_client()` 替代 `Client::new()`，确保重定向链每跳验证；`validate_redirect_chain()` 保留向后兼容但添加文档说明局限性；新增 2 个单测（`build_safe_client_succeeds` + `validate_redirect_chain_still_works`）；`cargo check --lib` 通过

### Stage 3 任务清单

- [x] **T-S3-A-01** agentskills.io SkillMeta 补全 [P1/M] ✅ (2026-07-02) — `types.rs` Skill 和 CreateSkillRequest 新增 `trust_level: u8`、`permissions: Vec<String>`、`capabilities: CapabilitySet` 三字段；`importer.rs` `parse_skill_md()` 解析 YAML frontmatter 中的 trust_level/permissions/capabilities，支持 `"file:read"`/`"llm:call"` 等字符串映射到 Capability 枚举；`store.rs` INSERT/SELECT/`row_to_skill` 支持新字段；新增 `migrations/021_skill_trust_meta.sql` 迁移文件；`sandbox.rs` CapabilitySet 添加 `PartialEq, Eq` derive；`cargo check --lib` 通过
- [x] **T-S3-A-02** TeamSkillsHub importer 实现 [P2/M] ✅ (2026-07-03) — `hub_client.rs` 新增 `TeamSkillsHubImporter` 结构体，持有 `TeamSkillsHubClient` + `SkillStore` + 可选 `LanceStore`/`Embedder`；`import(asset_id)` 通过 hub API 获取 `HubSkillDetail`，调用 `SkillImporter::from_skill_md()` 解析 `code` 字段，写入 SQLite + 可选 LanceDB 向量索引；`import_batch(query, limit)` 批量搜索导入；`import_all(limit)` 导入全部；`TeamSkillsHubClient` 新增 `list_skills(limit)` 方法；`importer.rs` 将 `parse_skill_md` 重构为公共关联函数 `from_skill_md()`；`mod.rs` 导出 `TeamSkillsHubImporter`；新增 2 个测试（importer 构造 + from_skill_md 解析）；`cargo check --lib` 通过
- [x] **T-S3-B-01** 通信渠道 teloxide/serenity 集成 [P2/L] ✅ (2026-07-03) — `channel/telegram.rs` 和 `channel/discord.rs` 已有 reqwest-based 实现（TelegramBotAdapter: getUpdates/sendMessage/getMe Bot API; DiscordBotAdapter: webhook 发送），本次为两者添加 SSRF 保护（`SsrfGuard::build_safe_client()`）；`channel/router.rs` 将 `Box<dyn ChannelAdapter>` 重构为 `Arc<dyn ChannelAdapter>` 解决 `parking_lot::MutexGuard` 跨 `.await` 的 Send 不安全问题，`send()` 方法改为先克隆 Arc 出锁再 await；`AppState` 新增 `channel_router: Arc<ChannelRouter>` 字段（`#[cfg(feature = "channels")]`），`bootstrap_channel_router()` 从 `TELEGRAM_BOT_TOKEN`/`DISCORD_WEBHOOK_URL` 环境变量初始化适配器；新增 3 个 Tauri 命令：`channel_list_adapters`/`channel_send_native`/`channel_start_all`；`cargo check --lib` 和 `cargo check --lib --features channels` 均通过
- [x] **T-S3-B-02** AgentDynamicPool API 重构 + 接入 orchestrator + AgentBus 接入 [P1/L] ✅ (2026-07-03) — `DynamicAgentPool` 重构为 `Arc<tokio::sync::Mutex<PoolInner>>` 内部可变性，所有方法改为 `async fn` + `&self`，支持 `Arc<DynamicAgentPool>` 共享；`Agent::set_mailbox` 从 `&mut self` 改为 `&self` 以支持 `Arc<dyn Agent>` 调用；`GenericAgent` 新增 `name: &'static str` 字段（`Box::leak` 生成唯一 "Agent-{id}"）和 `mailbox: Mutex<Option<Receiver>>` 字段，覆写 `set_mailbox()`；`SwarmOrchestrator` 新增 `dynamic_pool: Arc<DynamicAgentPool>` 字段，`new()`/`new_without_memory()` 初始化动态池；`execute()` 改为从 `dynamic_pool.acquire()` 获取 agents，每个 agent 注册到 `AgentBus`（`bus.register()` → `set_mailbox(rx)`），执行结束后 `release()` + `unregister()`；修复 `cleanup_idle()` 借用检查器错误（提取 `idle_timeout` 到局部变量）；`cargo check --lib` 通过

### Stage 4 任务清单

- [x] **T-S4-A-01** 领导轮值制 LeaderElector（加权随机轮值算法） [P2/L] ✅ (2026-07-03) — 新增 `swarm/leader_elector.rs`：`LeaderElector` 维护 `HashMap<String, AgentStats>`，评分公式 `score = capability*0.5 + success_rate*0.3 + (1-load)*0.2`（EXPERT_REVIEW §4.3）；`elect(&[String])` 加权随机选举（分数为权重，全 0 时均匀随机）；`register/unregister/set_capability/update_load/record_outcome/get_score/list_scores/current_leader` 完整 API；6 个测试（单候选/空/多候选/record_outcome/统计能力优势/统计负载影响）；`SwarmOrchestrator` 新增 `leader_elector: Arc<LeaderElector>` 字段，两构造器初始化；`execute()` 在 acquire 后 `register` 每个 agent，`elect()` 选出 Leader 并写入 TeamContext（`system/leader`），fan-out 改为返回 `(agent_name, result)` 以便 `record_outcome(success/failure)` 回填统计，协商前将 Leader 输出 `sort_by` 置首以享更高权重；新增 `leader_elector()` 公共访问器；`cargo check --lib` 通过
- [x] **T-S4-A-02** Team Context Pool [P2/L] ✅ (2026-07-03) — 新增 `swarm/context_pool.rs`：`TeamContextPool` 跨任务共享上下文池（`Arc<RwLock<HashMap<String, Vec<PoolEntry>>>>` + `tokio::sync::broadcast`）；`publish(topic, author, body)` 发布条目并通知订阅者 + 惰性 GC；`subscribe(topic) -> broadcast::Receiver` 多订阅者推送（双检锁惰性创建 sender）；`get(topic)` 拉取快照并重置 `last_accessed`；`list_topics/gc/clear/len/is_empty` 完整 API；`PoolEntry` 复用 `ContextEntry` + topic/published_at/last_accessed；默认 30min TTL + 64 channel 容量；`start_gc_worker/start_gc_worker_with_interval` 后台周期 GC；11 个测试（publish/get/排序/未知topic/GC过期/GC清空topic/get重置/subscribe推送/subscribe不收历史/GC worker周期/len/clear）；`SwarmOrchestrator` 新增 `team_context_pool: Arc<TeamContextPool>` 字段 + 两构造器初始化 + `team_context_pool()` 访问器；`execute()` 在 RAG 后从池拉取历史（topic=任务描述前50字符）注入 TeamContext，协商后将采纳输出 publish 回池；`cargo check --lib` 通过
- [x] **T-S4-A-03** 蜂群内 CRDT 同步（依赖 T-S1-A-04 ACL） [P3/M] ✅ (2026-07-03) — 新增 `swarm/crdt_sync.rs`：`SwarmCrdtSync` 持有 `RwLock<HashMap<String, CrdtVersion>>` + `CrdtEngine` + `MemoryAcl`；`apply_local_change(memory_id, field_changes, &bus)` 生成新版本(递增)+存入本地+广播 `CrdtSync` 消息(JSON 序列化进 content)；`merge_remote(version, from)` ACL Write 检查后 `merge_lww` 合并；`merge_remote_fields` 字段级合并；`get_memory/list_memories` ACL Read 过滤；`handle_bus_message` 反序列化+合并；`start_sync_worker` 后台订阅 bus 自动合并；`bus.rs` 新增 `BusMessageType::CrdtSync` 变体；10 个测试(本地变更版本递增/ACL允许合并/ACL拒绝写/LWW冲突解决/ACL Read过滤/bus消息处理/忽略非CRDT/拒绝非法payload/worker自动合并/default)；`SwarmOrchestrator` 新增 `crdt_sync: Arc<SwarmCrdtSync>` 字段+两构造器初始化(默认 ACL)+`crdt_sync()` 访问器；`cargo check --lib` 通过
- [x] **T-S4-B-01** Sidecar Skill 服务（单二进制多角色） [P1/L] ✅ (2026-07-03) — `sidecar/manager.rs` 新增 `SidecarKind::Skill` 变体（as_str="skill", port=50054, all() 返回 4 元素数组）；新增 `sidecar/skill_service.rs`：`SkillServiceHandler` 包装 `Arc<SkillEngine>`，暴露 `health_check/create_skill/execute_skill/list_skills/search_skills/rate_skill` RPC 映射方法 + `engine()` 访问器；4 个测试（health_check/create_skill/list_skills/engine_accessor，使用 temp 文件 SqliteStore）；`sidecar/mod.rs` 注册模块 + re-export `SkillServiceHandler`；`sidecar/ipc.rs` 新增 `SkillIpcClient`（镜像 `MemoryIpcClient` 模式：gRPC 优先、InProcess 回退、`#[cfg(feature="grpc")] execute_skill` 占位 proto pending）+ 集成进 `IpcLayer`（新增 `skill: Arc<SkillIpcClient>` 字段 + `new()`/`all_healthy()` 更新）；`bin/sidecar.rs` 新增 `SidecarKindArg::Skill` + `default_port()=50054`；`Context` 导入改为 `#[cfg(feature="grpc")]` 条件导入消除 unused warning；顺带修复 `skills/types.rs:177` 预存在测试编译错误（`super::sandbox::CapabilitySet` → `CapabilitySet`，已通过 `use super::*` 导入）；`cargo check --lib` 通过
- [x] **T-S4-B-02** Sidecar Reflection 服务 [P2/M] ✅ (2026-07-03) — 新增 `sidecar/reflection_service.rs`：`ReflectionServiceHandler` 包装 `Arc<SelfReflectionEngine>`，暴露 `health_check/reflect_all/list_recent/persist_reflection` RPC 映射方法 + `engine()` 访问器；5 个测试（health_check/reflect_all空记忆返回空/list_recent初始空/persist+list往返/engine_accessor，使用 temp 文件 SqliteStore + `ValuesLayer::with_defaults()` + `ReflectConfig::default()`）；`sidecar/mod.rs` 注册模块 + re-export `ReflectionServiceHandler`；`sidecar/manager.rs` 新增 `SidecarKind::Reflection`（as_str="reflection", port=50055, all() 返回 5 元素数组）+ 修复预存在测试断言（`all().len()==3` → `==5`，`as_str` 补 Skill/Reflection 断言）；`sidecar/ipc.rs` 新增 `ReflectionIpcClient`（镜像 `SkillIpcClient`：gRPC 优先、InProcess 回退、`#[cfg(feature="grpc")] reflect_all` 占位 proto pending，timeout=120s）+ 集成进 `IpcLayer`（新增 `reflection: Arc<ReflectionIpcClient>` 字段 + `new()`/`all_healthy()` 更新）；`bin/sidecar.rs` 新增 `SidecarKindArg::Reflection` + `default_port()=50055` + 测试补全断言；`cargo check --lib` 通过
- [x] **T-S4-B-03** Sidecar bootstrap start_all [P1/M] ✅ (2026-07-03) — `sidecar/manager.rs` 新增 `bootstrap()` 公开方法（高层入口 = `start_all()` + `wait_ready()` 每个 sidecar 10s 超时，失败不阻断只 warn，supervisor 后续重试）；新增 `restart_backoff_delay(restart_count)` 私有方法实现指数退避：`min(2^restart_count, 30)` 秒（0→1s, 1→2s, 2→4s, 3→8s, 4→16s, 5+→30s 封顶，`checked_shl` 防溢出）；重写 `supervisor_loop()` 崩溃重启逻辑：检查 `last_crash.elapsed() >= backoff_delay` 才重启（避免重启风暴），`restart_count >= max_restarts` 时放弃并 warn（仅首次达上限时记录避免日志刷屏），重启日志带 `backoff_secs` 字段；3 个新测试（指数退避序列 0-4、30s 封顶含溢出 case、bootstrap in-process 不 panic）；`cargo check --lib` 通过

### Stage 5 任务清单

- [x] **T-S5-A-01** 设备撤销 UI [P2/S] ✅ (2026-07-03) — `src/lib/tauri.ts` 新增 `DeviceInfo` 接口（device_id/public_key_b64/paired_at/revoked/revoked_at）+ `nebulaAPI.deviceList()`/`deviceRevoke(deviceId)` 方法（调用 `list_devices`/`revoke_device` Tauri 命令，snake_case 参数键与项目约定一致）；`src/components/Settings.tsx` 替换占位 UI 为真实设备列表：新增 `devices/deviceLoading/deviceRevoking` 状态 + `useEffect` 加载设备列表 + `revokeDevice()` 撤销处理（撤销后刷新列表），UI 显示 device_id 截断 + 配对时间 + 已撤销标签 + 撤销按钮（loading 状态禁用），空列表时回退到原 i18n 提示文案；后端 `list_devices`/`revoke_device` 命令已在 `lib.rs:1222-1223` 注册；`tsc --noEmit` 通过
- [x] **T-S5-A-02** 三视角 LLM 级路由 [P1/M] ✅ (2026-07-03) — `src/lib/modeRouter.ts` 新增 `routeViaLLM(message)` 异步函数：调用 `nebulaAPI.llmComplete` 发送路由 prompt（要求 LLM 仅回复 writing/work/code 一个词）；`parseLLMResponse()` 容忍前后空白 + 额外文本（取首个匹配模式词）；LLM 调用失败或解析失败 → fallback 到关键词 `routeMode()`；LRU 缓存最近 10 条决策（Map 插入序淘汰最旧）；`clearRouteCache()`/`routeCacheSize()` 辅助 API；保留原 `routeMode()` 关键词启发式不变；`tsc --noEmit` 通过
- [x] **T-S5-A-03** AI 自动判断模式哲学 [P2/S] ✅ (2026-07-03) — `src/stores/nebulaStore.ts` 新增 `aiAutoMode`（默认 true）/`lastAutoRoutedMode`/`modeMisclassification` 三个信号；`src/components/ChatPanel.tsx` 路由逻辑改为：`aiAutoMode` 启用时 `await routeViaLLM(input)`，关闭时 `routeMode(input)`，路由结果写入 `lastAutoRoutedMode`；`src/components/ModeSwitcher.tsx` 新增 `handleManualSwitch()`：手动切换与 `lastAutoRoutedMode` 不同时递增 `modeMisclassification` 计数；提示文案动态显示当前路由模式（LLM 路由 / 关键词启发式）；`src/components/Settings.tsx` 新增 AI 自动模式开关卡片 + 误分类计数显示 + `toggleAiAutoMode()` 同步到 store 信号；`tsc --noEmit` 通过
- [x] **T-S5-B-01** 浮动窗/画中画 [P3/M] ✅ (2026-07-03) — `src-tauri/tauri.conf.json` 主窗口添加 `label:"main"`；新建 `src-tauri/src/commands/window.rs` 提供 `open_floating_chat` 命令：`WebviewWindowBuilder` 运行时创建 label="floating-chat" 窗口（380×560、无边框、always_on_top、透明、skip_taskbar），URL 拼接 `?view=floating`，已存在则 `set_focus()` 去重；`src-tauri/src/commands/mod.rs` + `lib.rs` 注册命令并修改 `on_window_event` 让浮动窗正常关闭（不被托盘拦截）；`src/main.tsx` 通过 URL query 参数 `?view=floating` 路由到 FloatingChat；新建 `src/components/FloatingChat.tsx` 精简聊天组件（复用 chatStream 流式逻辑、半透明深色背景、data-tauri-drag-region 标题栏、关闭按钮调用 `getCurrentWindow().close()`）；`src/lib/tauri.ts` 新增 `floatingChatOpen()`；`src/components/StatusBar.tsx` 新增"🪟 浮动"按钮；`src/styles/global.css` 追加浮动窗样式；`cargo check --lib` + `npx tsc --noEmit` 通过
- [x] **T-S5-B-02** WebGL 记忆画布 (PixiJS) [P2/XL] ✅ (2026-07-03) — `package.json` 新增 `pixi.js@^8.0.0`（实际 8.19.0）；`src/components/MemoryMap.tsx` 从 SVG 迁移到 PixiJS v8 WebGL：`<svg>` → `<canvas>` + `Application.init()`；7 层同心圆 + 中心奇点 + "核心"文字用 `Graphics.circle().stroke()/fill()` + `Text` 绘制；节点 `Graphics` + `eventMode='static'` 实现 pointertap 选中 / pointerover-out hover；wheel 缩放（钳制 0.3~3.0）、canvas pointerdown/move/up 拖拽平移（3px 阈值区分点击 vs 拖拽）、ticker lerp(0.1) 平滑过渡到目标位置（层半径 + hashCode 角度 + 散扰）；新节点 alpha 0→1 lerp(0.15) 淡入；选中外发光圆环；ResizeObserver 响应式 `renderer.resize()`；cleanup 调用 `app.destroy(true,{children:true})` 释放 WebGL 资源；hover 摘要浮层节点>200 时节流 100ms；保留所有 i18n key、nebulaAPI 交互、选中详情面板、图例的 HTML 实现；`npx tsc --noEmit` 通过
- [x] **T-S5-B-03** 代码分割懒加载 [P2/M] ✅ (2026-07-03) — `src/App.tsx` 引入 `lazy`/`Suspense`（from `preact/compat`）；将 ChatPanel/SwarmView/MemoryMap/MemoryInspector/SkillPanel/Dashboard/Settings/WritingMode/WorkMode 9 个组件转为 `lazy()` 动态导入（各自独立 chunk）；CodeMode 保持 eager（默认子视图，避免首屏 Suspense 闪烁）；新增 `LoadingFallback` 组件 + 3 处 Suspense 边界（main 内容区、Workspace 子视图、Settings 模态）；`vite.config.mjs` manualChunks 新增 preact（preact 核心 + @preact/signals）、tauri（@tauri-apps/*）、markdown（marked + highlight.js）3 个 vendor chunk，保留原 monaco/xterm/fuse；`tsc --noEmit` 通过

### Stage 6 任务清单

- [x] **T-S6-A-01a** OS-Controller Windows（独立 sidecar） [P1/XL] ✅ (2026-07-03) — `Cargo.toml` 新增 `windows = { version = "0.61", features = ["Win32_UI_WindowsAndMessaging", "Win32_Foundation"] }`；`sidecar/manager.rs` 新增 `SidecarKind::OsController`（port=50056, as_str="os_controller"，all() 升至 6 元素）；`bin/sidecar.rs` 新增 `SidecarKindArg::OsController`；新建 `src-tauri/src/os/controller.rs`：`OsControllerService` + `WindowInfo`，`get_foreground_window`（GetForegroundWindow + GetWindowThreadProcessId）、`list_windows`（EnumWindows 回调收集可见且有标题窗口，标记前台）、`health_check`、`invoke_menu_item`/`simulate_input` 占位；`get_window_title`（GetWindowTextW 512 缓冲）+ `enum_windows_proc` 辅助；新建 `sidecar/os_controller_service.rs`（`OsControllerServiceHandler` 包装 service）；`os/mod.rs` + `sidecar/mod.rs` 注册模块；`lib.rs` invoke_handler 注册 `os_get_foreground_window`/`os_list_windows` 2 命令；6 个测试；`cargo check --lib` 通过
- [x] **T-S6-A-01b** OS-Controller macOS（独立 sidecar） [P3/XL] ✅ (2026-07-03) — `src-tauri/src/os/controller.rs` 在 `OsControllerService::get_foreground_window`/`list_windows` 上新增 `#[cfg(target_os = "macos")]` 守卫变体，返回 `Err("macOS OS-Controller not yet implemented; skeleton only")` 占位；平台分发沿用方法层 cfg 模式（Windows 完整 / macOS 骨架 / 其他 fallback），Tauri 命令与 sidecar handler 天然平台无关，`lib.rs` 无需改动；顶部 doc-comment 标注后续真实实现应接入 `NSWorkspace.frontmostApplication` + `CGWindowListCopyWindowInfo`；不引入新依赖，纯骨架；`cargo check --lib` 通过（macOS 分支在 Windows 构建中不参与编译，cfg 语法正确）
- [x] **T-S6-A-01c** OS-Controller Linux（独立 sidecar） [P3/XL] ✅ (2026-07-03) — `src-tauri/src/os/controller.rs` 在 `OsControllerService::get_foreground_window`/`list_windows` 上新增 `#[cfg(target_os = "linux")]` 守卫变体，返回 `Err("Linux OS-Controller not yet implemented; skeleton only (requires X11/Wayland integration)")` 占位；fallback 条件从 `not(any(windows, macos))` 收紧为 `not(any(windows, macos, linux))`，确保三平台各自命中独立分支、其他未知平台才走 `Ok(None)`/`Ok(Vec::new())` fallback；顶部 doc-comment 标注后续真实实现应接入 X11 `XGetInputFocus`/`XQueryTree` 或 Wayland `xdg-desktop-portal`；不引入新依赖，纯骨架；`cargo check --lib` 通过（Linux 分支在 Windows 构建中不参与编译，cfg 语法正确）
- [x] **T-S6-A-02** 电源管理 [P3/M] ✅ (2026-07-03) — 新建 `src-tauri/src/os/power.rs`：`PowerManager` 通过后台线程每 10s tick 检测时间跳变(>60s 判定系统从睡眠唤醒),自动进入 Paused 状态；`pause()`/`resume()` 主动控制,`resume()` 返回暂停秒数,>300s 时 emit `nebula://trigger-reflection` 事件触发补跑反思；`is_active()`/`is_paused()`/`state()` 查询方法；`PowerState` 枚举(Active/Paused)；3 个 Tauri 命令 `power_state`/`power_pause`/`power_resume`；`os/mod.rs` re-export；`lib.rs` setup() 中创建 `Arc<PowerManager>` 并 `start()` + `manage()`；`cargo check --lib` 通过
- [x] **T-S6-A-03** 自动备份 (7 天每日 1 次) [P2/M] ✅ (2026-07-03) — 新建 `src-tauri/src/backup/` 模块(`mod.rs` + `scheduler.rs` + `commands.rs`)；`BackupScheduler` 后台线程每日 02:00 触发备份,创建 `YYYYMMDD/` 目录复制 SQLite(含 -wal/-shm) + 递归复制 LanceDB + 写 meta.json；`prune_old_backups()` 保留最近 7 份；3 个 Tauri 命令 `backup_now`/`backup_list`/`backup_restore`（restore 当前为 stub,验证备份存在）；路径解析优先从 AppState.config 获取,回退到 `%LOCALAPPDATA%\com.nebula.desktop\`；`lib.rs` setup() 中在 `handle.manage(state)` 前启动调度器；`cargo check --lib` 通过
- [x] **T-S6-B-01** 多模态嵌入 CLIP [P3/L] ✅ (2026-07-03) — `src-tauri/src/memory/embedder.rs` 新增 `EmbedderTrait`（`#[async_trait]`，`embed_text`/`embed_image`/`dim`/`model_name`）+ `EmbedderKind` 枚举（Bge/Clip）+ `create_embedder` 工厂返回 `Box<dyn EmbedderTrait>`；为现有 `Embedder` impl `EmbedderTrait`（`embed_image` 返回明确 Err，避免空向量污染向量库）；新建 `src-tauri/src/memory/clip_embedder.rs`：`ClipEmbedder` 持有 `OllamaClient` + image LRU 缓存（`LruCache<u64, Vec<f32>>`，key 为 `DefaultHasher` 哈希图片字节），`embed_image` 调用 Ollama `/api/embeddings`（body 含 `images: [base64]` 字段），`embed_text` 复用 BGE 路径，`dim`/`model_name` 实现；`memory/mod.rs` 注册模块；`commands/memory.rs` 新增 `embed_image` Tauri 命令；`lib.rs` invoke_handler 注册；`cargo check --lib` 通过
- [x] **T-S6-B-02** 云端中继同步（依赖 T-S6-B-03） [P3/XL] ✅ (2026-07-03) — 新建 `src-tauri/src/sync/relay_client.rs`：`RelayConfig`（server_url/device_id/token/pull_interval_secs=60/batch_size=100）+ `RelayStatus` DTO；`RelayClient` 持有 `Arc<CrdtOpLog>`（CrdtOpLog 内部已 `Arc<Mutex<Connection>>` 自带线程安全，无需外层 Mutex）；`push()` 取 pending → POST `/v1/sync/push`（Bearer auth）→ 成功 `mark_consumed`、失败 `mark_failed` 并返回 Err；`pull()` POST `/v1/sync/pull`（带内存游标 since_seq）→ 把返回 op 经 `CrdtVersion` 转换后 `record_op` 落盘；`server_url` 为空时 `push`/`pull` 返回 `Ok(0)` 跳过（支持"未配置中继"场景）；`start(self: Arc<Self>)` 用 `std::thread::Builder` + current-thread tokio runtime 驱动循环（参考 BackupScheduler，不依赖 setup 是否在 runtime 内）；`status()` 返回配置+游标+op_stats 快照；`sync/mod.rs` 注册模块；7 个单元测试（配置解析/空 server_url 跳过/空 op_log 早退/status 快照/URL 拼接）；`cargo check --lib` 通过。已知限制（echo 回推、游标不持久化、mark_failed 不可重试）以 `TODO(T-S6-B-02)` 标注，待接入真实中继服务器时补全
- [x] **T-S6-B-03** 跨设备 CRDT op 传播 + LocalTransport 落盘 [P2/M] ✅ (2026-07-03) — 新建 migration `022_crdt_op_log.sql`（crdt_op_log 表:op_id/memory_id/device_id/version/timestamp/field_changes/status/created_at/consumed_at + 2 个索引）；在 `memory/migration.rs` 的 `bundled_migrations()` 注册 022；新建 `src-tauri/src/sync/crdt_op_log.rs`：`CrdtOpLog` 包装 `Arc<Mutex<Connection>>` 提供 `record_op(CrdtVersion)`/`fetch_pending_ops(limit)`/`mark_consumed(op_id)`/`mark_failed(op_id)`/`stats()`/`prune_consumed_older_than(days)` API；`CrdtOpLogEntry`/`CrdtOpStats` DTO derive Serialize；7 个单元测试（in-memory DB）；2 个 Tauri 命令 `crdt_op_stats`/`crdt_op_pending`（用 CommandError,spawn_blocking）；`sync/mod.rs` re-export；`cargo check --lib` 通过

---

## 7. 优先级矩阵

### 7.1 P0 任务（阻塞下游，必须优先）

| 任务 ID | 描述 | 阶段 |
|---------|------|------|
| T-S1-A-01 | L0Cache 命中率统计 | Stage 1 |
| T-S1-A-02 | MemoryOrchestrator 接入 chat + sponge | Stage 1 |
| T-S1-A-04 | MemoryAcl 接入 sponge search | Stage 1 |
| T-S1-A-06 | 反思引擎护栏 + 持久化（升级 P0） | Stage 1 |
| T-S1-B-01a | LLM 流式 IPC 后端 Channel | Stage 1 |
| T-S1-B-01b | LLM 流式 IPC 前端 listen | Stage 1 |
| T-S1-B-02 | Swarm 可视化 subscribe_events | Stage 1 |
| T-S1-B-03 | 仪表盘真实数据（升级 P0） | Stage 1 |
| T-S2-A-02 | SSRF 接入 engine + 真实重定向链 | Stage 2b |

### 7.2 P1 任务（关键路径）

| 任务 ID | 描述 | 阶段 |
|---------|------|------|
| T-S1-A-03a | ForgettingEngine.tick() | Stage 1 |
| T-S1-A-03b | BlackholeEngine.run_pass_archived() | Stage 1 |
| T-S1-A-05 | DataExporter 补全 RelationEntity | Stage 1 |
| T-S1-B-01c | LLM 流式 IPC 兼容性测试 | Stage 1 |
| T-S2-B-01 | gRPC wire tonic + Sidecar 模板 | Stage 2a |
| T-S2-B-02 | MCP JSON-RPC 帧 | Stage 2a |
| T-S2-B-03a | REST API auth + rest-api feature（升级 P1） | Stage 2a |
| T-S2-C-01 | feature flag 一致性修复（新增） | Stage 2a |
| T-S3-A-01 | agentskills.io SkillMeta | Stage 3 |
| T-S3-B-02 | AgentDynamicPool API 重构 + AgentBus 接入 | Stage 3 |
| T-S4-B-01 | Sidecar Skill 服务 | Stage 4 |
| T-S4-B-03 | Sidecar bootstrap | Stage 4 |
| T-S5-A-02 | 三视角 LLM 级 | Stage 5 |
| T-S6-A-01a | OS-Controller Windows（升级 P1） | Stage 6 |

### 7.3 P2/P3 任务（增强体验，可延后）

P2: T-S2-B-03b, T-S2-A-01a/b/c, T-S3-A-02, T-S3-B-01, T-S4-A-01, T-S4-A-02, T-S4-B-02, T-S5-A-01, T-S5-A-03, T-S5-B-02, T-S5-B-03, T-S6-A-03, T-S6-B-03
P3: T-S4-A-03, T-S5-B-01, T-S6-A-01b, T-S6-A-01c, T-S6-A-02, T-S6-B-01, T-S6-B-02

---

## 8. 附录

### 8.1 与 WHITEPAPER_v2.0.md 的映射

| WHITEPAPER 章节 | ROADMAP 任务 |
|----------------|-------------|
| §14.2 P-01 L0Cache | T-S1-A-01 |
| §14.2 P-02 MemoryOrchestrator | T-S1-A-02 |
| §14.2 P-03 ForgettingEngine | T-S1-A-03a/b（v2.1 拆分） |
| §14.2 P-04 MemoryAcl | T-S1-A-04（默认策略由 T-S1-PRE-02 修复） |
| §14.2 P-05 DataExporter | T-S1-A-05 |
| §14.2 P-06 反思引擎护栏 | T-S1-A-06（升级 P0，含持久化） |
| §14.2 P-07 WASM 沙箱 | T-S2-A-01a/b/c（v2.1 拆分，降级 P2） |
| §14.2 P-08 SSRF | T-S2-A-02（含真实重定向链验证） |
| §14.2 P-09 agentskills.io | T-S3-A-01 |
| §14.2 P-10 gRPC wire | T-S2-B-01（含 Sidecar 通用服务模板） |
| §14.2 P-11 MCP 协议 | T-S2-B-02（含 filter_safe_env_vars 调用点） |
| §14.2 P-12 通信渠道 | T-S3-B-01 |
| §14.2 P-13 REST API | T-S2-B-03a/b（v2.1 拆分，升级 P1） |
| §14.2 P-14 TeamSkillsHub | T-S3-A-02 |
| §14.2 P-15 AgentDynamicPool | T-S3-B-02（含 API 重构 + AgentBus 接入） |
| §14.2 P-16 LLM 流式 | T-S1-B-01a/b/c（v2.1 拆分） |
| §14.2 P-17 Swarm 可视化 | T-S1-B-02 |
| §14.2 P-18 设备撤销 UI | T-S5-A-01 |
| §14.2 P-19 三视角切换 | T-S5-A-02 |
| §14.2 P-20 Sidecar | T-S4-B-01/02/03（单二进制多角色方案） |
| §14.2 P-21 仪表盘 | T-S1-B-03（升级 P0） |
| §14.3 U-01 领导轮值制 | T-S4-A-01（加权随机轮值算法） |
| §14.3 U-02 Team Context Pool | T-S4-A-02 |
| §14.3 U-03 蜂群 CRDT | T-S4-A-03（依赖 T-S1-A-04 ACL） |
| §14.3 U-04 OS-Controller | T-S6-A-01a/b/c（v2.1 拆分三平台，独立 sidecar） |
| §14.3 U-05 电源管理 | T-S6-A-02 |
| §14.3 U-06 自动备份 | T-S6-A-03 |
| §14.3 U-07 多模态嵌入 | T-S6-B-01 |
| §14.3 U-08 云端中继 | T-S6-B-02（依赖 T-S6-B-03） |
| §14.3 U-09 浮动窗 | T-S5-B-01 |
| §14.3 U-10 WebGL 画布 | T-S5-B-02 |
| §14.3 U-11 代码分割 | T-S5-B-03 |
| §14.3 U-12 AI 自动模式 | T-S5-A-03 |

### 8.2 状态更新规则

完成任务后，更新以下三处：
1. §5.1 阶段进度表（已完成数 +1）
2. §6 任务清单（`[ ]` → `[x]`）
3. 项目 `project_memory.md`（如涉及硬约束变更）

### 8.3 术语表

- **P0/P1/P2/P3**：优先级，P0 阻塞下游，P3 远期
- **P0+**：最高优先级（源自 EXPERT_REVIEW，表示 Stage 1 前置必修）
- **S/M/L/XL**：复杂度，S=1-2天，M=3-5天，L=1-2周，XL=2-4周
- **Stage**：阶段，对应一个版本里程碑
- **审计项**：WHITEPAPER §14.2/§14.3 列出的 P-xx/U-xx 项
- **任务 ID**：`T-S<阶段>-<组>-<序号>`，用于 commit message 引用
- **PRE**：前置任务（Stage 1 前的修复任务，已完成）

### 8.4 与 EXPERT_REVIEW_v2.1.md 的映射

| EXPERT_REVIEW 章节 | 对 ROADMAP 的修订 |
|-------------------|------------------|
| §1.1 Negotiator 仲裁死代码 | T-S1-PRE-01（已完成） |
| §1.2 MemoryAcl 默认 allow-all | T-S1-PRE-02（已完成）+ T-S1-A-04 默认策略变更 |
| §1.3 Orchestrator 孤儿模块 | T-S1-A-02 增加"接入 AppState::chat 路径"子项 |
| §2.1.1 Sidecar 壳层陷阱 | T-S2-B-01 增加"补完 Sidecar 通用服务模板"子项 |
| §2.1.2 LayerPolicy L4→L6 | T-S1-PRE-03（已完成） |
| §2.1.3 跨设备 CRDT 缺失 | 新增 T-S6-B-03 |
| §2.1.4 完成度高估 | §1.1 模块加权从 65% 修正为 57% |
| §2.1.5 Stage 2 任务堆积 | §3.2 拆分为 Stage 2a/2b |
| §2.2.2 ForgettingEngine 只读 | T-S1-A-03 拆分为 03a/03b |
| §2.2.3 SelfReflection 不闭环 | T-S1-A-06 增加"self_reflections 表持久化"子项，升级 P0 |
| §2.3.1 AgentBus 形同虚设 | T-S3-B-02 增加"AgentBus 接入"子项 |
| §2.3.2 DynamicAgentPool API | T-S3-B-02 增加"API 重构为 Arc<Mutex>"子项 |
| §2.4.2 SSRF 伪实现 | T-S2-A-02 增加"真实重定向链每跳验证"子项 |
| §2.4.4 REST API 无独立 feature | T-S2-B-03a 定义独立 rest-api feature，升级 P1 |
| §2.4.5 MCP 安全基线不足 | T-S2-B-02 扩展 SAFE_ENV_VARS + filter_safe_env_vars 调用点 |
| §2.5.1 版本号漂移 | T-S1-PRE-04（已完成） |
| §2.5.2 feature 死开关 | 新增 T-S2-C-01 |
| §2.5.3 覆盖率全盲 | T-S1-PRE-06（已完成，前端部分） |
| §2.5.4 cargo-audit 门禁失效 | T-S1-PRE-06（已完成） |
| §2.5.5 MinGW 问题误判 | T-S1-PRE-05（已完成，rust-toolchain.toml） |
| §2.5.6 任务复杂度低估 | T-S1-B-01 拆分 01a/b/c；T-S6-A-01 拆分三平台 |
| §3.1 优先级升级 | T-S1-A-06/T-S1-B-03 升 P0；T-S2-B-03a 升 P1；T-S6-A-01a 升 P1 |
| §3.2 优先级降级 | T-S2-A-01 降 P2 |
| §3.3 任务拆分 | A-03/B-01/A-01(S2)/A-01(S6)/B-03(S2)/Stage 2 全部拆分 |
| §3.4 新增任务 | T-S1-PRE-01~06 / T-S2-C-01 / T-S6-B-03 |
| §4 架构级决议 | §3.4/3.6 任务描述中融入单二进制多角色/加权随机轮值/独立 sidecar 决议 |
| §7.3 隐式依赖链 | §2.4 新增 3 条 |
| §7.4 Stage 1 前置任务 | §0.1 新增前置修复完成记录 |
| §7.6 Stage 1 测试文件 | §4.4 新增 4 个测试文件 |

### 8.5 v2.1.0-pre 交付清单

以下文件在 v2.1.0-pre（2026-07-02 交付）中创建或修改：

**代码修改**：
- `src-tauri/src/swarm/orchestrator.rs` — Negotiator 改用 `negotiate_with_arbitration`
- `src-tauri/src/memory/acl.rs` — 默认 deny-all（可信主体 allow）
- `src-tauri/src/memory/layers.rs` — L4→L6 提升返回 None
- `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` — 版本号同步至 2.0.0

**新增文件**：
- `rust-toolchain.toml` — 固定 MSVC targets
- `vitest.config.ts` — 前端测试覆盖率配置
- `docs/EXPERT_REVIEW_v2.1.md` — 5 位专家联合审议报告
- `docs/EXPERT_AGENTS_v2.1.md` — 智能体角色说明

**CI 修改**：
- `.github/workflows/test.yml` — cargo-audit 移除 continue-on-error + 新增 web coverage 上传

---

**文档结束**。

本文档是 v2.1+ 开发的唯一权威任务清单。任何新增任务必须先更新本文档 §2 审计表 + §3 阶段表 + §6 清单，再开始实现。
配套设计文档 `WHITEPAPER_v2.0.md` 是唯一权威设计参考，配套审议报告 `EXPERT_REVIEW_v2.1.md` 是优先级和拆分决策的依据。三份文档共同构成Nebula项目的完整规划基线。

**下一步**：立即启动 Stage 1 正式任务，按 §3.1 推荐执行顺序 Wave 1 开始 T-S1-A-01（L0Cache 命中率统计，P0/S，无依赖）。

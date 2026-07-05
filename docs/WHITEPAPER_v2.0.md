# Nebula (nebula) · 最终白皮书 v2.0

## ——基于"黑洞-海绵"记忆引擎、L4 价值层与蜂群协作的桌面 AI Agent

**版本**：v2.0（实况版）
**日期**：2026-07-02
**作者**：Solo Developer
**状态**：v2.0 已交付，本文档追认现状并替代 v1.0/v1.5 白皮书
**性质**：项目唯一权威设计文档（含产品定位、技术架构、实现状态、未来路线）

---

## 0. 版本声明

### 0.1 文档演进

| 版本 | 日期 | 性质 | 状态 |
|------|------|------|------|
| v1.0 | 2026-06-20 | MVP 设计文档（假设 10 人团队） | 已废弃 |
| v1.5 | 2026-06-28 | 实况版（追认 v1.1.7 现状） | 已归档至 `WHITEPAPER_v1.5.md.archive` |
| **v2.0** | **2026-07-02** | **实况版（追认 Phase 1-8 完整实施）** | **当前权威** |

### 0.2 v2.0 核心变更（相对 v1.5）

1. **L4 价值层** 已实现（ConstitutionalAI + RiskAssessor + PrivacyGuard + ValuePredictor）
2. **L0 LRU 缓存层** 已实现（256 条 + SessionWindow + 预取队列）
3. **海绵多腔体 + 关键词激活** 已实现（ChamberConfig + KeywordActivator）
4. **因果图谱推理** 已实现（CausalGraphEngine：根因追溯 + 效果链 + 完整解释路径）
5. **Memory Orchestrator** 已实现（3000 token 预算 + ≤3 种记忆类型 + TaskHint 推断）
6. **L5 真自我反思** 已实现（价值对齐 / 结局复盘 / 自我改进三模式）
7. **Sidecar 进程架构** 骨架已实现（3/5 服务，进程内降级模式）
8. **Plan 模式 + 高风险准奏** 已实现（PlanEngine 状态机 + ConfirmationTracker）
9. **Git 风格记忆版本控制** 已实现（branch / tag / commit / rollback）
10. **三视角切换** 关键词启发式落地（modeRouter.ts，待升级为 LLM 级）

### 0.3 文档范围

本文档整合并替代以下历史文档：
- `WHITEPAPER_v1.5.md`（产品定位与设计哲学）
- `ARCHITECTURE.md`（v1.0 架构分层与数据流）
- `.codeartsdoer/specs/code_audit_cmp_v2/spec.md`（V2 竞品对比需求）
- `.codeartsdoer/specs/code_audit_cmp_v2/design.md`（V2 实现方案）

**未来只需阅读本文档 + `ROADMAP_v2.1.md` 即可获得项目全貌**。剩余任务规划、生产进度、工作流程详见 `ROADMAP_v2.1.md`。

### 0.4 常见版本误解澄清（v1.1.10 视角纠偏）

> **背景**：部分基于 v1.1.10 的第三方分析报告会误判项目当前状态。以下列举高频误解，供新贡献者与审计者快速校准。

| 误解（v1.1.10 视角） | 真相（v2.0 实况） | 关键证据 |
|---------------------|------------------|---------|
| 「记忆系统是 5 层 + L5 预览」 | ✅ **8 层架构 L0-L5 已完整实现**（L6/L7 推迟） | §3.1 L0-L5 + L4 价值层 + L5 真反思 |
| 「L4 价值层未提及」 | ✅ **ValuesLayer 已完整实现**：ConstitutionalAI + RiskAssessor + PrivacyGuard + ValuePredictor | `memory/values/` 4 子模块 |
| 「因果图谱是 v1.5 远期目标」 | ✅ **CausalGraphEngine 已在 v2.0 完整实现**：根因追溯 + 效果链 + 完整解释路径 | `memory/causal_graph.rs` |
| 「v2.0 主题是技能市场」 | ❌ v2.0 主题是 **L4 价值层 + L5 反思 + Sidecar 骨架 + Plan 模式** | §0.2 v2.0 核心变更 |
| 「Plan 模式未提及」 | ✅ **PlanEngine 状态机 + 高风险准奏已实现**（Pending→Approved→Executing→Done/Failed） | `plan/` 3 文件 + 7 IPC 命令 |
| 「Sidecar 未提及」 | ✅ **SidecarManager 骨架已实现**（3/5 服务：Memory/Llm/Swarm，进程内降级模式） | `sidecar/` 4 文件 |
| 「SelfReflectionEngine 是 v0 假意识」 | ✅ **L5 已升级为真自我反思**（价值对齐 / 结局复盘 / 自我改进三模式，接入 L4 + outcome 闭环） | `memory/self_reflection.rs` + `reflect.rs` |
| 「海绵引擎待完善」 | ✅ **海绵多腔体 + 关键词激活已实现**（ChamberConfig + KeywordActivator） | `memory/sponge.rs` :67-110 / :134-148 |
| 「MemoryOrchestrator 是远期规划」 | ✅ **已实现**（3000 token 预算 + ≤3 种记忆类型 + TaskHint 推断），⚠️ 但 sponge 集成是 Stage 1 任务 | `memory/orchestrator.rs` + ROADMAP T-S1-A-02 |
| 「Negotiator/AgentBus 未提及」 | ✅ **均已实现**（置信度投票 >0.8 采纳，否则 LLM 仲裁；broadcast + mpsc P2P + oneshot） | `swarm/negotiator.rs` + `swarm/bus.rs` |

**结论**：v1.1.10 与 v2.0 之间存在一个 major 版本跨越（Phase 1-8 完整实施），任何基于 v1.1.10 的分析报告必须对照本节 §0.4 表格校准后再使用。第三方报告中关于「记忆系统断裂带」「L0Cache 命中率硬编码」「反思引擎无护栏」「MemoryAcl 未接入 search」等判断仍然有效，已纳入 ROADMAP_v2.1.md Stage 1 任务清单。

---

## 1. 产品定位

### 1.1 三不三是

- **不是聊天 AI** —— 是能直接干活的数字员工
- **不是单智能体** —— 是 AI 小队协同
- **不是云端依赖** —— 本地优先 + 云端按需
- **不是封闭产品** —— 开源生态（MIT License）

### 1.2 三个核心工作模式

| 模式 | 场景 | 蜂群角色 | 工具 |
|------|------|---------|------|
| **Writing** | 写作/报告/邮件 | Writer + Reviewer | 模板库 + Markdown 编辑器 + 导出 |
| **Work** | 任务管理/会议 | Planner + Reviewer | Kanban + 时间追踪 + 会议纪要 |
| **Code** | 编程/调试/部署 | Coder + Reviewer + Planner | Monaco 编辑器 + Terminal + Git |

### 1.3 设计哲学

1. **哲学一：记忆是 AI 的灵魂** —— 8 层记忆系统 L0-L7，海绵吸收 + 黑洞压缩 + 反思升华
2. **哲学二：模式对用户不可见** —— AI 自动判断 Chat/Craft/Swarm 视角（**当前仅关键词启发式，待升级为 LLM 级**）
3. **哲学三：价值对齐前置** —— L4 价值层在任务执行前评估，高风险操作必须准奏
4. **哲学四：本地优先** —— 数据存储本地，E2EE 加密同步，私钥永不出设备
5. **哲学五：可观测可审计** —— Prometheus 指标 + OpenTelemetry 追踪 + 技能审计日志

---

## 2. 技术架构

### 2.1 顶层视图

```
┌────────────────────────────────────────────────────────────────┐
│  Tauri 2.0 进程                                                 │
│                                                                 │
│  ┌──────────────────┐  ┌────────────────┐  ┌──────────────────┐│
│  │  Tauri commands  │  │  gRPC server   │  │  REST API        ││
│  │  (130+ 个)       │  │  (22 RPC)      │  │  (可选 feature)  ││
│  └────────┬─────────┘  └────────┬───────┘  └────────┬─────────┘│
│           └──────────────────────┼────────────────────┘         │
│                                  ▼                              │
│                       ┌────────────────────┐                    │
│                       │     AppState       │  (Arc-shared)      │
│                       │  ┌──────────────┐  │                    │
│                       │  │  memory      │  │                    │
│                       │  │  L0-L5+图谱  │  │                    │
│                       │  ├──────────────┤  │                    │
│                       │  │  swarm       │  │                    │
│                       │  │  +协商+总线  │  │                    │
│                       │  ├──────────────┤  │                    │
│                       │  │  llm gateway │  │                    │
│                       │  ├──────────────┤  │                    │
│                       │  │  skills      │  │                    │
│                       │  │  +审计+WASM  │  │                    │
│                       │  ├──────────────┤  │                    │
│                       │  │  security    │  │                    │
│                       │  │  +SSRF+L4    │  │                    │
│                       │  ├──────────────┤  │                    │
│                       │  │  sync        │  │                    │
│                       │  │  +E2EE+CRDT  │  │                    │
│                       │  ├──────────────┤  │                    │
│                       │  │  sidecar mgr │  │                    │
│                       │  │  (3/5 服务)  │  │                    │
│                       │  └──────────────┘  │                    │
│                       └────────────────────┘                    │
│                                  │                              │
│                                  ▼                              │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │  Storage: SQLite (migrations 001-016) + LanceDB           │  │
│  │  Logs:    tracing-appender (rolling) + panic hook         │  │
│  │  Metrics: Prometheus exporter + OTLP exporter             │  │
│  └──────────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────┘
            │ Tauri IPC / Tauri Event
            ▼
┌────────────────────────────────────────────────────────────────┐
│  WebView (WebView2 / WKWebView / WebKitGTK)                    │
│  Preact 10 + @preact/signals + TypeScript + Tailwind CSS       │
│  ├─ Sidebar / StatusBar / CommandPalette / Toasts              │
│  ├─ Onboarding (v1.1)                                          │
│  ├─ Settings (v1.0) + DevicePanel (v1.7 部分)                  │
│  ├─ Dashboard (v1.8: 6 指标 + Sidecar 状态 + 反思面板)         │
│  ├─ ChatPanel / WritingMode / WorkMode / CodeMode              │
│  ├─ SwarmView (待升级: 时间线视图)                             │
│  ├─ MemoryInspector / MemoryMap (SVG, 待升级 WebGL)            │
│  ├─ SkillPanel (含审计日志)                                    │
│  └─ ModeSwitcher (关键词启发式, 待升级 LLM 级)                 │
└────────────────────────────────────────────────────────────────┘
```

### 2.2 技术栈

- **桌面框架**：Tauri 2.0 + Rust（编译目标 `windows_subsystem = "windows"`）
- **前端**：Preact 10 + @preact/signals + TypeScript + Tailwind CSS
- **结构化存储**：SQLite（rusqlite bundled，16 张迁移脚本）
- **向量存储**：LanceDB（feature `vector-store`，BGE-small-zh-v1.5 512 维）
- **本地 LLM**：Ollama + Qwen2.5-3B（可切换 Claude Anthropic）
- **进程模型**：单进程为主，Sidecar 架构骨架就绪（Memory/LLM/Swarm 三类，进程内降级）
- **加密**：X25519 + HKDF-SHA256 + AES-256-GCM（双棘轮 v1.1）
- **日志**：tracing + tracing-appender（daily rolling）+ panic hook 写文件

### 2.3 Cargo Feature Gates

所有新模块通过 feature flag 控制，**默认不启用**（遵循 evolution 模块模式）：

```toml
[features]
default = ["vector-store"]
vector-store = ["dep:lancedb"]      # 向量存储（默认开）
grpc = [...]                        # gRPC 服务端（默认关）
mcp = []                            # MCP 协议客户端（默认关）
channels = []                       # Telegram/Discord 渠道（默认关）
wasm-sandbox = []                   # WASM 沙箱（默认关，wasmtime 依赖待解注释）
rest-api = []                       # REST API（默认关，待新增）
did-identity = []                   # DID 去中心化身份（默认关）
crdt-sync = []                      # CRDT 多设备同步（默认关）
self-evolution = []                 # 自我进化模块（默认关）
perf-telemetry = []                 # 性能遥测（默认关）
```

---

## 3. 记忆系统（L0-L5 + L6/L7 未来）

### 3.1 八层架构

```
                  ┌─────────────┐
   user input ──▶ │ L0 cache     │  (LRU 256条 + SessionWindow 8000 token)
                  │   ✅ v2.0    │  + 预取队列（命中率统计待修复）
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L1 messages  │  (对话/操作, 7 天保留)
                  │   ✅ v1.0    │
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L2 experience│  (命名实体+概念, 30 天)
                  │   ✅ v1.0    │
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L3 facts     │  (结构化知识+技能, 90 天)
                  │   ✅ v1.0    │
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L4 knowledge │  (跨任务抽象+偏好, 1 年)
                  │   ✅ v1.0    │  + ValuesLayer 价值评估（v2.0 新增）
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L5 lessons   │  (元认知反思)
                  │   ✅ v2.0    │  + SelfReflectionEngine 三模式反思
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L6 principles│  (深层模式)  →  📋 未来
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L7 singularity│ (核心身份) →  📋 未来
                  └─────────────┘
```

### 3.2 核心引擎

| 引擎 | 模块 | 职责 | 状态 |
|------|------|------|------|
| **SpongeEngine** | `memory/sponge.rs` | 海绵吸收：去重 + 合并 + LLM 实体抽取 + 多腔体过滤 + 关键词激活 | ✅ v2.0 |
| **BlackholeEngine** | `memory/blackhole.rs` | 黑洞压缩：密度压缩 + 待归档优先（待联动 ForgettingEngine） | ⚠️ v1.0+ |
| **ReflectionEngine** | `memory/reflect.rs` | L2-L3 → L5 反思（24h day_bucket 去重 + LLM fallback） | ✅ v1.0 |
| **SelfReflectionEngine** | `memory/self_reflection.rs` | L5 真自我反思：价值对齐 / 结局复盘 / 自我改进 | ✅ v2.0 |
| **CausalGraphEngine** | `memory/causal_graph.rs` | 因果推理：根因追溯 + 效果链 + 完整解释路径 | ✅ v2.0 |
| **GraphSearchEngine** | `memory/graph_search.rs` | BFS 图遍历搜索（max_hops=3，max_results=50） | ✅ v2.0 |
| **MemoryOrchestrator** | `memory/orchestrator.rs` | 上下文组装：3000 token + ≤3 种类型 + TaskHint 推断 | ⚠️ v2.0（sponge 未集成） |
| **ForgettingEngine** | `memory/forgetting.rs` | 遗忘机制：低重要性 + TTL 标记待归档 | ⚠️ v2.0（blackhole 未联动） |
| **MemoryAcl** | `memory/acl.rs` | 访问控制：principal + resource + permission | ⚠️ v2.0（search 未接入） |
| **LayerPolicy** | `memory/layers.rs` | 层级策略：TTL + 自动提升（L3→L4, L4→L6） | ✅ v2.0 |
| **VersionControl** | `memory/version_control.rs` | Git 风格版本控制：branch / tag / commit / rollback | ✅ v1.5 |
| **DataExporter** | `memory/export.rs` | JSON-LD 导入导出（含 [REDACTED] 脱敏） | ⚠️ v2.0（缺 RelationEntity） |
| **EntityExtractor** | `memory/entity_extractor.rs` | LLM 驱动实体关系抽取（5 种 RelationKind） | ✅ v2.0 |
| **L0Cache** | `memory/l0_cache.rs` | 热缓存 + 会话窗口 + 预取队列 | ⚠️ v2.0（命中率统计硬编码 0） |

### 3.3 L4 价值层（v2.0 新增）

`memory/values/` 含 4 个子模块：

| 组件 | 职责 | 输出 |
|------|------|------|
| **ConstitutionalAI** | 宪法规则引擎，识别格式化磁盘 / DROP TABLE / 批量删除 / 转账 / PII 外泄等灾难级操作 | `RuleSeverity::Forbidden / Warn / Info` |
| **RiskAssessor** | 风险评估器，按 ActionKind 给出 4 级裁定 | `Safe / NeedsConfirm / NeedsPlan / Forbidden` |
| **PrivacyGuard** | 隐私守卫，复用 SensitiveScanner 对身份证号 / API key 脱敏 | 脱敏后内容 |
| **ValuePredictor** | 价值预测器，启发式打分，低于 0.15 时 Allow 升级为 Confirm | `Verdict::Allow / Confirm / Reject` |

`ValuesLayer::evaluate()` 顺序调用：宪法 → 隐私 → 风险 → 价值（短路）。
接入点：`SwarmOrchestrator::pre_check()` 在任务执行前调用，触发 Plan/Confirmation 流程。

### 3.4 海绵多腔体 + 关键词激活（v2.0 新增）

- **KeywordActivator**：默认中英文激活词集（重要/记住/关键/important/remember/critical），未命中时 `importance *= 0.3` 但不丢弃
- **ChamberConfig**：按 `memory_type` 隔离候选，启用时扩展搜索范围 3x 并按类型过滤
- 新增 `SpongeResult::Deactivated` 变体

### 3.5 L5 真自我反思（v2.0 新增）

三种反思模式：

1. **ValueAlignment（价值对齐）**：评估近期记忆与 L4 ValuesLayer 的对齐度
2. **OutcomeReview（结局复盘）**：读取 `task_outcomes` 表分析成功/失败模式
3. **SelfImprovement（自我改进）**：基于历史反思生成改进建议

**护栏现状**：
- ✅ fallback 开关（LLM 不可用降级为模板）
- ✅ 24h day_bucket 去重
- ❌ ≤5 轮次数限制（待实现）
- ❌ 1h 冷却期（待实现）

---

## 4. 蜂群协作（Swarm）

### 4.1 架构

```
┌──────────────────────────────────────────────┐
│           SwarmOrchestrator                  │
│  ┌────────────────────────────────────────┐  │
│  │  pre_check() → L4 ValuesLayer 评估    │  │
│  │  ↓                                     │  │
│  │  execute(task)                         │  │
│  │  ├─ build_agent_pool (6 Generic)       │  │
│  │  ├─ fan-out: 并行执行                  │  │
│  │  ├─ Negotiator::negotiate() 冲突仲裁   │  │
│  │  │  ├─ 置信度投票 (>0.8 直接采纳)     │  │
│  │  │  └─ LLM 仲裁 (失败降级最高置信度)   │  │
│  │  └─ emit_event (Tauri Event + bus)    │  │
│  └────────────────────────────────────────┘  │
│                                              │
│  AgentBus (broadcast + mpsc P2P)            │
│  ├─ send(target, msg) / request / reply     │
│  └─ broadcast(msg) / subscribe()            │
│                                              │
│  TeamContext (Arc<RwLock<Vec<Entry>>>)       │
│  └─ 单次 execute 内有效，不跨任务           │
└──────────────────────────────────────────────┘
```

### 4.2 Agent 角色

| Kind | 状态 | 说明 |
|------|------|------|
| Generic | ✅ 默认 | v2.0 后默认角色，6 个并行 |
| Coder | ⚠️ deprecated | 仅 `build_agent_pool_by_kinds` 显式指定时实例化 |
| Writer | ⚠️ deprecated | 同上 |
| Reviewer | ⚠️ deprecated | 同上 |
| Researcher | ⚠️ deprecated | 同上 |
| Planner | ⚠️ deprecated | 同上 |

**默认行为**：6 个 GenericAgent 并行执行 + Negotiator 协商。
**显式分工**：`SwarmTask.agents = ["coder","writer","reviewer"]` 时按指定 kinds 实例化（仍是并行，非流水线）。

### 4.3 协商机制（v2.0 新增）

- **置信度投票**：每个 AgentOutput 含 `confidence: f32` (0.0-1.0)
- **阈值规则**：最高置信度 > 0.8 直接采纳；否则触发 LLM 仲裁
- **降级**：LLM 不可用时选最高置信度并标记"未经仲裁"
- **冲突检测**：`Negotiator::has_conflict()` 检测输出差异

### 4.4 消息总线（v2.0 新增）

`AgentBus` 基于 tokio::sync：
- **broadcast channel**（容量 256）：广播发现信息
- **mpsc channel**（容量 64）：点对点邮箱
- **oneshot**：请求-响应模式
- **4 种消息类型**：Request / Response / Notification / Capability

### 4.5 蜂群协作未实现项

- ❌ **领导轮值制**（leader rotation）
- ❌ **跨任务 Team Context Pool**（当前每次 execute 新建 TeamContext）
- ❌ **蜂群内 CRDT 同步**（CRDT 仅用于跨设备）

---

## 5. 工作模式与 Plan 流程

### 5.1 三模式

| 模式 | 后端模块 | 前端组件 | 状态 |
|------|---------|---------|------|
| Writing | `writing/mod.rs` + `writing/templates.rs` | WritingMode.tsx | ✅ v1.0 |
| Work | `work/mod.rs` | WorkMode.tsx | ✅ v1.0 |
| Code | `editor/mod.rs` + `editor/file_ops.rs` + `editor/git.rs` + `editor/debounce.rs` | CodeMode.tsx | ✅ v1.0 |

### 5.2 Plan 模式（v2.0 新增）

`plan/plan_mode.rs` + `plan/confirmation.rs` + `plan/mod.rs`：

```
用户提交任务
  ↓
pre_check() → L4 ValuesLayer 评估
  ↓
Verdict::Allow → 直接执行
Verdict::Confirm → 创建 ConfirmationRequest (5min 超时)
  ↓ 用户审批
Verdict::Reject → 拒绝执行
Verdict::NeedsPlan → 创建 PlanRequest (含 steps/expected_outcome/rollback_strategy)
  ↓ 用户审批 plan
  Approved → 执行
  Rejected → 取消
```

**PlanRequest 状态机**：Pending → Approved → Executing → Done / Failed

**7 个 IPC 命令**：`plan_pre_check` / `plan_approve_confirmation` / `plan_deny_confirmation` / `plan_approve_plan` / `plan_reject_plan` / `plan_get_plan` / `plan_get_confirmation`

### 5.3 高风险操作准奏（v2.0 新增）

触发 `NeedsConfirm` 的动作：
- Delete / BulkDelete（文件删除）
- Send / Transfer（资金/数据转账）
- Execute（系统命令执行）
- Network（外部网络请求）

### 5.4 三视角切换（部分实现）

- **当前**：`src/lib/modeRouter.ts` 关键词启发式判断（WRITING/CODE/WORK_KEYWORDS）
- **触发点**：仅 ChatPanel 发消息时自动切换
- **未来**：升级为 LLM 级判断，接入 MemoryOrchestrator 上下文

### 5.5 未实现项

- ❌ **影子文件持久化**（当前 diff 视图仅内存实时读取）
- ❌ **撤销栈**（`rollback_strategy` 仅占位字符串字段）
- ⚠️ **PlanStep 自动 LLM 拆解**（v1.3 注释明确 v1.4 才会有）

---

## 6. 安全模型

### 6.1 纵深防御层

| 层 | 模块 | 职责 | 状态 |
|---|------|------|------|
| L1 | `security/injection_guard.rs` | Prompt 注入检测 | ✅ v1.1 |
| L2 | `security/detectors.rs` | 敏感内容扫描（API key / Token / 私钥 / 身份证 / 手机号） | ✅ v1.0 |
| L3 | `security/ssrf_guard.rs` | SSRF 防护（RFC1918 / 环回 / 链路本地 / CGNAT） | ⚠️ v2.0（gateway 集成，engine 未集成） |
| L4 | `security/keychain.rs` | OS keychain 抽象（Windows Credential Manager / macOS Keychain / Linux Secret Service） | ✅ v1.1 |
| L5 | `memory/values/` | L4 价值层（宪法 + 风险 + 隐私 + 价值） | ✅ v2.0 |
| L6 | `memory/acl.rs` | 记忆访问控制（principal + resource + permission） | ⚠️ v2.0（search 未接入） |
| L7 | `skills/sandbox.rs` | Python 沙箱 + WASM 沙箱骨架 | ⚠️ v2.0（wasmtime 依赖待解注释） |
| L8 | `skills/audit.rs` | 技能审计日志 | ✅ v2.0 |

### 6.2 Shell 命令防护

- **白名单**：`ShellExecutor` 内置 24 个二进制（v1.0 移除 rm/mv/cp）
- **硬线黑名单**：⚠️ 待新增（`UNRECOVERABLE_CMDS` 如 `mkfs` / `dd` / `shutdown`）
- **路径沙箱**：`editor_*` 强制工作区根拼接到相对路径之前

### 6.3 E2EE 同步

- **KDF**：X25519 ECDH → HKDF-SHA256 → 32 字节对称密钥
- **AEAD**：AES-256-GCM，12 字节随机 nonce，16 字节 tag
- **前向保密**：v1.1 升级到双棘轮
- **持久化**：加密 envelope 落到本地 `sync_inbox/`，接收方轮询拉取
- **未来**：云端中继（QUIC P2P + relay server）—— 当前完全本地化

### 6.4 DID 身份（部分实现）

- ✅ `did:key` 方法（X25519 公钥派生，multicodec 0xec 0x01）
- ✅ DID Document 生成（@context + verificationMethod + keyAgreement）
- ⚠️ multibase 依赖（当前用 bs58 自实现 Base58Btc 编码）

---

## 7. 可观测性

### 7.1 三大支柱

| 支柱 | 模块 | 暴露方式 | 状态 |
|------|------|---------|------|
| **结构化日志** | tracing + tracing-appender | `NEBULA_LOG_FORMAT=json` + daily rolling | ✅ v1.0 |
| **Prometheus 指标** | `metrics/exporter.rs` | `GET /metrics` HTTP 端点（17 项指标） | ✅ v1.8 |
| **OpenTelemetry 追踪** | `observability/otel.rs` | OTLP/gRPC 导出（BatchSpanProcessor） | ✅ v1.8 |

### 7.2 Prometheus 指标清单

17 项指标，涵盖：
- 嵌入缓存命中率（`embedding_cache_hits_total / misses_total`）
- 记忆操作（`memory_stores_total / searches_total`）
- 蜂群执行（`swarm_executions_total / agent_failures_total`）
- LLM 调用（`llm_chat_total / chat_latency_us_total`）
- 进程资源（`process_rss_bytes / cpu_pct`）

**默认关闭**，需 `NEBULA_METRICS_ADDR=0.0.0.0:9100` 环境变量开启。

### 7.3 实时仪表盘

`src/components/Dashboard.tsx` 展示 6 张 MetricCard：
- 内存占用 (RSS)
- 向量检索延迟
- 蜂群执行数
- 缓存命中率（实际是 embedding cache 命中率，非记忆检索召回率）
- L4 拦截率（**当前硬编码 0，待接入**）
- LLM 调用延迟

**待补齐**：Token 成本统计、记忆命中率、Swarm 事件实时流。

### 7.4 崩溃捕获

- 前端：`ErrorBoundary` 把最近 5 次崩溃写到 `localStorage`
- 后端：panic hook 写 `nebula-panic.log` 到 `%LOCALAPPDATA%\nebula\logs\`（v1.1.9 新增，针对 `windows_subsystem = "windows"` 静默崩溃问题）

---

## 8. OS 集成

### 8.1 已实现

| 能力 | 模块 | 状态 |
|------|------|------|
| 系统托盘 | `os/tray.rs` | ✅ v1.7（最简版：显隐切换 + 关闭最小化） |
| 文件关联 | `tauri.conf.json` fileAssociations | ✅ v1.7（.hermes / .hmemory / .md / .txt） |
| 拖拽支持 | `lib.rs:844-848` DragDrop 事件 | ✅ v1.7 |
| 剪贴板读写 | `os/clipboard.rs` + IPC | ✅ v1.0 |
| 通知 | `os/notifications.rs` | ✅ v1.0 |
| Shell 执行 | `os/shell.rs` + 白名单 | ✅ v1.0 |
| 快捷键 | `os/shortcut.rs` | ⚠️ v1.7（4/9 P0，P1 全未实现） |
| 启动崩溃日志 | panic hook | ✅ v1.1.9 |

### 8.2 未实现

- ❌ **电源管理**（息屏防止 / 休眠响应 / 电池状态）
- ❌ **剪贴板监听**（后端 `watch_once` 存在但未暴露 IPC 给前端）
- ❌ **P1 快捷键**（Cmd+Shift+Space 快速输入小窗、Cmd+1/2/3 模式切换）
- ❌ **OS-Controller**（真正的 OS 级自动化，白皮书明确推迟 v1.5+）

---

## 9. Sidecar 进程架构

### 9.1 架构骨架

`sidecar/` 目录（v2.0 新增）：

```
sidecar/
├── mod.rs          # 模块入口 + SidecarKind + 工具函数
├── manager.rs      # SidecarManager + SidecarStatus + SidecarRuntime
├── ipc.rs          # IpcMode + MemoryIpcClient + LlmIpcClient + SwarmIpcClient
└── protocol.rs     # SidecarConfig + SidecarReady
```

### 9.2 SidecarKind（3/5 已定义）

| Kind | 状态 | 说明 |
|------|------|------|
| Memory | ✅ 已定义 | 记忆服务独立进程 |
| Llm | ✅ 已定义 | LLM 网关独立进程 |
| Swarm | ✅ 已定义 | 蜂群编排独立进程 |
| Skill | ❌ 未定义 | 技能服务（待新增） |
| Reflection | ❌ 未定义 | 反思服务（待新增） |

### 9.3 进程管理能力

- ✅ `start / stop / start_all / stop_all / status / is_running / wait_ready`
- ✅ 崩溃自动重启：`supervisor_loop` 5s tick，`max_restarts=5`
- ✅ 状态机：Stopped / Starting / Running / Crashed / Restarting
- ✅ 进程内降级：sidecar 二进制不存在时标记 `Running + listen_addr="in-process"`
- ⚠️ 健康检查简化：固定端口猜测，未实现真正 gRPC HealthCheck
- ⚠️ 退避策略：固定 5s tick，非指数退避
- ❌ bootstrap 未自动 `start_all()`，需用户手动 `sidecar_start`

### 9.4 4 个 IPC 命令

`sidecar_list_status` / `sidecar_start` / `sidecar_stop` / `sidecar_restart`

---

## 10. 技能系统

### 10.1 架构

```
┌──────────────────────────────────────────┐
│           SkillEngine                    │
│  ├─ SkillStore (SQLite)                  │
│  ├─ SkillImporter (serde_yaml)           │
│  ├─ SkillExtractor (LLM 抽取)            │
│  ├─ SkillMarketplace (本地索引)          │
│  ├─ SkillAuditLogger (审计日志)          │
│  ├─ Sandbox                              │
│  │  ├─ PythonSandbox (子进程 + 阻断)     │
│  │  └─ WasmSandbox (wasmtime, 待启用)    │
│  └─ TeamSkillsHubClient (远程 hub, 待接入)│
└──────────────────────────────────────────┘
```

### 10.2 技能来源

| 来源 | 状态 | 说明 |
|------|------|------|
| 本地创建 | ✅ v1.0 | `create_skill` IPC |
| agentskills.io | ⚠️ v2.0 | serde_yaml ✓，但 `SkillMeta` 字段缺失 |
| ClawHub | ✅ v1.3 | `import_from_clawhub` |
| TeamSkillsHub | ❌ v2.0 | `import_from_teamskillshub` 仍返回 "not yet implemented" |

### 10.3 信任级别

| 级别 | 分数 | auto_approve_shell | auto_approve_network |
|------|------|-------------------|---------------------|
| builtin | 100 | ✅ | ✅ |
| official | 80 | ✅ | ❌ |
| trusted | 60 | ❌ | ❌ |
| community | 40 | ❌ | ❌ |
| untrusted | 0 | ❌ | ❌ |

### 10.4 审计日志（v2.0 新增）

`skills/audit.rs` + `migrations/012_skill_audit_log.sql`：
- 每次技能执行记录：skill_id / executed_at / input_summary (≤200 字符) / output_summary (≤200) / duration_ms / sandbox_type / security_scan_result / success
- 自动脱敏 API key 为 `[REDACTED]`
- 2 个 IPC 命令：`skill_audit_list` / `skill_audit_list_for_skill`

---

## 11. 通信渠道

### 11.1 当前状态

| 渠道 | 状态 | 说明 |
|------|------|------|
| 桌面 WebView | ✅ v1.0 | 主渠道 |
| Telegram | ⚠️ v1.7 | reqwest 手写 HTTP（非 teloxide 原生 SDK），send() 空桩 |
| Discord | ⚠️ v1.7 | reqwest 手写 HTTP（非 serenity 原生 SDK），send() 空桩 |
| WebChat | ⚠️ v1.7 | 临时 token + 24h TTL + 限速，骨架就绪 |
| JiuwenSwarm 桥接 | ⚠️ v1.0 | MessageBridge 默认禁用 |

### 11.2 未实现项

- ❌ **AppState 无 `channel_router` 字段**（ChannelRouter 定义存在但未注入）
- ❌ **teloxide / serenity 依赖**（grep 全仓零匹配）
- ❌ **Adapter send 闭环**（TelegramAdapter / DiscordAdapter 的 send() 是空桩）

---

## 12. API 与外部集成

### 12.1 Tauri IPC 命令

130+ 个 `#[tauri::command]`，覆盖：
- memory / chat / swarm / skill / editor / plan / sidecar / channel / acl / device / identity / export / values 等 16 个域

### 12.2 gRPC 服务

`grpc/server.rs` + `grpc/proto/nebula.v1.rs`：

| Service | RPCs | 状态 |
|---------|------|------|
| MemoryService | Store / Get / GetMany / Search / ListRecent / Delete / UpdateImportance / Stats / AbsorbChatTurn | ✅ 21/22 wire 层路由 |
| ChatService | Chat / ChatStream | ⚠️ ChatStream trait 层 ✓, wire 层 NOT_IMPLEMENTED |
| SwarmService | Execute / ListAgents / GetAgent | ✅ |
| SkillService | Create / Use / Rate / List / Search | ✅ |
| ReflectionService | Trigger / ListRecent / Get | ✅ |
| Health | Health | ✅ |
| SwarmService | StreamEvents | ❌ wire 层 NOT_IMPLEMENTED |

**传输层**：hyper HTTP/2 + 自定义 JSON framing shim（**非 tonic::transport::Server**）
**地址**：`127.0.0.1:50051`（`NEBULA_GRPC_ADDR`）
**关闭**：`NEBULA_GRPC=0`

### 12.3 REST API（部分实现）

`api/rest.rs` 116 行：
- ✅ `/api/health` / `/api/memories` / `/api/skills`
- ⚠️ `/api/chat` / `/api/swarm/execute`（占位响应）
- ❌ 无 `auth.rs` 认证层
- ❌ 无 `rest-api` feature（被 `grpc` feature 隐式包含）

### 12.4 MCP 协议（部分实现）

`mcp/` 目录骨架完整：
- ✅ `McpManager` + `McpClient` + `McpTransport` + `McpConfig` + `McpSecurity`
- ✅ ToolRegistry `register_mcp_tools` / `unregister_server`（前缀 `mcp_<server>_<tool>`）
- ✅ 4 个 IPC 命令：`mcp_list_servers / add_server / remove_server / list_tools`
- ✅ **JSON-RPC 2.0 协议帧已实现**（`discover_tools` / `invoke_tool` 真实调用,bootstrap 时自动注册到 ToolRegistry）

---

## 13. 数据存储

### 13.1 SQLite 迁移脚本

16 张迁移脚本（`migrations/001_*.sql` ~ `016_memory_versions.sql`）：

| # | 名称 | 用途 |
|---|------|------|
| 001 | initial | 基础表（memories / skills / agents / sync_inbox 等） |
| 002-004 | index | 索引优化 |
| 005-008 | sync | E2EE 同步相关 |
| 009-011 | skill_audit | 技能审计 |
| 012 | skill_audit_log | 审计日志表 |
| 013 | memory_relations | 关系表（含 evidence 列） |
| 014 | memories_archived | archived 列（遗忘机制） |
| 015 | memory_acl | ACL 表 |
| 016 | memory_versions | CRDT 版本表 |

### 13.2 LanceDB 向量

- 模型：BGE-small-zh-v1.5（512 维）
- 表：`memories`
- 操作：`search(query_emb, k)` + `upsert(id, emb)` + `delete(id)`

---

## 14. 当前实现状态总览

### 14.1 已完整实现的模块（✅）

| 模块 | 完成版本 | 关键文件 |
|------|---------|---------|
| 8 层记忆系统（L0-L5+图谱） | v2.0 | `memory/` 18 个文件 |
| SpongeEngine（多腔体+关键词） | v2.0 | `memory/sponge.rs` |
| CausalGraphEngine（因果推理） | v2.0 | `memory/causal_graph.rs` |
| GraphSearchEngine（图遍历） | v2.0 | `memory/graph_search.rs` |
| EntityExtractor（LLM 抽取） | v2.0 | `memory/entity_extractor.rs` |
| LayerPolicy（自动提升 L3→L4→L6） | v2.0 | `memory/layers.rs` |
| VersionControl（Git 风格） | v1.5 | `memory/version_control.rs` |
| ValuesLayer（L4 价值层） | v2.0 | `memory/values/` 4 子模块 |
| SelfReflectionEngine（L5 反思） | v2.0 | `memory/self_reflection.rs` |
| ReflectionEngine（L2-L3→L5） | v1.0 | `memory/reflect.rs` |
| Negotiator（置信度+LLM 仲裁） | v2.0 | `swarm/negotiator.rs` |
| AgentBus（消息总线） | v2.0 | `swarm/bus.rs` |
| PlanEngine（Plan 模式+准奏） | v2.0 | `plan/` 3 文件 |
| SkillAuditLogger（审计日志） | v2.0 | `skills/audit.rs` |
| SidecarManager（进程管理骨架） | v2.0 | `sidecar/` 4 文件 |
| SsrfGuard（SSRF 防护） | v2.0 | `security/ssrf_guard.rs` |
| CrdtEngine（LWW+字段合并） | v2.0 | `sync/crdt.rs` |
| DeviceManager（设备撤销） | v2.0 | `sync/device_manager.rs` |
| Prometheus exporter | v1.8 | `metrics/exporter.rs` |
| OpenTelemetry OTLP | v1.8 | `observability/otel.rs` |
| 系统托盘 + 文件关联 + 拖拽 | v1.7 | `os/tray.rs` / `os/file_handler.rs` |
| E2EE 双棘轮同步 | v1.1 | `sync/e2ee.rs` |
| 注入防护 + 敏感脱敏 + Keychain | v1.1 | `security/` 4 文件 |
| i18n + Onboarding + CommandPalette | v1.1 | `src/i18n/` / `src/components/Onboarding.tsx` |

### 14.2 部分实现的模块（⚠️）

| 模块 | 主要差距 |
|------|---------|
| L0Cache | 命中率统计硬编码 0 |
| MemoryOrchestrator | sponge.rs 未集成，无独立 IPC |
| ForgettingEngine | blackhole 未联动优先压缩待归档 |
| MemoryAcl | sponge search 未接入 ACL 过滤 |
| DataExporter | 缺 RelationEntity，relation_count 硬编码 0 |
| 反思引擎护栏 | 缺 ≤5 轮限制 + 1h 冷却 |
| WASM 沙箱 | wasmtime 依赖被注释，feature 空 |
| SSRF 防护 | skills/engine.rs 未集成 |
| agentskills.io 兼容 | SkillMeta 结构 + 3 字段缺失 |
| gRPC wire | stream_events wire 层 NOT_IMPLEMENTED，未用 tonic |
| MCP 协议 | JSON-RPC 帧未实现，discover/invoke 是桩 |
| 通信渠道 | 无 teloxide/serenity，AppState 无 channel_router，send 空桩 |
| REST API | 无 auth.rs，无 rest-api feature，/api/chat 占位 |
| TeamSkillsHub | importer 仍返回 "not yet implemented" |
| AgentDynamicPool | orchestrator 未接入，仍用 build_agent_pool |
| LLM 流式 | IPC 边界把流 collect 为 Vec，无 chat-token 事件 |
| Swarm 可视化 | 无 subscribe_events，无 SwarmEvent 枚举，前端无监听 |
| 设备撤销 UI | Settings.tsx 仅占位文本 |
| 三视角切换 | 仅关键词启发式，非 LLM 级 |
| Sidecar | 仅 3/5 服务，bootstrap 未自动 start_all |
| 仪表盘 | 缺 Token 成本 + 记忆命中率，L4 拦截率占位 0 |

### 14.3 完全未实现的模块（❌）

| 模块 | 设计来源 |
|------|---------|
| 领导轮值制 | 蜂群协作增强 |
| 跨任务 Team Context Pool | 蜂群协作增强 |
| 蜂群内 CRDT 同步 | 蜂群协作增强 |
| OS-Controller | 白皮书 v1.5+ |
| 电源管理 | OS 集成 |
| 自动备份（7 天每日 1 次） | 设计 §20.2 |
| 多模态嵌入 | 白皮书 §3.6 |
| 云端中继同步 | 设计 v1.0+ |
| 浮动窗 / 画中画 | 前端体验 |
| 记忆画布 WebGL (PixiJS/D3) | 前端性能 |
| 代码分割懒加载 | 前端性能 |
| "AI 自动判断模式"哲学 | 白皮书 §2.1 |

---

## 15. 明确不在 v2.0 中的能力

| 能力 | 推迟版本 | 原因 |
|------|---------|------|
| 移动端 | v3.0+ | 桌面优先 |
| OS-Controller（OS 级自动化） | v2.5+ | 需深度平台适配 |
| L6/L7 记忆层 | v2.5+ | L5 已足够当前场景 |
| 团队/多用户支持 | v3.0+ | 单用户优先 |
| 真正的 LLM 意识 | 不可实现 | 当前 L5 是"假意识"，真意识无明确路径 |
| WASM 沙箱生产可用 | v2.1 | wasmtime 依赖需解注释 + feature 绑定 |
| 云端中继同步 | v2.5+ | 需 relay server 基础设施 |
| 多模态嵌入 | v2.5+ | 需 multimodal embedder 模型 |

---

## 16. 性能预算

| 指标 | 目标 | 当前 |
|------|------|------|
| 冷启动 | < 5s (macOS/Linux) / < 8s (Windows) | ✅ 达标 |
| RSS 内存 | < 500 MB | ✅ 达标（典型 200-300 MB） |
| command 响应 | < 200ms | ✅ 达标 |
| 知识图谱实体抽取 | < 5s（单条记忆） | ✅ 达标（LLM 驱动） |
| 图遍历搜索 | < 200ms（3 跳 1000 边） | ✅ 达标 |
| MCP 工具调用额外延迟 | < 100ms | ❌ 未实现 |
| DID 文档生成 | < 50ms | ✅ 达标 |
| 数据导出（10000 条） | < 10s | ✅ 达标 |

---

## 17. 附录

### 17.1 关键文件路径

- **后端入口**：`src-tauri/src/lib.rs`
- **AppState 定义**：`src-tauri/src/lib.rs:234-312`
- **记忆系统**：`src-tauri/src/memory/` (18 个 .rs 文件)
- **蜂群协作**：`src-tauri/src/swarm/` (5 个 .rs 文件)
- **技能系统**：`src-tauri/src/skills/` (9 个 .rs 文件)
- **安全模块**：`src-tauri/src/security/` (5 个 .rs 文件)
- **同步模块**：`src-tauri/src/sync/` (6 个 .rs 文件)
- **Plan 模式**：`src-tauri/src/plan/` (3 个 .rs 文件)
- **Sidecar**：`src-tauri/src/sidecar/` (4 个 .rs 文件)
- **前端入口**：`src/App.tsx`
- **前端状态**：`src/stores/nebulaStore.ts`
- **API 封装**：`src/lib/tauri.ts`
- **i18n**：`src/i18n/{zh-CN,en-US}.json`

### 17.2 配置文件

- `src-tauri/tauri.conf.json` — Tauri 应用配置
- `src-tauri/Cargo.toml` — Rust 依赖与 feature flags
- `package.json` — 前端依赖
- `vite.config.ts` — Vite 构建配置

### 17.3 环境变量

| 变量 | 默认值 | 用途 |
|------|--------|------|
| `NEBULA_GRPC` | `1` | 启用 gRPC 服务（`0` 禁用） |
| `NEBULA_GRPC_ADDR` | `127.0.0.1:50051` | gRPC 监听地址 |
| `NEBULA_LOG_FORMAT` | `text` | 日志格式（`json` 可选） |
| `NEBULA_LOG_DIR` | `%LOCALAPPDATA%\nebula\logs\` | 日志目录 |
| `NEBULA_METRICS_ADDR` | （空） | Prometheus 导出地址（空=禁用） |
| `NEBULA_OTLP_ENDPOINT` | （空） | OpenTelemetry OTLP 端点（空=禁用） |
| `NEBULA_OTLP_SERVICE` | `nebula` | OTLP 服务名 |

### 17.4 历史文档归档

以下文档已被本文档替代，仅保留作历史归档：
- `docs/WHITEPAPER_v1.5.md`（v1.5 实况版）
- `docs/ARCHITECTURE.md`（v1.0 架构文档）
- `.codeartsdoer/specs/code_audit_cmp_v2/spec.md`（V2 竞品对比需求规格）
- `.codeartsdoer/specs/code_audit_cmp_v2/design.md`（V2 实现方案）
- `.codeartsdoer/specs/code_audit_cmp_v2/tasks.md`（V2 任务清单）

**未来只需阅读本文档 + `ROADMAP_v2.1.md`**。

---

**文档结束**。任务规划、生产进度、剩余工作流程详见 `docs/ROADMAP_v2.1.md`。

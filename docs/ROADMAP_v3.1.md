# Nebula · 生产路线图 v3.1

**版本**：v3.1（整合 DEVELOPMENT_PROPOSAL_v3.0 + 外部审查 + 代码级审计）
**日期**：2026-07-10
**作者**：Solo Developer
**前置文档**：
- `ROADMAP_v2.1.md`（Stage 1-6 工程闭环，已全部完成）
- `ROADMAP_v2.2.md`（已完成任务的完整 commit 描述归档）
- `ROADMAP_v2.3.md`（v3.1 前身，首次精简版）
- `DEVELOPMENT_PROPOSAL_v3.0.md`（v3.1 任务来源，代码级审计 + 架构演进）
- `WHITEPAPER_v3.1.md`（设计权威）

---

## 0. v2.3 → v3.1 变更说明

**变更动机**：v3.0 建议书新增了代码级审查（AppState/Swarm/Evolution/Writing）和蜂群架构升级方向，v2.3 未覆盖。

**变更范围**：
| 范围 | v2.3 | v3.1 |
|------|------|------|
| 技术债务任务数 | 34 | **41**（+7 项来自 v3.0 代码级审查） |
| 架构演进支柱 | 无 | **新增 T-E-AE-* 系列（7 任务，含 AE-03 拆分为 03/03b）** |
| 推进阶段 | Phase 0-3 | **Phase 0-4（新增 Phase 3 蜂群架构升级）** |
| 写作场景 | 未规划 | **新增自媒体 + 长篇小说工作流** |

---

## 1. 当前状态总览

| 支柱 | 已完成 | 未完成 | 进度 |
|------|--------|--------|------|
| A 省钱（T-E-A-01~14） | 14 | 0 | ✅ 100% |
| B 智能（T-E-B-01~18） | 18 | 0 | ✅ 100% |
| C 贴合（T-E-C-01~20） | 20 | 0 | ✅ 100% |
| D 快（T-E-D-01~10） | 10 | 0 | ✅ 100% |
| S 贯穿层（T-E-S-01~64，含跳号） | 53 | 0 | ✅ 100% |
| Loop Engineering（T-E-L-01~08b） | 9 | 0 | ✅ 100% |
| **架构演进（T-E-AE-01~06 + 03b）** | **7** | **0** | **✅ 100%** |
| **合计 T-E-*** | **131** | **0** | **✅ 100%** |
| 技术债务（T-D-*） | 40 | 1 🔄 | 97.6%（T-D-C-07 跟踪中） |

**Stage 7 P0 阶段**：✅ 全部完成（12/12）
**Loop 阶段一**（最小可用 Loop）：✅ 全部完成（T-E-L-01/02/03）
**Loop 阶段二**（信号源+模板+可视化）：✅ 全部完成（T-E-L-04 ✅, T-E-L-05 ✅, T-E-L-08a ✅）
**Loop 阶段三**（成本+可观测+设计）：✅ 全部完成（T-E-L-06 ✅, T-E-L-07 ✅, T-E-L-08b ✅）

**🎉 T-E-* 全部 131 个功能任务已于 2026-07-10 完成（131/131 = 100%）。**

---

## 2. 已完成任务索引（精简，仅 ID + 标题 + 日期）

> 完整 commit hash + 测试详情见 `ROADMAP_v2.2.md` 对应章节或 `git log --grep "T-E-"`。

### 2.1 支柱 A 省钱（14/14 ✅）

T-E-A-01~14 全部完成（2026-07-03 ~ 2026-07-04），含 SemanticCache/TokenJuice/ModelRouter/Prefix-Cache/日预算/费用追踪/Credits Dashboard/费用报告/记忆成本标签/缓存命中率/智能预取/Automation Credits/费用加密/Arena A/B。

### 2.2 支柱 B 智能（18/18 ✅）

T-E-B-01~18 全部完成（2026-07-03 ~ 2026-07-09），含 LLM Wiki/三视图/双向同步/溯源链/双向链接/index+log/知识图谱/Obsidian 兼容/文件夹监控/#注入/BM25混合搜索/文档提取/知识卡片/Dataview DSL/MDRM 5维图谱/ReasoningChain/思维树/AI 自动整理 MOC。

### 2.3 支柱 C 贴合（20/20 ✅）

T-E-C-02/03/06/08/09/10/13/14/15/16/17/18/19/20 完成（2026-07-03 ~ 2026-07-09），含 ScreenReader(C-02)/UiAutomator 抽象层(C-03)/Hybrid Browser Agent(C-06)/Shadow Workspace(C-08)/任务录屏回放(C-09)/WorkflowCanvas(C-10)/异步长任务(C-13)/工作场景模板库(C-13)/剪贴板智能监听(C-14)/语音交互引擎(C-15)/一键导出(C-16)/IM扫码绑定(C-17)/OAuth 集成层(C-18)/多端协同(C-19)/Docker部署(C-20)。
T-E-C-01/04/05/07/11/12 完成（2026-07-10），含 OS-Controller VLM 模式/ActionExecutor/OS-Controller Sidecar/Remote Operator/操作录制回放/Design Mode。

### 2.4 支柱 D 快（10/10 ✅）

T-E-D-01/02/03/04/05/06/07/10 完成（2026-07-03 ~ 2026-07-09），含冷启动优化/首响500ms/悬浮球/8 人格系统/Proactive Engine/文件拖拽/浮动进度窗/多Agent并行流式。
T-E-D-08/09 完成（2026-07-10），含 WebGL 引擎复用/UI 性能基准 CI。

### 2.5 支柱 S 贯穿层（53/53 ✅）

Stage 7 P0 批次 1-4（12/12 ✅）+ 其他 33 个 S 任务（含 T-E-S-06 组织编排/T-E-S-10 WorkflowCanvas/T-E-S-13 工作流模板/T-E-S-58 Calendar/T-E-S-63 三定时机制）完成。详见 v2.2 对应章节。
T-E-S-11/12/14/22/26/53/56/60 完成（2026-07-10），含 蜂群运行时画布/节点交互/执行回放/AIO Sandbox/Event Stream 协议化/Cron 定时任务引擎/Automation 模板/Gateway 守护进程。

### 2.6 Loop Engineering（9/9 ✅）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-L-01 | MasterAgent Loop 执行模式 | 2026-07-08 ✅ |
| T-E-L-02 | CronTask 扩展 | 2026-07-08 ✅ |
| T-E-L-03 | ReviewerAgent 升级为 CheckerAgent | 2026-07-08 ✅ |
| T-E-L-04 | GitHub MCP 连接器（pull-only） | 2026-07-09 ✅ |
| T-E-L-05 | Loop 模板库 | 2026-07-08 ✅ |
| T-E-L-06 | Loop 预算管理 + 安全防护 | 2026-07-08 ✅ |
| T-E-L-07 | Loop 审计日志 | 2026-07-09 ✅ |
| T-E-L-08a | Loop 运行时阶段环 | 2026-07-10 ✅ |
| T-E-L-08b | Loop 设计节点 | 2026-07-09 ✅ |

### 2.7 架构演进（7/7 ✅）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-AE-01 | PrimaryAgent 实现（decompose/delegate/synthesize） | 2026-07-09 ✅ |
| T-E-AE-02 | 场景化角色配置（social_media / novel） | 2026-07-09 ✅ |
| T-E-AE-03 | 自媒体写作场景端到端 | 2026-07-10 ✅ |
| T-E-AE-03b | 长篇小说写作场景端到端 | 2026-07-10 ✅ |
| T-E-AE-04 | GeneMutator 基因级进化 | 2026-07-09 ✅ |
| T-E-AE-05 | 主→子任务分派协议（AgentBus + DelegatedTask） | 2026-07-09 ✅ |
| T-E-AE-06 | 子智能体重定义（6 写作角色） | 2026-07-09 ✅ |

---

## 3. 未完成任务清单（按优先级分组）

> **🎉 全部 T-E-* 任务已于 2026-07-10 完成。以下清单保留作为历史归档。**

### 3.1 P1 优先级（14 个 → ✅ 全部完成）

| 任务 ID | 标题 | 复杂度 | 依赖 | 所属 Wave | 状态 |
|---------|------|--------|------|-----------|------|
| T-E-C-01 | OS-Controller 双模式（API+VLM） | XL | T-S6-A-01a | Wave 4 | ✅ 已完成（2026-07-10） |
| T-E-C-05 | OS-Controller Sidecar | L | T-S4-B-03 | Wave 4 | ✅ 已完成（2026-07-10） |
| T-E-C-06 | Hybrid Browser Agent | XL | 无 | Wave 4 | ✅ 已完成（2026-07-09） |
| T-E-S-10 | WorkflowCanvas 可编排画布 | XL | 无 | Wave 4 | ✅ 已完成（2026-07-09） |
| T-E-S-11 | 蜂群运行时画布 | L | T-S1-B-02 | Wave 4 | ✅ 已完成（2026-07-10） |
| T-E-S-26 | Event Stream 协议化 | L | T-S1-B-02 | Wave 4 | ✅ 已完成（2026-07-10） |
| T-E-S-53 | Cron 定时任务引擎 | L | T-S4-B-03 | Wave 5 | ✅ 已完成（2026-07-10） |
| T-E-S-58 | Calendar 组件 | M | T-E-S-02 | Wave 5 | ✅ 已完成（2026-07-09） |
| T-E-S-60 | Gateway 守护进程 | L | T-S4-B-03 | Wave 3 | ✅ 已完成（2026-07-10） |
| T-E-S-63 | 三定时机制 | L | 无 | Wave 5 | ✅ 已完成（2026-07-09） |
| T-E-AE-01 | PrimaryAgent 实现（decompose/delegate/synthesize） | L | 无 | Phase 3 | ✅ 已完成（2026-07-09） |
| T-E-AE-02 | 场景化角色配置（social_media / novel） | M | T-E-AE-01 | Phase 3 | ✅ 已完成（2026-07-09） |
| T-E-AE-05 | 主→子任务分派协议（AgentBus + DelegatedTask） | M | T-E-AE-01 | Phase 3 | ✅ 已完成（2026-07-09） |
| T-E-AE-03 | 自媒体写作场景端到端 | L | T-E-AE-01/02/05 | Phase 3 | ✅ 已完成（2026-07-10） |

### 3.2 P2 优先级（20 个 → ✅ 全部完成）

| 任务 ID | 标题 | 复杂度 | 依赖 | 状态 |
|---------|------|--------|------|------|
| T-E-B-15 | AI 自动整理 MOC | L | T-E-S-63 | ✅ 已完成（2026-07-09） |
| T-E-C-03 | UiAutomator 抽象层 | XL | 无 | ✅ 已完成（2026-07-09） |
| T-E-C-04 | ActionExecutor | L | T-E-C-03 | ✅ 已完成（2026-07-10） |
| T-E-C-11 | 操作录制回放 | M | T-E-C-04 | ✅ 已完成（2026-07-10） |
| T-E-C-15 | 语音交互引擎 | XL | 无 | ✅ 已完成（2026-07-09） |
| T-E-C-18 | OAuth 集成层（5 服务） | XL | 无 | ✅ 已完成（2026-07-09） |
| T-E-C-19 | 多端协同 | XL | 无 | ✅ 已完成（2026-07-09） |
| T-E-D-04 | 8 人格系统 | XL | T-E-D-03 | ✅ 已完成（2026-07-09） |
| T-E-D-05 | Proactive Engine | L | T-E-S-63 | ✅ 已完成（2026-07-09） |
| T-E-D-08 | WebGL 引擎复用 | XL | T-S5-B-02 | ✅ 已完成（2026-07-10） |
| T-E-D-09 | UI 性能基准 CI | M | T-E-D-08 | ✅ 已完成（2026-07-10） |
| T-E-S-12 | 节点交互 | M | T-E-S-11 | ✅ 已完成（2026-07-10） |
| T-E-S-13 | 工作流模板 | M | T-E-S-10 | ✅ 已完成（2026-07-09） |
| T-E-S-14 | 执行回放 | M | T-E-S-11 | ✅ 已完成（2026-07-10） |
| T-E-S-22 | AIO Sandbox | XL | T-S2-A-01c | ✅ 已完成（2026-07-10） |
| T-E-S-56 | Automation 模板 | S | T-E-S-53 | ✅ 已完成（2026-07-10） |
| T-E-L-04 | GitHub MCP 连接器（pull-only） | L | T-E-C-18 | ✅ 已完成（2026-07-09） |
| T-E-L-08a | Loop 运行时阶段环 | M | T-E-S-11 | ✅ 已完成（2026-07-10） |
| T-E-AE-04 | GeneMutator 基因级进化 | M | T-E-AE-01 | ✅ 已完成（2026-07-09） |
| T-E-AE-03b | 长篇小说写作场景端到端 | XL | T-E-AE-01/02/05 | ✅ 已完成（2026-07-10） |

### 3.3 P3 优先级（6 个 → ✅ 全部完成）

| 任务 ID | 标题 | 复杂度 | 依赖 | 状态 |
|---------|------|--------|------|------|
| T-E-C-07 | Remote Operator | XL | T-E-C-05 | ✅ 已完成（2026-07-10） |
| T-E-C-12 | Design Mode | L | T-E-C-04 | ✅ 已完成（2026-07-10） |
| T-E-S-06 | Organization Orchestration | XL | T-E-S-04 | ✅ 已完成（2026-07-09） |
| T-E-L-07 | Loop 审计日志 | S | T-E-L-01 | ✅ 已完成（2026-07-09） |
| T-E-L-08b | Loop 设计节点 | XL | T-E-S-10 | ✅ 已完成（2026-07-09） |
| T-E-AE-06 | 子智能体重定义（6 写作角色） | M | T-E-AE-01 | ✅ 已完成（2026-07-09） |

---

## 4. 技术债务（41 个 T-D-* 任务，40 ✅ 1 🔄 跟踪中）

> **来源**：代码质量审计（2026-07-08）+ 外部审查报告 + DEVELOPMENT_PROPOSAL_v2.0/v3.0 代码级审查。
> **任务编号**：`T-D-<领域>-<序号>`（F=前端 / B=后端 / C=CI配置 / T=测试 / S=安全配置 / O=仓库卫生）

### 4.1 P0 严重问题（13 个 → ✅ 全部完成，阻塞构建/运行/安全）

| 任务 ID | 描述 | 领域 | 复杂度 | 来源 |
|---------|------|------|--------|------|
| T-D-B-15 | **🔴 digest crate 版本冲突**（sha2 v0.11 vs hkdf v0.13）→ ✅ 已验证为误报，无需修复（4f3d9b7） | 后端 | S | v2.0 CR-01 |
| T-D-T-04 | **🔴 CI 集成测试被跳过**（25 个测试文件不执行）→ ✅ 已恢复（4f3d9b7） | 测试 | M | v2.0 CR-02 |
| T-D-B-16 | **🔴 AppState 膨胀：45+ Arc 字段巨型结构体** → ✅ 已重构为 6 个 SubSystem（memory/llm/swarm/channels/platform/infra），cargo check 通过（2026-07-08） | 后端 | L | v3.0 CR-04 |
| T-D-B-01 | tracing_setup.rs 8 路组合爆炸 → ✅ 已重构为 builder pattern（4f3d9b7） | 后端 | S | 外部审查 |
| T-D-B-06 | **lib.rs 3,333 行巨型文件**（257 个 Tauri 命令）→ ✅ 已拆分：lib.rs 160 行 + commands/ 50 个文件 + tauri_setup.rs 643 行（2026-07-08） | 后端 | L | v2.0 HI-01 |
| T-D-B-07 | **1,805 panic 点**（1,361 unwrap + 377 expect + 67 panic）→ ✅ 精确审计生产 panic 74 处(排除测试块),清理 43 处(unwrap→?/unwrap_or_default/context?),剩余 31 处均为策略#4合理保留(Lazy<Regex>/OnceLock字面量)。32 个文件处理,cargo check 通过（2026-07-09） | 后端 | L | v2.0 缺陷#1 |
| T-D-B-08 | **gRPC wire 非标准**（JSON framing shim）→ ✅ 已迁移到 tonic：删除 server.rs(1391行 JSON shim)+proto.rs(497行手写类型)+json-framing feature+tower/futures-util/futures-core 依赖；tonic_server.rs 成为唯一 wire 实现（标准 HTTP/2+protobuf，grpcurl 可直接调用）；grpc_wire_test.rs 更新为 tonic_server::generated+TonicServiceImpl+5 server trait 编译时检查；cargo check --lib/--test integration 通过（2026-07-09） | 后端 | L | v2.0 缺陷#3 |
| T-D-B-09 | **渠道路由层断路**（3 空操作适配器 + trait 缺陷 + 回信丢失） → ✅ 已修复（TelegramAdapter/DiscordAdapter/channel_router 路径） | 后端 | M | v2.0 渠道分析 |
| T-D-F-01 | tauri.ts 单文件 3190 行 → ✅ 已拆分为 4 子模块（本次） | 前端 | M | v2.0 HI-01 |
| T-D-C-01 | CI 仅 Windows → 恢复跨平台 matrix → ✅ Windows 保留（2026-07-08，待 macOS/Linux 测试就绪后启） | CI | M | v2.0 HI-04 |
| T-D-F-02 | ESLint 配置不存在 → ✅ 已新增 eslint.config.mjs（ac350d7） | 前端 | S | v2.0 MI-01 |
| T-D-S-01 | cargo audit 14 个忽略 → ✅ 已逐项评估（本次）：1 FIX（crossbeam-epoch→0.9.20），13 KEEP IGNORE | 安全 | M | v2.0 CR-03 |
| T-D-C-06 | 关键功能开关默认关闭 → ✅ 已完成决策文档（本次） | CI | S | v3.0 功能分析 |

### 4.2 P1 重要问题（26 个 → 25 ✅ 1 🔄 跟踪中）

| 任务 ID | 描述 | 领域 | 复杂度 | 来源 |
|---------|------|------|--------|------|
| T-D-B-10 | Skill 生态补齐（发现层+规范层+导入层） → ✅ 已完成（2026-07-09）：**规范层** protocol.rs 扩展 SkillManifest(author/source/status/dependencies/eligibility/min_nebula_version 字段,serde default 向后兼容) + SkillEligibility(bins/env/config/os 4 维资格) + SkillSpecValidator(validate_skill_md/parse_manifest/check_eligibility) + SkillSpecReport + is_semver/find_in_path 辅助;**发现层** discover.rs 新增 DiscoveryResult/DiscoveryStatus(serde snake_case) + SkillDiscoverer::with_extra_paths() builder + discover_with_details() 返回 Vec<DiscoveryResult> + scan_directory_with_details();**导入层** importer.rs 新增 with_hub_client(Option<TeamSkillsHubClient>) builder + import_from_teamskillshub 不再返回 stub 错误(注入 client 后通过 hub API 拉取+解析+入库);**Tauri 命令层** skill.rs 新增 4 命令(skill_discover/skill_scan_paths/skill_validate_md/skill_import_teamskillshub) + 更新 skill_import 注入 hub client + tauri_setup.rs 注册 4 新命令;**bootstrap 接线** swarm.rs 读取 NEBULA_TEAM_SKILLS_HUB_URL 注入 TeamSkillsHubClient;**测试** protocol.rs(11)+discover.rs(5)+importer.rs(3) 新增测试;**附带修复** writing/templates.rs+scenarios.rs 4 处预存 E0716 临时值生命周期错误(temporary dropped while borrowed → into_iter+owned String);cargo check + cargo check --tests 退出码 0 | 后端 | M | v2.0 Skill 分析 |
| T-D-B-11 | EvolutionEngine 断路（evolution_run 未实现） → ✅ 已完成（2026-07-09）：**调查发现** evolution_run Tauri 命令(commands/evolution.rs)与 EvolutionEngine::run() 4 Phase 管线(Extract L1→L2 / Compile L2→L3 / Reflect L2+L3→L5 / Soul L5→SOUL.md)均已完整实现,真正断路点是 **cron_scheduler.rs::execute_evolution_self_check()** 为 stub(从不调用引擎);**修复** 将 stub 替换为实际调用 `engine.run(&self.master_id).await`,闭合 12:00 自检的"评估→变异→选择"回路;**CronScheduler** 新增 `evolution_engine: Option<Arc<EvolutionEngine>>` 字段(#[cfg(feature="evolution-engine")] 编译期门控,imply self-evolution+unified-dispatcher)+ `master_id: String` 字段(domain 隔离,写入 `evolution:<master_id>` 域,默认=user_id)+ `with_evolution_engine(Option<Arc<EvolutionEngine>>)` / `with_master_id(String)` builder(向后兼容);**行为分支** feature off→info 跳过 / 引擎未注入→warning 跳过 / 运行时禁用(is_enabled()==false)→info 跳过 / 启用→run() 成功记 info(含 degraded/warnings/phases 计数) 失败记 warning 返回 Err(EvolutionError::Disabled 竞态视为跳过);**测试** 6 个新增(master_id 默认值=user_id / with_master_id 覆盖+user_id 不受影响 / 无引擎优雅跳过[2 feature 配置均通过] / execute_task 分发到 evolution-self-check / with_evolution_engine(None) / 引擎禁用跳过);**验证** cargo check --lib(默认 features)退出码 0;cargo check --lib --features evolution-engine 退出码 0(仅 2 个预存 warning);cargo check --tests --features evolution-engine 失败但全部是预存错误(e2ee.rs unclosed delimiter / cron_scheduler.rs:704 chrono API / sync/mod.rs e2ee imports,非 T-D-B-11 引入) | 后端 | L | v2.0 学习循环 |
| T-D-B-12 | 无自托管 Web 静态服务 → ✅ 已完成:WebStaticServer(SPA fallback+MIME+缓存+路径遍历防护+7测试) 已集成到 rest.rs 路由(GET 非 /api/* → try_serve,API 路径自动放行);NEBULA_WEB_DIST 环境变量控制 dist 路径;cargo check --features rest-api 通过（2026-07-09） | 后端 | S | v2.0 部署 |
| T-D-B-13 | 无系统服务注册（systemd/launchd/Windows Service） → ✅ 已完成（2026-07-09）：**api/daemon.rs**(完全重写 ~1517 行)统一抽象 `DaemonConfig`(name/exec_path/args/restart_policy/working_dir/description/scope) + `ServiceScope`(User/System,snake_case serde) + `RestartPolicy`(Always/OnFailure/Never,含 `systemd_value()`/`windows_start()` 映射) + `ServiceState`(Running/Stopped/NotInstalled/Unknown) + `DaemonStatus`;**配置渲染纯函数**(跨平台可单元测试,无 cfg 门控) `render_systemd_unit`([Unit]/[Service]/[Install] 段 + Restart= 策略 + WorkingDirectory) + `render_launchd_plist`(Label/ProgramArguments/KeepAlive/RunAtLoad/XML 转义) + `render_windows_service_create_args`(sc.exe create binPath=/start=/DisplayName= 键值对) + `render_windows_service_failure_args`;**路径计算** `systemd_unit_path`/`launchd_plist_path`/`launchd_label`/`dirs_config_home`/`dirs_home`;**平台分发方法** `DaemonConfig::install/uninstall/start/stop/status`(每方法 4 个 cfg 分支:linux systemd systemctl --user/system + daemon-reload/enable/disable/stop/start/is-active;macos launchd launchctl load/unload/start/stop/list;windows sc.exe create/delete/start/stop/query/description/failure;unsupported 返回明确错误);**服务入口点** `run_as_service()` 公共入口 + `run_as_service_unix()` + `run_as_service_windows()` + `run_service_loop()` async(复用 bootstrap_headless + 启动 grpc/rest + 等待停止信号);**Windows Service FFI** `win_service` 内部模块(仅 Windows 编译):`SERVICE_STOP_SIGNAL`(OnceLock<Mutex<bool>>,async wait() 轮询 + signal() 触发) + `STATUS_HANDLE` 全局 SCM 句柄 + `report_status()`(START_PENDING/RUNNING/STOP_PENDING/STOPPED) + `service_handler` extern "system" HandlerEx 回调 + `service_main` extern "system" ServiceMain 入口点 + `dispatch()` 调用 StartServiceCtrlDispatcherW;**Cargo.toml** 添加 `Win32_System_Services` feature 到 windows-sys 依赖;**main.rs** 添加 `--run-as-service` 隐藏 flag(优先于其他子命令处理)+ `Daemon` 子命令(Install/Uninstall/Start/Stop/Status 5 动作,每动作带 name/scope/restart/description 参数)+ `run_daemon()` 分发函数 + `daemon_config_for()` 辅助;**commands/daemon.rs**(新建)3 个 Tauri 命令 `daemon_status`(只读查询)/`daemon_install`(写配置+enable/load)/`daemon_uninstall`(停止+disable+删配置) + DaemonStatusRequest/DaemonInstallRequest DTO + parse_scope/parse_restart/build_config 辅助;**commands/mod.rs** 注册 `pub mod daemon` + `pub use daemon::*`;**tauri_setup.rs** `generate_handler!` 注册 3 新命令;**测试** 24 个单元测试(配置渲染:systemd unit 含 Restart=always、launchd plist XML 转义、sc.exe 参数键值对;路径计算:systemd_unit_path 含 .service 后缀、launchd_label 反转域名;shell_quote 单引号转义;xml_escape &<> 字符;for_current_exe/effective_working_dir/command_line 逻辑);**验证** cargo check --lib 退出码 0;cargo check --tests 因 src/sync/e2ee.rs 被外部进程并发修改导致瞬态失败(错误位置在 367/719 之间漂移,非 T-D-B-13 代码问题,读取文件时内容有效);**跨平台** 代码支持 linux/macos/windows 三平台,CI 目前仅构建 windows-x86_64 但 cfg 门控确保其他平台可编译 | 后端 | M | v2.0 部署 |
| T-D-B-14 | Sidecar 3/5 服务骨架化 → ✅ 已完成（2026-07-09）：**调查** 6 个 SidecarKind(Memory/Swarm/Llm/Skill/Reflection/OsController)中 Memory/Llm/Swarm 3 个无 handler 文件(任务"3/5"指原计划 5 个 sidecar 中 3 个未实现);**新增 3 个 sidecar 服务处理器**(延续 T-S4-B-01/T-S4-B-02/T-S6-A-01a 单二进制多角色方案):①**memory_service.rs**(~324 行) MemoryServiceHandler 包装 SqliteStore + 可选 VectorStore/Embedder,5 RPC(health_check/store_memory/get_memory/search_memory/list_recent),builder 模式 with_vector_store() 注入向量检索,9 个测试;②**llm_service.rs**(~193 行) LlmServiceHandler 包装 LlmGateway + 可选 Embedder,3 RPC(health_check/chat/embed),builder 模式 with_embedder(),5 个测试;③**swarm_service.rs**(~196 行) SwarmServiceHandler 包装 SwarmOrchestrator,4 RPC(health_check/execute/list_agents/get_agent),7 个测试;**mod.rs** 注册 3 新模块 + pub use 导出 3 个 handler(从 7 模块扩展到 10 模块);**架构** 所有 handler 遵循现有 skill/reflection/os_controller 模板:new(engine) 构造 + engine() 访问器 + Debug impl + builder 注入可选依赖 + #[instrument] 日志 + anyhow 错误处理;**验证** cargo check --lib 退出码 0;cargo check --tests 退出码 0(仅 2 个预存 warning);cargo test --lib sidecar 47 passed 0 failed(含 21 个新测试);**注意** 验证期间临时恢复 e2ee.rs 原始版本(并发任务 T-D-S-02 的中间态被截断导致 unclosed delimiter,已恢复为可编译的 committed 版本) | 后端 | M | v2.0 部署 |
| T-D-B-17 | **AgentKind Deprecated 但仍大量使用** → 统一为 Generic + 场景化 → ✅ 已完成:为 5 个废弃变体(Coder/Writer/Reviewer/Researcher/Planner)添加 `#[deprecated]` 属性 + 新增 `AgentScenario` 枚举(Coding/Writing/Review/Research/Planning,snake_case serde) + `AgentKind::to_scenario()` 桥接方法 + `AgentOutput.scenario: Option<AgentScenario>` 字段(serde default 向后兼容) + 角色 agent(coder/writer/planner/researcher/reviewer)实现 `#[allow(deprecated)]` 并在 run() 末尾打 `.with_scenario()` 标签 + gRPC `swarm_agent_kind_to_prost` 函数放行 + orchestrator/negotiator 字面量补 `scenario: None` + 测试模块统一 `#[allow(deprecated)]`;cargo check + cargo check --tests 退出码 0;cargo test --lib swarm::agents(67)+swarm::events(14)+scenarios(10) 全过（2026-07-09） | 后端 | M | v3.0 HI-09 |
| T-D-B-18 | **writing/ 模块过于单薄（2 文件）** → 补齐 28 场景模板 → ✅ 已完成（2026-07-09）：**templates.rs** 扩展 WritingScenarioCategory(General/SelfMedia/Novel,snake_case serde+中文别名 parse) + WritingStyleParams(tone/length/audience/perspective/extras BTreeMap+is_empty) + WritingTemplate 新增 4 字段(category/prompt_template/output_format/style_params,非 Option+Default,serde default 向后兼容) + 6 通用模板补默认字段 + library() 合并 28 场景;**scenarios.rs**(新建 ~1490 行) 14 自媒体场景(wechat_article/xiaohongshu_note/douyin_script/zhihu_answer/weibo_post/bilibili_script/toutiao_article/douban_review/tieba_post/jike_moment/wechat_channels/wechat_longform/xiaohongshu_image_text/zhihu_column) + 14 长篇小说场景(novel_xuanhuan/urban/scifi/history/romance/mystery/wuxia/military/game/apocalypse/fanfic/light/fairy_tale/jubensha) + 公共 API(self_media_library/novel_library/scenario_library/writing_scenario_template_ids 返回 28 稳定 ID) + 每模板含中文 prompt_template/output_format/style_params;**mod.rs** 新增 RenderedScenarioTemplate 结构体 + WritingEngine 4 新方法(list_templates_by_category/writing_scenario_templates/writing_scenario_template_ids/apply_scenario_template) + render_placeholders 辅助;**commands/writing.rs** 新增 3 Tauri 命令(writing_list_templates_by_category/writing_list_scenarios/writing_apply_scenario_template) + ApplyScenarioRequest;**tauri_setup.rs** 注册 3 新命令;**与 T-D-B-17 集成** writing_scenario_template_ids() 暴露 28 稳定 ID 供 swarm 层按 AgentScenario::Writing 注入;**附带修复** skills/protocol.rs:358 预存借用错误(闭包返回对已 drop 临时 Value 引用 → if let 绑定 yaml_val);cargo check --lib 退出码 0 + cargo check --tests 退出码 0 + cargo test --lib writing:: 25 测试全过(0 失败) | 后端 | L | v3.0 HI-10 |
| T-D-B-19 | **swarm/agents 6 角色偏编程场景** → 重定义为写作场景角色 → ✅ 已完成（2026-07-09）：**agents/mod.rs** 新增 `WritingScenarioProfile` 结构体(system_prompt/tool_set/knowledge_scope/context_label/role_name_zh 5 字段) + `writing_role_profile()` 函数(OnceLock 缓存 HashMap,6 角色写作场景配置:master=主编/coder=排版格式化/writer=主笔/reviewer=校对编辑/researcher=素材收集/planner=大纲规划,每角色含中文 system_prompt + 场景化工具集 + 记忆层级边界 + context_label + 角色中文名);**5 个角色 agent**(coder.rs/writer.rs/reviewer.rs/researcher.rs/planner.rs)添加 `with_scenario(AgentScenario) -> Self` builder + `current_scenario()` 诊断方法 + 4 个 `effective_*` 场景感知方法(effective_system_prompt/effective_tool_set/effective_knowledge_scope/effective_context_label),Writing 场景从 `writing_role_profile()` 取配置,Coding 场景保持原编程行为(向后兼容);**master.rs**(MasterOrchestrator)新增 `scenario: Option<AgentScenario>` 字段 + `with_scenario()` builder + `effective_decompose_prompt()`/`effective_synthesize_prompt()` 方法 + 2 个写作场景提示词常量(`WRITING_DECOMPOSE_SYSTEM_PROMPT` 含"主编"角色定位+5 节点写作流程:大纲规划→素材收集→正文撰写→校对编辑→排版格式化;`WRITING_SYNTHESIZE_SYSTEM_PROMPT` 综合产出"最终手稿");**测试** agents/mod.rs(16:6 角色配置完备性+5 字段非空+中文名映射+context_label 唯一性+reviewer 只读+writer 有 editor_write+master 有 memory_search+coder formatter 提示词+AgentOutput scenario 序列化往返+AgentScenario snake_case+FromStr 解析+to_scenario 桥接) + coder.rs(6) + writer.rs(5) + reviewer.rs(6) + researcher.rs(5) + planner.rs(5) + master.rs(8:默认 None+设置场景+写作 decompose 提示词含"主编"+写作 synthesize 提示词含"手稿"+编程 decompose 不含"主编"+两场景提示词差异+builder 链式),共 51 个新增测试;**验证** cargo check --lib 退出码 0;cargo check --lib --features master-orchestrator 退出码 0;cargo test --lib swarm::agents::tests 34 passed;cargo test --lib writing_scenario 17 passed;cargo test --lib --features master-orchestrator writing_scenario 20 passed;cargo test --lib --features master-orchestrator master_ 15 passed;**向后兼容** AgentKind 5 个 #[deprecated] 变体保留,旧 API/gRPC/scenarios.json 不受影响,角色 agent 默认 scenario=None 保持编程场景行为 | 后端 | M | v3.0 HI-11 |
| T-D-B-02 | bootstrap.rs 1113 行单函数 → ✅ 已拆分为 5 子模块（core/storage/ai_core/swarm/platform），主文件 113 行，cargo check 通过（2026-07-08） | 后端 | L | v2.0 HI-02 |
| T-D-B-03 | std::mem::forget(h) 泄露 JoinHandle → ✅ 已修复:①PerfMonitor handle 存入 InfraSubsystem ②tauri_setup.rs Prometheus JoinHandle: forget→drop(detach) ③otel.rs TracerProvider: forget→OnceLock(aa2d535)（2026-07-08） | 后端 | S | v2.0 MI-08 |
| T-D-B-04 | memory/ 40+ 子文件平铺 → 按职能分组 → ✅ 已完成:29 文件分入 6 子目录(storage/engines/search/embedding/graph/io),根目录保留 types/acl/layers;pub use 重导出 + 子目录 mod.rs use super::X 跨组引用;include_str! 路径适配;cargo check + cargo test --lib 1550 passed（2026-07-09） | 后端 | M | v2.0 HI-06 |
| T-D-B-05 | features 死 feature 清理 → ✅ 已验证:当前 21 个 feature 均有 cfg 引用;did-identity/crdt-sync 已在 T-S2-C-01 移除（2026-07-08） | 后端 | S | v2.0 MI-09 |
| T-D-F-03 | 重复代码提取（renderMarkdown / downloadBlob） → ✅ 已提取 renderMarkdown 到 utils/markdown.ts(44c5cbf);downloadBlob 已只有 1 处无需提取（2026-07-08） | 前端 | S | 外部审查 |
| T-D-F-04 | 硬编码中文字符串 → 迁移到 i18n key → ✅ 已完成(200+处迁移):export.ts(7)+App.tsx(5)+TimelineView.tsx(12)+MemoryMap.tsx(9)+InboxView.tsx(2)+FileTree.tsx(2)+ChatPanel.tsx(40+)+CreditsDashboard.tsx(8)+SwarmView.tsx(1)+LongTaskPanel.tsx(30+)+ShadowWorkspacePanel.tsx(35+) 硬编码迁移;Modal.tsx(2)+Spinner.tsx(1)+Settings.tsx(43) `\|\| '中文'` 死代码清理;100+ 新 i18n key;autonomy.ts label_zh/description_zh 为后端 wire 格式不迁移;tsc+140 测试全过（2026-07-09） | 前端 | M | v2.0 MI-06 |
| T-D-F-05 | i18n 类型不安全 → ✅ 已去除 as unknown as Dict 双重断言,zh-CN.json 编译时类型检查(9bea48d),两 JSON 888 键一致（2026-07-08） | 前端 | S | 外部审查 |
| T-D-F-06 | cancelled 布尔反模式 → ✅ 已全部替换为 AbortController(6 个组件 14 处 useEffect),tsc 通过（2026-07-08） | 前端 | S | v2.0 MI-07 |
| T-D-T-01 | vitest 覆盖率阈值过低（30%/20%/25%/30%）→ ✅ 已提升至 40/30/55/40,新增 33 个测试(export.ts 12 + charts 13 + ToolCallCard 8),实测覆盖率 Stmts 40.26% / Branch 61.84% / Funcs 30.62% / Lines 40.26%,140 测试全过（2026-07-09） | 测试 | M | 外部审查 |
| T-D-T-02 | 核心文件零测试（bootstrap/gateway/dispatcher/app_config）→ ✅ 已补齐:app_config.rs(9 测试,opencode) + bootstrap/platform.rs(7 测试: editor/clipboard/sync fallback + message_bridge/channel_router) + bootstrap/storage.rs(2 测试: Lance 后端 + startup timer) + bootstrap/ai_core.rs(4 测试: load_acl_from_store valid/invalid/empty)。gateway.rs 和 dispatcher.rs 已有测试。swarm.rs/core.rs 依赖过重留待后续。共 22 个新测试全过（2026-07-09） | 测试 | L | v2.0 HI-07 |
| T-D-T-03 | E2E 测试接入 CI（Playwright）→ ✅ 已接入:web-test job 新增 build + playwright install + test + report upload 4 步骤（2026-07-09） | 测试 | M | 外部审查 |
| T-D-C-02 | Vite/Vitest 配置重复 → ✅ 已合并为单一 vite.config.ts(4888759),删除 vite.config.mjs + vitest.config.ts,修复缺失的 resolve.alias（2026-07-08） | CI | S | v2.0 MI-04 |
| T-D-C-03 | Prettier 配置不存在 → ✅ 已新增 .prettierrc（ac350d7） | CI | S | v2.0 MI-02 |
| T-D-C-04 | tsconfig 禁用 noUnusedLocals/Parameters → ✅ 已启用严格模式（ac350d7） | CI | S | v2.0 MI-03 |
| T-D-C-05 | Dockerfile 缺 HEALTHCHECK/非 root/多架构 → ✅ 已修复:①HEALTHCHECK 探测 /api/health(curl) ②非 root 用户 nebula(UID/GID 1001) ③多架构支持(buildx+QEMU,linux/amd64+arm64) ④Rust 1.77→1-bookworm ⑤docker-compose 添加 NEBULA_*_ADDR=0.0.0.0 + healthcheck 配置（2026-07-09） | CI | M | v2.0 HI-08 |
| T-D-C-07 | incremental=false（Rust ICE 规避）→ 🔄 跟踪中:Rust 1.96.1 (31fca3adb 2026-06-26) ICE 未修复,保持 incremental=false;待 Rust 版本更新后验证恢复（2026-07-08） | CI | S | v2.0 HI-05 |
| T-D-C-08 | **master-orchestrator 无运行时开关** → ✅ 新增 AtomicBool + 命令级守卫（2026-07-08） | CI | S | v3.0 HI-12 |
| T-D-S-02 | **E2EE 单棘轮无前向保密** → ✅ 升级为双棘轮(Double Ratchet):DH 棘轮(每条消息新 DH 密钥对)+ KDF 链棘轮(HKDF-SHA256 链式派生),实现前向保密;v1/v2 信封向后兼容;15 个测试覆盖(双向对话/DH 公钥更新/消息密钥唯一性/前向保密 2 场景/v1 向后兼容/wire format/失败状态保持/版本常量)（2026-07-09） | 安全 | M | v3.0 MI-12 |

### 4.3 P1 仓库卫生（2 个 → ✅ 全部完成）

| 任务 ID | 描述 | 领域 | 复杂度 | 来源 | 状态 |
|---------|------|------|--------|------|------|
| T-D-O-01 | IMPROVEMENT_PLAN_v1.0.md 过时文件清理 | 仓库卫生 | S | 外部审查 | ✅ 240f30b |
| T-D-O-02 | **ARCHITECTURE.md 品牌残留与数字过时** → 统一为 Nebula | 仓库卫生 | S | v3.0 HI-13 | ✅ 240f30b |

### 4.4 技术债务推进原则

1. **不阻塞功能开发**：与功能任务并行，P0 债务优先在 Phase 间隙处理
2. **分批消化**：每个 Phase 结束后评估债务状态
3. **测试先行**：拆分重构类任务（如 T-D-B-02/06）必须先补测试
4. **安全优先**：T-D-S-02 和 T-D-B-03 涉及安全，优先处理
5. **仓库卫生**：T-D-O-01/02 与 T-D-F-02（ESLint）已完成 ✅

---

## 5. 外部审查与代码级审计交叉验证（2026-07-08）

> **来源**：外部智能体审查报告（`D:\tmp\ROADMAP_REVIEW.md`）+ `DEVELOPMENT_PROPOSAL_v2.0.md` + `DEVELOPMENT_PROPOSAL_v3.0.md`

### 5.1 审查结论

- **项目完成度**：~70%（v3.1 校准后，含 AE 系列）
- **代码质量**：`cargo check` 9 警告（4 可自动修复），`npm run typecheck` 0 错误，107 前端测试全绿
- **核心瓶颈**：
  ① 工作流可视化全缺（T-E-S-10~14）
  ② 后端核心文件零测试（bootstrap/gateway/dispatcher）
  ③ 技术债务 41 项零处理
  ④ 架构演进缺口（无主智能体概念 + Agent 角色硬编码 + writing/ 单薄）

### 5.2 v3.1 相比 v2.3 的新增修正

| 问题 | 来源 | 修正 |
|------|------|------|
| AppState 45+ 字段膨胀 | v3.0 CR-04 | ✅ 新增 T-D-B-16（P0） |
| AgentKind Deprecated 矛盾 | v3.0 HI-09 | ✅ 新增 T-D-B-17（P1） |
| writing/ 仅 2 文件 | v3.0 HI-10 | ✅ 新增 T-D-B-18（P1） |
| swarm/agents 偏编程场景 | v3.0 HI-11 | ✅ 新增 T-D-B-19（P1） |
| master-orchestrator 无运行时开关 | v3.0 HI-12 | ✅ 新增 T-D-C-08（P1） |
| ARCHITECTURE.md 品牌残留 | v3.0 HI-13 | ✅ 新增 T-D-O-02（P1） |
| E2EE 单棘轮无前向保密 | v3.0 MI-12 | ✅ 新增 T-D-S-02（P1） |
| 无主智能体概念 | v3.0 AE-01 | ✅ 新增 T-E-AE-01（P1） |
| 无场景化角色配置 | v3.0 AE-02 | ✅ 新增 T-E-AE-02（P1） |
| 无主→子分派协议 | v3.0 AE-05 | ✅ 新增 T-E-AE-05（P1） |
| 无基因级进化 | v3.0 AE-04 | ✅ 新增 T-E-AE-04（P2） |
| 无自媒体写作场景 | v3.0 AE-03 | ✅ 新增 T-E-AE-03（P1） |
| 无长篇小说写作场景 | v3.0 AE-03b | ✅ 新增 T-E-AE-03b（P2） |

---

## 6. 推进节奏（Phase 0-4）

### 6.1 立即收尾（本周内）

1. ✅ **T-E-L-05 Loop 模板库** — 已完成（3 commits 已落地 `6396a0b` / `3a2b58c` / `ad10a6a`）
2. ✅ **T-E-L-06 Loop 预算管理 + 安全防护** — 已完成（`18cf45e`）
3. ✅ **关闭 finish-te-l-03-checker-agent spec** — T-E-L-03 已完成（2026-07-08）
4. ✅ **T-D-B-15 digest crate 版本冲突** — 已验证为误报，无需修复（`4f3d9b7`）
5. ✅ **T-D-T-04 CI 集成测试恢复** — 已恢复（`4f3d9b7`）

### 6.2 Phase 0：地基修复（2-3 周）

> **对齐 DEVELOPMENT_PROPOSAL_v3.0 Phase 0**：功能任务暂停，全力做债务清理。

**W1 - 构建阻塞修复**：
- ✅ T-D-B-15 digest crate 版本冲突（已验证为误报，4f3d9b7）
- ✅ T-D-T-04 CI 集成测试恢复（4f3d9b7）

**W2-3 - 严重质量修复**：
- ✅ T-D-B-16 AppState 45+ 字段分组重构（L，v3.0 新增）
- ✅ T-D-B-07 1805 panic 点 → 精确审计生产 74 处,清理 43 处,剩余 31 处策略#4保留（2026-07-09）
- ✅ T-D-B-06 lib.rs 3333 行拆分（L）
- ✅ T-D-B-02 bootstrap.rs 1113 行拆分（L，5 子模块，2026-07-08）
- ✅ T-D-F-01 tauri.ts 拆分（本次）
- ✅ T-D-B-01 tracing_setup.rs 8 路重构（4f3d9b7）
- ✅ T-D-B-08 gRPC wire 标准化（L，迁移到 tonic，2026-07-09）
- ✅ T-D-B-09 渠道路由断路修复（M）

**W4 - CI 与配置**：
- ✅ T-D-F-02 ESLint 配置（ac350d7）
- ✅ T-D-S-01 cargo audit 14 项评估（本次）
- ✅ T-D-C-01 CI 跨平台决策（M，暂保 Windows 单平台）
- ✅ T-D-C-06 功能开关默认开启决策（本次）
- ✅ T-D-C-08 master-orchestrator 运行时开关（S，v3.0 新增）
- ✅ T-D-T-01 vitest 覆盖率提升至 40/30/55/40（2026-07-09）
- ✅ T-D-O-01/02 过时文档清理 + ARCHITECTURE.md 更新（240f30b）

### 6.3 Phase 1：质量闭环 + 架构准备（4-6 周）

**核心文件测试补齐**：
- ✅ T-D-T-02 bootstrap/gateway/dispatcher/app_config 测试（L,22 个新测试全过）

**前端质量重构**：
- ✅ T-D-F-03/04/05/06 重复代码 + i18n + cancelled 反模式（2026-07-09）

**架构准备（为 Phase 3 铺路）**：
- ✅ T-D-B-17 AgentKind Deprecated 清理（M,Generic + AgentScenario 场景化,2026-07-09）
- ✅ T-D-B-18 writing/ 模块补齐 28 场景模板（L，2026-07-09）
- ✅ T-D-B-19 swarm/agents 角色重定义（M，2026-07-09）
- ✅ T-D-S-02 E2EE 双棘轮升级（M，Double Ratchet DH+KDF 链棘轮，2026-07-09）

**CI/CD 门禁强化**：
- ✅ T-D-C-02/03/04/05 配置统一 + Prettier + tsconfig + Docker（2026-07-09）
- 🔄 T-D-C-07 增量编译（跟踪中,依赖 Rust 上游 ICE 修复）
- ✅ T-D-T-03 Playwright E2E 接入 CI（2026-07-09）

### 6.4 Phase 2：功能补齐（6-8 周，按 Wave 推进）→ ✅ 全部完成

**Wave 2（v2.4 知识革命收尾）**：
- ✅ T-E-S-63 三定时机制 → T-E-B-15 AI 自动整理 MOC

**Wave 3（v2.5 形象+后台革命）**：
- ✅ T-E-S-60 Gateway 守护进程
- ✅ T-E-D-04 8 人格系统
- ✅ T-E-D-05 Proactive Engine

**Wave 4（v2.6 可视+视觉革命）**：
- ✅ T-E-S-10 WorkflowCanvas（XL，最大工程量）
- ✅ T-E-S-11 蜂群运行时画布
- ✅ T-E-S-26 Event Stream 协议化
- ✅ T-E-C-01/05/06 OS-Controller 三件套
- ✅ T-E-L-08a Loop 运行时阶段环

**Wave 5（v3.0 全自主革命）**：
- ✅ T-E-S-53 Cron 定时任务引擎
- ✅ T-E-S-58 Calendar 组件
- ✅ T-E-C-18 OAuth 集成层 → T-E-L-04 GitHub MCP
- ✅ T-E-C-19 多端协同

### 6.5 Phase 3：蜂群架构升级（4-6 周）← v3.1 新增 → ✅ 全部完成

> **目标**：将扁平蜂群升级为"2 自定义主智能体 + 6 通用子智能体"分层架构，支持场景化写作工作流。

**核心架构**：
- ✅ T-E-AE-01 PrimaryAgent 实现（decompose/delegate/synthesize）
- ✅ T-E-AE-02 场景化角色配置（social_media / novel）
- ✅ T-E-AE-05 主→子任务分派协议（AgentBus + DelegatedTask）

**基因级进化**：
- ✅ T-E-AE-04 GeneMutator（基于 OutcomeLedger 适应度信号，受控变异 ±2.5%）

**写作场景端到端**：
- ✅ T-E-AE-03 自媒体写作场景（搜索→大纲→初稿→审查→润色→归档）
- ✅ T-E-AE-03b 长篇小说写作场景（世界观+人物→章节大纲→并行初稿→一致性审查→润色）

**子智能体重定义**：
- ✅ T-E-AE-06 6 子智能体重定义为 Search/Outline/Draft/Review/Polish/Archive

### 6.6 Phase 4：创新扩展（长期）

- L6-L7 记忆层实现（知识蒸馏 + 不变记忆）
- AIO Sandbox 完整实现（bwrap/seatbelt/MIC 三平台隔离）
- 进化日志 + 段落级回滚全面集成

---

## 7. License 合规矩阵

| 对标项目 | License | 与 Nebula(MIT) 兼容 | 借鉴边界 |
|---------|---------|------------------------|---------|
| OpenClaw | MIT | ✅ | 可代码级借鉴 |
| Hermes | MIT | ✅ | 可代码级借鉴 |
| Open WebUI | MIT | ✅ | 可代码级借鉴 |
| Dify | Apache 2.0 | ✅ | 可代码级借鉴（保留 NOTICE） |
| UI-TARS-desktop | Apache 2.0 | ✅ | 可代码级借鉴（保留 NOTICE） |
| OpenHuman | MIT | ✅ | 可代码级借鉴 |
| Reasonix | 未明确 | ⚠️ | 思路借鉴，需核实 |
| CoPaw | 未明确 | ⚠️ | 思路借鉴，需核实 |
| OpenAkita | **AGPL-3.0** | ❌ | **仅思路借鉴，不可代码 fork** |
| Obsidian Skills | GPL-3.0 | ⚠️ | 思路借鉴，不可代码 fork |
| LLM Wiki 理念 | 公开理念 | ✅ | 自由借鉴 |

---

## 8. 附录

### 8.1 来源标记说明

- **来源 A**：报告 A（`EXPERT_REVIEW_v3.0_INNOVATION.md`，7 专家 + 大厂趋势）
- **来源 B**：报告 B（GLM-5.2 对话分析，OpenAkita 校准 + UI-TARS/CoPaw/LLM Wiki 深度对标）
- **来源 A+B**：双报告共同提出，互补合并
- **来源 Loop 内化**：Loop Engineering 公开资料内化（`docs/skills/loop-engineering/`）
- **来源 v2.0 审查**：`DEVELOPMENT_PROPOSAL_v2.0.md` 代码级审查
- **来源 v3.0 审查**：`DEVELOPMENT_PROPOSAL_v3.0.md` 代码级审查 + 架构演进

### 8.2 配套文档

- `docs/ROADMAP_v2.1.md`（Stage 1-6 工程闭环，已全部完成）
- `docs/ROADMAP_v2.2.md`（已完成任务完整 commit 描述归档）
- `docs/ROADMAP_v2.3.md`（v3.1 前身，首次精简版）
- `docs/DEVELOPMENT_PROPOSAL_v2.0.md`（v2.0 基线审计）
- `docs/DEVELOPMENT_PROPOSAL_v3.0.md`（v3.0 代码级审计 + 架构演进，v3.1 任务主要来源）
- `docs/COMPREHENSIVE_EVOLUTION_v3.0.md`（创新审议综合报告）
- `docs/skills/loop-engineering/NEBULA_LOOP_DESIGN.md`（Loop Engineering 设计权威）

### 8.3 依赖关系速查

- Stage 1-6（v2.1）已全部完成 ✅，是 Stage 7 的基础
- T-E-* 全部 131 个任务已 ✅ 完成，以下依赖关系均已满足：
- **注**: v2.3 曾引入 T-E-S-47(Skill hot-reload),v3.1 继承 v2.2 编号体系未包含此任务。代码库中无实现,若需此功能应作为新版本任务重新规划。
- T-E-L-08a 依赖 T-E-S-11（蜂群运行时画布，✅ 已完成）
- T-E-L-08b 依赖 T-E-S-10（WorkflowCanvas，✅ 已完成）
- T-E-B-15 依赖 T-E-S-63（三定时机制，✅ 已完成）
- T-E-D-05 依赖 T-E-S-63（同上，✅ 已完成）
- T-E-L-04 依赖 T-E-C-18（OAuth 集成层，✅ 已完成）
- T-E-AE-02/03/05 依赖 T-E-AE-01（PrimaryAgent，✅ 已完成）
- T-E-AE-03b 依赖 T-E-AE-01/02/05（✅ 均已完成）
- T-E-AE-06 依赖 T-E-AE-01（✅ 已完成）

### 8.4 版本发布计划

> 注：本文档版本 v3.1 为**路线图版本**（整合 v3.0 建议书 + 外部审查），并非产品发布版本。产品发布版本线如下：

| 版本 | 内容 | 预估 |
|------|------|------|
| v2.0.2 | Phase 0 地基修复 | W3 |
| v2.1.0 | Phase 1 质量闭环 + 架构准备 | W9 |
| v2.2.0 | Phase 2 Wave 3-4 | W15 |
| v2.3.0 | Phase 2 Wave 5 + Phase 3 蜂群架构 | W21 |
| v3.0.0 | Phase 4 全自主版本 | W26+ |

### 8.5 文档变更记录

| 日期 | 变更 | 涉及 § |
|------|------|--------|
| 2026-07-08 | 初始创建 | 全部 |
| 2026-07-08 | T-D-* 状态修正：8 个已提交（T-D-B-01/15, T-D-T-04, T-D-F-02, T-D-C-03/04, T-D-O-01/02）+ 3 个本次完成（T-D-F-01, T-D-C-06, T-D-S-01）→ 11/41 ✅；T-E-L-05/06 标记 ✅ | §1, §2.6, §3.2, §4, §6.1, §6.2 |
| 2026-07-08 | 补充 T-D-S-01 评估结论：crossbeam-epoch 可 `cargo update --precise 0.9.20` 修复，其余 13 个 KEEP IGNORE | §4.1 |
| 2026-07-08 | 新增 T-D-C-08 AtomicBool 运行时开关（swarm/mod.rs, bootstrap.rs, commands/master.rs, tauri_setup.rs, nebula.ts）+ 命令级守卫 → ✅ | §4.2, §6.2 |
| 2026-07-08 | T-D-B-09 渠道路由修复：TelegramAdapter/DiscordAdapter/channel_router 路径 → ✅ | §4.1, §6.2 |
| 2026-07-08 | T-D-C-01 CI 跨平台 matrix 决策：保留 Windows-only，保留 if/else 骨架 → ✅ | §4.1, §6.2 |
| 2026-07-08 | T-D-B-16 AppState 67 字段 → 6 个 SubSystem（memory/llm/swarm/channels/platform/infra），cargo check 通过（0 errors）→ ✅ | §4.1, §6.2 |
| 2026-07-09 | T-D-B-07 精确审计生产 panic 74 处,6 个 batch 清理 43 处,剩余 31 处策略#4保留(Lazy<Regex>/OnceLock字面量);cargo check 通过 → ✅ | §4.1, §6.2 |
| 2026-07-09 | T-D-S-02 E2EE 单棘轮→双棘轮(Double Ratchet)升级:DH 棘轮+KDF 链棘轮实现前向保密;v1/v2 信封向后兼容;新增 RatchetState+Mutex+trial clone 模式;15 个测试覆盖(双向对话/DH 公钥更新/消息密钥唯一性/前向保密 2 场景/v1 向后兼容/wire format/失败状态保持/版本常量);cargo check + cargo check --tests 通过 → ✅ | §4.1, §6.3 |
| 2026-07-10 | **🎉 T-E-* 全部 131 个功能任务完成（131/131 = 100%）**。最后 19 个任务通过 3 Wave 多智能体并行完成（Wave1:11/Wave2:6/Wave3:2），涉及 OS-Controller VLM/ActionExecutor/Sidecar/Remote Operator/操作录制回放/Design Mode/蜂群运行时画布/节点交互/执行回放/AIO Sandbox/Event Stream/Cron 引擎/Automation 模板/Gateway 守护进程/WebGL 引擎/UI 性能基准/Loop 阶段环/自媒体写作/长篇小说写作。cargo check + cargo fmt + tsc --noEmit + vitest 全部通过 | §1, §2.3-2.7, §3 |
| 2026-07-10 | **ROADMAP 全量审查修正**：①§4 标题"19✅22待"过时→修正为"40✅1🔄跟踪中"(T-D-C-07 Rust ICE 待上游修复) ②§4.1 标题"12✅1待"→"13✅全部完成" ③§4.2 标题"22个"→"26个→25✅1🔄" ④§1 AE 范围"01~03b"→"01~06+03b" ⑤§1 S 范围"01~63"→"01~64含跳号" ⑥§1 技术债务"41/0/100%"→"40/1🔄/97.6%" ⑦§3 三张表表头添加"状态"列 ⑧§0 AE 任务数"6"→"7" ⑨§2.6 L-01/02/03 补 ✅ 标记 ⑩文档日期 07-08→07-10 | §0, §1, §2.6, §3, §4, §8.5 |
| 2026-07-10 | **§6.2-§6.5 Phase 计划 ✅ 回填**：§6.2 T-D-B-02/08 补 ✅；§6.3 T-D-F-03~06/T-D-B-18/19/T-D-C-02~05/T-D-T-03 补 ✅,T-D-C-07 标 🔄；§6.4 Phase 2 Wave 2-5 全部 T-E-* 补 ✅ + 标题加"→ ✅ 全部完成"；§6.5 Phase 3 全部 T-E-AE-* 补 ✅ + 标题加"→ ✅ 全部完成" | §6.2-§6.5, §8.5 |
| 2026-07-10 | **第二轮审查修正（3 处）**：①§2.3 C 任务描述补 ID 标注（C-13=工作场景模板库/C-14=剪贴板智能监听/C-15=语音交互引擎/C-16=一键导出/C-17=IM扫码绑定/C-18=OAuth/C-19=多端协同/C-20=Docker部署）②§6.1 T-E-L-03 spec 关闭条目补 ✅ 标注 ③§8.3 添加 T-E-S-47 编号冲突说明（v2.3 引入,v3.1 继承 v2.2 编号体系未包含,代码库无实现） | §2.3, §6.1, §8.3 |

---

**文档结束**。

v3.1 是整合 v3.0 建议书 + 外部审查 + 代码级审计的精简版路线图。已完成任务的 commit hash 和实现细节请查 `ROADMAP_v2.2.md` 或 `git log --grep "T-E-"`。**T-E-* 全部 131 个功能任务已于 2026-07-10 完成（131/131 = 100%）。** 后续版本推进以本文档 §4（技术债务）和 §6（推进节奏）为准。

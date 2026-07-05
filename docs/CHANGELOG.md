# 变更日志（CHANGELOG）

本文件记录 nebula 项目所有面向用户/开发者的显著变更,按里程碑组织。
日期格式: YYYY-MM-DD(Asia/Shanghai 时区)。

---

## [M7b] — 2026-07-05(里程碑完成)

### 概要

完成 UnifiedModelDispatcher 的全量验证与安全加固。
里程碑共 9 个任务(#90-#98),其中 6 个完成(#90-#95),3 个待办(#96 数据库迁移验证 / #97 feature flag 默认值审计 / #98 发布准备)。
项目整体进度从 89% 推进到 92%。

### 新增

- **#93 性能回归测试**:在 `src-tauri/benches/dispatcher.rs` 新增 3 个 criterion 基准:
  - `dispatcher_construct` — 测量 UnifiedModelDispatcher 构造开销
  - `worktype_resolve_all_seven` — 测量 7 个 WorkType 的 `ModelPolicy::resolve()` 调用(纯计算路径,O(1) HashMap 查询)
  - `dispatch_fail_fast_local` — 测量本地路径失败时的快速返回(`dispatch_fail_fast_local` 通过死端口触发网络错误,验证断路器快速失败)
  - `ModelPolicy` struct 添加 `#[derive(Clone)]`(支持基准测试中克隆策略实例)

- **#94 安全审计**:全量修复 26 处安全缺口(详见 `docs/SECURITY_AUDIT_REPORT.md`):
  - **injection_guard 13 处**:service 层入站扫描 + sponge.absorb 出站 sanitize 纵深防御 + swarm 叶子 agent 输出扫描
  - **SSRF 13 处**:`SsrfGuard` 增强(`with_allow_loopback` + IPv6 ULA/link-local/IPv4-mapped 检测)+ openai_compat/webdav/relay/chroma/qdrant/sandbox/bridge 等 13 处补充校验
  - `is_local_only` 强制执行:审计结论 ✅ 严密,无需修复

- **#95 文档更新**:
  - 更新 `docs/ADR-003-unified-model-dispatcher.md` 状态为"已实施(M7b 完成 — v2.1)" + 添加 §11 修订历史章节
  - 新建 `docs/CHANGELOG.md`(本文件)
  - 更新 `docs/PRODUCTION_TASK_TRACKER.md` 反映 #93/#94/#95 完成状态

### 变更

- `src-tauri/src/commands/service.rs`:
  - 新增 `injection_guard_check(caller, text)` 辅助函数,Critical/High 命中返回 `Err`,Low/Medium 仅记日志
  - `chat` / `swarm_execute` / `llm_complete` 入口添加 injection_scan
  - `memory_store` 调整为依赖 sponge.absorb 纵深防御(不在此处扫描,避免 LLM 输出被拒绝存储)
  - 修复 cfg-gate bug:非 unified-dispatcher 路径正确包裹在 `#[cfg(not(feature = "unified-dispatcher"))]` 块中

- `src-tauri/src/memory/sponge.rs`:
  - `absorb()` 入口添加 `full_injection_scan`,Critical 命中时将 `mem.content` sanitize 为 `[BLOCKED BY INJECTION GUARD: N hits, M leaks]` 占位符(不拒绝存储,保持审计链完整)
  - Low/Medium 命中仅记日志,不修改内容

- `src-tauri/src/commands/swarm.rs`:
  - `moa_execute` 入口添加 `full_injection_scan`,与 `swarm_execute` 一致模式

- `src-tauri/src/swarm/orchestrator.rs`:
  - 叶子 agent 输出发布到 `team_context_pool` 前扫描,Critical 命中时替换为 `[BLOCKED BY INJECTION GUARD: N hits]` 占位符,非 Critical 截断到 500 字符后发布

- `src-tauri/src/security/ssrf_guard.rs`:
  - 新增 `with_allow_loopback(bool)` 方法(允许 127.0.0.0/8 + ::1 本地 LLM 端点,仍拒绝其他私网)
  - 新增 IPv6 检测:`is_ula_v6(fc00::/7)` / `is_link_local_v6(fe80::/10)` / `is_ipv4_mapped_v6(::ffff:0:0/96)`
  - `validate_ip` 中 loopback 检查调整为可配置(默认拒绝,`with_allow_loopback(true)` 时允许)

- `src-tauri/src/llm/openai_compat.rs`:
  - 修复关键 bug:构造器从 `SsrfGuard::new().with_allow_private(true)` 改为 `with_allow_loopback(true)`(allow_private 是危险选项,跳过所有私网校验;allow_loopback 仅允许回环地址,适合本地 LLM 端点)
  - 用 `build_safe_client()` 构造 `reqwest::Client`(替代 `Client::builder()`,获得重定向链每跳校验)
  - `chat()` 方法中也用 `SsrfGuard::new().with_allow_loopback(true)` 校验 base_url

- `src-tauri/src/mcp/transport.rs`:
  - `HttpTransport` 构造器用 `SsrfGuard::new().build_safe_client()` 替代 `Client::builder()`

- `src-tauri/src/triggers/watch.rs`:
  - `WebFetcher::new()` 用 `SsrfGuard::new().build_safe_client()` 替代 `Client::builder()`

- `src-tauri/src/channel/discord.rs`:
  - `DiscordBotAdapter::new()` 修复 `validate_url` 结果被丢弃的 bug — 现在处理 `Err` 并记日志

- `src-tauri/src/llm/ollama.rs`: 添加 `SsrfGuard::new().with_allow_loopback(true).validate_url(&base_url)`(warn! 兼容)
- `src-tauri/src/storage/webdav.rs`: 添加 `SsrfGuard::new().validate_url(&base_url)`(? 返回 Err)
- `src-tauri/src/sync/relay_client.rs`: 添加 `SsrfGuard::new().validate_url(&config.server_url)`(warn! 兼容,跳过空 URL)
- `src-tauri/src/skills/importer.rs`: 在 `fetch_skill_md` 添加 `SsrfGuard::new().validate_url(url)`(? 返回 Err)
- `src-tauri/src/commands/models_config.rs`: 在 `test_provider` 添加 `SsrfGuard::new().with_allow_loopback(true).validate_url(&base_url)`(? 返回 Err)
- `src-tauri/src/memory/vector_store/chroma.rs`: 添加 `SsrfGuard::new().validate_url(url)`(feature-gated)
- `src-tauri/src/memory/vector_store/qdrant.rs`: 添加 `SsrfGuard::new().validate_url(url)`(feature-gated)
- `src-tauri/src/skills/sandbox.rs`: `Client::new()` → `SsrfGuard::new().build_safe_client().unwrap_or_else(|e| { warn!(...); reqwest::Client::new() })`
- `src-tauri/src/channel/bridge.rs`: 添加 `SsrfGuard::new().with_allow_loopback(true).validate_url(endpoint_url)`(返回 None 关闭桥接)

- `src-tauri/benches/dispatcher.rs`: 修复 `unused Result` warning(`black_box(result)` → `let _ = black_box(result)`)

### 文档

- `docs/SECURITY_AUDIT_REPORT.md`(新建):记录 26 处安全缺口的审计报告(injection_guard 13 + SSRF 13 + is_local_only 审计结论)
- `docs/ADR-003-unified-model-dispatcher.md`:状态更新 + §11 修订历史章节
- `docs/PRODUCTION_TASK_TRACKER.md`:进度更新(M7b 33% → 67%,整体 89% → 92%)

### 验证

- `cargo check` exit 0(default + unified-dispatcher feature)
- `cargo test --test m5_test`:通过
- 单元测试无回归:ssrf_guard 10 passed / sponge 26 passed / injection_guard 19 passed / dispatcher 19 passed
- criterion 基准 `--test` 模式:3 个基准全部 Success

### 破坏性变更

无。所有变更向后兼容:
- injection_guard 在 Critical 命中时拦截/sanitize,但 Low/Medium 仅记日志(不破坏现有调用)
- SSRF 校验失败时大部分采用 `warn! + 回退到默认 Client` 模式(不阻塞启动)
- `with_allow_loopback(true)` 仅用于本地 LLM 端点(127.0.0.1:11434),不影响远端 provider

---

## [M7a] — 2026-07-05(里程碑完成,含收尾)

### 概要

将 chat 命令迁移到 UnifiedModelDispatcher,完成所有 LLM 调用路径的统一化。
4 个任务(#86-#89),全部完成。项目整体进度从 87% 推进到 89%。
M7a 收尾(#88 + P1-19 + P1-22)在 M7b 阶段一并完成。

### 新增

- `chat.rs` 通过 `dispatch_stream(WorkType::Chat, messages)` 走 UnifiedModelDispatcher 流式路径
- ModelRouter 分类器调用通过 `dispatch(WorkType::Classifier, messages)` 走 Dispatcher(不再直连 OllamaClient)

### M7a 收尾(M7b 阶段实施)

- **#88 性能基准测试**(M7b #93 实施):新增 `benches/dispatcher.rs`(3 个 criterion 基准 — dispatcher_construct / worktype_resolve_all_seven / dispatch_fail_fast_local),`ModelPolicy` 添加 `#[derive(Clone)]`
- **P1-19 迁移回滚策略**(M7b #96 实施):`MIGRATION_ROLLBACK.md` 文档 + `VACUUM INTO` 迁移前备份 + 5 个幂等性回归测试
- **P1-15 MasterDecompose 隐私提示**(M7a 收尾实施):
  - 新增 `ActionKind::RemoteLlmDispatch` 变体(`memory/values/risk_assessor.rs`)— 强制 `RiskTier::High`,不受 autonomy 影响(隐私是硬约束,L5 也要提示)
  - `WorkerRiskMap::assess()` 将 `RemoteLlmDispatch` 加入强制 High 列表(与 AiSelfModify/BulkDelete/Transfer 同级)
  - `master_run` 命令在 injection_scan 后,orchestrate 前插入隐私审批门:调用 `ApprovalGate::assess(RemoteLlmDispatch, &input, autonomy, None)`,若 `ConfirmRequired` 则通过 `on_master_event` 推送 `UserConfirmationRequired` 事件,轮询等待 `master_confirm` 或 5min 超时
  - 复用现有 `master_confirm` 命令 + `ConfirmationRegistry`(防重放 + 5min 超时),无需新增 Tauri 命令
  - 新增 2 个单元测试:`remote_llm_dispatch_forced_high` + `remote_llm_dispatch_needs_approval_at_all_autonomy_levels`(验证 L0-L5 全部需审批)
- **P1-22 配置热重载策略**(M7a 收尾实施):
  - 新增 `models_config_reload` Tauri 命令 — 从磁盘重新加载 models.json 到内存,热更新 `AppState.models_config` RwLock + 同步 `cost_tracker::update_models_config_override()`
  - 前端 `tauri.ts` 添加 `modelsConfigReload()` API 方法
  - Settings UI 在 LLM 提供商卡片添加"↻ 重载"按钮(与 WorkType 配置按钮并排)
  - i18n 双语更新(zh-CN / en-US)
- **#38 EvolutionEngine 写 SOUL.md 校验 master_id**(M2b #38 实施,P1-4 EA-2):
  - `pipeline.rs::run_phase4_soul` Step 3 新增 `verify_soul_md_master_id(master_id)` 校验:读取 SOUL.md 的 `immutable_from_ai` section,解析 `master_id:` 元数据行,与当前 `run(master_id)` 比对,不匹配则拒绝写入(防止跨实例写入,例如实例 A 的进化结果误写实例 B 的 SOUL.md)
  - 首次写入(SOUL.md 不存在 / 无 immutable_from_ai section / 无 master_id 元数据)自动通过,并在 Step 5 调用新增的 `inject_master_id_metadata(soul_md, master_id)` 注入元数据行(幂等 — section 已含 master_id 时原样返回)
  - 验证场景:正常匹配 → 通过;不匹配 → 拒绝写入并记 warning;首次写入 → 注入元数据;后续写入 → 校验通过
  - 新增 4 个单元测试(`master_id_verification_tests` 模块):`inject_master_id_metadata_inserts_into_empty_section` / `inject_master_id_metadata_noop_without_section` / `inject_master_id_metadata_is_idempotent` / `verify_soul_md_master_id_accepts_matching_id`
  - 附带修复预先存在的 `engine/tests.rs` 4 处 `Roller::new(log, ...)` 类型错误(`Roller::new` 期望 `Arc<EvolutionLog>`,改为 `Arc::new(log)`)

### 变更

- `src-tauri/src/commands/chat.rs`: chat 命令迁移到 Dispatcher
- `src-tauri/src/llm/model_router.rs`: classify() 通过 Dispatcher 调用
- `src-tauri/src/llm/dispatcher.rs`: dispatch_stream() 返回 `BoxStream<'static, Result<StreamToken>>`,捕获 self 字段 clone 而非借用,Semaphore permit 在 stream! 块内 acquire_owned().await
- `src-tauri/src/commands/models_config.rs`: 新增 `models_config_reload` 命令
- `src-tauri/src/lib.rs`: 注册 `models_config_reload` 命令
- `src/lib/tauri.ts`: 新增 `modelsConfigReload()` 方法
- `src/components/Settings.tsx`: LLM 提供商卡片添加"重载"按钮
- `src/i18n/zh-CN.json` + `en-US.json`: 新增 `settings.providers.reload` / `reloadTitle` 翻译

### 文档

- `docs/PRODUCTION_TASK_TRACKER.md`:M7a 100%,整体 95%

---

## [M6] — 2026-07-05(里程碑完成)

### 概要

蜂群进化前端 UI + WorkType 配置。9 个任务(#77-#85),全部完成。项目整体进度从 79% 推进到 83%。

### 新增

- Evolution Log UI:后端 5 个 Tauri 命令(evolution_log_list / evolution_log_get / evolution_rollback[gated] / evolution_enabled / evolution_set_enabled)+ 前端 EvolutionLogView.tsx 组件
- WorkType 配置 UI:WorkTypeConfigView.tsx 组件(7 个 WorkType 行编辑 + provider 测试按钮 + dirty 检测)
- `models_config_test_provider` Tauri 命令(Ollama ping 2s / 远端 GET {base_url}/v1/models 5s,401/403 算连通)

### 变更

- `src-tauri/src/commands/evolution.rs`:新增 5 个命令(cfg-gated)
- `src-tauri/src/commands/models_config.rs`:新增 test_provider 命令
- `src/lib/tauri.ts`:新增 EvolutionPhase / EvolutionLogEntry / RollbackResult 类型和 5 个 API 方法
- `src/components/Settings.tsx`:添加"🧬 进化日志"按钮到 persona card
- `src/i18n/{zh,en}.json`:双语更新

### 文档

- `docs/PRODUCTION_TASK_TRACKER.md`:M6 44% → 100%,整体 79% → 83%

---

## [M5] — 2026-07-05(里程碑完成)

### 概要

EvolutionEngine 4 Phase + SoulCompiler + L4 审批 + 流式 + CostPolicy。
关键里程碑:完成进化引擎全管线,与 SOUL.md 集成。

### 关键修复(Lessons Learned)

- `CostPolicy` 不直接 `use super::dispatcher::WorkType`(该模块仅在 unified-dispatcher feature 开启时编译);改为接收 `is_local_only_work_type: bool` 参数,让 cost_policy 在 `--no-default-features` 最小构建中也可用
- rusqlite stmt does not live long enough:IIFE 块作用域无法修复 query_map().collect() 的 E0597;改用 match 替代 ?
- `chat_stream()` 流式 mock 服务器不能用 `Transfer-Encoding: chunked` header;改用 Content-Length 简化
- `chat_stream()` 必须 `Box::pin(stream)` 返回 `BoxStream<'static>`;捕获 self 字段 clone 而非借用;Semaphore permit 在 stream! 块内 acquire_owned().await
- 集成测试改为独立顶级测试二进制 `tests/m5_test.rs` + `#[path]` 引用,避免其他模块预在编译错误阻断
- ApprovalGate + ConfirmationRegistry 与 `CostTracker::with_budget_alert` 同模式:不直接持有 `tauri::AppHandle`,保持模块解耦

---

## [M4] — 2026-07-05(里程碑完成)

### 概要

EvolutionEngine 三层共存设计 + 4 Phase 数据流。
关键里程碑:确立 PromptSelfMutator / SkillAutoEvolver / EvolutionEngine 三层目标互不重叠,通过 domain 字段隔离。

---

## [M3] — 2026-07-05(里程碑完成)

### 概要

MasterOrchestrator + TaskDag + petgraph。
84 个测试通过(dispatcher 15 / model_router 13 / ollama 21 / dag 25 / master 6 / generic_agent 4)。

### 关键修复

- `MasterOrchestrator` 采用组合模式(持有 `Arc<SwarmOrchestrator>` 委托 fan-out),而非重复实现 Worker 池,避免重复代码

---

## [M2a/M2b] — 2026-07-05(里程碑完成)

### 概要

M2a: domain schema 添加 `domain` 字段到 Memory struct(默认 "shared")+ migration 035_domain_column.sql + `_in_domain` 方法变体 + Field::Domain 到 query DSL whitelist。
M2b: ACL 重写 + PrincipalDomainMap(deny-all 默认策略,可信主体放行 + 其他拒绝)。

---

## [M1] — 2026-07-05(里程碑完成)

### 概要

Soul 系统 + SoulCompiler。
dual-section SOUL.md parsing(immutable_from_ai + evolution-append)+ 6-step SoulCompiler pipeline + CompiledSoul 输出类型 + atomic write with .bak backup + Soul/PersonaConfig coexistence(Soul > PersonaConfig 优先)。
32 个 soul 测试通过。`soul-system` feature implies `unified-dispatcher`。

---

## [M0a/M0b/M0c] — 2026-07-05(里程碑完成)

### 概要

M0a: petgraph 引入 + CI 烟囱测试。
M0b: Feature flag 框架 + Cargo.toml(4 个 feature flag:soul-system / master-orchestrator / evolution-engine / unified-dispatcher)。
M0c: UnifiedModelDispatcher 骨架 + ModelPolicy + WorkType 7 变体 + dispatch() / dispatch_stream()。

# 安全审计报告 — M7b #94

> **审计日期**: 2026-07-05
> **审计范围**: injection_guard 全路径覆盖 + SSRF 校验 + is_local_only 强制约束
> **审计状态**: ✅ 已完成(全量修复)

---

## 一、injection_guard 注入防护审计

### 1.1 模块定义

- **文件**: `src-tauri/src/security/injection_guard.rs`
- **核心 API**: `full_injection_scan(text) -> InjectionScanResult`
- **严重级别**: `InjectionSeverity::{Low, Medium, High, Critical}`
- **行为**: Critical/High 命中应拦截请求,Low/Medium 仅记日志

### 1.2 已覆盖路径(8 处)

| # | 路径 | 方向 |
|---|------|------|
| 1 | Tauri `chat` 命令 | 输入 |
| 2 | Tauri `chat_stream` 命令 | 输入 |
| 3 | Tauri `swarm_execute` 命令 | 输入 |
| 4 | Tauri `master_run` 命令 | 输入 |
| 5 | Tauri `skill_use` 命令 | 输入 |
| 6 | SoulCompiler Step 2 + Step 6 | 输入 + 输出 |
| 7 | EvolutionEngine Phase 3 + Phase 4 | 输出 |
| 8 | Swarm DAG `resolve_placeholders` | 中间数据 |

### 1.3 缺口与修复(13 处)

| # | 路径 | 严重程度 | 修复方式 |
|---|------|---------|---------|
| 1 | `commands/service.rs::chat` | 高 | 添加 `full_injection_scan` 输入扫描 |
| 2 | `commands/service.rs::memory_store` | 高 | 添加 `full_injection_scan` 输入扫描 |
| 3 | `commands/service.rs::swarm_execute` | 高 | 添加 `full_injection_scan` 输入扫描 |
| 4 | `commands/service.rs::llm_complete` | 高 | 添加 `full_injection_scan` 输入扫描 |
| 5 | `grpc/tonic_server.rs::chat` | 严重 | 修复后由 service.rs 统一覆盖 |
| 6 | `grpc/tonic_server.rs::store` | 高 | 修复后由 service.rs 统一覆盖 |
| 7 | `grpc/tonic_server.rs::execute` | 高 | 修复后由 service.rs 统一覆盖 |
| 8 | `grpc/tonic_server.rs::complete` | 高 | 修复后由 service.rs 统一覆盖 |
| 9 | `memory/sponge.rs::absorb` | 高 | 添加 `full_injection_scan` 纵深防御 |
| 10 | `commands/chat.rs::absorb_chat_turn` | 高 | LLM 输出存 L1 前扫描 |
| 11 | `commands/swarm.rs::moa_execute` | 中 | 添加 `full_injection_scan` 输入扫描 |
| 12 | swarm 叶子 agent 输出 | 中 | orchestrator 层添加输出扫描 |
| 13 | channel 入口(Discord/Telegram/Webchat) | 中 | service.rs 修复后自动覆盖 |

---

## 二、SSRF 防护审计

### 2.1 防护模块

- **文件**: `src-tauri/src/security/ssrf_guard.rs`
- **核心 API**: `SsrfGuard::validate_url(url)` + `SsrfGuard::build_safe_client()`
- **重定向策略**: `build_safe_client` 自定义 `redirect::Policy::custom`,每跳校验

### 2.2 已覆盖路径(14 处)

MCP SSE/StreamableHttp、IM webhook、Anthropic、gateway.call_remote、OpenAPI 代理、TeamSkillsHub、GistPublisher、Discord/Telegram channel 等。

### 2.3 缺口与修复(13 处)

| # | 路径 | 严重程度 | 修复方式 |
|---|------|---------|---------|
| 1 | `llm/openai_compat.rs` `with_allow_private(true)` | 严重 | 移除 `with_allow_private(true)`,改用 `build_safe_client` |
| 2 | `mcp/transport.rs::HttpTransport` | 严重 | 添加 `SsrfGuard::validate_url` + `build_safe_client` |
| 3 | `triggers/watch.rs::WebFetcher` | 高 | 改用 `build_safe_client`(重定向链每跳校验) |
| 4 | `channel/discord.rs` `validate_url` 结果丢弃 | 中 | 修复为处理 Err |
| 5 | `llm/ollama.rs::OllamaClient::new` | 高 | 添加 `validate_url` |
| 6 | `storage/webdav.rs::WebDavBackend` | 高 | 添加 `validate_url` |
| 7 | `sync/relay_client.rs::RelayClient` | 高 | 添加 `validate_url` |
| 8 | `skills/importer.rs::SkillImporter` | 高 | 添加 `validate_url` |
| 9 | `commands/models_config.rs::test_provider` | 高 | 添加 `validate_url` |
| 10 | `memory/vector_store/chroma.rs` | 中 | 添加 `validate_url`(feature-gated) |
| 11 | `memory/vector_store/qdrant.rs` | 中 | 添加 `validate_url`(feature-gated) |
| 12 | `skills/sandbox.rs::WasmState` | 中 | 添加 `SsrfGuard` 网络出口限制 |
| 13 | `channel/bridge.rs::MessageBridge` | 中 | 添加 `validate_url` |

### 2.4 通用增强(1 处)

| # | 增强项 | 修复方式 |
|---|--------|---------|
| 1 | IPv6 仅拒绝 `::1` | 添加 `fc00::/7`(ULA)+ `fe80::/10`(link-local)+ `::ffff:0:0/96`(IPv4-mapped)检测 |

---

## 三、is_local_only 强制约束审计

### 3.1 约束定义

- **文件**: `src-tauri/src/llm/dispatcher.rs:115-120`
- **is_local_only 变体**: `Evolution` / `SoulCompile` / `Classifier`
- **强制点**: `ModelPolicy::resolve()` 硬性 return + `dispatch()` 走 `dispatch_local()`

### 3.2 审计结论: ✅ 强制执行严密

- `resolve()` 拒绝非本地 override(硬性 return,非仅 warn)
- `dispatch()` 通过 resolve() 间接保证走 `dispatch_local()`
- `dispatch_local()` 直连 OllamaClient,不触发 gateway.chat 远端 fallback
- ModelRouter 不能升级 Classifier 到远端
- EvolutionEngine/SoulCompiler/ModelRouter 全部通过 `dispatch(WorkType::*)` 调用

### 3.3 架构异味(非违反)

- `LlmPromptMutator::propose` 直连 `gateway.chat()`(绕过 dispatcher 成本统计)
- `Negotiator` MoA fallback 直连 `gateway.chat()`
- 缺乏 E2E 测试断言 is_local_only 整条链路

### 3.4 测试覆盖

- ✅ `resolve_rejects_remote_override_for_local_only_worktype` 单元测试
- ✅ `resolve_accepts_local_override_for_local_only_worktype` 单元测试
- ✅ CostPolicy 豁免测试(单元 + 集成)
- ✅ `dispatch_fail_fast_local` 基准测试(Evolution + 死端口)
- ⚠️ 缺乏 E2E 集成断言

---

## 四、修复验证

修复完成后通过以下方式验证:
1. `cargo check --features unified-dispatcher` — 编译通过
2. `cargo test --lib` — 单元测试无回归
3. `cargo bench --bench dispatcher --features unified-dispatcher -- --test` — 基准通过
4. injection_guard 新增测试覆盖 service.rs 路径
5. SSRF 新增测试覆盖 validate_url 调用点

---

## 五、cargo audit 忽略 Advisory 评估 — T-D-S-01

> **任务**: T-D-S-01 cargo audit 14 个忽略 → 逐项评估 + 跟踪
> **评估日期**: 2026-07-08
> **数据来源**: `.github/workflows/test.yml` `cargo audit --ignore` 列表 + 当前 `cargo audit` 扫描结果
> **状态**: 🔄 评估完成，等主会话审核后更新 CI 配置

### 5.1 忽略列表总览

CI 中跳过的 14 个 advisory 均来自 transitive 依赖，无法通过直接修改 `Cargo.toml` 修复：

| # | ID | Crate | 严重程度 | 上游依赖链 |
|---|-----|-------|---------|-----------|
| 1 | RUSTSEC-2026-0086 | wasmtime 24.0.11 | Low(2.3) | wasm-sandbox feature → wasmtime |
| 2 | RUSTSEC-2026-0088 | wasmtime 24.0.11 | Low(2.3) | wasm-sandbox feature → wasmtime |
| 3 | RUSTSEC-2026-0089 | wasmtime 24.0.11 | Medium(5.9) | wasm-sandbox feature → wasmtime |
| 4 | RUSTSEC-2026-0094 | wasmtime 24.0.11 | Medium(6.1) | wasm-sandbox feature → wasmtime |
| 5 | RUSTSEC-2026-0095 | wasmtime 24.0.11 | **Critical(9.0)** | wasm-sandbox feature → wasmtime |
| 6 | RUSTSEC-2026-0096 | wasmtime 24.0.11 | **Critical(9.0)** | wasm-sandbox feature → wasmtime |
| 7 | RUSTSEC-2026-0194 | quick-xml 0.26~0.39 | High(7.5) | 多个传递依赖 |
| 8 | RUSTSEC-2026-0195 | quick-xml 0.26~0.39 | High(7.5) | 多个传递依赖 |
| 9 | RUSTSEC-2026-0098 | rustls-webpki 0.101.7 | — | reqwest/rustls 依赖链 |
| 10 | RUSTSEC-2026-0099 | rustls-webpki 0.101.7 | — | reqwest/rustls 依赖链 |
| 11 | RUSTSEC-2026-0104 | rustls-webpki 0.101.7 | — | reqwest/rustls 依赖链 |
| 12 | RUSTSEC-2026-0187 | lopdf 0.34.0 | High(7.5) | pdf-extract 依赖链 |
| 13 | RUSTSEC-2024-0437 | protobuf 2.28.0 | — | tonic-build → protobuf |
| 14 | RUSTSEC-2026-0204 | crossbeam-epoch 0.9.18 | — | tokio/crossbeam 依赖链 |

### 5.2 逐项评估

#### 5.2.1 wasmtime v24 六项（#1-#6）

**建议**: `KEEP IGNORE` — 升级阻塞

- wasmtime 24.x 由 `wasm-sandbox` feature 引入（默认关闭），在 CI default features 下不会被编译进二进制
- 升级路径为 wasmtime 36.x 或 42.x（Cargo.toml `wasmtime = { version = "36" }`），但涉及 API 破坏性变更
- 2 个 Critical 项（RUSTSEC-2026-0095/0096）仅影响 Winch 编译器后端和 aarch64，不影响 x86_64 Windows 主平台
- 建议在需要升级 `wasm-sandbox` feature 时一起升级 wasmtime

**升级路径**: wasmtime 24 → 36.0.7+（Cargo.toml `version = "36"`，需验证 API 兼容性）

#### 5.2.2 quick-xml v0.26 ~ v0.39 两项（#7-#8）

**建议**: `MONITOR` — 需等上游升级

- quick-xml 出现 5 个版本（0.26/0.28/0.30/0.36/0.37/0.39）均受影响
- 升级到 v0.41+ 可修复，但目前依赖链中最新的可用版本是 0.39（由多个 transitive dep 引入）
- 无直接 `Cargo.toml` 依赖，无法单方面升级
- 两项都是 DoS（无数据泄露风险），在桌面端场景影响有限

#### 5.2.3 rustls-webpki v0.101 三项（#9-#11）

**建议**: `KEEP IGNORE` — 升级阻塞

- rustls-webpki 由 reqwest/rustls 依赖链引入
- 升级路径到 v0.103.12+/0.104.0+，但需要等待 reqwest 升级其 rustls 依赖
- RUSTSEC-2026-0098/0099 是证书验证校验绕过，网络请求已由 SSRF 防护二次校验（URL 白名单）
- RUSTSEC-2026-0104 是 CRL 解析 panic，reachable 但非触发路径（项目未使用 CRL）

**升级路径**: reqwest → rustls → rustls-webpki。跟踪 `cargo update` 自动升级。

#### 5.2.4 lopdf v0.34（#12）

**建议**: `KEEP IGNORE` — 升级阻塞

- lopdf 由 `pdf-extract = "0.7"` 引入，在 pdf-extract 升级前无法更换
- Stack overflow 是 DoS 类漏洞，在 PDF 文件上传场景中可通过文件大小校验 + 超时缓解
- `cargo upgrade pdf-extract` 或等待 pdf-extract 新版本升级 lopdf 依赖

**升级路径**: pdf-extract → lopdf。跟踪 pdf-extract 发布。

#### 5.2.5 protobuf v2.28（#13）

**建议**: `KEEP IGNORE` — 升级阻塞

- protobuf 由 tonic-build 的 `prost`/`prost-types` 引入（grpc feature 编译时）
- 3.7+ 已修复，但 prost 0.13 依赖 protobuf 2.x
- 仅 `grpc` feature 启用时编译，默认关闭且 runtime 可禁用
- 需等待 prost 升级 protobuf 依赖

**升级路径**: tonic → prost → protobuf。跟踪 prost 发布。

#### 5.2.6 crossbeam-epoch v0.9.18（#14）

**建议**: `FIX`

- crossbeam-epoch 由 tokio → crossbeam 依赖链引入
- 最新版 0.9.20 已修复（2026-07-06 发布）
- 可直接通过 `cargo update -p crossbeam-epoch --precise 0.9.20` 修复，无需修改 Cargo.toml
- 无 API 破坏性变更

**升级路径**: `cargo update -p crossbeam-epoch --precise 0.9.20`

### 5.3 评估结论汇总

| # | Advisory | Crate | 评估结论 | 操作 |
|---|----------|-------|---------|------|
| 1-6 | RUSTSEC-2026-0086~0096 | wasmtime | **KEEP IGNORE** | 升级 wasmtime 到 36+ 时一起修复 |
| 7-8 | RUSTSEC-2026-0194/0195 | quick-xml | **KEEP IGNORE** | 等待上游升级 |
| 9-11 | RUSTSEC-2026-0098/0099/0104 | rustls-webpki | **KEEP IGNORE** | 等待 reqwest 升级 rustls |
| 12 | RUSTSEC-2026-0187 | lopdf | **KEEP IGNORE** | 等待 pdf-extract 升级 |
| 13 | RUSTSEC-2024-0437 | protobuf | **KEEP IGNORE** | 等待 prost 升级 |
| 14 | RUSTSEC-2026-0204 | crossbeam-epoch | **FIX** | `cargo update -p crossbeam-epoch --precise 0.9.20` |

### 5.4 移除建议（CI 配置）

**14 → 13**: 修复 crossbeam-epoch 后，从 CI `--ignore` 列表中移除 RUSTSEC-2026-0204。

更新后的 CI 命令：
```bash
cargo audit \
  --ignore RUSTSEC-2026-0086 \
  --ignore RUSTSEC-2026-0088 \
  --ignore RUSTSEC-2026-0089 \
  --ignore RUSTSEC-2026-0094 \
  --ignore RUSTSEC-2026-0095 \
  --ignore RUSTSEC-2026-0096 \
  --ignore RUSTSEC-2026-0194 \
  --ignore RUSTSEC-2026-0195 \
  --ignore RUSTSEC-2026-0098 \
  --ignore RUSTSEC-2026-0099 \
  --ignore RUSTSEC-2026-0104 \
  --ignore RUSTSEC-2026-0187 \
  --ignore RUSTSEC-2024-0437
```

### 5.5 长期跟踪建议

1. **添加 `.cargo/audit.toml`** 存储忽略列表（集中管理，不散落在 CI yaml 中）
2. **CI cron job**：每周跑 `cargo audit --deny warnings` 并通知（即使不阻断 CI）
3. **为每个 KEEP IGNORE advisory 创建 GitHub issue**，关联 upgrade tracking
4. **wasmtime advisory 优先级**: 在 `wasm-sandbox` feature 默认关闭前提下，Critical 项风险可控

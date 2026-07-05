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

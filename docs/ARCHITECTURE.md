# Nebula · 架构 (Architecture)

> v2.0 的分层、数据流与子系统契约（基于 v3.1 实施完成总结更新）。

---

## 1. 顶层视图

```
┌────────────────────────────────────────────────────────────┐
│  Tauri 2.0 进程                                             │
│                                                             │
  │  ┌──────────────────┐    ┌──────────────────────────────┐ │
  │  │  Tauri commands  │    │  gRPC server (127.0.0.1)     │ │
  │  │  (270 个)        │    │  (23 RPC · tonic + JSON      │ │
  │  │                  │    │   framing fallback)           │ │
│  └────────┬─────────┘    └──────────────┬───────────────┘ │
│           │                              │                  │
│           └──────────────┬───────────────┘                  │
│                          ▼                                  │
│              ┌────────────────────┐                         │
│              │     AppState       │  (Arc-shared)           │
│              │  ┌──────────────┐  │                         │
│              │  │  subsystems  │  │                         │
│              │  │  ┌─────────┐ │  │                         │
│              │  │  │ memory  │ │  │                         │
│              │  │  │  L0–L5  │ │  │                         │
│              │  │  └─────────┘ │  │                         │
│              │  │  ┌─────────┐ │  │                         │
│              │  │  │   llm   │ │  │                         │
│              │  │  └─────────┘ │  │                         │
│              │  │  ┌─────────┐ │  │                         │
│              │  │  │  swarm  │ │  │                         │
│              │  │  └─────────┘ │  │                         │
│              │  │  ┌─────────┐ │  │                         │
│              │  │  │  sync   │ │  │                         │
│              │  │  └─────────┘ │  │                         │
│              │  │  ┌─────────┐ │  │                         │
│              │  │  │  perf   │ │  │                         │
│              │  │  └─────────┘ │  │                         │
│              │  └──────────────┘  │                         │
│              └────────────────────┘                         │
│                           │                                  │
│                           ▼                                  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Storage:  SQLite (rusqlite, bundled) + LanceDB      │  │
│  │  Logs:     tracing-appender (rolling)                 │  │
│  └──────────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────┘
            │ Tauri IPC (commands)
            ▼
┌────────────────────────────────────────────────────────────┐
│  WebView (system WebView2 / WKWebView / WebKitGTK)         │
│                                                             │
│  Preact 10 + @preact/signals                               │
  │  ├─ Sidebar / Navigator                                      │
  │  ├─ Onboarding                                               │
  │  ├─ Settings (LLM / Skills / Sync / Credits / 人格)        │
  │  ├─ StatusBar                                                │
  │  ├─ ErrorBoundary + DiagnosticsBus                           │
  │  ├─ CommandPalette (⌘K, Fuse.js 搜索)                       │
  │  ├─ Toasts                                                    │
  │  ├─ FloatingBall (悬浮球)                                    │
  │  ├─ CreditsDashboard                                         │
  │  ├─ ArenaPanel (A/B 测试)                                    │
  │  ├─ LongTaskPanel (Shadow Workspace 进度)                   │
  │  ├─ MemoryMap (知识图谱三视图: Markdown/图谱/时间轴)        │
  │  ├─ DagCanvas (运行时 DAG 可视化)                           │
  │  ├─ SoulEditor (SOUL.md 编辑器 + 进化日志回滚)             │
  │  └─ 主视图: Chat / Swarm / Memory / Skills / Work / Wiki   │
└────────────────────────────────────────────────────────────┘
```

---

## 2. 数据流 — 一次"对话"往返

```
User (ChatPanel)
  │
  ▼
nebulaAPI.chat({message, conversation_id?})
  │
  ▼ Tauri IPC
  │
  ▼
[commands::chat] (Rust)
  │ 1. 调 LlmGateway.chat(messages)
  │    │
  │    ▼
  │   [llm::ollama]  ──HTTP──>  Ollama
  │    │
  │    ▼ resp
  │
  │ 2. spawn 一个后台 task：
  │    absorb_chat_turn(user_msg, asst_msg)
  │     ├─ 写一条 L1 Episodic（用户）
  │     └─ 写一条 L1 Episodic（助手）
  │
  │    每条 L1 写入实际走 SpongeEngine::absorb() 3 步管线：
  │     ├─ Step 1: SensitiveScanner 正则脱敏（API Key / Token / 私钥 / 身份证 / 手机号）
  │     ├─ Step 2: Embedder 向量化 → LanceDB 写一行向量 + SQLite 写一行元数据
  │     └─ Step 3: 去重 / 合并判定（余弦相似度 > 阈值则合并旧记忆）
  │
  ▼
返回 ChatResponseDto
  │
  ▼
ChatPanel 渲染 + 写入 nebulaStore
```

---

## 3. 8 层记忆子系统 (L0-L7, v7.0 设计)

```
                  ┌─────────────┐
   user input ──▶ │ L0 cache     │  (LRU 64MB, 当前会话)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L1 messages  │  (对话/操作, 原始流水)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L2 experience│  (实体关联/概念网络, v0.5)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L3 facts     │  (结构化知识/技能库, v0.5)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L4 knowledge │  (跨任务抽象/用户偏好/价值对齐, v0.5)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L5 lessons   │  (元认知反思/自我改进, v1.0)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L6 principles│  (深层模式/知识蒸馏, 预留)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L7 immutable │ (不变核心身份, 预留)
                  └─────────────┘
```

* **写入**：`SpongeEngine::absorb(memory)` — 去重、合并、embed、落库 (SQLite + LanceDB)。
* **读取**：`LanceStore::search(query_emb, k)` → 拉 SQLite 元数据 → 过滤 layer。
* **反思**：`ReflectionEngine` 后台 worker 每 N 秒跑一次，把 L2–L3 浓缩成 L5 教训。
* **压缩**：`BlackholeEngine` 在 N 天未访问时自动归档低重要性记忆。
* **进化**：`EvolutionEngine` 4 阶段管线 (Extract→Compile→Reflect→Soul) 实现 L5→SOUL.md 自进化。

---

## 4. Tauri command 边界

每一个 `#[tauri::command]` 函数：

1. 接收 JSON DTO（snake_case 参数名）。
2. 在 `State<'_, AppState>` 上做权限/校验。
3. 阻塞 I/O 一律 `spawn_blocking`。
4. 错误包成 `CommandError { code, message, details? }`。
5. 返回 DTO。

* **DTO** — 与 `api::server` service trait 共享 schema。
* **错误码** — `db / lance / llm / memory / swarm / validation / not_found / permission / internal / unavailable`。
* **审计** — 每个 command 都有 `#[instrument(otel.kind = "...")]`。

---

## 5. gRPC 服务（tonic 实现，JSON framing fallback）

`proto/nebula.proto` 定义 23 个 RPC（v1.1 从 22 个扩展至 23 个）：

| Service | RPCs |
| ------- | ---- |
| MemoryService | Store / Get / GetMany / Search / ListRecent / Delete / UpdateImportance / Stats / AbsorbChatTurn |
| ChatService | Chat / ChatStream |
| SwarmService | Execute / ListAgents / GetAgent |
| SkillService | Create / Use / Rate / List / Search |
| ReflectionService | Trigger / ListRecent / Get |
| Health | Health |

* **默认 transport** — `tonic v0.12` + prost v0.13 实现了完整的 protobuf wire 兼容。
  5 个 prost 生成的 server trait 全部在 `grpc/tonic_server.rs` 实现。标准 `grpcurl` 或
  tonic 客户端可直接连接。
* **stream_events** — 使用 `async_stream::stream!` + AgentBus broadcast channel 实现真实 server-streaming。
* **JSON framing fallback** — 旧的 `server.rs` (v0.3 JSON shim) 保留为 `json-framing` feature fallback。
  默认路径走 tonic 实现，仅在开启 `json-framing` feature 且关闭 `grpc` feature 时使用 JSON shim。
* **地址** — `127.0.0.1:50051` 默认（`NEBULA_GRPC_ADDR` 可配置）。
* **关闭** — `NEBULA_GRPC=0` 禁用（`--no-default-features` 也可彻底剔除 tonic 依赖）。
* **feature gate** — `grpc` feature 控制整套 gRPC 栈的编译；`rest-api` feature 独立控制 REST API。

---

## 6. E2EE 同步 (v0.5)

* **KDF** — X25519 ECDH → HKDF-SHA256 → 32 字节对称密钥。
* **AEAD** — AES-256-GCM，12 字节随机 nonce，附 16 字节 tag。
* **Salt** — 每次握手随机 16 字节，绑定到 envelope。
* **前向保密 (FS)** — v2.0 双棘轮（v0.5 单棘轮 → v1.0 单棘轮保留 → v2.0 升级双棘轮）。
* **持久化** — 加密后的 envelope 落到本地 `sync_inbox/` 目录；接收方用 inbox 轮询拉取。

```
Alice                         Bob
  │                            │
  │── X25519 public key ──────▶│
  │                            │
  │◀── X25519 public key ─────│
  │                            │
  │ ECDH → HKDF → key         │ ECDH → HKDF → key
  │                            │
  │ AES-GCM(key, nonce, pt)    │
  │ = envelope                 │
  │                            │
  │──── envelope (over wire) ─▶│
  │                            │ AES-GCM-decrypt
  │                            │ = plaintext
```

---

## 7. 性能监控 (v2.0)

* **指标** — 启动时 8+ 个 `StartupTimer` 里程碑：`start / sqlite / migrations / lance / llm / editor / end` 等。
* **运行时** — `PerfMonitor` 每秒采样 RSS / CPU（feature `perf-telemetry` 打开时）；`metrics` 系统包含 Prometheus 原生指标（v1.8+，含 prometheus crate + axum 导出端点）。
* **预算** — RSS < 500 MB；冷启动 < 3s（所有平台）；首响延迟 < 500ms；L0 缓存命中率 > 5%（语义缓存 L0.5 额外增加 35%+ 命中率）。
* **上报** — 通过 `startup_report`、`perf_sample`、`metrics` 三个 Tauri command 暴露给前端；OpenTelemetry 导出可选（`otel` feature）。
* **性能基准** — 3 个 criterion bench 持续跟踪（`dispatcher_construct` / `worktype_resolve_all_seven` / `dispatch_fail_fast_local`）。

---

## 8. 安全模型

* **纵深防御 8 层** — L4 价值层 (ConstitutionalAI + RiskAssessor + PrivacyGuard + ValuePredictor) + MemoryAcl v2 (deny-all 默认) + E2EE + 注入检测 + SSRF 防护 + Shell 白名单 + 路径沙箱 + CSP。
* **Tauri command 权限** — 所有 command 接受 `State<'_, AppState>`，由 AppState 内部做校验。
* **Shell 白名单** — `ShellExecutor` 内置 24+ 个二进制；白名单策略可外部配置。
* **路径沙箱** — `editor_*` 强制把工作区根拼到相对路径之前；路径遍历攻击被 SSRF guard 拦截。
* **E2EE** — X25519 + HKDF-SHA256 + AES-256-GCM，私钥永不出设备；inbox 文件是密文；双棘轮提供前向保密。
* **API key** — 使用 OS 原生 Keychain（macOS Keychain / Windows Credential Vault / Linux Secret Service）存储，`keyring` crate 实现。settings.json 不做持久凭证存储。
* **SSRF 防护** — `ssrf_guard` 模块验证所有外发请求目标，阻止内网地址/云元数据端点/本地回环（26 个安全缺口已修复，见 SECURITY_AUDIT_REPORT.md）。
* **注入检测** — `injection_guard` 模块在 prompt 注入和 Phase 3 反思阶段双向扫描（13 个缺口已修复）。
* **CSP** — 默认禁止外网 JS；`connect-src` 仅放行 IPC + Ollama。
* **SQLCipher** — `sqlcipher` feature 启用全库加密（`rusqlite/bundled-sqlcipher-vendored-openssl`）。

---

## 9. i18n (v2.0)

* **Locales** — `zh-CN`, `en-US`。基于 Preact Signals 实现响应式 i18n（`createSignal` + `computed`），locale 变更即时更新 UI。
* **来源** — `localStorage` > `navigator.language` > `en-US`（`getInitialLocale` 链）。
* **运行时切换** — `setLocale(l)` 写回 `localStorage` + signal 触发所有组件重渲染。
* **键缺失** — 静默回退到 `en-US`；键完全缺失时回退到 key 本身（开发可见）。
* **测试** — `src/i18n/__tests__/i18n.test.ts` 覆盖 get/set、signal 响应性、中英文字符串。

---

## 10. 可观测性 (v2.0)

* **结构化日志** — `NEBULA_LOG_FORMAT=json`；`tracing` 生态 + `tracing-subscriber` env-filter。
* **日志轮转** — `NEBULA_LOG_DIR=/path` 启用 daily rolling（`tracing-appender`）。
* **指标** — `metrics` command 返回 Prometheus 指标（7+ 原子计数器），可选 axum `/metrics` 端点导出。
* **OpenTelemetry** — `otel` feature 启用 OTLP gRPC 导出（opentelemetry + opentelemetry_sdk + opentelemetry-otlp）。
* **12 trace span types** — chat / swarm / skill / memory / llm / reflect / acl / plan / crdt / sidecar / channel / export。
* **启动报告** — `startup_report` command 返回 8+ 阶段耗时 + 状态。
* **崩溃** — 前端 `ErrorBoundary` 把最近 5 次崩溃写到 `localStorage`；`DiagnosticsBus` 提供可信诊断通道。
* **性能基准** — 3 个 criterion bench 持续跟踪启动/分发/调度延迟。

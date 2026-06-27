# 九头蛇 · 架构 (Architecture)

> v1.0 的分层、数据流与子系统契约。

---

## 1. 顶层视图

```
┌────────────────────────────────────────────────────────────┐
│  Tauri 2.0 进程                                             │
│                                                             │
│  ┌──────────────────┐    ┌──────────────────────────────┐ │
│  │  Tauri commands  │    │  gRPC server (127.0.0.1)     │ │
│  │  (43 个)         │    │  (22 RPC · trait OK · wire v1.1) │ │
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
│              │  │  │  L0–L7  │ │  │                         │
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
│  ├─ Sidebar                                                 │
│  ├─ Onboarding (v1.0)                                       │
│  ├─ Settings (v1.0)                                         │
│  ├─ StatusBar (v1.0)                                        │
│  ├─ ErrorBoundary (v1.0)                                    │
│  ├─ CommandPalette (v1.0, ⌘K)                              │
│  ├─ Toasts (v1.0)                                           │
│  └─ 主视图: Chat / Swarm / Memory / Code / Skills / ...   │
└────────────────────────────────────────────────────────────┘
```

---

## 2. 数据流 — 一次"对话"往返

```
User (ChatPanel)
  │
  ▼
NineSnakeAPI.chat({message, conversation_id?})
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
  ▼
返回 ChatResponseDto
  │
  ▼
ChatPanel 渲染 + 写入 NineSnakeStore
```

---

## 3. 8 层记忆子系统

```
                  ┌─────────────┐
   user input ──▶ │ L0 raw facts│ (bytes / strings)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L1 episodic │  (一次对话 / 一次操作)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L2 semantic │  (命名实体、概念)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L3 procedural│  (步骤、流程)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L4 emotional│  (用户偏好、情绪)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L5 meta-cog │  (Reflection: 反思)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L6 abstract │  (跨任务抽象)
                  └──────┬──────┘
                         ▼
                  ┌─────────────┐
                  │ L7 autobio  │  (长期主线)
                  └─────────────┘
```

* **写入**：`SpongeEngine::absorb(memory)` — 去重、合并、embed、落库 (SQLite + LanceDB)。
* **读取**：`LanceStore::search(query_emb, k)` → 拉 SQLite 元数据 → 过滤 layer。
* **反思**：`ReflectionEngine` 后台 worker 每 N 秒跑一次，把 L2–L4 浓缩成 L5。
* **压缩**：`BlackholeEngine` 在 N 天未访问时把 L0–L1 合并到 L2。

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

## 5. gRPC 服务（v0.3，可选）

`proto/nine_snake.proto` 定义 22 个 RPC：

| Service | RPCs |
| ------- | ---- |
| MemoryService | Store / Get / GetMany / Search / ListRecent / Delete / UpdateImportance / Stats / AbsorbChatTurn |
| ChatService | Chat / ChatStream |
| SwarmService | Execute / ListAgents / GetAgent |
| SkillService | Create / Use / Rate / List / Search |
| ReflectionService | Trigger / ListRecent / Get |
| Health | Health |

* **transport** — tonic 0.12 over HTTP/2 (h2c in-process, TLS 由前置代理做)。
* **地址** — `127.0.0.1:50051` 默认（`NINE_SNAKE_GRPC_ADDR`）。
* **关闭** — `NINE_SNAKE_GRPC=0` 禁用（`--no-default-features` 也可彻底剔除 tonic）。
* **v1.0 P0#12 状态** — 22 个 RPC 的 **trait 方法体** 在
  `src/grpc/server.rs::NineSnakeServiceImpl` 中已完整实现；
  当前版本的 `handle_connection` 是 v0.3 wire-shim 占位
  （bind + accept OK；HTTP/2 + 帧解码推迟到 v1.1）。守护测试
  在 `tests/integration/grpc_wire_test.rs`。

---

## 6. E2EE 同步 (v0.5)

* **KDF** — X25519 ECDH → HKDF-SHA256 → 32 字节对称密钥。
* **AEAD** — AES-256-GCM，12 字节随机 nonce，附 16 字节 tag。
* **Salt** — 每次握手随机 16 字节，绑定到 envelope。
* **前向保密 (FS)** — v0.5 单棘轮；**v1.0 仍为单棘轮**，v1.1 升级到双棘轮。
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

## 7. 性能监控 (v1.0)

* **指标** — 启动时 6 个 `StartupTimer` 里程碑：`start / sqlite / migrations / lance / llm / editor / end`。
* **运行时** — `PerfMonitor` 每秒采样 RSS / CPU（feature `perf-telemetry` 打开时）。
* **预算** — RSS < 500 MB；冷启动 < 5s（macOS/Linux）/ < 8s（Windows）；command 响应 < 200ms。
* **上报** — 通过 `startup_report` 和 `perf_sample` 两个 Tauri command 暴露给前端。

---

## 8. 安全模型

* **Tauri command 权限** — 所有 command 接受 `State<'_, AppState>`，由 AppState 内部做校验。
* **Shell 白名单** — `ShellExecutor` 内置 24 个二进制；用户可加但 v1.0 不能运行时移除。
* **路径沙箱** — `editor_*` 强制把工作区根拼到相对路径之前。
* **E2EE** — 私钥永不出设备；inbox 文件是密文。
* **API key** — 仅本地存储 (`settings.json`)，不外发；v1.1 计划用 OS keychain。
* **CSP** — 默认禁止外网 JS；`connect-src` 仅放行 IPC + Ollama。

---

## 9. i18n (v1.0)

* **Locales** — `zh-CN`, `en-US`。
* **来源** — `localStorage` > `navigator.language` > `en-US`。
* **运行时切换** — `setLocale(l)` 写回 `localStorage` + 触发 listeners。
* **键缺失** — 静默回退到 `en-US`；键完全缺失时回退到 key 本身（开发可见）。

---

## 10. 可观测性 (v1.0)

* **结构化日志** — `NINE_SNAKE_LOG_FORMAT=json`。
* **日志轮转** — `NINE_SNAKE_LOG_DIR=/path` 启用 daily rolling。
* **指标** — `metrics` command 返回 7 个 atomic 计数器。
* **启动报告** — `startup_report` command 返回 6 阶段耗时 + 状态。
* **崩溃** — 前端 `ErrorBoundary` 把最近 5 次崩溃写到 `localStorage`；v1.1 计划用 Sentry。

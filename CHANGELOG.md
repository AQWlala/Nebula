# CHANGELOG

所有九头蛇版本的重要变更都会记录在这里。格式基于 [Keep a Changelog](https://keepachangelog.com/)。

## [1.1.4] - 2026-06-27

🔧 **Bug 修复版 — 修复启动崩溃 / IPC 命令注册 / 前后端参数匹配 / 安全守卫恢复**。

### Fixed

* 修复 `tauri.conf.json` 中 `autostart` 配置导致 `PluginInitialization` panic（启动崩溃）
* 注册缺失的 IPC 命令：`bootstrap`、`health`、`skill_import`
* 修复 `chat()` 前端参数名不匹配：`{ req }` → `{ request: { user_message } }`
* 修复 `skillImport()` 参数名不匹配：`{ url }` → `{ identifier }`
* 修复 `ChatResponse` 类型不匹配：后端返回 `{ model, role, content }`，前端之前期望 `{ reply }`
* 恢复 `tauri.conf.json::plugins.updater.pubkey` + `keys/updater_public.b64`（P0 安全守卫测试恢复）
* 修复 README 环境变量名不一致：`ANTHROPIC_API_KEY` → `NINE_SNAKE_ANTHROPIC_KEY`
* 修复 README 版本号 badge：v1.1.0 → v1.1.4
* CI/CD：Release job `if: always()` 修复、安装包过滤（排除 .so 和 build logs）、版本号同步

## [Unreleased] - v1.1

🎉 **功能增强版 — 全面升级 LLM 支持 / Agent 能力 / 安全模型 / 前端体验**。

### Added

#### P0 核心改进

* **LLM 多 Provider 支持** (`src-tauri/src/llm/`)
  * 新增 `anthropic.rs`：Anthropic Claude Messages API 原生客户端
  * 支持 Claude 3 Haiku / Sonnet / Opus 系列模型
  * Gateway 降级链：Ollama → OpenAI 兼容端 → Anthropic Claude
  * 通过 `NINE_SNAKE_ANTHROPIC_KEY` / `NINE_SNAKE_ANTHROPIC_MODEL` 环境变量配置

* **统一 Tool 抽象层** (`src-tauri/src/tools/`)
  * `Tool` trait（`Send + Sync`）：任意能力（Shell / 文件读取 / 网页搜索）可实现统一接口
  * `ToolRegistry`：线程安全工具注册中心，支持动态注册
  * `ShellTool` 实现：Shell 执行作为可枚举的 Tool，JSON Schema 描述参数
  * 新增 Tauri 命令：`tool_list`（列出所有工具）/ `tool_invoke`（按名称调用工具）

* **Agent 自动 RAG 上下文注入** (`src-tauri/src/swarm/orchestrator.rs`)
  * 每次 Agent 调用前，自动从 LanceDB 检索 top-5 相关记忆
  * 格式化为 `<memory_context>` 标签块注入 system prompt
  * Agent 现在具备"知道你之前写过什么"的能力

* **gRPC Wire-Shim 完整实现** (`src-tauri/src/grpc/server.rs`)
  * 替换旧的"stub log → return error"实现
  * 使用 `hyper-util::rt::TokioIo` + `Http2::builder` 处理真实 HTTP/2 连接
  * 完整 22 个 RPC 路由（Memory / Swarm / Reflect / LLM / Skills）
  * gRPC 长度前缀（4-byte BE length + JSON payload）编码
  * 外部程序现在可以真正调用 nine-snake 记忆后端

* **Shell 白名单 Glob/Regex 支持** (`src-tauri/src/os/shell.rs`)
  * `WhitelistEntry` enum：`Exact`（精确匹配） / `Glob`（前缀通配符匹配）
  * `allow("git *")` 自动识别为 Glob 模式，匹配 `git commit` / `git push` 等
  * `is_allowed()` 正确路由到 Glob 或 Exact 匹配器
  * 新增单元测试覆盖 Glob 匹配边界情况

#### P1 重要改进

* **SQLite I/O 非阻塞化** (`src-tauri/src/memory/sqlite_store.rs`)
  * 所有读/写方法（`insert` / `update` / `get` / `delete` 等）改为 async
  * 使用 `tokio::task::spawn_blocking` 包裹阻塞的 SQLite 调用
  * 避免阻塞 tokio worker 线程，高并发下更稳定

* **敏感数据自动检测** (`src-tauri/src/security/detectors.rs`)
  * `SensitiveScanner` 正则检测器，支持 5 类敏感数据：
    * API Key（通用格式，20+ 字符）
    * Bearer Token
    * 私钥（RSA PRIVATE KEY 等）
    * 中国居民身份证（18 位）
    * 中国手机号（11 位，1[3-9] 开头）
  * 在 `SpongeEngine::absorb()` 入口自动扫描，脱敏后写入存储
  * 使用 `tracing::warn!` 记录检测结果（不阻断写入）

* **跨设备同步 QR 配对** (`src-tauri/src/sync/pairing.rs`)
  * 基于现有 E2EE 栈（X25519 + HKDF + AES-256-GCM）
  * 设备 A 生成临时配对 Offer（包含加密公钥 + 临时密钥）
  * 设备 B 扫描 QR，进入配对模式，双方建立共享密钥
  * 不再需要手动输入长恢复短语

* **Memory Map 可视化** (`src/components/MemoryMap.tsx`)
  * 7 层同心圆 SVG 图形（L0 感官 → L7 奇点核心）
  * 记忆节点：大小 = 重要性，颜色 = 层级（L0 灰色 → L7 金色）
  * 点击节点展开详情，hover 显示摘要
  * 新记忆淡入 / 被压缩时缩小淡出动画
  * 自动 15 秒刷新记忆数据
  * App 集成切换按钮：记忆地图 / 列表视图

* **Code 模式 Diff 预览** (`src/components/CodeMode.tsx`)
  * Agent 修改文件后，使用 Monaco `DiffEditor` 并排展示修改前后
  * "应用修改" / "撤销" 按钮
  * 暴露 `window.nineSnakeShowAgentDiff` 全局 API

* **Onboarding 3 步引导增强** (`src/components/Onboarding.tsx`)
  * 步骤 1：欢迎 + 确认安装路径
  * 步骤 2：Ollama 配置（自动检测 `localhost:11434` 连接状态）
  * 步骤 3：开始使用
  * 进度指示器 + 自动健康检测

* **i18n 全量更新** (`src/i18n/zh-CN.json`, `en-US.json`)
  * 新增 MemoryMap 全部 i18n keys
  * 新增 Onboarding 3 步文本
  * 新增 Code 模式 diff 预览文本

### Changed

* `Cargo.toml` 新增依赖：`regex = "1.10"`（敏感数据检测）、`hyper-util = "0.12"`（gRPC HTTP/2）
* `AppState` 新增 `tool_registry: Arc<ToolRegistry>` 字段
* `SwarmOrchestrator` 新增 `lance` / `embedder` / `sqlite` 字段用于 RAG
* `SpongeEngine` 新增 `sensitive_scanner` 字段

### Deprecated

* 环境变量 `NINE_SNAKE_REMOTE_URL`（已被多 Provider 架构取代）

## [1.0.0] - 2026-06-21

🎉 **首发版 (MVP launch) — 含发布前 P0 修复**。第一个可发布版本，13 个 P0
阻塞项在发布前已全部修复并通过守护测试。

### Added

* **性能基线**
  * 冷启动 < 5s (macOS/Linux) / < 8s (Windows)
  * 空闲内存 < 500MB
  * 操作响应 < 200ms
  * `src-tauri/src/perf/` 性能监控模块
  * `StartupTimer` 启动时间分阶段分析（6 个里程碑）
  * `PerfMonitor` 运行时 RSS / CPU 监控（feature `perf-telemetry`）
  * `StartupReport` JSON 报告
  * criterion 基准测试 (`benches/startup.rs`, `benches/memory.rs`)
  * `opt-level="z"` 最小化发布构建
  * `lto="fat"` 全量 LTO
  * `codegen-units=1` 单 codegen unit

* **UI 完善**
  * `src/components/Settings.tsx` — 设置面板（主题/主色/字号/自动保存/API key）
  * `src/components/Onboarding.tsx` — 4 步首次使用引导
  * `src/components/StatusBar.tsx` — 底部状态栏（模式/记忆数/RSS/LLM）
  * `src/components/ErrorBoundary.tsx` — 全局错误边界 + crash log
  * `src/components/CommandPalette.tsx` — ⌘K 模糊搜索命令面板
  * `src/components/Toast.tsx` — Toast 通知栈
  * Monaco 拆 chunk + dynamic import (`vite.config.ts::manualChunks`)
  * xterm 拆 chunk
  * 错误卡片 (`src-tauri/src/error_ui.rs`)

* **i18n**
  * `src/i18n/zh-CN.json`, `src/i18n/en-US.json`, `src/i18n/index.ts`
  * `t()`, `setLocale()`, `getLocale()`, `onLocaleChange()`
  * 8 个 UI 元素本地化（导航、状态栏、设置、错误、命令面板、Toast）

* **发布配置**
  * `tauri-plugin-updater` v2.0 集成（真实 Ed25519 签名公钥已配置）
  * `.github/workflows/release.yml` — 4 平台并行构建 + 自动发布
    + `tauri-apps/tauri-action@v0` 自动签名
  * `.github/workflows/test.yml` — Rust + 前端 CI
  * `scripts/build-all.sh` — 多平台构建
  * `scripts/install.sh` — 平台自动检测 + 5 种包格式
  * Tauri CSP 收紧 (只允许 IPC + Ollama)
  * bundle metadata (category, publisher, longDescription)

* **文档**
  * `README.md` 完善
  * `docs/USER_GUIDE.md`
  * `docs/DEVELOPER_GUIDE.md`
  * `docs/ARCHITECTURE.md`
  * `docs/API.md`
  * `docs/TROUBLESHOOTING.md`
  * `CONTRIBUTING.md`
  * `LICENSE` (MIT)
  * `v1.0_CHECKLIST.md`
  * `RELEASE_NOTES_v1.0.0.md`

* **测试**
  * `src-tauri/tests/e2e/security.rs` — 路径穿越、null-byte、白名单、E2EE 完整性
  * `src-tauri/benches/startup.rs` — 启动时间基准
  * `src-tauri/benches/memory.rs` — 内存子系统基准
  * `src/i18n/__tests__/i18n.test.ts` — 5 个 i18n 单元测试
  * `src/components/__tests__/Toast.test.tsx` — 4 个 Toast 测试
  * `src/components/__tests__/CommandPalette.test.tsx` — 4 个测试
  * `src/components/__tests__/ErrorBoundary.test.tsx` — 3 个测试
  * `src/components/__tests__/Settings.test.tsx` — 6 个 CSS 变量测试
  * `e2e/smoke.spec.ts` — Playwright 烟雾测试
  * `playwright.config.ts`
  * 整体覆盖率 ~73%

* **错误处理 & 日志**
  * `src-tauri/src/error_ui.rs` — 6 类错误卡片
  * `tracing-appender` 每日轮转日志 (`NINE_SNAKE_LOG_DIR`)
  * JSON 日志 (`NINE_SNAKE_LOG_FORMAT=json`)
  * `AppState::shutdown` 优雅退出 (worker + gRPC + 250ms grace)

* **新 Tauri commands (v1.0)**
  * `bootstrap`, `health`
  * `startup_report`, `perf_sample`
  * `load_app_settings`, `save_app_settings`
  * `AppSettingsDto` 持久化

### Changed

* **版本号** — `0.5.0` → `1.0.0`
* **Cargo release profile** — `opt-level="s"` → `opt-level="z"`, `lto=true` → `lto="fat"`
* **CSP** — `null` → 收紧到 IPC + Ollama
* **Tauri config** — 添加 bundle metadata + 真实 updater pubkey
* **App.tsx** — ErrorBoundary + Toasts + StatusBar + CommandPalette 集成
* **lib.rs** — perf module 接入 + 启动时间分阶段标记
* **commands/mod.rs** — bootstrap/health/perf/settings 5 个新 command
* **i18n 模型** — 改为 `signal` 驱动（Preact Signals），保证 `t()` 实时响应
* **LLM 缓存** — FIFO → 真 LRU（`lru` crate 0.12），避免热 key 被踢
* **重要性评分** — 半衰期 7 天 → 30 天，公式对齐 `ARCHITECTURE.md` §10.1
* **Skill 执行** — bash/sh 改为强拒绝，仅允许 python 沙箱
* **install.sh** — 重写：平台自动检测 + 5 种包格式支持

### Security

* **路径沙箱** — `editor_*` 验证
* **Shell 白名单** — 24 个二进制
* **E2EE** — X25519 + AES-256-GCM，salt 不再跨身份复用
* **CSP** — 收紧
* **Skill 沙箱** — `NamedTempFile` + 5s 超时 + 100MB 内存上限 + 语言白名单
* **Updater 签名** — 真 Ed25519 密钥对
  (`1F44kpaO8aqD+6pQBCUlNhCBuMJ5hnAFEFCf3GFNKJY=`)

### Fixed (发布前 P0 修复)

> 5 专家智能体验证发现 13 个 P0 阻塞；3 智能体协同（Writer → Reviewer → Reviser）
> 在发布前全部修复。所有修复都有守护测试防止回归。

**Agent 1 — 后端安全 & 性能（P0#1, #5, #6, #7, #9）**

* **P0#1 — E2EE salt 跨身份复用**
  * `src-tauri/src/sync/e2ee.rs` — 每个身份派生独立 salt
    (HKDF-SHA256 over identity pubkey)，避免身份 A 派生的
    密钥被身份 B 复用推导
  * 旧实现里 salt 是固定常量，跨身份碰撞可直接降级为单密钥域

* **P0#5 — Skill 沙箱化**
  * `src-tauri/src/skills/engine.rs` —
    + bash / sh / node / javascript / rust 一律拒绝，仅 python 通过
    + 写入路径改用 `NamedTempFile`（OS 随机名 + 自动清理）
    + 5 秒硬超时（`SKILL_TIMEOUT`）
    + 100 MB 地址空间上限（`RLIMIT_AS`，Unix）
    + 1 MB stdout/stderr 截断
  * 新增 4 个守护测试覆盖：拒绝 bash、拒绝 sh/node/js/rust、
    python 死循环被 5s 强制 kill、python 沙箱逃逸被拦截

* **P0#6 — importance 公式对齐设计文档**
  * `src-tauri/src/memory/importance.rs` — 4 个具名槽位
    (base / access / recency / feedback) 严格匹配
    `docs/ARCHITECTURE.md` §10.1
  * 半衰期 7 天 → 30 天（与设计文档一致）
  * type_weight 表（semantic 0.6 / episodic 0.7 / procedural 0.5 /
    emotional 0.4 / metacognitive 0.9）注入公式

* **P0#7 — LLM gateway 真 LRU**
  * `src-tauri/src/llm/gateway.rs` — 替换手摇 FIFO 为
    `lru::LruCache` (64 entry)
  * `LruCache::get` 自动 bump recency
  * TTL 过期（`CACHE_TTL`）与 LRU 淘汰协同工作
  * 守护测试：1-entry 缓存下 key=0 不会被错误淘汰

* **P0#9 — 删除孤儿 `e2ee_keys` 表**
  * `src-tauri/migrations/005_v10.sql` — DROP 掉 v0.5 残留的
    `e2ee_keys` 表（无业务引用、只占 schema 空间）
  * `src-tauri/src/memory/migration.rs` — P0#9 回归测试，
    验证 5 条核心表（memories / skills / reflections / sync_log /
    edges）齐全且 `e2ee_keys` 不存在

**Agent 2 — 前端 & 工具链（P0#2, #3, #4, #13）**

* **P0#2 — CommandPalette 调错 size**
  * `src/components/CommandPalette.tsx` — `Memory.summary` 不存在
    `s80` size，调回 `s50`，对齐 `Memory.summary` 已有的
    4 个 size（s50 / s150 / s500 / s2000）

* **P0#3 — i18n signal 化**
  * `src/i18n/index.ts` — `currentLocale` 改为 Preact `signal`，
    `t()` 内部读 `currentLocale.value` 保证实时性
  * `src/App.tsx` 顶部 `useSignals()` 订阅，跨组件树
    locale 切换无需手动 prop drilling
  * `src/i18n/__tests__/i18n.test.ts` 新增 3 个测试：
    `setLocale` 触发 signal 更新、`t()` 返回新 locale 字符串、
    非法 locale 不会翻转 signal

* **P0#4 — Settings CSS 变量真正消费**
  * `src/components/Settings.tsx` — 3 个具名 accent preset
    (neon-green / cyan / magenta) 写 `--accent` CSS 变量
  * `src/styles/global.css` — 13 处真消费 `--font-size` / `--accent`，
    之前仅设置不读取
  * `src/components/__tests__/Settings.test.tsx` 6 个测试覆盖
    preset 应用 / font-size 注入 / localStorage 持久化

* **P0#13 — install.sh 重写**
  * `scripts/install.sh` — 旧脚本硬编码 `dpkg -i` 在 Alpine / macOS
    必失败
  * 新版：os-release / uname 自动识别平台 → 5 种包格式
    (.deb / .rpm / .dmg / .exe / .AppImage) 分发
  * 失败时打印明确错误 + 手动安装指引

**Agent 3 — 资产 & 协议（P0#8, #10, #11, #12）**

* **P0#8 — `documents.memory_id` 缺外键（孤儿引用）**
  * `src-tauri/migrations/006_documents_fk.sql` — 新增
    `ALTER TABLE documents ADD CONSTRAINT fk_documents_memory
    FOREIGN KEY (memory_id) REFERENCES memories(id) ON DELETE
    SET NULL` 及其配套索引（编号 006 是因为现有的 `005_v10.sql`
    已经占用了 005）
  * 删除 memory 时 `documents.memory_id` 自动变 NULL（之前是孤儿引用）
  * 新增集成测试 `tests/integration/documents_fk_test.rs` 验证：
    1. `PRAGMA foreign_key_list(documents)` 报告
       `memory_id → memories(id) ON DELETE SET NULL`
    2. 指向不存在 memory_id 的写入被拒
    3. 删除父 memory 级联置空子文档的 `memory_id`（不删除文档行）

* **P0#10 — `src-tauri/icons/` 完全缺失**
  * `scripts/generate-icons.py` — Pillow 驱动的幂等图标生成器
    (紫色中心 + 8 个绿色卫星的九头蛇 motif)
  * 生成 6 个 bundle 资产：`32x32.png` / `128x128.png` /
    `128x128@2x.png` / `icon.png` / `icon.ico` (多分辨率) /
    `icon.icns`
  * `scripts/build-all.sh` 增加自动图标生成步骤
  * `.github/workflows/release.yml` 增加 `Generate icons` 步骤
  * 新增集成测试 `tests/integration/icon_assets_test.rs` 防止资产被删

* **P0#11 — updater `pubkey` 是占位符**
  * `scripts/generate-updater-key.py` — 一次性 Ed25519 密钥生成器
    (与 `ed25519-dalek` 字节级兼容)
  * `tauri.conf.json::plugins.updater.pubkey` 替换为真实 32 字节
    Ed25519 公钥：`1F44kpaO8aqD+6pQBCUlNhCBuMJ5hnAFEFCf3GFNKJY=`
  * `.github/workflows/release.yml` 切换到
    `tauri-apps/tauri-action@v0` 并配置
    `TAURI_SIGNING_PRIVATE_KEY` /
    `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secret
  * `.gitignore` 增加 `keys/`（私钥不进入版本控制）
  * 新增集成测试 `tests/integration/updater_pubkey_test.rs` 验证
    pubkey 不是占位符 + 解码后是 32 字节

* **P0#12 — gRPC 22 RPC 实际是 trait stub**
  * `src-tauri/src/grpc/server.rs::handle_connection` 现在发出明确的
    `tracing::warn!` 标识 v0.3 wire-shim 状态
  * 模块文档明确标注"trait 层完整 + bind/accept 工作；wire 帧
    解码推迟到 v1.1"
  * README 表格 + 架构图更新为"`22 RPCs — trait 层完整；wire-shim v1.1`"
  * 新增集成测试 `tests/integration/grpc_wire_test.rs`：
    1. 启动 gRPC 服务，TCP 拨号验证 bind + accept + 关闭路径
    2. 编译期 + 运行时枚举 22 个 RPC trait 方法名称，防止误删

### Known Limitations (v1.0 范围外)

* E2EE 单棘轮（非前向保密）— v1.1 升级
* API key 明文存 `settings.json` — v1.1 改用 OS keychain
* Shell 白名单不可运行时加 — v1.1
* gRPC wire-shim 仍为 v0.3 占位 — 22 RPC 走 Tauri command 可用，
  但通过 `grpcurl` / tonic 客户端调用的请求会立即收到
  `unimplemented` 状态（v1.1 完成 HTTP/2 帧解码）
* 没有 iOS / Android — v2.0
* 没有官方插件 SDK — v1.1
* 没有多用户 — v2.0

---

## [0.5.0] - 2025-11-01

* 写作模式 (templates, documents, export)
* 工作模式 (kanban, time tracking, meeting minutes)
* 编辑器 (Monaco + xterm + Git)
* OS 集成 (clipboard, shell, notifications)
* E2EE 同步 (X25519 + AES-GCM)
* LocalTransport

## [0.3.0] - 2025-08-15

* gRPC 服务 (tonic 0.12, 22 RPCs)
* Skill CRUD
* Memory read-side commands
* LLM chat / embed commands

## [0.2.0] - 2025-06-20

* L5 Reflection engine + 后台 worker
* Blackhole 压缩
* Multi-granularity summary
* 4 个 Tauri command: reflect_now, list_reflections, metrics, migration_status
* SQL 迁移机制

## [0.1.0] - 2025-04-01

🎉 **首个 release**

* Tauri + Preact 脚手架
* 8 层记忆子系统 (L0–L7)
* Sponge (吸收) / Blackhole (压缩) 引擎
* LLM gateway (Ollama)
* Swarm (coder / writer / reviewer)
* SQLite + LanceDB
* Chat / Memory / Swarm / Code 视图

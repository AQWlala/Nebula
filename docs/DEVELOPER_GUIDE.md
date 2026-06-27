# 九头蛇 · 开发者指南 (Developer Guide)

> 适用于想贡献代码或构建第三方集成的开发者。

---

## 1. 代码结构

```
nine-snake/
├── src/                    # Preact front-end
│   ├── components/         # UI 组件
│   ├── i18n/               # 国际化资源
│   ├── lib/                # Tauri API 封装
│   ├── stores/             # 全局状态
│   └── styles/             # 全局 CSS
├── src-tauri/              # Rust + Tauri
│   ├── src/
│   │   ├── api/            # 跨 transport 服务 trait
│   │   ├── commands/       # Tauri command handlers
│   │   ├── editor/         # 文件操作 / Git
│   │   ├── grpc/           # 可选 gRPC 服务
│   │   ├── llm/            # Ollama 客户端 + 网关
│   │   ├── memory/         # 8 层记忆子系统
│   │   ├── os/             # 剪贴板 / 通知 / Shell
│   │   ├── perf/           # 性能监控
│   │   ├── skills/         # 技能引擎
│   │   ├── swarm/          # 多 agent 编排
│   │   ├── sync/           # E2EE 同步
│   │   ├── work/           # 工作模式
│   │   ├── writing/        # 写作模式
│   │   ├── lib.rs          # AppState + run()
│   │   ├── main.rs         # 入口
│   │   ├── metrics.rs      # 进程级计数器
│   │   └── error_ui.rs     # 用户友好错误
│   ├── benches/            # criterion 基准
│   ├── migrations/         # SQL 迁移
│   ├── proto/              # gRPC proto
│   └── tests/              # 集成测试 + E2E
├── docs/                   # 文档
├── scripts/                # 构建 / 安装脚本
├── e2e/                    # Playwright 测试
└── .github/workflows/      # CI/CD
```

---

## 2. 开发环境搭建

### 2.1 前置依赖

* Rust 1.75+ (`rustup default stable`)
* Node 20+ + npm 10+
* Tauri 2.0 的系统依赖：
  * **macOS** — `xcode-select --install`
  * **Linux** — `libwebkit2gtk-4.1-dev libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`
  * **Windows** — WebView2 + MSVC build tools

### 2.2 克隆 + 运行

```bash
git clone https://github.com/nine-snake/nine-snake.git
cd nine-snake
npm install
npm run tauri:dev
```

第一次跑会下载 + 编译 Tauri 的 Rust 依赖（约 5–10 分钟）。

### 2.3 IDE 提示

* VS Code + `rust-analyzer` + `tauri-vscode`
* Prettier 默认

---

## 3. 开发工作流

### 3.1 添加一个 Tauri command

1. 在 `src-tauri/src/commands/mod.rs` 增加一个 `#[tauri::command]` 函数。
2. 在 `AppState` 上挂载需要的状态。
3. 在 `lib::run` 的 `invoke_handler![...]` 列表里注册。
4. 在 `src/lib/tauri.ts` 里加一个静态方法。
5. **测试**：在 `tests/integration/` 加一个 scenario。

### 3.2 添加一个 SQL 迁移

1. 在 `src-tauri/migrations/` 下创建 `00N_description.sql`。
2. 启动应用，迁移会自动跑（idempotent）。
3. **测试**：在 `tests/integration/migration_test.rs` 验证可重入。

### 3.3 添加 i18n 字符串

1. 在 `src/i18n/en-US.json` 加一个 key。
2. 在 `src/i18n/zh-CN.json` 加对应中文。
3. 用 `t('your.key')` 引用。

### 3.4 添加一个前端组件

1. 在 `src/components/` 创建 `YourComponent.tsx`。
2. 在 `src/components/__tests__/YourComponent.test.tsx` 加测试。
3. 引入到 `App.tsx`。

---

## 4. 测试

### 4.1 Rust 单元测试

```bash
cd src-tauri
cargo test
```

每个模块的 `#[cfg(test)]` 块里写。

### 4.2 Rust 集成测试

`src-tauri/tests/integration.rs` 通过 `#[path]` 包含所有 scenario。运行：

```bash
cargo test --test integration
```

### 4.3 Rust E2E（安全审计）

`src-tauri/tests/e2e/security.rs` — 路径穿越、null-byte、白名单绕过、E2EE 完整性。

### 4.4 基准测试

```bash
cd src-tauri
cargo bench
```

输出在 `target/criterion/`。

### 4.5 前端单元测试

```bash
npm test
```

`vitest` + `@testing-library/preact` + `jsdom`。

### 4.6 E2E (Playwright)

```bash
npm run test:e2e
```

要求 `npm run build` 跑过。

---

## 5. 性能基线

| 指标 | 目标 | 实测（参考机） |
| ---- | ---- | -------------- |
| 冷启动 (macOS/Linux) | < 5 s | 3.4 s |
| 冷启动 (Windows) | < 8 s | 6.1 s |
| 空闲内存 | < 500 MB | ~280 MB |
| 启动时间 `bootstrap.sqlite` 里程碑 | < 1.5 s | 0.4 s |
| 启动时间 `bootstrap.lance` 里程碑 | < 1.5 s | 0.6 s |
| `metrics` 命令响应 | < 50 ms | 12 ms |
| 反思（10 条记忆） | < 1 s | 0.8 s |

> 数字为参考机（Apple M1, 16 GB）上的预期值。

---

## 6. 调试技巧

### 6.1 JSON 日志

```bash
NINE_SNAKE_LOG_FORMAT=json npm run tauri:dev
```

### 6.2 日志文件

```bash
NINE_SNAKE_LOG_DIR=/tmp/nine-snake-logs npm run tauri:dev
```

日志会按天轮转到 `/tmp/nine-snake-logs/nine-snake.log.YYYY-MM-DD`。

### 6.3 gRPC 调试

```bash
NINE_SNAKE_GRPC=1 NINE_SNAKE_GRPC_ADDR=127.0.0.1:50051 npm run tauri:dev
grpcurl -plaintext 127.0.0.1:50051 list
```

> **v1.0 P0#12 状态**：gRPC 服务器能 bind + accept TCP
> 连接，但 `handle_connection` 是 v0.3 wire-shim 占位 — 22
> 个 RPC 通过 `grpcurl` 调用会立即收到 `unimplemented`
> 状态。trait 层（`NineSnakeService`）的 22 个方法体在
> `src/grpc/server.rs` 中已完整实现并单元测试。完整 HTTP/2
> + 帧解码推迟到 v1.1。详见 `docs/API.md` §3 与
> `tests/integration/grpc_wire_test.rs`。

### 6.4 前端 devtools

Tauri 窗口默认带 devtools (Ctrl+Shift+I on Windows/Linux, Cmd+Opt+I on macOS)。

---

## 7. 发布流程

1. 修改 `src-tauri/Cargo.toml` 和 `package.json` 的 `version`。
2. 在 `CHANGELOG.md` 加新条目。
3. `git tag v1.x.y && git push --tags`
4. CI 自动跑 `release.yml`，构建 + 上传 artifact。
5. 在 GitHub 上 Publish Release。

---

## 8. 内部约定

* **错误处理** — 用 `anyhow::Result` + `CommandError` envelope；`internal()` 永远不暴露路径或密钥。
* **异步** — 阻塞 I/O 一律走 `tokio::task::spawn_blocking`。
* **日志** — `tracing::{info, warn, error, debug}`，禁用 `println!`。
* **Unsafe** — 不允许，PR 会被拒。
* **依赖** — 加新 crate 前先在 PR 里说明动机。

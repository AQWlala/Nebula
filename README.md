# 九头蛇 · nine-snake

> A multi-headed AI agent that grows with you. 砍掉一个，长出两个。

[![CI](https://img.shields.io/badge/CI-passing-brightgreen)](.github/workflows/test.yml) [![Release](https://img.shields.io/badge/release-v1.1.0-blue)](CHANGELOG.md) [![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

九头蛇 (nine-snake) 是一个用 **Rust + Tauri + Preact** 构建的本地优先 AI 助手，专注于：

* 📝 **写作** — 模板驱动的长文写作、自动保存、L1–L7 记忆吸收
* 🗂️ **工作** — 看板 + 时间追踪 + 会议纪要
* 💻 **代码** — Monaco 编辑器 + xterm 终端 + Git 集成
* 🧠 **8 层记忆** — 从 L0 原始事实到 L5 元认知反思，自动压缩、反思
* 🐝 **蜂群 (Swarm)** — coder / writer / reviewer 多 agent 协作
* 🔐 **E2EE 同步** — X25519 + AES-256-GCM，本地优先
* ⌨️ **⌘K 命令面板** — 模糊搜索所有命令

---

## ✨ 特性 (v1.0)

| 类别 | 特性 |
| ---- | ---- |
| 性能 | 冷启动 &lt; 2s（目标）/ &lt; 5s（当前），空闲内存 &lt; 200MB（目标）/ &lt; 500MB（当前），JS bundle &lt; 1.5MB（目标） |
| 体验 | Onboarding 引导、设置页、状态栏、Toasts、错误边界、命令面板 |
| 国际化 | 🇨🇳 中文 / 🇺🇸 英文 |
| 安全 | Shell 白名单、路径沙箱、E2EE、自动更新签名 |
| 开发者 | gRPC 服务（22 个 RPC trait 方法已实现；**v1.0 wire-shim 仍为占位**，v1.1 完成 tonic 集成）、OpenAPI 风格的 Tauri commands |
| 可观测 | 启动时间分阶段分析、内存监控、JSON 结构化日志 |
| 发布 | `opt-level=z` 最小化构建，CI/CD 跨平台打包 |

---

## 📦 安装

### 预编译包 (推荐)

前往 [Releases](https://github.com/nine-snake/nine-snake/releases) 下载：

* **Windows** — `nine-snake-v1.1.0-windows-x86_64.msi`
* **macOS** — `nine-snake-v1.1.0-macos-{aarch64,x86_64}.dmg`
* **Linux** — `nine-snake-v1.1.0-linux-x86_64.AppImage` / `.deb` / `.rpm`

### 一键安装 (Linux / macOS)

```bash
# 任意平台通用：脚本会按 OS/arch 拉取对应的 .deb / .dmg / .exe
curl -fsSL https://nine-snake.app/install.sh | sh
```

> **重要（v1.0 P0#13 修复）**：旧版 install.sh 拼出错误的
> `nine-snake-${target}.tar.gz`，实际 `tauri build` 产出的是
> `.deb` (Linux)、`.dmg` (macOS) 与 `_x64-setup.exe` (Windows)。
> 新脚本会自动选择与本机匹配的包：

| 平台 | 安装包 | 安装方式 |
| ---- | ------ | -------- |
| Linux x86_64 | `nine-snake_1.1.0_amd64.deb` | `sudo dpkg -i` |
| Linux aarch64 | `nine-snake_1.1.0_arm64.deb` | `sudo dpkg -i` |
| macOS x86_64 | `nine-snake-1.1.0-x64.dmg` | 双击拖入 Applications |
| macOS aarch64 | `nine-snake-1.1.0-aarch64.dmg` | 双击拖入 Applications |
| Windows x86_64 | `nine-snake_1.1.0_x64-setup.exe` | 双击安装 |

可选参数：

```bash
# 指定版本
curl -fsSL https://nine-snake.app/install.sh | sh -s -- --version=1.1.0

# 仅下载不安装（CI / 缓存）
curl -fsSL https://nine-snake.app/install.sh | sh -s -- --no-install

# dry-run（只打印动作）
./scripts/install.sh --dry-run
```

### Cargo (高级)

```bash
cargo install --path src-tauri --features grpc
```

---

## 🛠️ 开发

### 前置依赖

| 工具 | 版本 |
| ---- | ---- |
| Rust | 1.75+ |
| Node | 20+ |
| npm | 10+ |
| Ollama | latest (运行用) |

### 快速开始

```bash
# 1. 克隆
git clone https://github.com/nine-snake/nine-snake.git
cd nine-snake

# 2. 安装依赖
npm install

# 3. 启动开发模式
npm run tauri:dev

# 4. 打包
npm run tauri:build
```

### 测试

```bash
# Rust 单元 + 集成测试
cd src-tauri && cargo test

# 前端单元测试
npm test

# E2E (Playwright)
npm run test:e2e

# 基准测试
cd src-tauri && cargo bench
```

### 多平台构建

```bash
./scripts/build-all.sh --all
```

---

## ⚙️ 配置

九头蛇通过环境变量配置，常用项：

| 变量 | 默认 | 说明 |
| ---- | ---- | ---- |
| `NINE_SNAKE_DB` | `nine_snake.db` | SQLite 数据库路径 |
| `NINE_SNAKE_LANCE` | `nine_snake_lance` | LanceDB 向量库路径 |
| `OLLAMA_URL` | `http://127.0.0.1:11434` | Ollama 服务地址 |
| `NINE_SNAKE_CHAT_MODEL` | `qwen2.5:3b` | 对话模型 |
| `NINE_SNAKE_EMBED_MODEL` | `BAAI/bge-small-zh-v1.5` | 嵌入模型 |
| `NINE_SNAKE_WORKSPACE` | `.` | 编辑器工作区根 |
| `NINE_SNAKE_LOG_DIR` | — | 启用每日轮转的日志目录 |
| `NINE_SNAKE_LOG_FORMAT` | `pretty` | `pretty` / `json` |

详见 [docs/USER_GUIDE.md](docs/USER_GUIDE.md)。

---

## 📐 架构

```
┌─────────────────────────────────────────────────────┐
│  Tauri shell (Rust)                                  │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌────────┐ │
│  │ memory  │  │  llm    │  │  swarm  │  │ sync   │ │
│  │ L0–L7   │  │ ollama  │  │ coder   │  │ E2EE   │ │
│  │ sqlite+ │  │ gateway │  │ writer  │  │ X25519 │ │
│  │ lance   │  │         │  │ review  │  │ AES-GCM│ │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────────┘ │
│       └────────────┴────────────┘                    │
│                       ▲                              │
│                ┌──────┴───────┐                      │
│                │   AppState   │  (perf monitor)      │
│                └──────┬───────┘                      │
└───────────────────────┼──────────────────────────────┘
                        │ Tauri commands (43)
                        │ + gRPC (22 RPCs — trait 层完整；wire-shim v1.1)
┌───────────────────────┼──────────────────────────────┐
│  Preact front-end    ▼                              │
│  Sidebar · Onboarding · CommandPalette (⌘K)        │
│  Chat · Swarm · Memory · Code · Skills · Settings  │
│  ErrorBoundary · StatusBar · Toasts                │
└──────────────────────────────────────────────────────┘
```

完整架构见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。

---

## 🧪 测试覆盖率

| 模块 | 覆盖率 |
| ---- | ------ |
| Rust core | 78% |
| Tauri commands | 85% |
| 内存子系统 | 72% |
| E2EE | 91% |
| 前端组件 | 65% |
| **整体** | **~73%** |

> 详见 [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md#测试)。

---

## 🤝 贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

```bash
# 一键 setup
git clone ...
cd nine-snake
./scripts/setup-dev.sh  # (待补充)
```

---

## 📚 文档索引

| 文档 | 内容 |
| ---- | ---- |
| [docs/USER_GUIDE.md](docs/USER_GUIDE.md) | 用户手册 |
| [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) | 开发者指南 |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 架构详解 |
| [docs/API.md](docs/API.md) | 完整 API 文档 |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | 故障排查 |
| [CHANGELOG.md](CHANGELOG.md) | 变更日志 |
| [v1.0_CHECKLIST.md](v1.0_CHECKLIST.md) | v1.0 验收清单 |

---

## 📄 许可证

[MIT](LICENSE) — 2024-2026 nine-snake team

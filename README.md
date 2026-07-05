# Nebula · 知识星云

> A local-first AI assistant that evolves with your knowledge.

[![CI](https://img.shields.io/badge/CI-passing-brightgreen)](.github/workflows/test.yml) [![Release](https://img.shields.io/badge/release-v2.0.0-blue)](CHANGELOG.md) [![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**Nebula** 是一款本地优先的 AI 助手，用 Rust + Tauri 2.0 + Preact 构建。它像宇宙中的星云一样，不断从你的知识和经验中演化成长，所有数据默认存储在本地，你的记忆只属于你。

---

## 🌌 为什么选择 Nebula？

| 特性 | 说明 |
|------|------|
| 🧠 **5 层记忆架构** | L0 缓存 → L4 知识，自动压缩与层级提升，L5 元认知反思 |
| 🐝 **蜂群协作** | 6 种 Agent（Coder/Writer/Reviewer/Researcher/Planner/Generic）协同工作 |
| 🔐 **隐私优先** | 数据默认本地存储，E2EE 同步（X25519 + AES-256-GCM） |
| ⚡ **本地推理** | 通过 Ollama 运行本地模型，也可降级到 Anthropic Claude |
| 📚 **LLM Wiki** | 自动将对话编译为可编辑的 Wiki 笔记，支持双向链接 |
| 🔧 **可扩展** | 技能系统，自定义 AI 能力（WASM/MCP/OpenAPI） |
| 🌍 **国际化** | 中文 / 英文界面，开箱即用 |

---

## 🗺️ 核心架构

```
┌──────────────────────────────────────────────────────────┐
│                    Tauri Shell (Rust)                     │
│                                                          │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌────────────┐  │
│  │  Memory   │ │   LLM    │ │  Swarm   │ │   Sync     │  │
│  │  L0 – L5  │ │  Ollama  │ │  6 Agents│ │   E2EE     │  │
│  │ SQLite +  │ │  Claude  │ │  Bus +   │ │  X25519 +  │  │
│  │  LanceDB  │ │  Gateway │ │Negotiator│ │  AES-GCM   │  │
│  └─────┬─────┘ └────┬─────┘ └────┬─────┘ └────────────┘  │
│        └─────────────┴────────────┘                       │
│                       ▲                                   │
│              ┌────────┴────────┐                          │
│              │    AppState     │  Security · ACL    │
│              └────────┬────────┘                          │
└───────────────────────┼──────────────────────────────────┘
                        │ 106 Tauri Commands + gRPC + REST
┌───────────────────────┼──────────────────────────────────┐
│              Preact Front-end  ▼                          │
│                                                          │
│   Chat · Swarm · Memory · Code · Skills · Settings      │
│   Streaming Chat · Wiki Notes · Device Management        │
│   Command Palette (⌘K) · i18n · Dark Mode               │
└──────────────────────────────────────────────────────────┘
```

---

## ✨ 功能亮点

### 🧠 记忆系统

5 层记忆架构，模拟人类记忆的层级递进：

| 层级 | 名称 | 说明 | 状态 |
|------|------|------|------|
| L0 | 缓存 | 最近访问 + 会话上下文（LRU, 64MB） | ✅ |
| L1 | 消息 | 对话/操作原始记录（7天保留） | ✅ |
| L2 | 经验 | 命名实体、概念关联 | ✅ |
| L3 | 事实 | 结构化知识 + 技能库 | ✅ |
| L4 | 知识 | 跨任务抽象 + 用户偏好 | ✅ |
| L5 | 教训 | 元认知反思与自我改进 | ⚠️ 预览 |

- **自动压缩**：低重要性记忆自动归档，保持系统轻盈
- **向量搜索**：LanceDB 驱动的语义检索 + SQLite 全文搜索
- **BM25 混合搜索**：关键词精确匹配 + 向量语义搜索的完美结合
- **访问控制**：Memory ACL 管理记忆的读写权限
- **实体抽取**：LLM 驱动的实体与关系自动发现

### 📚 LLM Wiki

将对话自动编译为结构化知识：

- **智能编译**：每次对话完成后自动生成 Wiki 笔记
- **双向链接**：`[[笔记名]]` 语法创建知识网络
- **可编辑**：Markdown 视图直接编辑，修改后自动重新向量化
- **索引维护**：自动生成 `index.md` 和 `log.md`，追踪知识演化
- **知识卡片**：点击链接查看完整知识卡片，包含定义、相关实体和来源

### 🐝 蜂群 (Swarm)

多 Agent 协作完成复杂任务：

- **6 种 Agent**：Coder / Writer / Reviewer / Researcher / Planner / Generic
- **AgentBus 消息总线**：点对点 + 广播通信
- **Negotiator 协商**：置信度投票 + LLM 仲裁 + 降级策略
- **动态 Agent 池**：按任务复杂度动态调整 Agent 数量
- **函数调用**：原生 LLM Function Calling 支持

### 🔐 安全与隐私

- **SSRF 防护**：拦截对内网地址的请求
- **注入检测**：扫描 Prompt 注入和凭证泄露
- **Shell 白名单**：仅允许预授权命令执行
- **E2EE 同步**：X25519 密钥交换 + AES-256-GCM 加密
- **设备管理**：配对设备注册与撤销
- **KeyVault**：OS Keychain 优先 + AES-256-GCM 文件降级

### 💰 成本优化

- **SemanticCache**：语义缓存层，相似查询直接返回，降低 70% Token 消耗
- **TokenJuice**：三级压缩策略，减少输入 Token 数量
- **ModelRouter**：智能路由到最优模型（本地/远程）
- **日预算限制**：超限自动切换到免费本地模型
- **成本追踪**：详细的 Token 费用统计与可视化

---

## 📦 安装

### 预编译包

前往 [Releases](https://github.com/AQWlala/nebula/releases) 下载最新版本：

| 平台 | 安装包 |
|------|--------|
| Windows x86_64 | `.msi` / `.exe` (NSIS) |
| macOS Apple Silicon | `.dmg` |
| macOS Intel | `.dmg` |
| Linux x86_64 | `.AppImage` |

---

## 🛠️ 开发

### 前置依赖

| 工具 | 版本 |
|------|------|
| Rust | 1.75+ |
| Node.js | 20+ |
| npm | 10+ |
| Ollama | latest (可选，运行时需要) |

### 快速开始

```bash
git clone https://github.com/AQWlala/nebula.git
cd nebula
npm install
npm run tauri:dev
```

### 构建

```bash
npm run tauri:build
```

### 测试

```bash
# Rust 测试
cd src-tauri && cargo test

# 前端测试
npm test

# E2E 测试
npm run test:e2e
```

---

## ⚙️ 配置

通过环境变量配置，常用项：

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `NEBULA_DB` | `nebula.db` | SQLite 数据库路径 |
| `NEBULA_LANCE` | `nebula_lance` | LanceDB 向量库路径 |
| `OLLAMA_URL` | `http://127.0.0.1:11434` | Ollama 服务地址 |
| `NEBULA_CHAT_MODEL` | `qwen2.5:3b` | 对话模型 |
| `NEBULA_EMBED_MODEL` | `BAAI/bge-small-zh-v1.5` | 嵌入模型 |
| `NEBULA_ANTHROPIC_KEY` | — | Anthropic Claude API Key |
| `NEBULA_ANTHROPIC_MODEL` | `claude-3-5-haiku-20241022` | Claude 模型名 |

---

## 🛡️ 技术栈

| 层 | 技术 |
|----|------|
| 后端 | Rust · Tauri 2.0 · Tokio · rusqlite · LanceDB · tonic (gRPC) |
| 前端 | Preact · TypeScript · Vite · Tailwind CSS · Monaco Editor · xterm.js |
| 安全 | X25519 · AES-256-GCM · Ed25519 · HKDF-SHA256 |
| AI | Ollama · Anthropic Claude · DeepSeek · 自定义 LLM Gateway |
| 同步 | E2EE · CRDT (LWW) |

---

## 📖 文档

| 文档 | 内容 |
|------|------|
| [用户指南](docs/USER_GUIDE.md) | 安装、配置、使用 |
| [开发者指南](docs/DEVELOPER_GUIDE.md) | 开发环境搭建、贡献流程 |
| [架构详解](docs/ARCHITECTURE.md) | 系统架构与设计决策 |
| [API 文档](docs/API.md) | 完整 API 参考 |
| [故障排查](docs/TROUBLESHOOTING.md) | 常见问题与解决方案 |
| [变更日志](CHANGELOG.md) | 版本变更记录 |

---

## 🤝 贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解详情。

---

## 📄 许可证

[MIT](LICENSE) © 2024-2026 nebula team

---

> **Nebula** — 你的知识，如星云般不断演化。
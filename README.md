<div align="center">

# 🌌 Nebula · 知识星云

**你的 AI 第二大脑，本地优先，与你的知识共同演化。**

[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Release](https://img.shields.io/badge/release-v2.0.0-blue)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/Rust-102K+-orange)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2.0-yellow)](https://tauri.app/)

**5 层记忆 · 6 Agent 蜂群 · E2EE 同步 · 本地推理 · MIT 开源**

</div>

---

> **云端 AI 越强大，你的思考越被托管。**
>
> 别把第二大脑，租给别人。

---

## 🌌 这不是又一个 ChatGPT 套壳

Nebula 是一款**本地优先**的 AI 助手，但它的真正野心不是"聊天"，而是 **记忆**。

- 你和它聊过的每一句话，它会**记住**
- 你写过的代码、调研过的资料，它会**沉淀**
- 你三个月前的灵感，它会**主动回忆**
- 它会**反思**自己的判断，**进化**自己的能力
- 所有这一切，**数据只属于你**——本地存储，端到端加密同步

**别人卖你 AI，Nebula 卖你对 AI 的主权。**

📖 完整产品哲学见 [营销白皮书](docs/WHITEPAPER_v2.0_marketing.md)

---

## 💔 你也有这些困扰吗？

| 痛点 | Nebula 的回答 |
|------|--------------|
| 🤯 **遗忘焦虑** — 三个月前的灵感像从未发生过 | L4/L5 记忆主动召回，"三年前的论证逻辑"也能引用 |
| 😰 **隐私让渡** — 把大脑租给 OpenAI/Notion | 本地存储 + E2EE + SQLCipher，私钥永不出设备 |
| 😩 **AI 失忆** — 每次对话重新解释你是谁 | 5 层演化记忆，AI 越用越懂你 |
| 🗂️ **笔记失控** — Notion/Obsidian 越用越乱 | LLM Wiki 双向链接 + 黑洞引擎自动压缩 |
| 💸 **Token 烧钱** — 跑一次复杂任务几十刀 | SemanticCache 降 70% Token + Ollama 本地零费用 |

---

## ✨ 四个让 Nebula 不同的理由

### 1. 🧠 5 层记忆架构 — 不是聊天记录，是会演化的记忆

业界大部分"AI 记忆"只是把历史对话塞回 Prompt。Nebula 不一样：

| 层级 | 名称 | 角色 | 状态 |
|------|------|------|------|
| **L0** | 缓存 | 当前会话上下文，毫秒级响应 | ✅ |
| **L1** | 消息 | 原始对话/操作流水 | ✅ |
| **L2** | 经验 | 实体关联、概念网络 | ✅ |
| **L3** | 事实 | 结构化知识 + 技能库 | ✅ |
| **L4** | 知识 | 跨任务抽象 + 用户偏好 + 价值对齐 | ✅ |
| **L5** | 教训 | **元认知反思，自我改进** ⚡ | ✅ |

**黑洞引擎**压缩低价值记忆，**海绵引擎**吸收高价值记忆——你的记忆像真正的星云，**有生有灭，自我演化**。

### 2. 🐝 蜂群协作 — 一个 Agent 不够，那就六个

复杂任务从来不是一个人能搞定的。Nebula 内置 6 种专业 Agent：

| Agent | 角色 | 典型场景 |
|-------|------|---------|
| 🎨 **Writer** | 撰写文档、博客、提案 | 技术博客、API 文档 |
| 💻 **Coder** | 实现代码、修复 Bug | 代码实现、PR 审查 |
| 🔍 **Reviewer** | 审查、校对、挑刺 | 事实核查、一致性检查 |
| 📚 **Researcher** | 调研背景、整理资料 | 主题调研 |
| 📋 **Planner** | 拆解任务、分配调度 | 任务分解 |
| 🛠️ **Generic** | 通用兜底 | 任意场景 |

通过 **AgentBus** 通信，由 **Negotiator** 协商置信度，必要时 LLM 仲裁——这是真正的**协作工作流**，不是简单的多 Agent。

### 3. 🔐 隐私优先 — 你的记忆，只属于你

| 安全能力 | 实现 |
|---------|------|
| **本地存储** | SQLite + LanceDB，数据默认不上云 |
| **E2EE 同步** | X25519 + HKDF-SHA256 + AES-256-GCM，私钥永不出设备 |
| **SSRF 防护** | 拦截对内网的请求 |
| **Prompt 注入检测** | 扫描恶意输入 |
| **Shell 白名单** | 仅允许预授权命令 |
| **KeyVault** | OS Keychain 优先 |
| **DB 加密** | SQLCipher 全库加密 |

**你不用把记忆上传到别人的服务器，也能跨设备同步。**

### 4. 💰 成本可控 — 不烧钱的 AI 才能用得起

| 优化手段 | 效果 |
|---------|------|
| **SemanticCache** | 语义缓存，**降 70% Token** |
| **TokenJuice** | 三级压缩 |
| **ModelRouter** | 智能路由，本地优先 |
| **日预算** | 超限自动切换免费本地模型 |
| **本地推理** | Ollama 跑 qwen2.5/deepseek，零 API 费用 |

---

## 🎯 谁在用 Nebula？

### 📝 写作者 — 让 AI 记住你的写作偏好

> "三年前我写过一篇关于分布式锁的文章，Nebula 不仅记得，还能在新文章里自动引用我当时的论证逻辑。"

### 💻 开发者 — 让 AI 记住你的代码风格

> "我对 Reviewer Agent 说'检查这个 PR'，它直接引用了我半年前的代码评审标准。它真的'懂'我。"

### 📚 研究者 — 让 AI 帮你构建知识网络

> "三个月前调研的论文，Nebula 自动整理成 Wiki 笔记，今天写论文时一搜就有，还带双向链接。"

### 🧠 知识工作者 — 让 AI 成为你的外脑

> "我把每天的灵感随手丢给 Nebula，半年后回看，它已经替我织出了一张知识星图。"

---

## 🚀 60 秒上手

```bash
git clone https://github.com/AQWlala/Nebula.git
cd Nebula
npm install
npm run tauri:dev
```

或下载预编译包：[Releases](https://github.com/AQWlala/Nebula/releases)

| 平台 | 安装包 |
|------|--------|
| Windows | `.msi` / `.exe` |
| macOS Apple Silicon | `.dmg` |
| macOS Intel | `.dmg` |
| Linux | `.AppImage` / `.deb` |

> **零配置开箱即用**。可选配置 Ollama（本地推理）或 DeepSeek/Anthropic API Key（远程模型）。不配置也能跑。

---

## ⚖️ 与同类产品对比

| 能力 | **Nebula** | ChatGPT | Notion AI | MemGPT | Supermemory | Obsidian+AI |
|------|-----------|---------|-----------|--------|-------------|-------------|
| **本地优先** | ✅ | ❌ | ❌ | 部分 | ❌ | ✅ |
| **5 层记忆** | ✅ + L5 反思 | ❌ | ❌ | 3 层 | ❌ | ❌ |
| **多 Agent** | ✅ 6 蜂群 | ❌ | ❌ | ❌ | ❌ | ❌ |
| **E2EE 同步** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **本地推理** | ✅ Ollama | ❌ | ❌ | ✅ | ❌ | 部分 |
| **LLM Wiki** | ✅ 双向链接 | ❌ | 部分 | ❌ | ❌ | 部分 |
| **开源** | ✅ MIT | ❌ | ❌ | ✅ | 部分 | 插件 |
| **数据归属** | **你** | OpenAI | Notion | 你 | 他们 | 你 |
| **月费** | **$0** | $20+ | $10+ | $0 | $10+ | $0+ |

> **核心差异**：别人卖你 AI，Nebula 卖你**对 AI 的主权**。

---

## 🧬 技术深度

```
┌──────────────────────────────────────────────────────────────────┐
│                       Tauri Shell (Rust)                          │
│                                                                  │
│   ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐      │
│   │  Memory   │  │   LLM    │  │  Swarm   │  │   Sync     │      │
│   │  L0 – L5  │  │  Gateway │  │  6 Agents│  │   E2EE     │      │
│   │ SQLite +  │  │ Ollama   │  │ AgentBus │  │ X25519 +   │      │
│   │ LanceDB   │  │ DeepSeek │  │Negotiator│  │ AES-GCM    │      │
│   │ 黑洞+海绵 │  │ Claude   │  │  Master  │  │  CRDT      │      │
│   └─────┬─────┘  └────┬─────┘  └────┬─────┘  └────────────┘      │
│         └─────────────┴─────────────┘                             │
│                        ▲                                          │
│               ┌────────┴────────┐                                 │
│               │    AppState     │  Security · ACL · Cost          │
│               └────────┬────────┘                                 │
└────────────────────────┼─────────────────────────────────────────┘
                         │ 257 Tauri Commands + 23 gRPC RPCs
┌────────────────────────┼─────────────────────────────────────────┐
│                Preact Frontend  ▼                                  │
│   Chat · Swarm · Memory · Code · Skills · Arena · Wiki            │
│   Streaming · DAG Canvas · Knowledge Cards · ⌘K Palette           │
└────────────────────────────────────────────────────────────────────┘
```

**一些数字**：
- **102,743 行 Rust 代码** · 287 个源文件
- **257 个 Tauri 命令** · 23 个 gRPC RPC
- **36 个 SQL 迁移** · 完整数据层演进
- **28 个场景模板** · 开箱即用的工作流
- **17+ Feature Flag** · 三种部署形态可裁剪

---

## 📖 文档

| 文档 | 内容 |
|------|------|
| [营销白皮书](docs/WHITEPAPER_v2.0_marketing.md) | 产品哲学、定位、竞品、商业模式 ⭐ |
| [技术白皮书](docs/WHITEPAPER_v2.0.md) | 完整技术架构与实现状态 |
| [用户指南](docs/USER_GUIDE.md) | 安装、配置、使用 |
| [架构详解](docs/ARCHITECTURE.md) | 系统架构与设计决策 |
| [开发者指南](docs/DEVELOPER_GUIDE.md) | 开发环境、贡献流程 |
| [路线图 v2.1](docs/ROADMAP_v2.1.md) | 下一版本规划 |
| [变更日志](CHANGELOG.md) | 版本变更记录 |

---

## 🗺️ 路线图

- ✅ **v2.0** — 5 层记忆 + 蜂群 + E2EE + LLM Wiki + Arena（已交付）
- 🔧 **v2.1** — Skill Marketplace Hub · 多模态视觉 · MCP 生态 · 双棘轮 E2EE
- 🔬 **v2.2** — Soul 灵魂编辑器 · 自主进化引擎 · Master Orchestrator
- 🌌 **v3.0** — 知识星图可视化 · 跨用户知识共享 · Agent 领导者选举

---

## 🤝 贡献

Nebula 是 MIT 开源项目，欢迎一切形式的贡献：

- ⭐ **Star** 这个仓库（让更多人看到）
- 🐛 [提交 Issue](https://github.com/AQWlala/Nebula/issues) 反馈问题
- 🔀 [发起 PR](https://github.com/AQWlala/Nebula/pulls) 改进代码
- 💬 分享你的使用场景到 [Discussions](https://github.com/AQWlala/Nebula/discussions)

**Good First Issue**：
1. `[i18n]` 补充日语/韩语翻译
2. `[docs]` 为 6 个 Agent 各写一篇使用案例
3. `[ui]` 给 DAG Canvas 添加节点右键菜单
4. `[test]` 为 SemanticCache 补充测试用例

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## 💭 Nebula 宣言

> 我们相信，AI 应该是大脑的延伸，不是大脑的替代。
>
> 当所有 AI 都在往"更大、更强、更云端"狂奔时，我们选择另一条路：
>
> **让 AI 回到本地，让记忆回到你的手中，让思考回到你的主权。**
>
> Nebula 不是 OpenAI 的复制品，不是 ChatGPT 的套壳。
> 它是你**真正拥有的**第二大脑——会记住、会思考、会进化，但**永远属于你**。
>
> 你的知识，如星云般不断演化。
>
> 这就是 Nebula。

---

## 📄 许可证

[MIT](LICENSE) © 2024-2026 Nebula Team

---

<div align="center">

**如果 Nebula 让你眼前一亮，给个 ⭐ 让更多人看到它。**

**你的知识，如星云般不断演化。** 🌌

</div>

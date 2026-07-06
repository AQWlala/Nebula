<div align="center">

# 🌌 Nebula · 知识星云

**本地优先的 AI 第二大脑——记忆、人格与智能体，全部装进你的设备。**

> 别把第二大脑租给别人——你的思考，不该成为别人的养料。

[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Release](https://img.shields.io/badge/release-v2.0.1-blue)](CHANGELOG.md)
[![Rust](https://img.shields.io/badge/Rust-102K+-orange)](https://www.rust-lang.org/)
[![Tauri](https://img.shields.io/badge/Tauri-2.0-yellow)](https://tauri.app/)
[![Local First](https://img.shields.io/badge/local-first-100%25%20offline-purple)](#)

**6 层记忆 · 双主控 + 蜂群 worker · E2EE 同步 · 本地推理 · 进化引擎 · MIT 开源**

</div>

---

## 🪐 Why Nebula

> 你的灵感，不该是别人模型的预训练集。

云端 AI 越强大，你的思考越被托管。每一句对话、每一个灵感、每一次反思，都在为别人的模型添砖加瓦——而你自己，反而要花钱订阅才能再把它问出来。

这不是进步，这是一场缓慢的**让渡**：把记忆让渡给服务器，把风格让渡给 Prompt，把思考的节奏让渡给一个你无法审计的黑盒。

Nebula 选择另一条路：

- 你的每一句话，落盘在**你自己的硬盘**
- 你的每一次推理，跑在**你自己的 CPU**
- 你的人格印记，写在**你可以打开的文件里**
- 你的进化轨迹，**只属于你**

> 把思考留在本地，把进化交给自己。

---

## ✨ Features

> Nebula 是一个本地优先的 AI 第二大脑。它把记忆、人格与智能体装进同一台设备——所有数据落盘在你自己的硬盘，所有思考在你自己的 CPU 上完成。

### 本地优先 · 0 字节上行

默认离线可用，端到端加密同步可选。SQLite + LanceDB 落盘存储，SQLCipher 全库加密，私钥永不出设备。**不联网，也能用；要同步，才加密。**

为何这样设计？因为**数据归属权**不能依赖服务条款——只有数据物理上在你手里，主权才真正属于你。

### 分层记忆 · 六层结构

模拟人类记忆的深浅快慢，从毫秒级缓存到元认知反思，**有生有灭，自我演化**：

| 层级 | 名称 | 角色 |
|------|------|------|
| **L0** | 缓存 | 当前会话上下文，毫秒级响应 |
| **L1** | 消息 | 原始对话与操作流水 |
| **L2** | 经验 | 实体关联、概念网络 |
| **L3** | 事实 | 结构化知识与技能库 |
| **L4** | 知识 | 跨任务抽象 + 用户偏好 + 价值对齐 |
| **L5** | 教训 | 元认知反思，自我改进 ⚡ |

**黑洞引擎**压缩低价值记忆，**海绵引擎**吸收高价值记忆。你不需要手动整理，星云会自己呼吸。

### 人格可塑 · SOUL.md 注入 + 4阶段进化引擎

通过 `SOUL.md` / `AGENTS.md` / `TOOLS.md` 注入角色灵魂——**编辑文件即改变 Agent 行为，无需重编译**。人格是数据驱动的印记，不是写死在代码里的常量。

**4 阶段进化引擎**（Extract → Compile → Reflect → Soul）让 Agent 从你的使用中持续学习：
- **Phase 1 Extract**：从 L1 原始对话提取 L2 经验
- **Phase 2 Compile**：将 L2 经验编译为 L3 结构化事实
- **Phase 3 Reflect**：L2+L3 反思生成 L5 元认知教训
- **Phase 4 Soul**：将反思写入 SOUL.md，人格随使用进化

每次进化全程可审计、可回滚——进化日志记录每一步，段落级回滚让你随时撤销不该写进去的东西。

为何这样设计？因为性格不该被发版绑架。你想让回答更冷峻、更啰嗦、更引用密集，打开文件改两行就好。

### 多智能体协作 · 编排 + 人格 + 并行 worker

主星·编排者负责拆解与调度，化身·灵魂分身决定回答的腔调，星尘群并行铺开分头验证。分工不打架，置信度有协商。

### 多渠道接入 · 统一收件箱

原生支持 Telegram Bot、Discord Webhook、WebChat 三种渠道，通过统一收件箱管理所有来源的消息。跨渠道对话上下文不丢失——你在 Telegram 开的头，在 WebChat 接着聊，星云记得住。

> 所有渠道适配器都经过 SSRF 安全验证，速率限制内置，不会因为一条消息触发 API 封禁。

### 技能生态 · 可创建可进化可分享

内置技能引擎支持两种技能语言：**LLM 提示技能**（纯文本指导）和 **Python 沙箱技能**（子进程执行）。技能可以：
- **手动创建**：在 UI 中编写，或从 agentskills.io / ClawHub 导入
- **自动进化**：使用 5+ 次的低评分技能自动改进；复杂任务完成后自动沉淀新技能
- **分享发布**：一键发布到 GitHub Gist 或本地文件，兼容 agentskills.io 开放标准

### 无头部署 · 自托管 Docker

不只是桌面应用——`cargo build --features headless` 编译出无窗口的服务端二进制，支持：
- **Docker 部署**：一行 `docker-compose up` 启动，暴露 gRPC (50051) + REST API (8080)
- **REST API**：6 个核心端点（health/memories/skills/chat/swarm/memory/search）
- **gRPC**：22 个 RPC 涵盖 Memory/Chat/Swarm/Skill/Reflection/Health
- **系统服务**：支持 systemd / launchd / Windows Service 注册，开机自启

### 开源可审计 · 每一行都可追溯

MIT 协议，102K+ Rust 代码全部开源。36 个 SQL 迁移记录数据层演进，17+ Feature Flag 支持三种部署形态裁剪。**没有黑盒，没有遥测，没有暗箱。**

> 第二大脑的第一原则：它只属于你。

### 安全栈 · 竞品没有的完整防护

| 能力 | 实现 | 竞品对比 |
|------|------|---------|
| 本地存储 | SQLite + LanceDB，默认不上云 | 与 OpenHuman 一致 |
| E2EE 同步 | X25519 + HKDF-SHA256 + AES-256-GCM | **竞品无 E2EE** |
| SSRF 防护 | 拦截对内网的请求 | **竞品无** |
| Prompt 注入检测 | 扫描恶意输入，Phase 3 反思阶段也扫描 | **竞品无** |
| Shell 白名单 | 仅允许预授权命令 | **竞品无** |
| KeyVault | OS Keychain 优先 | 与 OpenHuman 一致 |
| DB 加密 | SQLCipher 全库加密 | **竞品无全库加密** |
| 进化可回滚 | SOUL.md 段落级回滚 + 进化日志 | Hermes 有 snapshot 但无段落级 |

---

## 🏛️ Architecture

> 两个大脑，一支军团。主星·编排者负责拆解思考的节奏，化身·灵魂分身决定回答的腔调——它的人格由 SOUL.md 注入，并随你的使用自进化。每个任务到来时，2–6 个星尘并行铺开，分头研究、分头验证、再汇成一份答案。不是一只万能的 AI，而是一支懂你的小队。

### 双主控 + 蜂群 worker + persona 自进化

```
                        ┌─────────────────────────┐
                        │   主星 · 编排者          │
                        │   MasterAgent            │
                        │   · DAG 任务拆解          │
                        │   · 调度与结果聚合         │
                        └────────────┬─────────────┘
                                     │
                ┌────────────────────┼────────────────────┐
                │                    │                    │
     ┌──────────▼──────────┐  ┌──────▼──────────┐  ┌──────▼──────────┐
     │  化身 · 灵魂分身      │  │  星尘群 (×2–6)   │  │  星尘群 (×2–6)   │
     │  GenericAgent        │  │  GenericAgent    │  │  GenericAgent    │
     │  + SOUL.md persona   │  │  无 persona      │  │  无 persona      │
     │  · 决定回答腔调        │  │  · 独立思考       │  │  · 独立思考       │
     │  · 随使用自进化        │  │  · 相互校验       │  │  · 相互校验       │
     └──────────────────────┘  └─────────────────┘  └─────────────────┘
                │
                ▼
     ┌──────────────────────────┐
     │  自进化层 · EvolutionEngine │
     │  persona 随使用自我迭代      │
     │  数据驱动，无需重编译         │
     └──────────────────────────┘
```

**四个核心概念：**

- **主星·编排者（MasterAgent）**：DAG 任务拆解、调度、结果聚合。决定"什么时候做什么"。
- **化身·灵魂分身（带 persona 的 GenericAgent）**：通过 `SOUL.md` / `AGENTS.md` / `TOOLS.md` 注入角色灵魂，可自定义。决定"以什么腔调做"。
- **星尘群（无 persona 的 GenericAgent）**：每个任务并行生成 2–6 个，独立思考、相互校验。决定"做得对不对"。
- **星魂（SOUL.md）+ 自进化层（EvolutionEngine）**：数据驱动的人格印记，编辑文件即改变行为；persona 随使用自我迭代，越用越懂你。

### 五层记忆图

```
┌──────────────────────────────────────────────────────┐
│  L5  教训   │  元认知反思 · 自我改进 · 进化引擎     ⚡  │
├──────────────────────────────────────────────────────┤
│  L4  知识   │  跨任务抽象 · 用户偏好 · 价值对齐      │
├──────────────────────────────────────────────────────┤
│  L3  事实   │  结构化知识 · 技能库                   │
├──────────────────────────────────────────────────────┤
│  L2  经验   │  实体关联 · 概念网络                   │
├──────────────────────────────────────────────────────┤
│  L1  消息   │  原始对话 · 操作流水                   │
├──────────────────────────────────────────────────────┤
│  L0  缓存   │  当前会话上下文 · 毫秒级响应            │
└──────────────────────────────────────────────────────┘
        ▲ 黑洞引擎压缩低价值         ▼ 海绵引擎吸收高价值
              ▲ 4阶段进化引擎：Extract → Compile → Reflect → Soul
```

### 技术栈

```
┌──────────────────────────────────────────────────────────────────┐
│                       Tauri Shell (Rust)                          │
│                                                                  │
│   ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐      │
│   │  Memory   │  │   LLM    │  │  Swarm   │  │   Sync     │      │
│   │  L0 – L5  │  │  Gateway │  │  Workers │  │   E2EE     │      │
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

**一些数字：**

- **102,743 行 Rust 代码** · 287 个源文件
- **257 个 Tauri 命令** · 23 个 gRPC RPC
- **36 个 SQL 迁移** · 完整数据层演进
- **28 个场景模板** · 开箱即用的工作流
- **17+ Feature Flag** · 三种部署形态可裁剪

**安全能力：**

| 能力 | 实现 |
|------|------|
| 本地存储 | SQLite + LanceDB，默认不上云 |
| E2EE 同步 | X25519 + HKDF-SHA256 + AES-256-GCM |
| SSRF 防护 | 拦截对内网的请求 |
| Prompt 注入检测 | 扫描恶意输入 |
| Shell 白名单 | 仅允许预授权命令 |
| KeyVault | OS Keychain 优先 |
| DB 加密 | SQLCipher 全库加密 |

**成本可控：**

| 优化手段 | 效果 |
|---------|------|
| SemanticCache | 语义缓存，降 70% Token |
| TokenJuice | 三级压缩 |
| ModelRouter | 智能路由，本地优先 |
| 日预算 | 超限自动切换免费本地模型 |
| 本地推理 | Ollama 跑 qwen2.5/deepseek，零 API 费用 |

---

## 🚀 Quick Start · 60 秒上手

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

## 🗺️ Roadmap

- ✅ **v2.0** — 6 层记忆 + 蜂群 worker + E2EE + LLM Wiki + Arena（已交付）
- ✅ **v2.0.1** — 渠道路由修复 + evolution_run 实现 + Git 仓库修复（地基修复版）
- 🔧 **v2.1** — OAuth 集成生态 · Skill 自动发现 · gRPC 标准协议 · 多模态视觉 · MCP 生态
- 🔬 **v2.2** — Honcho 辩证式建模 · Cron 三计时 · Skill 闭环进化 · Soul 灵魂编辑器
- 🌌 **v3.0** — 知识星图可视化 · 跨用户知识共享 · Agent 领导者选举

---

## ❓ FAQ

**Q：我必须联网才能用 Nebula 吗？**
A：不必。默认 100% 离线可用——本地存储 + 本地推理（Ollama）。只有开启跨设备同步或外部服务集成时才需要网络，且同步走端到端加密。

**Q：我的数据会传到哪里？**
A：0 字节上行是默认态。数据落盘在你自己的硬盘（SQLite + LanceDB）。若启用云同步，则经 E2EE 加密后才上传，私钥永不出设备，服务端无法解密。

**Q：Agent 的人格可以自定义吗？**
A：可以。打开 `SOUL.md` / `AGENTS.md` / `TOOLS.md`，编辑文件即改变 Agent 行为，无需重编译。4 阶段进化引擎还会将反思自动写入 SOUL.md，人格随使用进化——而且每一步都可回滚。

**Q：可以部署在服务器上吗？**
A：可以。`cargo build --features headless` 编译出无窗口服务端，支持 Docker 部署。gRPC (50051) + REST API (8080) 双协议接入，可注册为系统服务开机自启。

**Q：和云端 AI 工具的本质区别是什么？**
A：我们不卖你 AI 的使用权，我们卖你对 AI 的**主权**。云端工具的"记忆"在他们的服务器上，Nebula 的记忆在你的硬盘上——主权归属，不靠服务条款，靠物理位置。

**Q：和 OpenHuman / Hermes Agent / OpenClaw 相比有什么不同？**
A：Nebula 的差异化在于**最深记忆 + 最强安全 + 可审计可回滚**。6 层记忆比竞品的 2-3 层更深；E2EE+SQLCipher+SSRF+注入检测的完整安全栈是竞品没有的；进化引擎的段落级回滚和完整日志让自进化过程完全可审计。我们不追求 118+ OAuth 全覆盖，但 5 个核心服务（Gmail/GitHub/Notion/Obsidian/Feishu）的深度集成正在路上。

**Q：开源是真的开源吗？**
A：MIT 协议，102K+ 行 Rust 代码全部公开。没有"开源核心 + 闭源高级版"的把戏，没有遥测，没有暗箱。每一行代码、每一次迁移都可追溯。

**Q：本地推理性能够用吗？**
A：Ollama 跑 qwen2.5 / deepseek 在主流硬件上体验流畅；SemanticCache 降 70% Token 消耗；ModelRouter 智能路由，复杂任务才走远程模型，简单任务本地秒回。

---

## 🤝 Contributing

Nebula 是 MIT 开源项目，欢迎一切形式的贡献：

- ⭐ **Star** 这个仓库（让更多人看到）
- 🐛 [提交 Issue](https://github.com/AQWlala/Nebula/issues) 反馈问题
- 🔀 [发起 PR](https://github.com/AQWlala/Nebula/pulls) 改进代码
- 💬 分享你的使用场景到 [Discussions](https://github.com/AQWlala/Nebula/discussions)

**Good First Issue：**

1. `[i18n]` 补充日语/韩语翻译
2. `[docs]` 为蜂群 worker 工作流写一篇使用案例
3. `[ui]` 给 DAG Canvas 添加节点右键菜单
4. `[test]` 为 SemanticCache 补充测试用例

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## 📖 文档

| 文档 | 内容 |
|------|------|
| [营销白皮书](docs/WHITEPAPER_v2.0_marketing.md) | 产品哲学、定位、商业模式 ⭐ |
| [技术白皮书](docs/WHITEPAPER_v2.0.md) | 完整技术架构与实现状态 |
| [开发建议书 v1.0](docs/DEVELOPMENT_PROPOSAL_v1.0.md) | 代码差距分析 + 竞品对标 + 改进路径 |
| [用户指南](docs/USER_GUIDE.md) | 安装、配置、使用 |
| [架构详解](docs/ARCHITECTURE.md) | 系统架构与设计决策 |
| [开发者指南](docs/DEVELOPER_GUIDE.md) | 开发环境、贡献流程 |
| [路线图 v2.1](docs/ROADMAP_v2.1.md) | 下一版本规划 |
| [变更日志](CHANGELOG.md) | 版本变更记录 |

---

## 💭 Nebula 宣言

> 我们相信，AI 应该是大脑的延伸，不是大脑的替代。
>
> 当所有 AI 都在往"更大、更强、更云端"狂奔时，我们选择另一条路：
>
> **让 AI 回到本地，让记忆回到你的手中，让思考回到你的主权。**
>
> 它是你**真正拥有的**第二大脑——会记住、会思考、会进化，但**永远属于你**。
>
> 你的知识，如星云般不断演化。
>
> 这就是 Nebula。

---

## 📄 License

[MIT](LICENSE) © 2024-2026 Nebula Team

---

<div align="center">

**如果 Nebula 让你眼前一亮，给个 ⭐ 让更多人看到它。**

**你的知识，如星云般不断演化。** 🌌

</div>

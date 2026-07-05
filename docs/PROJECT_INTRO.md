# Nebula · 知识星云 — 项目介绍

> **本地优先的 AI 第二大脑——记忆、人格与智能体,全部装进你的设备。**
>
> 别把第二大脑租给别人——你的思考,不该成为别人的养料。

---

## 项目定位

**Nebula 是一个本地优先的 AI 第二大脑**。它把记忆、人格与智能体装进同一台设备——所有数据落盘在你自己的硬盘,所有思考在你自己的 CPU 上完成,默认 0 字节上行,端到端加密同步可选。

**一句话定位**：省钱的自主式知识型桌面 AI 伙伴——它记得你的一切知识（可读/可编辑/可追溯），帮你操作电脑（API+VLM 双模式+L4 审批），替你省 Token 钱（智能路由+三级压缩+Credits），6 级自主度按需选择（L0 补全→L5 无人值守），24/7 自动化（Cron+Trigger+Watch），而且一直陪在你桌面上（悬浮球+8 人格+语音）。

---

## 项目状态

**当前版本**：v2.0.0

| 维度 | 指标 |
|------|------|
| Rust 代码 | 102,743 行 / 287 个源文件 |
| 前端代码 | ~20,000 行 TypeScript/TSX / 70 个文件 |
| Tauri 命令 | 270 个 / 53 个命令模块 |
| 测试总计 | 1,500+（单元 + 集成 + E2E + 安全 + 性能基准） |
| SQL 迁移 | 36 个（完整数据层演进） |
| Feature Flag | 22 个（按需裁剪部署形态） |
| 文档 | 28 个 markdown / ~12,000 行 |
| 总代码量 | ~140K+ 行（Rust + TypeScript + 测试 + 文档） |

---

## 核心价值主张

### 1. 数据主权 · 0 字节上行

默认离线可用,端到端加密同步可选。SQLite + LanceDB 落盘存储,SQLCipher 全库加密,私钥永不出设备。**不联网,也能用；要同步,才加密。**

> 数据归属权不能依赖服务条款——只有数据物理上在你手里,主权才真正属于你。

### 2. 分层记忆 · 五层结构

模拟人类记忆的深浅快慢,从毫秒级缓存到元认知反思,**有生有灭,自我演化**：

| 层级 | 名称 | 角色 |
|------|------|------|
| **L0** | 缓存 | 当前会话上下文,毫秒级响应 |
| **L1** | 消息 | 原始对话与操作流水 |
| **L2** | 经验 | 实体关联、概念网络 |
| **L3** | 事实 | 结构化知识与技能库 |
| **L4** | 知识 | 跨任务抽象 + 用户偏好 + 价值对齐 |
| **L5** | 教训 | 元认知反思,自我改进 ⚡ |

**黑洞引擎**压缩低价值记忆,**海绵引擎**吸收高价值记忆。你不需要手动整理,星云会自己呼吸。

### 3. 人格可塑 · SOUL.md 注入

通过 `SOUL.md` / `AGENTS.md` / `TOOLS.md` 注入角色灵魂——**编辑文件即改变 Agent 行为,无需重编译**。人格是数据驱动的印记,不是写死在代码里的常量。

### 4. 多智能体协作 · 编排 + 人格 + 并行 worker

主星·编排者负责拆解与调度,化身·灵魂分身决定回答的腔调,星尘群并行铺开分头验证。分工不打架,置信度有协商。

### 5. 开源可审计 · 每一行都可追溯

MIT 协议,102K+ Rust 代码全部开源。36 个 SQL 迁移记录数据层演进,22 个 Feature Flag 支持三种部署形态裁剪。**没有黑盒,没有遥测,没有暗箱。**

---

## 信任三原则

> **核心宣言**：「你无法信任一段你无法阅读的记忆」

| 原则 | 含义 |
|------|------|
| **可读（Readable）** | 所有记忆以人类可读的 Markdown 渲染；LLM Wiki 编译输出；图谱/时间轴/Markdown 三视图 |
| **可编辑（Editable）** | 用户可任意修改记忆,AI 写入与人类编辑双向同步,每次编辑记录版本 |
| **可追溯（Traceable）** | 每条记忆携带 provenance（来源/时间/hash/修改链）,决策可回溯到具体记忆 |

**与同类工具的根本差异**：

同类工具的记忆多为黑盒向量库（用户无法阅读）、或可导出但单向（AI 写入、用户只读）、或 Append-only 历史（可追溯但不可编辑）。**Nebula 的记忆是可读+可编辑+可追溯的"信任记忆"**——把记忆主权交还给用户。

---

## 四大支柱

| 支柱 | 核心能力 | 关键实现 |
|------|---------|----------|
| **更省钱** | 事前预算 + 事中压缩 + 事后审计 | CostEngine + TokenJuice 三级压缩 + ModelRouter 智能路由 + Credits 计费 |
| **更智能** | 黑盒向量库 → 可读 Markdown Wiki | LLM Wiki 编译引擎 + 三视图 + 双向同步 + MDRM 5 维关系图谱 |
| **更贴合** | API + VLM 双模式电脑操作 | OS-Controller + 视觉 Agent + Shadow Workspace + 场景闭环 |
| **更快** | 冷启动 <3s + 首响 <500ms | L0 缓存 + 流式 IPC + Ollama 预热 + 悬浮球桌面形象 |

---

## 六大趋势落地

| 趋势 | 实现状态 |
|------|---------|
| **自主度滑块 L0-L5** | ✅ AutonomyLevel 6 档 + AutonomyRouter + L4 ApprovalGate + L5 后台 Evolution |
| **Shadow Workspace** | 🔧 snapshot/rollback 引擎 + git branch 隔离基础 |
| **视觉驱动 Agent** | 🔧 screenshots + image（vision feature）+ describe_screenshot 命令 |
| **Credits 计费** | ✅ CostTracker + CostPolicy + credits_overview 命令 + CreditsDashboard UI |
| **24/7 Automations** | ✅ triggers（file/message/store/watch/webhook）+ backup scheduler + Cron |
| **多端同源** | ✅ gRPC + REST API + CLI（clap）+ channels（Telegram/Discord/飞书）+ PWA |

---

## 技术栈

### 桌面框架
- **Tauri 2.0**（tray-icon）+ 8 个 Tauri 插件（shell/fs/dialog/clipboard/notification/autostart/global-shortcut/updater）
- 应用标识：`com.nebula.desktop`,version `2.0.0`
- 窗口：1200×800（最小 800×600）

### 后端（Rust,edition 2021,rust-version 1.75）
- **异步运行时**：tokio 1.35（rt-multi-thread/macros/time/io-util/sync/process/fs/signal/net）
- **数据库**：rusqlite 0.31（bundled）+ 可选 SQLCipher 全库加密
- **向量存储**：lancedb 0.31 + arrow-array/schema 58.0（可选,默认开）
- **HTTP 客户端**：reqwest 0.12（json + stream）
- **加密栈**：aes-gcm 0.10 + x25519-dalek 2.0 + hkdf 0.12 + sha2 0.10
- **DAG**：petgraph 0.6（default-features=false）
- **gRPC**：tonic 0.12 + prost 0.13 + tonic-health 0.12
- **Keychain**：keyring 3（macOS apple-native / Windows windows-native / Linux sync-secret-service）
- **WASM 沙箱**：wasmtime 24 + wasmtime-wasi 24（可选）
- **可观测性**：tracing 0.1 + tracing-subscriber 0.3 + prometheus 0.13 + axum 0.7 + OpenTelemetry（可选）
- **文档处理**：pdf-extract 0.7 + docx-rs 0.4

### 前端
- **框架**：Preact 10.22 + @preact/signals 1.2 + @preact/preset-vite 2.8
- **构建工具**：Vite 5.0 + TypeScript 5.3 + tailwindcss 3.4
- **编辑器**：Monaco Editor 0.50
- **可视化**：mermaid 11.0 + pixi.js 8.19 + highlight.js 11.9
- **Markdown**：marked 12.0 + dompurify 3.4
- **搜索**：fuse.js 7.0（模糊搜索）
- **终端**：xterm 5.3 + xterm-addon-fit 0.8

---

## 架构概览

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
     │  数据驱动,无需重编译         │
     └──────────────────────────┘
```

### 五层记忆图

```
┌──────────────────────────────────────────────────────┐
│  L5  教训   │  元认知反思 · 自我改进            ⚡  │
├──────────────────────────────────────────────────────┤
│  L4  知识   │  跨任务抽象 · 用户偏好 · 价值对齐      │
├──────────────────────────────────────────────────────┤
│  L3  事实   │  结构化知识 · 技能库                  │
├──────────────────────────────────────────────────────┤
│  L2  经验   │  实体关联 · 概念网络                  │
├──────────────────────────────────────────────────────┤
│  L1  消息   │  原始对话 · 操作流水                  │
├──────────────────────────────────────────────────────┤
│  L0  缓存   │  当前会话上下文 · 毫秒级响应           │
└──────────────────────────────────────────────────────┘
        ▲ 黑洞引擎压缩低价值         ▼ 海绵引擎吸收高价值
```

---

## 8 大核心模块

| 模块 | 职责 |
|------|------|
| **lib.rs** | crate 入口,29 个 pub mod 声明,组织 AppState 协作 |
| **llm/dispatcher.rs** | ADR-003 核心 — UnifiedModelDispatcher 单一入口,7 个 WorkType,双维度成本统计 |
| **evolution/** | 闭环 agent 级自进化循环 — 4 Phase pipeline（L1→L2→L3→L5→SOUL.md）+ 三层共存 |
| **soul/** | M1 Soul 系统 — SoulCompiler 6 Step + 双扫描 + 原子写入 + CompiledSoul 输出 |
| **memory/** | 5 层记忆系统核心 — SQLite + LanceDB + 黑洞压缩 + 海绵吸收 + L5 元认知反射 |
| **swarm/** | 多智能体编排 — DynamicAgentPool + LeaderElector + Negotiator + TaskDag + MasterOrchestrator |
| **autonomy/** | 6 档自主度 L0-L5 + ApprovalGate 审批门 + WorkerRiskMap 三级风险 |
| **security/** | L4 价值层 + MemoryAcl deny-all + injection_guard + ssrf_guard + E2EE |

---

## 安全能力

| 能力 | 实现 |
|------|------|
| 本地存储 | SQLite + LanceDB,默认不上云 |
| E2EE 同步 | X25519 + HKDF-SHA256 + AES-256-GCM + 双棘轮 |
| SSRF 防护 | SsrfGuard 拦截对内网的请求（26 处缺口已修复） |
| Prompt 注入检测 | full_injection_scan（Critical/High 拦截,Low/Medium 记日志） |
| Shell 白名单 | 仅允许预授权命令（regex 匹配） |
| KeyVault | OS Keychain 优先（macOS Keychain / Windows DPAPI / Linux libsecret） |
| DB 加密 | SQLCipher 全库加密（bundled-sqlcipher-vendored-openssl） |
| L4 价值层 | ConstitutionalAI + RiskAssessor + PrivacyGuard + ValuePredictor |
| Plan 准奏 | 高风险操作需用户审批,5min 超时 |
| 文件快照回滚 | Skill 执行前快照工作区,失败后回滚 |

---

## 成本可控

| 优化手段 | 效果 |
|---------|------|
| SemanticCache | 语义缓存,降 70% Token |
| TokenJuice | 三级压缩（脱敏 / HTML→MD / 摘要） |
| ModelRouter | 智能路由,本地优先（简单→Ollama,中等→DeepSeek,复杂→Claude） |
| 日预算 | 超限自动切换免费本地模型 |
| 本地推理 | Ollama 跑 qwen2.5/deepseek,零 API 费用 |
| WorkType 路由 | Evolution/SoulCompile/Classifier 强制本地（零远端成本） |

---

## 快速开始

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

## 架构决策（ADR）

| ADR | 标题 | 核心决策 |
|-----|------|----------|
| ADR-001 | MasterOrchestrator 组合模式 | 编排者委托蜂群执行 fan-out,不重复实现 Worker 池 |
| ADR-002 | TaskDag + petgraph | 用 petgraph 表达任务 DAG,零手写拓扑排序 |
| ADR-003 | UnifiedModelDispatcher | 所有 LLM 调用经统一入口,按 WorkType 路由 + 双维度成本统计 |
| ADR-004 | Feature Flag 策略 | v2.0 新能力默认 off,编译期 + 运行时双层 gate |

---

## 已实现能力一览

| 能力域 | 已交付 |
|--------|--------|
| **记忆系统** | 5 层分层记忆 · 黑洞压缩 · 海绵吸收 · L5 元认知反思 · git 风格版本控制 · 因果图谱 · BM25+向量混合搜索 |
| **智能体** | 双主控编排 + 蜂群 worker（2-6 并行）· TaskDag 依赖编排 · 6 种 Agent 角色 · 结果协商与仲裁 |
| **人格系统** | SOUL.md 双分区（不可变+可进化）· SoulCompiler 6 步编译 · 注入扫描双保险 · 原子写入 |
| **自进化** | 4 Phase 进化管线（L1→L2→L3→L5→SOUL.md）· 三层共存（Worker/Skill/Master）· 进化日志与回滚 |
| **模型调度** | UnifiedModelDispatcher 单一入口 · 7 种 WorkType 路由 · 双维度成本统计 · 语义缓存 · 智能路由 |
| **自主度** | 6 档 L0-L5 · ApprovalGate 审批门 · 三级风险分级 · L5 后台例外 |
| **安全** | E2EE 同步 · SQLCipher 加密 · SSRF 防护 · Prompt 注入检测 · L4 价值层 · deny-all ACL |
| **多端** | Tauri 桌面 · gRPC + REST API · CLI · Telegram/Discord/飞书 通道 · PWA |
| **自动化** | 文件/消息/存储/Webhook 触发器 · 定时备份 · Cron 调度 |
| **可观测** | tracing + OpenTelemetry · Prometheus 指标 · 诊断面板 |

---

## 文档导航

| 文档 | 内容 |
|------|------|
| [WHITEPAPER_v3.1.md](docs/WHITEPAPER_v3.1.md) | **创新白皮书 + 实施总结** ⭐ |
| [WHITEPAPER_v2.0.md](docs/WHITEPAPER_v2.0.md) | 基础架构权威 |
| [CHANGELOG.md](docs/CHANGELOG.md) | 版本变更日志 |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | 系统架构与设计决策 |
| [USER_GUIDE.md](docs/USER_GUIDE.md) | 安装、配置、使用 |
| [DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) | 开发环境、贡献流程 |
| [SECURITY_AUDIT_REPORT.md](docs/SECURITY_AUDIT_REPORT.md) | 安全审计报告 |

---

## FAQ

**Q：我必须联网才能用 Nebula 吗？**
A：不必。默认 100% 离线可用——本地存储 + 本地推理（Ollama）。只有开启跨设备同步时才需要网络,且走端到端加密。

**Q：我的数据会传到哪里？**
A：0 字节上行是默认态。数据落盘在你自己的硬盘（SQLite + LanceDB）。若启用云同步,则经 E2EE 加密后才上传,私钥永不出设备,服务端无法解密。

**Q：Agent 的人格可以自定义吗？**
A：可以。打开 `SOUL.md` / `AGENTS.md` / `TOOLS.md`,编辑文件即改变 Agent 行为,无需重编译。persona 还会随使用自我迭代。

**Q：和云端 AI 工具的本质区别是什么？**
A：我们不卖你 AI 的使用权,我们卖你对 AI 的**主权**。云端工具的"记忆"在他们的服务器上,Nebula 的记忆在你的硬盘上——主权归属,不靠服务条款,靠物理位置。

**Q：开源是真的开源吗？**
A：MIT 协议,102K+ 行 Rust 代码全部公开。没有"开源核心 + 闭源高级版"的把戏,没有遥测,没有暗箱。每一行代码、每一次迁移都可追溯。

**Q：本地推理性能够用吗？**
A：Ollama 跑 qwen2.5 / deepseek 在主流硬件上体验流畅；SemanticCache 降 70% Token 消耗；ModelRouter 智能路由,复杂任务才走远程模型,简单任务本地秒回。

---

## 贡献

Nebula 是 MIT 开源项目,欢迎一切形式的贡献：

- ⭐ Star 这个仓库（让更多人看到）
- 🐛 [提交 Issue](https://github.com/AQWlala/Nebula/issues) 反馈问题
- 🔀 [发起 PR](https://github.com/AQWlala/Nebula/pulls) 改进代码
- 💬 分享你的使用场景到 [Discussions](https://github.com/AQWlala/Nebula/discussions)

详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

---

## License

[MIT](LICENSE) © 2024-2026 Nebula Team

---

> **核心宣言**：「你无法信任一段你无法阅读的记忆」——Nebula 是唯一做到**可读+可编辑+可追溯+可审计+可加密**的本地优先 AI Agent,且 100% 完成全部设计落地。
>
> 你的知识,如星云般不断演化。🌌

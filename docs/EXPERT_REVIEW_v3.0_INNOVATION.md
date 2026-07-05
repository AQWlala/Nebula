# Nebula (nebula) · 7 专家创新审议报告 v3.0

## ——更快·更省钱·更智能·更贴合工作场景

**版本**：v3.1（创新审议版 + 国内大厂趋势补充）
**日期**：2026-07-03
**审议基线**：`WHITEPAPER_v2.0.md` + `ROADMAP_v2.1.md` + `EXPERT_REVIEW_v2.1.md` + 竞品最新版分析
**新增专家**：EA-6（UX与多渠道专家）、EA-7（协议与集成专家）
**竞品参考**：OpenClaw / Hermes(GPT-Runner) / Open WebUI / Dify / Reasonix / OpenAKit / OpenHuman / Mavis / Obsidian
**国内大厂参考**：智谱 AutoClaw/AutoGLM/GLM-PC / 月之暗面 Kimi K2.6 / Cursor / Notion AI

---

## 0. 竞品画像速览

| 竞品 | 定位 | 核心优势 | 与Nebula的差异 |
|------|------|---------|--------------|
| **OpenClaw** | 个人AI助手（本地优先） | 20+ 通信渠道、语音唤醒、Live Canvas、Skills生态、Gateway 守护进程 | Nebula有更深的记忆系统（8层）和价值层，但渠道仅4个 |
| **Hermes/GPT-Runner** | AI预设管理器 | `.gpt.md` 文件即预设、VSCode/CLI/Web 三端、团队共享 | Nebula有完整技能系统但缺预设文件化+版本控制 |
| **Open WebUI** | 自托管AI平台 | RAG 9向量库、插件体系(Filter/Action/Pipe/Tool)、Channels、日历、自动化、企业认证 | Nebula有更深的记忆但缺RAG管道、缺企业级特性 |
| **Dify** | LLM应用开发平台 | 可视化Workflow、100+模型提供商、50+内置工具、LLMOps、BaaS API | Nebula是桌面端，Dify是Web平台；Dify工作流编排远超Nebula蜂群 |
| **Reasonix** | 推理增强框架 | 深度推理链、思维树、反思式推理、语义缓存、费用管理、缓存命中率 | Nebula有L5反思但缺结构化推理链、缺费用管理 |
| **OpenAKit** | AI工具包 | MCP工具服务器、OpenAPI集成、可视化工作流、跨平台SDK | Nebula有MCP骨架但缺OpenAPI工具服务器、缺可视化工作流 |
| **OpenHuman** | 数字人助手 | 电脑管理、UI自动化、屏幕感知 | Nebula有OS-Controller规划但未实现 |
| **Mavis** | 桌面AI伙伴 | 桌面形象化、悬浮交互、状态感知 | Nebula是传统窗口模式，无桌面形象 |
| **Obsidian** | 本地知识库 | 双向链接、图谱可视化、本地Markdown、社区插件 | Nebula有记忆系统但不是知识库、缺双向链接和图谱 |
| **智谱 AutoClaw/AutoGLM** | 桌面AI操作员 | "Every PC, 1 Minute"、50+步长链操作、跨App执行、CogAgent-9B开源视觉模型 | Nebula有OS-Controller规划但无视觉能力、无长链操作 |
| **Kimi K2.6** | 多模态Agent | Agent Swarm集群、Deep Research、Kimi Claw机器人、桌面端+浏览器插件 | Nebula有蜂群但缺深度研究、缺移动端 |
| **Cursor** | AI编程Agent | Cloud Agents自主计算机、Automations定时触发、Design Mode视觉指令、Marketplace、iOS端+Slack+CLI | Nebula无后台自动化、无视觉指令、无市场 |
| **Notion AI** | 工作空间AI | Custom Agents 24/7无人值守、Enterprise Search跨应用搜索、AI Meeting Notes、Credits计费 | Nebula无定时任务、无跨应用搜索、无会议纪要、无计费体系 |

---

## 1. EA-1 首席架构师：更快 —— 性能与缓存革命

### 1.1 痛点

用户等 AI 回复时，80% 的请求其实可以不走 LLM。当前Nebula每次对话都调 LLM，即使问题与上次几乎相同。

### 1.2 Reasonix 的启发：语义缓存 + 缓存命中率

Reasonix 的核心不是"推理链"，而是**让重复推理不再发生**：

- 语义缓存：新请求与历史请求语义相似度 >0.92 时直接返回缓存结果
- 缓存命中率仪表盘：实时显示"省了多少 Token / 省了多少钱"
- 缓存穿透报警：命中率 <30% 时提示用户优化 prompt

### 1.3 Nebula的创新方案：三层缓存架构

```
用户请求
  │
  ▼
┌─────────────────────────────────┐
│ L0 Exact Cache (当前已有)        │  ← 精确匹配，命中率 5%
│ LRU 256条，key=model+messages    │
└──────────┬──────────────────────┘
           │ miss
           ▼
┌─────────────────────────────────┐
│ L0.5 Semantic Cache (新增)       │  ← 语义匹配，命中率 35%+
│ embed(query) → LanceDB 近邻搜索  │
│ 阈值 cosine > 0.92 → 直接返回    │
│ TTL 1h，自动过期                  │
└──────────┬──────────────────────┘
           │ miss
           ▼
┌─────────────────────────────────┐
│ LLM Gateway (现有降级链)          │  ← 真正调 LLM
│ Ollama → DeepSeek → Anthropic   │
└─────────────────────────────────┘
```

**关键创新**：L0.5 语义缓存复用现有 LanceDB 基础设施，**零新增依赖**。SpongeEngine 的 `search_with_graph()` 已经能做语义搜索，只需在 LlmGateway 入口加一层"查缓存 → 命中则返回"的短路逻辑。

### 1.4 具体任务

| 任务 | 描述 | 复杂度 | 预期收益 |
|------|------|--------|---------|
| SemanticCache 层 | `LlmGateway::chat()` 入口加 `semantic_cache.check(embed(query))` | S | 重复问题 0 Token 消耗 |
| 缓存命中率仪表盘 | Dashboard 新增"缓存命中率"卡片 + "已省 Token 数" + "已省金额" | S | 用户可感知省钱效果 |
| 智能预取 | 用户打开文件时，预取该文件相关的历史对话缓存 | M | 文件切换时秒级响应 |
| Prompt Caching | Anthropic/OpenAI 的 prompt caching API 集成，长 system prompt 只计一次 | M | 长上下文场景省 90% |

**预期效果**：日常使用中 30-40% 的请求被语义缓存命中，Token 消耗降低 30%+，响应速度从"等 LLM 2-5s"变为"缓存命中 <50ms"。

### 1.5 Gateway 守护进程模式 → 借鉴 OpenClaw

**OpenClaw 的 Gateway 模式**：

- `openclaw onboard --install-daemon` 将 Gateway 注册为 launchd/systemd 用户服务
- `openclaw gateway status` 实时监控
- 后台常驻，前端/渠道/技能都通过 Gateway 通信

**Nebula现状**：

- Tauri 单进程模型，应用关闭即停止
- Sidecar 骨架存在但 `start_all` 未自动触发
- 无守护进程概念

**借鉴建议**：

- **P1** | 新增 `nebula gateway` 子命令，将 Tauri 核心逻辑抽为可独立运行的守护进程
- **P1** | SidecarManager 在 bootstrap 时自动 `start_all()`，并注册为系统服务（Windows Service / launchd / systemd）
- **P2** | 引入 OpenClaw 的 `openclaw doctor` 健康检查模式，一键诊断配置/权限/连接问题

### 1.6 可视化工作流编排 → 借鉴 Dify

**Dify 的 Workflow**：

- 可视化画布拖拽构建 AI 工作流
- 节点类型：LLM / 知识检索 / 代码 / 条件分支 / 迭代
- 支持调试、版本管理、发布

**Nebula现状**：

- 蜂群 `execute()` 是代码硬编码的 fan-out + negotiate
- 无可视化编排能力
- Plan 模式是状态机，非工作流

**借鉴建议**：

- **P2** | 在 SwarmView 中引入简化版工作流画布（基于 React Flow / xyflow）
- **P3** | 定义 `WorkflowSpec` YAML 格式，与 Dify DSL 互操作
- **P3** | SwarmOrchestrator 支持 DAG 执行模式（当前仅 fan-out）

### 1.7 多进程架构演进 → 借鉴 OpenClaw Sidecar + Open WebUI Terminal

**OpenClaw**：5 个 Sidecar 服务独立进程，进程内降级
**Open WebUI**：`open-terminal` 独立计算环境，`terminals` 企业版 per-user 隔离容器

**Nebula现状**：

- 3/5 Sidecar 仅 health_check，业务 RPC 未实现
- 单二进制多角色方案已决议但未落地

**借鉴建议**：

- **P1** | 加速 T-S4-B-01/02（Sidecar Skill/Reflection），补完 5/5 服务
- **P2** | 引入 Open WebUI 的 per-session 容器隔离概念，为 OS-Controller 做准备
- **P2** | Sidecar 健康检查从"固定端口猜测"升级为 gRPC HealthCheck 协议

---

## 2. EA-2 记忆系统专家：更智能 —— 本地知识库 + Obsidian 模式

### 2.1 痛点

用户的知识散落在：微信聊天记录、Obsidian 笔记、PDF 论文、代码仓库、浏览器书签。Nebula的记忆系统只记住了"和 AI 的对话"，没有记住"用户已有的知识"。

### 2.2 Obsidian 的启发：双向链接 + 图谱可视化 + 本地优先

Obsidian 之所以成为知识管理标杆，不是因为编辑器，而是因为：

1. **双向链接 `[[]]`**：笔记之间自动关联
2. **图谱视图**：知识网络可视化
3. **本地 Markdown**：数据永远在用户手里
4. **社区插件**：无限扩展

### 2.3 Nebula的创新方案：记忆即知识库

**核心理念**：Nebula的 8 层记忆系统不应该只存"对话记录"，而应该成为用户的**第二大脑**——自动从用户的工作中提取知识，构建双向链接的知识图谱。

```
┌──────────────────────────────────────────────────────┐
│                   Nebula知识库                          │
│                                                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐           │
│  │ 对话记忆  │  │ 文件知识  │  │ 网页知识  │           │
│  │ L0-L5    │  │ .md/.txt │  │ #url 注入 │           │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘           │
│       │             │             │                    │
│       └─────────────┼─────────────┘                   │
│                     ▼                                  │
│            ┌─────────────────┐                        │
│            │ EntityExtractor  │ ← 已有，5种RelationKind │
│            │ + 双向链接 [[]]   │ ← 新增：自动生成反向引用  │
│            └────────┬────────┘                        │
│                     ▼                                  │
│            ┌─────────────────┐                        │
│            │ CausalGraphEngine│ ← 已有                 │
│            │ + 知识图谱视图    │ ← 新增：Obsidian 风格   │
│            └─────────────────┘                        │
│                                                       │
│  输入源：                                               │
│  ├─ 对话自动提取 (已有)                                  │
│  ├─ 文件夹监控自动索引 (新增)                             │
│  ├─ Obsidian vault 导入 (新增)                          │
│  └─ 剪贴板/拖拽导入 (已有)                               │
└──────────────────────────────────────────────────────┘
```

### 2.4 具体任务

| 任务 | 描述 | 复杂度 | 创新点 |
|------|------|--------|--------|
| 文件夹监控索引 | 监控指定目录（如 Obsidian vault），文件变更时自动 `SpongeEngine::absorb_file()` | M | 知识自动入库，零手动操作 |
| 双向链接 `[[]]` | EntityExtractor 新增 `[[entity_name]]` 语法，自动生成反向引用 | M | 知识网络自组织 |
| 知识图谱视图 | MemoryMap 升级为 Obsidian 风格力导向图（D3 force-layout），节点=实体，边=关系 | L | 替代 SVG，支持 1000+ 节点 |
| Obsidian vault 兼容 | 直接读取 `.obsidian/` 配置 + Markdown 文件，双向同步 | M | 30M Obsidian 用户零迁移成本 |
| 知识卡片 | AI 回复中 `[[实体]]` 可点击，弹出知识卡片（定义+关联+来源） | M | 对话即知识浏览 |

**预期效果**：Nebula从"聊天工具"进化为"知识工作者的第二大脑"，这是任何纯聊天 AI（OpenClaw/Open WebUI）做不到的——它们没有 8 层记忆系统。

### 2.5 RAG 管道增强 → 借鉴 Open WebUI

**Open WebUI 的 RAG**：

- 9 种向量数据库（ChromaDB / PGVector / Qdrant / Milvus / ES / Pinecone / S3Vector / Oracle 23ai / OpenSearch）
- 混合搜索（BM25 + 向量）+ 重排序
- 9 种内容提取引擎（Tika / Docling / Document Intelligence / Mistral OCR / PaddleOCR-vl）
- `#` 命令直接注入文档到对话

**Nebula现状**：

- 仅 LanceDB 单一向量库
- 纯向量搜索，无 BM25 混合
- 无文档提取引擎（仅文本输入）
- 无 `#` 命令注入

**借鉴建议**：

- **P0** | 实现 BM25 + 向量混合搜索（Hybrid Search），当前纯向量在关键词精确匹配场景召回率低
- **P1** | 新增 `#` 命令语法，ChatPanel 解析 `#filename` 触发 `SpongeEngine::absorb_file()`
- **P1** | 集成至少 1 个文档提取引擎（推荐 `docling`，Rust 友好，可 WASM 化）
- **P2** | 向量库抽象层 `VectorStore trait`，支持切换 LanceDB / Qdrant / ChromaDB

### 2.6 持久化记忆增强 → 借鉴 Open WebUI + OpenClaw

**Open WebUI**：跨对话持久记忆，AI 自动记住用户偏好
**OpenClaw**：`SOUL.md` + `AGENTS.md` + `TOOLS.md` 注入 prompt，workspace skills

**Nebula现状**：

- 8 层记忆架构设计超前，但 L5 反思不写库（T-S1-A-06 已修复）
- MemoryOrchestrator 是孤儿模块（T-S1-A-02 已修复）
- 无 prompt 文件注入机制

**借鉴建议**：

- **P1** | 新增 `~/.nebula/workspace/SOUL.md` + `AGENTS.md` + `TOOLS.md` 注入机制（OpenClaw 模式），用户可自定义 AI 人格
- **P1** | L4 ValuesLayer 的宪法规则可从 `SOUL.md` 动态加载，而非硬编码
- **P2** | 记忆导出格式兼容 OpenClaw 的 workspace 格式，实现跨平台记忆迁移

### 2.7 结构化推理链 → 借鉴 Reasonix

**Reasonix**（推理增强框架）：

- 思维树（Tree of Thought）：多分支推理 + 剪枝
- 反思式推理（Reflection-based Reasoning）：推理后自我验证
- 推理链可视化

**Nebula现状**：

- L5 反思是"事后复盘"，非"推理中验证"
- CausalGraphEngine 有因果推理但无推理过程记录
- 无思维树/思维链结构化输出

**借鉴建议**：

- **P1** | 新增 `ReasoningChain` 结构体，记录每步推理的 `premise → inference → confidence → evidence`
- **P2** | 在 SwarmOrchestrator 中引入思维树模式：多 Agent 各走一条推理路径，Negotiator 比较路径质量
- **P2** | L5 反思引擎增加"推理中验证"模式：在 LLM 生成过程中插入自检点

---

## 3. EA-3 蜂群与AI工程师：更省钱 —— 费用管理 + 智能路由

### 3.1 痛点

用户不知道每次对话花了多少钱。简单问题用了 GPT-4 级模型，复杂问题却用了小模型。没有预算控制，月底账单吓人。

### 3.2 Reasonix 的启发：费用透明 + 智能路由

Reasonix 的费用管理不是"事后统计"，而是**事前决策**：

- 每次请求前评估：这个任务需要多大模型？
- 简单分类/提取 → 本地小模型（Qwen2.5-3B，免费）
- 复杂推理/创作 → 云端大模型（按需付费）
- 实时费用仪表盘：今日/本周/本月花费 + 预算预警

### 3.3 Nebula的创新方案：智能模型路由 + 费用管家

```
用户请求
  │
  ▼
┌─────────────────────────────────┐
│      ModelRouter (新增)          │
│                                  │
│  1. 任务分类器 (本地小模型)        │
│     ├─ 简单 (分类/提取/翻译)      │ → Ollama Qwen2.5-3B (免费)
│     ├─ 中等 (总结/改写/问答)      │ → DeepSeek (¥0.001/1K)
│     └─ 复杂 (推理/创作/代码)      │ → Claude/GPT-4 (¥0.03/1K)
│                                  │
│  2. 预算检查                      │
│     ├─ 日预算未超 → 放行           │
│     └─ 日预算已超 → 降级到本地模型  │
│                                  │
│  3. 语义缓存检查 (L0.5)           │
│     ├─ 命中 → 0 Token 消耗       │
│     └─ 未命中 → 按分类路由         │
└─────────────────────────────────┘
```

**关键创新**：用本地小模型做"任务分类器"，成本几乎为零，但能把 60%+ 的请求路由到免费/低价模型。

### 3.4 具体任务

| 任务 | 描述 | 复杂度 | 预期省钱 |
|------|------|--------|---------|
| ModelRouter 模块 | `LlmGateway` 前置路由层，本地小模型分类 → 按类别选模型 | M | 60% 请求走免费模型 |
| Token 费用追踪 | 每次 LLM 调用记录 input_tokens + output_tokens + model + cost | S | 费用可见化 |
| 费用仪表盘 | Dashboard 新增：今日花费 / 本月花费 / 预算进度条 / 费用趋势图 | M | 用户掌控成本 |
| 日预算限制 | `settings.json` 新增 `daily_budget_usd`，超限自动降级到 Ollama | S | 防止意外超支 |
| 费用报告 | `nebula cost report` 命令，输出本月各模型费用明细 | S | 月度对账 |

**预期效果**：日常使用中 60% 请求走本地免费模型，30% 走低价模型，仅 10% 走高价模型。月度 Token 费用降低 **70-80%**。

### 3.5 Agent 角色专业化 → 借鉴 Dify + Open WebUI

**Dify**：Agent 基于 Function Calling / ReAct，50+ 内置工具
**Open WebUI**：Models & Agents 系统，可包装基础模型 + 自定义指令 + 工具 + 知识

**Nebula现状**：

- 6 个 GenericAgent 并行，仅用序号 `i` 区分
- Coder/Writer/Reviewer 等角色已 deprecated
- 无工具调用能力（Function Calling）

**借鉴建议**：

- **P0** | 恢复 Agent 角色专业化，但采用 Dify 模式：每个角色绑定不同的 system_prompt + tool_set + knowledge_scope
- **P0** | 实现 LLM Function Calling 支持，当前蜂群只能"文本协商"不能"工具调用"
- **P1** | 引入 Open WebUI 的 Agent 预设导入机制，社区可共享 Agent 配置

### 3.6 动态 Agent 池 → 借鉴 OpenClaw Multi-Agent Routing

**OpenClaw**：多 Agent 路由，按渠道/账户/对等节点路由到隔离的 Agent（独立 workspace + session）

**Nebula现状**：

- DynamicAgentPool 已定义但 `&mut self` 不异步友好
- 每次执行固定 6 个 Agent

**借鉴建议**：

- **P1** | DynamicAgentPool 重构为 `Arc<tokio::sync::Mutex<DynamicAgentPool>>`（EA-3 之前已建议）
- **P1** | 引入 OpenClaw 的路由模式：按任务类型/渠道/用户自动选择 Agent 组合
- **P2** | Agent 数量从固定 6 个改为按任务复杂度动态调整（简单任务 2 个，复杂任务 8+ 个）

---

## 4. EA-4 安全与可观测性工程师：更贴合 —— 电脑管理 + OS-Controller

### 4.1 痛点

用户说"帮我打开项目里的那个报错文件"，AI 只能回答文字，不能真的操作电脑。OpenHuman 的理念是：AI 应该能像人一样操作电脑。

### 4.2 OpenHuman 的启发：AI 即电脑操作员

OpenHuman 的核心不是"聊天"，而是**AI 直接操作电脑**：

- 打开应用、切换窗口、点击按钮
- 读取屏幕内容、识别 UI 元素
- 执行系统命令、管理文件
- 全程可审计、可回滚

### 4.3 Nebula的创新方案：OS-Controller + 视觉感知

```
┌──────────────────────────────────────────────────┐
│              OS-Controller (独立 Sidecar)          │
│                                                   │
│  ┌──────────────┐  ┌──────────────┐              │
│  │ ScreenReader  │  │ UiAutomator  │              │
│  │ (截图+OCR+   │  │ (Windows UIA │              │
│  │  视觉理解)    │  │  /macOS AX   │              │
│  └──────┬───────┘  │  /Linux AT-SPI)│             │
│         │          └──────┬───────┘              │
│         ▼                 ▼                       │
│  ┌────────────────────────────────┐              │
│  │        ActionExecutor          │              │
│  │  ├─ click(x, y)               │              │
│  │  ├─ type(text)                │              │
│  │  ├─ screenshot() → 视觉理解    │              │
│  │  ├─ open_app(name)            │              │
│  │  └─ switch_window(title)      │              │
│  └────────────┬───────────────────┘              │
│               │                                   │
│  ┌────────────▼───────────────────┐              │
│  │     L4 价值层审批 (已有)         │              │
│  │  ├─ click → NeedsConfirm       │              │
│  │  ├─ type → Allow (白名单应用)   │              │
│  │  └─ delete → Forbidden          │              │
│  └────────────────────────────────┘              │
│                                                   │
│  审计日志 → skills/audit.rs (已有)                 │
│  回滚机制 → VersionControl (已有)                  │
└──────────────────────────────────────────────────┘
```

**关键创新**：OS-Controller 不是"远程桌面"，而是**AI 原生的电脑操作层**——每一步操作都经过 L4 价值层审批，每一步都有审计日志，每一步都可回滚。这是 OpenHuman 没有的安全深度。

### 4.4 具体任务

| 任务 | 描述 | 复杂度 | 创新点 |
|------|------|--------|--------|
| ScreenReader | 截图 + OCR + 视觉理解（本地小模型描述屏幕内容） | L | AI "看见" 屏幕 |
| UiAutomator | Windows UIA / macOS AX / Linux AT-SPI 统一抽象 | XL | 跨平台 UI 操作 |
| ActionExecutor | click/type/open/switch 原子操作 + L4 审批 | L | 每步操作都经过安全审批 |
| OS-Controller Sidecar | 独立进程运行，与主进程 IPC 通信 | L | 隔离高权限操作 |
| 操作录制回放 | 记录用户操作序列 → AI 可回放 | M | "看一遍就会" |

**预期效果**：用户说"帮我把这个 Excel 的数据整理一下"，AI 真的打开 Excel、选中数据、复制粘贴。不是"告诉你怎么做"，而是"帮你做"。

### 4.5 企业级认证 → 借鉴 Open WebUI

**Open WebUI**：

- LDAP/Active Directory 集成
- SSO（OAuth 提供商 + Trusted Headers）
- SCIM 2.0 自动化身份配置（Okta / Azure AD / Google Workspace）
- RBAC + 用户组

**Nebula现状**：

- MemoryAcl 默认 deny-all（已修复）
- 无用户认证系统（单用户桌面应用）
- REST API Bearer token + API key 双模式（T-S2-B-03a 已实现）

**借鉴建议**：

- **P2** | 如果未来走向多用户/团队版，需引入 Open WebUI 的 RBAC + 用户组模型
- **P2** | 当前单用户场景下，MemoryAcl 的 principal 概念可扩展为"设备级身份"（DID），为跨设备同步做准备
- **P3** | SCIM 2.0 支持作为企业版特性推迟

### 4.6 可观测性深度 → 借鉴 Dify + Open WebUI

**Dify**：LLMOps（日志分析 + 性能监控 + 标注 + 持续改进），集成 Opik / Langfuse / Arize Phoenix
**Open WebUI**：OpenTelemetry 原生支持（traces + metrics + logs），Usage Analytics 仪表盘

**Nebula现状**：

- Prometheus 17 项指标 + OTLP 导出
- 5 项指标缺口（T-S1-B-03 已修复）
- 无 LLMOps 级别的标注/持续改进

**借鉴建议**：

- **P1** | 引入 Dify 的"标注+持续改进"模式：用户可对 AI 回复标注"好/坏"，标注数据回流到 LLM 微调数据集
- **P1** | OpenTelemetry 从"导出"升级为"原生集成"：tracing span 覆盖全链路（chat → LLM → memory → swarm）
- **P2** | Usage Analytics 仪表盘增加 Token 成本趋势图、模型对比 A/B 测试（借鉴 Open WebUI Arena 模式）

---

## 5. EA-5 产品工程与质量经理：更形象 —— 桌面形象化 + Mavis 模式

### 5.1 痛点

Nebula现在是一个"窗口"，用户需要主动切换过去才能用。但真正好用的助手应该像同事一样——**一直在那里，需要时出现，不需要时不打扰**。

### 5.2 Mavis 的启发：桌面伙伴

Mavis 的理念：AI 不是工具，是伙伴。它有形象、有表情、有状态，让你觉得"有人在帮我"。

### 5.3 Nebula的创新方案：Nebula桌面形象

```
┌──────────────────────────────────────────────┐
│            Nebula桌面形象系统                    │
│                                               │
│  ┌────────────────────────────────────┐      │
│  │  桌面悬浮球 (默认形态)               │      │
│  │  ┌──┐                              │      │
│  │  │🐍│ ← 点击展开对话 / 拖拽到文件上  │      │
│  │  └──┘                              │      │
│  │  状态：🟢空闲 🟡思考 🔴执行 ⚡通知   │      │
│  └────────────────────────────────────┘      │
│                                               │
│  ┌────────────────────────────────────┐      │
│  │  浮动小窗 (对话形态)                 │      │
│  │  ┌──────────────────────┐          │      │
│  │  │ 🐍 正在帮你整理报告... │          │      │
│  │  │ ████████░░ 80%       │          │      │
│  │  │ [查看详情] [暂停]     │          │      │
│  │  └──────────────────────┘          │      │
│  └────────────────────────────────────┘      │
│                                               │
│  ┌────────────────────────────────────┐      │
│  │  全屏工作台 (深度工作形态)           │      │
│  │  = 现有 Tauri 主窗口               │      │
│  └────────────────────────────────────┘      │
│                                               │
│  交互方式：                                    │
│  ├─ 全局快捷键 ⌘+Shift+Space → 唤起悬浮球     │
│  ├─ 文件拖拽到悬浮球 → AI 分析文件             │
│  ├─ 选中文本 + 右键"问Nebula" → 上下文问答     │
│  └─ 剪贴板监听 → 自动识别可操作内容            │
└──────────────────────────────────────────────┘
```

**关键创新**：三种形态（悬浮球/浮动窗/全屏工作台）对应三种工作深度，不是"要么全开要么全关"。悬浮球是Nebula独有的——Tauri 2.0 的多窗口 API 原生支持。

### 5.4 具体任务

| 任务 | 描述 | 复杂度 | 创新点 |
|------|------|--------|--------|
| 悬浮球窗口 | Tauri 2.0 `WebviewWindowBuilder` 创建无边框透明悬浮窗 | M | 常驻桌面，一键唤起 |
| 状态指示器 | 悬浮球颜色/动画反映 AI 状态（空闲/思考/执行/通知） | S | 用户一眼知道 AI 在干嘛 |
| 文件拖拽交互 | 拖拽文件到悬浮球 → 自动 absorb + 分析 | M | 零门槛文件交互 |
| 右键菜单集成 | 系统右键菜单"问Nebula" → 选中文本直接提问 | M | 系统级集成 |
| 浮动进度窗 | AI 执行长任务时显示进度条 + 中断按钮 | S | 不阻塞用户工作 |

**预期效果**：Nebula从"需要打开的应用"变为"一直在的伙伴"，使用频率从"每天几次"提升到"每天几十次"。

### 5.5 插件/技能生态 → 借鉴 Open WebUI + Dify

**Open WebUI**：5 种插件类型（Filter / Action / Pipe / Tool / Skill），MCP / MCPO / OpenAPI 工具服务器
**Dify**：50+ 内置工具，可视化工具配置
**OpenClaw**：ClawHub 技能市场，`.agents/skills/` 目录

**Nebula现状**：

- SkillEngine + SkillStore + SkillAuditLogger
- agentskills.io 兼容（部分）
- TeamSkillsHub 未实现
- 无插件类型分层

**借鉴建议**：

- **P0** | 技能系统引入 Open WebUI 的 5 层插件模型：
  - **Filter**：请求/响应过滤器（如内容审查、格式化）
  - **Action**：用户触发的操作（如"翻译选中文字"）
  - **Pipe**：数据管道（如"将对话同步到 Notion"）
  - **Tool**：AI 可调用的工具（Function Calling）
  - **Skill**：复合能力（Tool + Prompt + Knowledge 组合）
- **P1** | 新增 MCP 工具服务器支持（当前 MCP 仅 JSON-RPC 帧，T-S2-B-02 已实现），需补完 `tools/list` + `tools/call` 的实际调用
- **P1** | 新增 OpenAPI 工具服务器支持（借鉴 Open WebUI），自动从 OpenAPI spec 生成 Tool 定义
- **P2** | ClawHub 兼容升级为双向：Nebula技能可导出为 ClawHub 格式

### 5.6 社区与分发 → 借鉴 OpenClaw + Open WebUI

**OpenClaw**：ClawHub.ai 技能市场 + npm/pnpm/bun 三包管理器
**Open WebUI**：Open WebUI Community 预设共享 + pip/Docker/K8s 多安装方式
**Dify**：Dify Cloud + Self-hosted + AWS Marketplace

**Nebula现状**：

- Tauri 单一安装方式
- 无技能市场
- 无云服务

**借鉴建议**：

- **P1** | 新增 `nebula skill publish` 命令，技能可发布到社区市场
- **P2** | 考虑提供 Docker 镜像（headless 模式），用于服务器部署
- **P3** | 云端中继同步（U-08）可参考 Dify Cloud 模式，提供托管版

---

## 6. EA-6 UX与多渠道专家：更可视 —— 工作流画布 + OpenAKit 模式

### 6.1 痛点

蜂群执行过程是黑盒——6 个 Agent 并行跑，用户只看到最终结果。不知道中间发生了什么，无法干预，无法调试。

### 6.2 OpenAKit 的启发：可视化工作流

OpenAKit 的核心：**AI 的工作过程应该是可见的、可编辑的、可重放的**。

### 6.3 Nebula的创新方案：蜂群工作流画布

```
┌──────────────────────────────────────────────────────┐
│                  蜂群工作流画布                         │
│                                                       │
│   ┌─────────┐                                        │
│   │ 用户任务 │                                        │
│   │ "写报告" │                                        │
│   └────┬────┘                                        │
│        │ L4 评估: Allow                               │
│        ▼                                              │
│   ┌─────────┐     ┌─────────┐     ┌─────────┐       │
│   │ Agent-1 │     │ Agent-2 │     │ Agent-3 │       │
│   │ Writer  │     │ Writer  │     │ Writer  │       │
│   │ temp=0.3│     │ temp=0.7│     │ temp=1.0│       │
│   │ ██████░ │     │ ███████ │     │ ████░░░ │       │
│   └────┬────┘     └────┬────┘     └────┬────┘       │
│        │               │               │              │
│        ▼               ▼               ▼              │
│   ┌──────────────────────────────────────────┐       │
│   │          Negotiator 协商                   │       │
│   │   置信度: 0.85 → 直接采纳 Agent-2          │       │
│   │   [查看差异] [手动选择] [LLM 仲裁]         │       │
│   └──────────────────────────────────────────┘       │
│                                                       │
│   用户可操作：                                         │
│   ├─ 点击 Agent 节点 → 查看详细输出                    │
│   ├─ 拖拽连接线 → 修改执行顺序                         │
│   ├─ 右键 → 添加/删除 Agent                            │
│   ├─ 双击 Negotiator → 切换仲裁策略                    │
│   └─ 保存为模板 → 下次复用                             │
└──────────────────────────────────────────────────────┘
```

**关键创新**：不是 Dify 那种"设计时编排"（用户画流程图再执行），而是**"运行时可视化"**——AI 自动执行，用户实时观看+干预。这是Nebula独有的——Dify/Open WebUI 没有蜂群协商机制，所以无法可视化协商过程。

### 6.4 具体任务

| 任务 | 描述 | 复杂度 | 创新点 |
|------|------|--------|--------|
| SwarmEvent 可视化 | SwarmView 接收 SwarmEvent 实时渲染为节点+连线 | L | 蜂群过程透明化 |
| 节点交互 | 点击 Agent 节点查看输出、点击 Negotiator 切换策略 | M | 用户可干预协商 |
| 工作流模板 | 保存执行图为 YAML 模板，下次直接复用 | M | 一次配置反复使用 |
| 执行回放 | 记录 SwarmEvent 时间线，支持回放/快进 | M | 事后复盘 |

**预期效果**：蜂群从"黑盒"变为"玻璃盒"，用户信任度大幅提升。

### 6.5 多渠道收件箱 → 借鉴 OpenClaw

**OpenClaw**：20+ 通信渠道（WhatsApp / Telegram / Slack / Discord / Signal / iMessage / Teams / Matrix / 飞书 / 微信 / QQ / WebChat...），统一收件箱

**Nebula现状**：

- 4 个渠道（桌面 WebView / Telegram 骨架 / Discord 骨架 / WebChat 骨架）
- 无 ChannelRouter 注入 AppState
- 无统一收件箱概念

**借鉴建议**：

- **P0** | 引入 OpenClaw 的统一收件箱模型：所有渠道消息汇入 `ChatPanel`，AI 统一回复
- **P1** | ChannelRouter 注入 AppState（P-12 已规划），实现渠道路由
- **P1** | 优先接入 Telegram（teloxide）和 Discord（serenity），这两个 SDK 成熟度最高
- **P2** | 飞书/微信/QQ 渠道作为中国市场差异化特性

### 6.6 语音交互 → 借鉴 OpenClaw

**OpenClaw**：Voice Wake（唤醒词）+ Talk Mode（连续语音）+ ElevenLabs TTS + 系统 TTS fallback
**Open WebUI**：语音/视频通话，多 STT/TTS 提供商

**Nebula现状**：

- 无语音交互

**借鉴建议**：

- **P2** | 新增 Voice Mode：Whisper STT + 系统 TTS，通过 Tauri 音频 API
- **P3** | Voice Wake 唤醒词作为 OS-Controller 的一部分

### 6.7 日历与自动化 → 借鉴 Open WebUI

**Open WebUI**：内置日历（月/周/日视图）+ AI 日程管理 + Automations（定时任务）+ 消息队列

**Nebula现状**：

- 无日历功能
- 无定时任务（Cron 在 ROADMAP 但未实现）
- 无消息队列

**借鉴建议**：

- **P1** | 新增 Calendar 组件（月/周/日视图），AI 可通过 Function Calling 管理日程
- **P1** | 实现 Cron 定时任务（ROADMAP 已有设计），与日历集成
- **P2** | 消息队列：当 AI 正在响应时，用户可排队发送消息（Open WebUI 的 Message Flow 模式）

---

## 7. EA-7 协议与集成专家：更闭环 —— 工作场景全链路打通

### 7.1 痛点

用户的工作不是孤立的"聊天"，而是一个完整链路：**需求 → 调研 → 方案 → 执行 → 交付**。当前Nebula只覆盖了"聊天"环节，其他环节断裂。

### 7.2 全链路工作场景

```
┌──────────────────────────────────────────────────────────┐
│                    Nebula工作场景闭环                       │
│                                                           │
│  ┌──────┐    ┌──────┐    ┌──────┐    ┌──────┐    ┌──────┐│
│  │ 需求  │───▶│ 调研  │───▶│ 方案  │───▶│ 执行  │───▶│ 交付  ││
│  │      │    │      │    │      │    │      │    │      ││
│  │语音/  │    │知识库 │    │工作流 │    │OS-Ctrl│    │渠道  ││
│  │剪贴板 │    │RAG   │    │画布   │    │代码   │    │导出  ││
│  └──────┘    └──────┘    └──────┘    └──────┘    └──────┘│
│                                                           │
│  贯穿始终：                                                │
│  ├─ 8层记忆：每个环节的上下文自动传递                       │
│  ├─ L4价值层：每个环节的安全审批                            │
│  ├─ 费用管家：每个环节的 Token 消耗透明                     │
│  └─ 审计日志：每个环节的操作可追溯                          │
└──────────────────────────────────────────────────────────┘
```

### 7.3 三大工作场景闭环

#### 场景一：写作者闭环

```
灵感(剪贴板/语音) → 素材收集(知识库RAG) → 大纲生成(蜂群协商)
→ 初稿撰写(Agent Writer) → 审校修改(Agent Reviewer) → 导出发布(渠道)
```

**创新点**：Obsidian 笔记自动成为 RAG 素材库，写作时 AI 自动引用用户自己的笔记。

#### 场景二：程序员闭环

```
需求描述(自然语言) → 代码搜索(知识库) → 方案设计(蜂群+Plan)
→ 代码生成(Agent Coder) → 自动测试(Agent Reviewer) → Git提交(OS-Controller)
```

**创新点**：代码仓库自动索引到知识库，AI 理解整个项目上下文而非单个文件。

#### 场景三：管理者闭环

```
会议纪要(语音转写) → 任务拆解(蜂群) → 日程安排(日历)
→ 执行跟踪(OS-Controller) → 进度汇报(渠道) → 复盘反思(L5)
```

**创新点**：L5 反思引擎自动从任务结果中学习，下次类似任务自动优化。

### 7.4 具体任务

| 任务 | 描述 | 复杂度 | 创新点 |
|------|------|--------|--------|
| 工作场景模板 | 预置 Writer/Coder/Manager 三套场景模板，一键切换 | M | 场景即产品 |
| 剪贴板智能监听 | 监听剪贴板，自动识别可操作内容（URL/代码/表格） | M | 零门槛输入 |
| 语音快速输入 | Whisper 本地转写 → 直接进入对话 | M | 语音即输入 |
| 一键导出 | 对话/报告/代码 → Markdown/PDF/DOCX/Git | M | 交付即导出 |
| 渠道发布 | 写完的报告直接发到飞书/Slack/邮件 | L | 写完即发布 |

### 7.5 MCP 工具服务器 → 借鉴 OpenAKit + Open WebUI

**OpenAKit**：MCP 工具服务器 + OpenAPI 集成
**Open WebUI**：MCP / MCPO / OpenAPI 工具服务器，插件可连接外部服务

**Nebula现状**：

- MCP JSON-RPC 帧已实现（T-S2-B-02）
- `discover_tools` / `invoke_tool` 是桩
- 无 OpenAPI 工具服务器

**借鉴建议**：

- **P0** | MCP `tools/list` + `tools/call` 必须补完真实实现，否则 MCP 形同虚设
- **P1** | 新增 OpenAPI 工具服务器：自动解析 OpenAPI 3.0 spec，生成 Tool 定义，AI 可直接调用 REST API
- **P1** | MCP stdio 子进程管理：自动发现并启动本地 MCP 服务器（如 filesystem / github / sqlite）
- **P2** | MCPO（MCP over HTTP）支持，允许远程 MCP 服务器

### 7.6 模型提供商生态 → 借鉴 Dify + Open WebUI

**Dify**：100+ 模型提供商，统一接口
**Open WebUI**：Ollama + OpenAI 兼容 API + LMStudio / GroqCloud / Mistral / OpenRouter / vLLM

**Nebula现状**：

- Ollama + DeepSeek + Anthropic 三家
- LlmGateway 有降级链但断路器失效（已修复）

**借鉴建议**：

- **P1** | LlmGateway 新增 OpenAI 兼容层：任何 OpenAI API 兼容的服务（vLLM / LMStudio / OpenRouter）都可直接接入
- **P1** | 模型配置从硬编码改为动态：`models.json` 配置文件，用户可自行添加提供商
- **P2** | 引入 Dify 的模型评估模式：Arena A/B 测试 + ELO 排行榜

### 7.7 数据库与存储弹性 → 借鉴 Open WebUI

**Open WebUI**：SQLite（可选加密）/ PostgreSQL，本地 / S3 / GCS / Azure Blob 存储

**Nebula现状**：

- SQLite（rusqlite bundled）+ LanceDB
- 无加密选项
- 无云存储

**借鉴建议**：

- **P1** | SQLite 加密选项（SQLCipher），与 E2EE 同步配合
- **P2** | 存储抽象层 `StorageBackend trait`，支持本地 / S3 / WebDAV
- **P3** | PostgreSQL 选项作为多用户/团队版特性

---

## 8. 七位专家共识：创新升级路线图

### 8.1 核心原则

> **不是做更多功能，而是做更少但更深的闭环。**
>
> 每个创新都必须同时满足：更快（可量化提速）、更省钱（可量化降本）、更智能（可感知的 AI 质量提升）、更贴合（可感知的工作场景贴合度）。

### 8.2 创新升级路线图

| 阶段 | 主题 | 核心创新 | 预期效果 |
|------|------|---------|---------|
| **v2.1** | 记忆闭环 (当前) | T-S1 全部任务 | 57% → 75% |
| **v2.2** | 协议+安全 | Stage 2a/2b | 75% → 88% |
| **v2.3** | **省钱革命** | 语义缓存 + 智能路由 + 费用管家 | Token 成本降 70% |
| **v2.4** | **知识革命** | Obsidian 兼容 + 文件夹索引 + 知识图谱 | 从"聊天"到"第二大脑" |
| **v2.5** | **形象革命** | 悬浮球 + 浮动窗 + 右键集成 | 使用频率 5x 提升 |
| **v2.6** | **可视革命** | 蜂群画布 + 工作流模板 + 执行回放 | 蜂群从黑盒到玻璃盒 |
| **v3.0** | **操作革命** | OS-Controller + ScreenReader + 场景闭环 | 从"告诉你"到"帮你做" |

### 8.3 量化目标

| 维度 | 当前 | v2.3 目标 | v3.0 目标 |
|------|------|----------|----------|
| **平均响应时间** | 2-5s (LLM) | <1s (40% 缓存命中) | <200ms (80% 本地) |
| **月度 Token 成本** | ~$30/月 | ~$9/月 (降 70%) | ~$3/月 (降 90%) |
| **日活跃次数** | 3-5 次 | 10-15 次 (悬浮球) | 30-50 次 (OS-Controller) |
| **知识覆盖** | 仅对话 | +本地文件 | +全工作场景 |
| **可操作范围** | 仅文本 | +文件操作 | +电脑操作 |

### 8.4 与竞品的终极差异化

| 维度 | OpenClaw | Open WebUI | Dify | **Nebula v3.0** |
|------|---------|-----------|------|---------------|
| 记忆深度 | 无层 | 持久记忆 | 无 | **8 层 L0-L7** |
| 费用管理 | 无 | Usage Analytics | 无 | **智能路由+预算控制** |
| 本地知识库 | 无 | RAG | RAG | **Obsidian 兼容+图谱** |
| 桌面形象 | 菜单栏 | Web UI | Web UI | **悬浮球+三形态** |
| 电脑操作 | 无 | Terminal | 无 | **OS-Controller+视觉** |
| 安全深度 | DM pairing | RBAC | 无 | **L4 价值层+审计+回滚** |
| 工作流可视化 | 无 | 无 | 画布(设计时) | **蜂群画布(运行时)** |

### 8.5 一句话定位

> Nebula v3.0 = **省钱的知识型桌面 AI 伙伴**——它记得你的一切知识，帮你操作电脑，替你省 Token 钱，而且一直陪在你桌面上。

---

## 9. 国内大厂桌面级 Agent 趋势调研（v3.1 新增）

### 9.1 调研对象与核心发现

| 产品 | 厂商 | 核心定位 | 关键创新 |
|------|------|---------|---------|
| **AutoClaw / GLM-PC** | 智谱 | "Every PC, 1 Minute" 桌面AI操作员 | AutoGLM 自主规划+50步长链操作+跨App执行+CogAgent-9B开源视觉模型 |
| **Kimi K2.6** | 月之暗面 | 多模态Agent+深度研究 | Agent Swarm集群+Deep Research+Kimi Claw机器人+桌面端 |
| **Cursor** | Anysphere | AI编程Agent | Cloud Agents+Automations+Design Mode+Marketplace+iOS端+Slack集成 |
| **Notion AI** | Notion | 工作空间AI | Custom Agents 24/7+Enterprise Search+AI Meeting Notes+Credits计费 |

### 9.2 趋势一：自主度滑块 —— 从辅助到自主的连续谱

**Cursor 的核心洞察**（Andrej Karpathy 评价）：

> "最好的 LLM 应用都有一个自主度滑块：Tab 补全 → Cmd+K 定向编辑 → 全自主 Agent 模式"

**Nebula现状**：只有"全自主"模式（6个Agent并行），没有"轻量辅助"模式。

**借鉴方案**：

```
自主度滑块 (Autonomy Slider)

Level 0: 内联补全     → 输入时自动建议（类似 Copilot Tab）
Level 1: 定向编辑     → 选中文字 + 指令 → 局部改写（类似 Cmd+K）
Level 2: 对话问答     → 当前 ChatPanel 模式
Level 3: Plan 模式    → 已有，高风险操作需审批
Level 4: 全自主 Agent → 当前蜂群模式
Level 5: 后台自动化   → 定时/触发器驱动的无人值守任务
```

**关键任务**：

| 任务 | 描述 | 优先级 | 复杂度 |
|------|------|--------|--------|
| Level 0 内联补全 | ChatPanel 输入框增加 AI 建议补全（调用本地小模型，零成本） | P0 | M |
| Level 1 定向编辑 | 选中文字 + 快捷键 → AI 局部改写（不进入完整对话） | P1 | M |
| Level 5 后台自动化 | Cron 定时任务 + 事件触发器（借鉴 Cursor Automations + Notion Custom Agents） | P1 | L |

### 9.3 趋势二：Cloud Agent / Shadow Workspace —— Agent 有自己的电脑

**Cursor Cloud Agents**：

- Agent 在云端独立计算机上运行，构建/测试/演示全流程
- 用户可以关闭电脑，Agent 继续工作
- 完成后提供录屏回放 + 摘要

**智谱 AutoGLM**：

- 50+ 步长链自主操作
- 跨 App 执行任务
- CogAgent-9B 开源视觉模型驱动

**Nebula现状**：所有操作在用户电脑上同步执行，用户必须等待。

**借鉴方案**：

```
Nebula Shadow Workspace（影子工作区）

┌─────────────────────────────────────┐
│  用户电脑 (主进程)                    │
│  ├─ 提交任务 → "帮我重构这个模块"     │
│  ├─ 继续其他工作...                   │
│  └─ 收到通知 → "任务完成，查看结果"   │
│                                      │
│  ┌─────────────────────────────┐    │
│  │  Shadow Workspace (隔离)    │    │
│  │  ├─ git checkout -b agent/xxx│    │
│  │  ├─ Agent 独立工作           │    │
│  │  ├─ 自动测试                 │    │
│  │  └─ 生成 diff + 录屏         │    │
│  └─────────────────────────────┘    │
└─────────────────────────────────────┘
```

**关键任务**：

| 任务 | 描述 | 优先级 | 复杂度 |
|------|------|--------|--------|
| Shadow Workspace | 任务执行在独立 git branch + 临时目录中，不影响用户当前工作 | P1 | L |
| 任务录屏回放 | 记录 Agent 操作序列（文件修改+命令执行），用户可回放审查 | P2 | M |
| 后台执行通知 | Agent 完成后通过系统通知 + 悬浮球状态变化提醒 | P2 | S |

### 9.4 趋势三：视觉驱动 Agent —— AI 看见屏幕、操作屏幕

**智谱 CogAgent-9B**：

- 仅需屏幕截图作为输入（无需 HTML）
- 9B 参数即可驱动 GUI 操作
- 已开源

**Cursor Design Mode**：

- 用户在 UI 上直接画框/标注，Agent 根据视觉提示操作
- "视觉即指令"

**Nebula现状**：OS-Controller 有规划但未实现，且无视觉能力。

**借鉴方案**：

```
视觉驱动 Agent 三层架构

Layer 1: 截图理解（轻量）
  └─ 本地小模型（Qwen2.5-VL-3B）描述屏幕内容
  └─ 输出："当前是 VSCode 编辑器，打开了 main.rs，光标在第 42 行"

Layer 2: UI 元素定位（中等）
  └─ Windows UIA / macOS AX 获取可交互元素树
  └─ 输出：[{type: "button", text: "Run", rect: {x:100, y:200, w:60, h:30}}]

Layer 3: 操作执行（重度）
  └─ click / type / scroll / screenshot 循环
  └─ 每步经过 L4 价值层审批
  └─ 每步记录审计日志
```

**关键任务**：

| 任务 | 描述 | 优先级 | 复杂度 |
|------|------|--------|--------|
| Layer 1 截图理解 | 集成 Qwen2.5-VL-3B（本地运行），`screenshot()` → 文字描述 | P1 | M |
| Layer 2 UI 元素定位 | UIA/AX 抽象层，获取可交互元素树 | P2 | XL |
| Layer 3 操作执行 | ActionExecutor + L4 审批闭环 | P3 | XL |
| Design Mode | 用户在 UI 上画框/标注 → Agent 根据视觉提示操作 | P3 | L |

### 9.5 趋势四：Credits 计费 + 费用透明化

**Notion Credits**：

- Custom Agents 按 credits 计费
- 管理员可在 dashboard 查看用量
- 信用不足时 Agent 自动暂停

**Cursor**：

- 按请求量计费（500次/月 Pro）
- 不同模型不同定价

**Nebula现状**：无费用追踪，用户对 Token 消耗无感知。

**借鉴方案**（与 EA-3 费用管家方案对齐，但增加 Credits 模式）：

```
Nebula Credits 系统

┌─────────────────────────────────────┐
│  Credits Dashboard                  │
│                                      │
│  本月预算: $20.00                    │
│  已使用:  $12.35 (61.7%)            │
│  ████████████░░░░░░░░                │
│                                      │
│  模型分布:                           │
│  ├─ Ollama (免费):    847 次 (62%)  │
│  ├─ DeepSeek ($0.001): 312 次 (23%) │
│  └─ Claude ($0.03):    201 次 (15%) │
│                                      │
│  费用明细:                           │
│  ├─ 对话: $4.20                      │
│  ├─ 蜂群: $5.80                      │
│  ├─ 反思: $1.35                      │
│  └─ 知识库索引: $1.00                │
│                                      │
│  预算预警:                           │
│  ├─ 日预算 $1.00 → 剩余 $0.23       │
│  └─ 月预算 $20.00 → 剩余 $7.65      │
└─────────────────────────────────────┘
```

**关键任务**：

| 任务 | 描述 | 优先级 | 复杂度 |
|------|------|--------|--------|
| Token 费用追踪 | 每次 LLM 调用记录 tokens + model + cost | P0 | S |
| Credits Dashboard | 费用仪表盘 + 预算进度条 + 模型分布 + 费用明细 | P1 | M |
| 日/月预算限制 | 超限自动降级到 Ollama，借鉴 Notion "信用不足自动暂停" | P1 | S |
| Automation Credits | 24/7 自动化任务独立计费，与对话费用分离 | P2 | M |

### 9.6 趋势五：Agent 即服务 —— 24/7 无人值守

**Notion Custom Agents**：

- 设置触发器或计划，Agent 24/7 自动运行
- 一个人构建，整个团队受益
- 按使用量计费（credits）

**Cursor Automations**：

- 定时任务：每天凌晨运行测试
- 触发器任务：PR 创建时自动审查
- 后台运行，完成后通知

**Nebula现状**：无定时任务，无事件触发器。

**借鉴方案**：

```
Nebula Automations

┌─────────────────────────────────────────┐
│  Automation 类型                         │
│                                          │
│  1. 定时任务 (Cron)                      │
│     ├─ 每天 9:00 → 生成昨日工作摘要       │
│     ├─ 每周五 → 整理本周知识库             │
│     └─ 每月 1号 → 费用报告                │
│                                          │
│  2. 事件触发 (Trigger)                    │
│     ├─ 文件变更 → 自动索引到知识库         │
│     ├─ 新消息到达 → 智能分类+摘要          │
│     └─ 代码提交 → 自动生成 changelog      │
│                                          │
│  3. 条件监控 (Watch)                      │
│     ├─ 网页价格变动 → 通知                 │
│     ├─ 日历提醒 → 准备会议材料             │
│     └─ 系统资源 >90% → 告警               │
│                                          │
│  执行环境：Shadow Workspace（隔离）        │
│  结果通知：悬浮球 + 系统通知 + 渠道推送     │
│  费用归属：Automation Credits 独立统计     │
└─────────────────────────────────────────┘
```

**关键任务**：

| 任务 | 描述 | 优先级 | 复杂度 |
|------|------|--------|--------|
| Cron 定时任务引擎 | `cron.rs` + SQLite 存储 + Sidecar 执行 | P1 | L |
| 事件触发器 | 文件监听 + 消息监听 + Webhook 接收 | P2 | M |
| 条件监控 | 网页抓取 + 系统指标 + 日历事件 | P2 | M |
| Automation 模板 | 预置常用自动化模板（日报/周报/费用报告） | P2 | S |

### 9.7 趋势六：多端同源 —— 桌面 + CLI + 移动 + 渠道

**Cursor**：Desktop IDE + CLI + Slack Bot + iOS App + GitHub PR Review
**Kimi**：Web + 桌面端 + 浏览器插件 + API
**Notion**：Web + 桌面端 + 移动端 + Calendar + Mail

**Nebula现状**：仅桌面 WebView + 4个渠道骨架。

**借鉴方案**：

```
Nebula多端架构

┌──────────────────────────────────────┐
│           Core Engine (Rust)          │
│  memory / swarm / llm / skills / ... │
└──────────┬───────────────────────────┘
           │
     ┌─────┴─────┬──────────┬──────────┐
     ▼           ▼          ▼          ▼
┌─────────┐ ┌────────┐ ┌────────┐ ┌────────┐
│ Desktop │ │  CLI   │ │ Mobile │ │  API   │
│ (Tauri) │ │(clap)  │ │(PWA)   │ │(gRPC)  │
└─────────┘ └────────┘ └────────┘ └────────┘
     │           │          │          │
     └─────┬─────┴─────┬────┴─────┬────┘
           ▼           ▼          ▼
     ┌─────────┐ ┌────────┐ ┌────────┐
     │Telegram │ │Discord │ │ 飞书    │
     └─────────┘ └────────┘ └────────┘
```

**关键任务**：

| 任务 | 描述 | 优先级 | 复杂度 |
|------|------|--------|--------|
| CLI 模式 | `nebula chat "帮我总结这个文件"` 命令行直接调用 | P1 | M |
| 移动端 PWA | 响应式布局 + Service Worker 离线缓存 | P2 | L |
| API 网关 | gRPC + REST 双协议，第三方可集成 | P2 | M |
| 浏览器插件 | 类似 Kimi 浏览器助手，选中文字右键"问Nebula" | P2 | M |

---

## 10. 整合路线图：v3.0 + 国内大厂趋势（v3.1 更新）

### 10.1 趋势汇总

| 趋势 | 优先级 | 对应 ROADMAP 扩展 | 预期效果 |
|------|--------|------------------|---------|
| **自主度滑块** | P0 | 新增 AutonomySlider 模块 | 从"全有全无"到"按需选择"，降低使用门槛 |
| **费用透明+Credits** | P0 | 扩展 Dashboard + ModelRouter | Token 成本降 70%，用户掌控预算 |
| **视觉驱动Agent** | P1 | 扩展 OS-Controller | AI 看见屏幕、操作屏幕 |
| **Shadow Workspace** | P1 | 新增 ShadowWorkspace 模块 | Agent 后台工作，用户不阻塞 |
| **24/7 Automations** | P1 | 扩展 Cron + Trigger | 无人值守自动化 |
| **多端同源** | P2 | 扩展 CLI + PWA + API | 从桌面应用到全平台 |

### 10.2 整合后的升级路线图

| 阶段 | 原主题 | 新增趋势 | 整合后主题 |
|------|--------|---------|-----------|
| **v2.1** | 记忆闭环 (当前) | 不变 | 记忆闭环 |
| **v2.2** | 协议+安全 | 不变 | 协议+安全 |
| **v2.3** | 省钱革命 | +Credits计费+自主度滑块L0-L1 | **省钱+低门槛革命** |
| **v2.4** | 知识革命 | 不变 | **知识革命** |
| **v2.5** | 形象革命 | +Shadow Workspace+后台通知 | **形象+后台革命** |
| **v2.6** | 可视革命 | +视觉驱动Agent(Layer1截图理解) | **可视+视觉革命** |
| **v3.0** | 操作革命 | +24/7 Automations+多端CLI | **全自主革命** |

### 10.3 更新后的量化目标

| 维度 | 当前 | v2.3 目标 | v3.0 目标 |
|------|------|----------|----------|
| **平均响应时间** | 2-5s (LLM) | <1s (40% 缓存命中) | <200ms (80% 本地) |
| **月度 Token 成本** | ~$30/月 | ~$9/月 (降 70%) | ~$3/月 (降 90%) |
| **日活跃次数** | 3-5 次 | 10-15 次 (悬浮球) | 30-50 次 (OS-Controller) |
| **知识覆盖** | 仅对话 | +本地文件 | +全工作场景 |
| **可操作范围** | 仅文本 | +文件操作 | +电脑操作 |
| **自主度等级** | 仅 Level 4 | Level 0-4 | Level 0-5 |
| **自动化任务** | 0 | 0 | 5+ 个定时/触发任务 |
| **可用终端** | 仅桌面 | +CLI | +CLI+PWA+渠道 |

### 10.4 更新后的终极差异化

| 维度 | OpenClaw | Open WebUI | Dify | 智谱 AutoGLM | Cursor | **Nebula v3.0** |
|------|---------|-----------|------|-------------|--------|---------------|
| 记忆深度 | 无层 | 持久记忆 | 无 | 无 | 代码索引 | **8 层 L0-L7** |
| 费用管理 | 无 | Usage Analytics | 无 | 无 | 按量计费 | **智能路由+Credits+预算控制** |
| 本地知识库 | 无 | RAG | RAG | 无 | 代码索引 | **Obsidian 兼容+图谱** |
| 桌面形象 | 菜单栏 | Web UI | Web UI | Web UI | IDE | **悬浮球+三形态** |
| 电脑操作 | 无 | Terminal | 无 | **50步长链** | Cloud Agent | **OS-Controller+视觉+L4审批** |
| 安全深度 | DM pairing | RBAC | 无 | 无 | 无 | **L4 价值层+审计+回滚** |
| 工作流可视化 | 无 | 无 | 画布(设计时) | 无 | 无 | **蜂群画布(运行时)** |
| 自主度 | 全自主 | 全自主 | 全自主 | 全自主 | **滑块** | **6级滑块 L0-L5** |
| 自动化 | 无 | Automations | 无 | 无 | **Automations** | **Cron+Trigger+Watch** |
| 多端 | 多渠道 | Web+Docker | Web+API | Web+App | **Desktop+CLI+Slack+iOS** | **Desktop+CLI+PWA+渠道** |

### 10.5 更新后的一句话定位

> Nebula v3.0 = **省钱的自主式知识型桌面 AI 伙伴**——它记得你的一切知识，帮你操作电脑，替你省 Token 钱，6级自主度按需选择，24/7 无人值守自动化，而且一直陪在你桌面上。

---

**审议结束**。

本报告由 7 位专家从"更快/更省钱/更智能/更贴合"四个维度审议，并整合国内大厂（智谱/月之暗面/Cursor/Notion）6大趋势，所有创新点均以**可量化的用户价值**为驱动，而非功能堆砌。建议将本报告作为 `ROADMAP_v2.1.md` 的后续规划参考，在 Stage 1 完成后按 v2.3→v2.4→v2.5→v2.6→v3.0 顺序迭代。
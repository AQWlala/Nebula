# 智能体产品优化建议 V1.0

**文档版本**：V1.0
**发布日期**：2026-07-10
**编制单位**：7专家虚拟产品设计委员会
**适用产品**：Nebula v2.0.0
**会议性质**：7专家综合设计会（AI基础设施专家 / 插件生态架构师 / UX/UI交互设计师 / 竞品分析师 / 后端系统工程师 / 产品经理 / 技术文档专家）
**会议议题**：模型配置缺陷 / 技能生态匮乏 / 交互体验不佳 / 竞品深度对标

---

## 1. 执行摘要 (Executive Summary)

### 1.1 核心问题概述

经过 7 专家对 Nebula v2.0.0 代码级审计 + 4 款对标产品（JiuwenSwarm / Hermes / CoPaw / OpenAkita / OpenClaw）的深度分析,识别出 4 类核心痛点：

| 编号 | 痛点 | 严重度 | 根因 |
|------|------|--------|------|
| P1 | 模型配置入口缺失感 | 🔴 高 | 后端已支持云端 API(DeepSeek/Anthropic/OpenAI-compat),但前端 Settings 页面未提供显式 API 地址/Key 输入框,且 UnifiedDispatcher 默认关闭 |
| P2 | 技能生态冷启动 | 🔴 高 | 架构完备(三层+市场+导入),但仅 1 个内化技能(loop-engineering),且未对接 OpenClaw/Hermes 协议 |
| P3 | 对话框布局刚性 | 🟡 中 | 消息气泡 max-width:80% 硬编码,无用户可配置宽度,无沉浸模式,超宽屏体验差 |
| P4 | 差异化定位模糊 | 🟡 中 | 功能堆叠完整但用户感知度低,缺少"5分钟上手"向导和可视化面板 |

### 1.2 本次优化的三大战略方向

**战略方向一：可见性提升（Make the Invisible Visible）**
> Nebula 后端能力远超用户感知。模型调度、语义缓存、Arena A/B、CostTracker 等能力已实现但前端未充分暴露。首要任务是把这些"藏起来"的能力变成用户可见、可配、可控的界面元素。

**战略方向二：生态破壁（Ecosystem Breakthrough）**
> 从"自建封闭生态"转向"协议兼容开放生态"。对接 OpenClaw 的 agentskills.io 规范和 Hermes 的 SKILL.md 自发明机制,把技能市场从"空货架"变成"繁华集市"。

**战略方向三：上手零门槛（Zero to Hero in 5 Minutes）**
> 对标 OpenAkita 的"下载→安装→填 API Key→开始用"向导式体验,消除"功能强大但不会用"的困境。

### 1.3 预期达成的关键指标

| 指标 | 当前值 | 目标值 | 衡量方式 |
|------|--------|--------|---------|
| 模型配置耗时 | 需编辑 models.json | <2分钟（UI向导） | 首次配置完成时间 |
| 内置技能数 | 1 个内化 + 6 个种子 | 25+ 内置技能 | docs/skills/ 目录计数 |
| 协议兼容数 | 0（仅 agentskills.io） | 3（+OpenClaw +Hermes +MCP） | SkillImporter source 枚举 |
| 对话框宽度可配 | 否（80% 硬编码） | 是（窄/中/宽/沉浸 4档） | Settings 配置项 |
| 首次上手时间 | 需查文档 | <5分钟（配置向导） | 新用户测试通过率 |

---

## 2. 竞品对标深度分析

### 2.1 对标矩阵表

| 维度 | Nebula v2.0 | JiuwenSwarm | Hermes | CoPaw | OpenAkita | OpenClaw |
|------|-------------|-------------|--------|-------|-----------|----------|
| **架构** | Tauri+Rust+Preact | Python 蜂群 | CLI+SKILL.md | Python AgentScope | Tauri+React+Python | 自托管网关 |
| **模型接入** | ✅ 三家内置+OpenAI-compat（但UI弱） | ✅ 多模型Web配置 | ✅ 多模型 | ✅ 多模型 | ✅ 30+提供商向导式 | ✅ 网关式 |
| **技能格式** | SKILL.md（YAML frontmatter） | 需进一步调研 | SKILL.md（自发明） | Skills（可插拔模块） | 89种内置工具 | SKILL.md（agentskills.io兼容） |
| **技能生态** | agentskills.io+ClawHub+TeamHub（空） | 需进一步调研 | ~/.hermes/skills 自发明 | 内置丰富+第三方扩展 | 89工具16类别 | ClawHub 社区市场 |
| **记忆系统** | ✅ 8层L0-L7+黑洞海绵 | 需进一步调研 | Append-only | 基础 | 3层+7类型 | Markdown内存 |
| **桌面UI** | 悬浮球+三形态（对话框窄） | Web端 | CLI为主 | 桌面App | **11面板全图形化** | Web网关 |
| **上手难度** | 中高（需查文档） | 中 | 中高（CLI） | 中 | **低（5分钟向导）** | 中高 |
| **IM接入** | 6渠道 | 需进一步调研 | 无 | 钉钉/飞书/QQ/Discord/iMessage | 6平台 | 无 |
| **自主度** | L0-L5 6级滑块 | 蜂群自主 | Agent驱动 | 基础 | 多Agent+Plan Mode | Agent Loop |
| **数据主权** | ✅ E2EE+本地优先 | 需进一步调研 | 本地 | 本地 | 本地+POLICIES.yaml | 自托管 |

### 2.2 我方产品 SWOT 分析

```
┌───────────────────────────────┬───────────────────────────────┐
│         STRENGTHS (优势)       │        WEAKNESSES (劣势)       │
├───────────────────────────────┼───────────────────────────────┤
│ • 8层记忆系统(行业最深)        │ • 前端配置入口不完整           │
│ • E2EE双棘轮(安全最强)         │ • 技能生态冷启动(仅1内化)      │
│ • 四大支柱全部落地(131任务)    │ • 对话框布局刚性(80%硬编码)    │
│ • Rust后端(性能+安全)          │ • 无首次使用向导              │
│ • Loop Engineering内化         │ • UnifiedDispatcher默认关闭    │
│ • 270+Tauri命令(功能最全)      │ • 无模型健康度可视化面板      │
│ • Credits成本控制(唯一)        │ • 技能沙箱仅Python            │
├───────────────────────────────┼───────────────────────────────┤
│      OPPORTUNITIES (机会)      │         THREATS (威胁)         │
├───────────────────────────────┼───────────────────────────────┤
│ • OpenClaw/Hermes生态兼容空白  │ • OpenAkita上手门槛更低        │
│ • 桌面AI Agent市场增长快       │ • CoPaw内置技能更丰富          │
│ • 数据主权法规趋严(有利本地)   │ • JiuwenSwarm蜂群架构竞争      │
│ • SKILL.md成行业标准           │ • 功能堆叠但用户感知度低       │
└───────────────────────────────┴───────────────────────────────┘
```

### 2.3 差异化竞争策略建议

**策略一：记忆深度碾压**
> Nebula 的 8 层记忆 + 黑洞海绵 + E2EE 双棘轮是**行业唯一**。竞品最多 3 层记忆,无 E2EE。营销应聚焦"你无法信任一段你无法阅读的记忆"——这是无法被快速复制的护城河。

**策略二：成本控制唯一性**
> Credits Dashboard + TokenJuice 三级压缩 + ModelRouter 智能路由 = **唯一有费用感知的桌面 AI Agent**。OpenAkita/CoPaw/Hermes 均无成本控制。这是个人用户的核心痛点。

**策略三：协议兼容破壁**
> 率先同时兼容 agentskills.io（OpenClaw）+ SKILL.md 自发明（Hermes）+ MCP 协议,成为**技能生态聚合器**。竞品各自为战,Nebula 做"技能市场的统一入口"。

**策略四：信任三原则壁垒**
> 可读+可编辑+可追溯+可审计+可加密的五位一体,是 OpenAkita（无加密）、CoPaw（无溯源）、Hermes（Append-only）都不具备的。这是企业/专业用户的选择理由。

---

## 3. 核心模块优化方案

### 3.1 模型配置中心重构

#### 3.1.1 现状诊断

**后端能力（已就绪）**：
- `src-tauri/src/llm/models_config.rs`：models.json v2 动态配置,已内置 DeepSeek/Anthropic/Ollama 三家 provider
- `src-tauri/src/llm/model_router.rs`：ModelRouter 按 simple/medium/complex 路由（Ollama/DeepSeek/Anthropic）
- `src-tauri/src/llm/openai_compat.rs`：OpenAI 兼容层（vLLM/LMStudio/OpenRouter/DeepSeek）
- `src-tauri/src/llm/dispatcher.rs`：ADR-003 UnifiedModelDispatcher（**默认关闭**,需 `--features unified-dispatcher`）
- API Key 存储：OS keychain（keyring crate）,**绝不持久化到 localStorage**

**前端缺口（问题根因）**：
- `src/components/Settings.tsx`：虽有 `llmProvider`/`openaiCompatUrl`/`openaiCompatKey` 字段,但**未提供显式的 API 地址输入框和 Key 输入框 UI**
- 用户无法在 UI 上直接添加自定义云端 provider（需手动编辑 models.json）
- 无模型健康度/延迟/成本可视化面板

#### 3.1.2 架构图描述（本地+云端混合模式）

```
┌─────────────────────────────────────────────────────────────┐
│                  模型配置中心 (Settings UI)                   │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │ Provider列表  │  │ API配置表单   │  │ 模型健康面板  │       │
│  │              │  │              │  │              │       │
│  │ ● Ollama     │  │ Provider名:  │  │ 延迟: 234ms  │       │
│  │ ● DeepSeek   │  │ API地址:     │  │ 成本: $0.12  │       │
│  │ ● Anthropic  │  │ API Key:     │  │ 命中率: 42%  │       │
│  │ ● Custom +   │  │ 模型列表:    │  │ 状态: 🟢     │       │
│  └──────────────┘  └──────────────┘  └──────────────┘       │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│              UnifiedModelDispatcher (启用为默认)              │
│                                                              │
│  WorkType 路由:                                              │
│  ├─ Chat → 用户选定 provider                                 │
│  ├─ SwarmWorker → DeepSeek (性价比)                          │
│  ├─ Evolution/SoulCompile/Classifier → Ollama (本地强制)     │
│  └─ MasterTask → Anthropic (高质量)                          │
└──────────────────────────┬──────────────────────────────────┘
                           │
           ┌───────────────┼───────────────┐
           ▼               ▼               ▼
    ┌─────────────┐ ┌─────────────┐ ┌─────────────┐
    │  本地层      │ │  云端层      │ │  缓存层      │
    │             │ │             │ │             │
    │ Ollama      │ │ DeepSeek    │ │ L0 Exact    │
    │ (免费/隐私) │ │ Anthropic   │ │ L0.5 Semantic│
    │             │ │ OpenAI-compat│ │ (LanceDB)   │
    │ 数据主权红线 │ │ Custom URL  │ │             │
    └─────────────┘ └─────────────┘ └─────────────┘
```

#### 3.1.3 API 配置交互流程设计（参考 JiuwenSwarm）

**借鉴点**：JiuwenSwarm 支持在 Web 平台配置 DeepSeek/通义千问/MiniMax 等 API Key 并关联智能体。Nebula 借鉴其"配置→测试→关联"三步流程,但增加**本地 keychain 安全存储**和**SSRF 防护**。

**交互流程**：
```
Step 1: 选择 Provider
  └─ 内置: Ollama / DeepSeek / Anthropic / OpenAI-compat
  └─ 自定义: 输入 Provider 名称

Step 2: 填写配置
  ├─ API 地址 (base_url): 预填默认值,可修改
  ├─ API Key: 密码框输入,存入 OS keychain
  ├─ 模型列表: 手动添加 或 自动拉取(/v1/models)
  └─ 连接测试: 发送 ping 请求,显示延迟和状态

Step 3: 设置默认
  ├─ 默认 Provider: 全局默认
  ├─ 按 WorkType 路由: Chat/Swarm/Evolution 分别指定
  └─ 保存并启用
```

#### 3.1.4 关键技术实现要点

| 要点 | 实现方案 | 涉及文件 |
|------|---------|---------|
| 前端 API 配置表单 | 新增 `ModelConfigPanel.tsx`,含 Provider 列表/地址/Key/模型列表/测试按钮 | `src/components/` |
| Keychain 交互 | 复用现有 `keyring` crate,新增 Tauri 命令 `set_provider_key`/`test_provider_connection` | `src-tauri/src/commands/llm.rs` |
| 模型自动发现 | Ollama 调 `/api/tags`,OpenAI-compat 调 `/v1/models`,自动填充模型列表 | `src-tauri/src/llm/models_config.rs` |
| UnifiedDispatcher 默认启用 | 移除 `unified-dispatcher` feature gate,改为默认开启,环境变量保留为 override | `src-tauri/Cargo.toml` + `dispatcher.rs` |
| Provider 热更新 | 将 `Arc<RwLock<ModelsConfig>>` 的 provider 列表修改改为热生效（重建 reqwest::Client） | `src-tauri/src/llm/models_config.rs` |
| 模型健康面板 | 新增 `ModelHealthPanel.tsx`,展示延迟/成本/命中率/断路器状态,数据来自 CostTracker+Gateway | `src/components/` + `src-tauri/src/llm/cost_tracker.rs` |

---

### 3.2 技能生态体系建设

#### 3.2.1 内置技能清单推荐

**对标分析**：
- **CoPaw**：内置文档处理(file_reader)、新闻阅读、文件管理、桌面整理、社交内容爬取、视频脚本草稿等实用技能
- **Hermes**：Agent 自动发明技能（重复操作→自动创建 skill 文件存入 ~/.hermes/skills/）
- **OpenAkita**：89 种工具覆盖 16 类别（含定时任务、桌面操作、IM 通知等）
- **OpenClaw**：agentskills.io 兼容,社区驱动 ClawHub 市场

**Nebula 推荐内置技能清单（25+，分 5 类）**：

| 类别 | 技能名 | 功能描述 | 对标来源 | 优先级 |
|------|--------|---------|---------|--------|
| **文档处理** | file-reader | 读取并摘要 .txt/.md/.pdf/.docx | CoPaw | P0 |
| | doc-writer | 创建/编辑文档(Markdown/HTML) | CoPaw | P0 |
| | pdf-extractor | PDF 内容提取+结构化 | Hermes | P1 |
| | meeting-notes | 会议纪要自动生成 | OpenAkita | P1 |
| **信息收集** | web-search | 网页搜索+结果摘要 | OpenAkita | P0 |
| | news-digest | 新闻阅读与总结 | CoPaw | P1 |
| | social-monitor | 社交平台热门内容爬取整理 | CoPaw | P2 |
| | competitor-track | 竞品动态追踪 | OpenAkita | P2 |
| **代码开发** | code-review | 代码审查+改进建议 | Hermes | P0 |
| | code-refactor | 代码重构建议 | Hermes | P1 |
| | git-helper | Git 操作辅助(commit/branch/merge) | OpenClaw | P1 |
| | test-generator | 自动生成测试用例 | Hermes | P2 |
| **效率工具** | file-organizer | 桌面/文件夹自动整理 | CoPaw | P1 |
| | clipboard-manager | 剪贴板智能监听+历史 | CoPaw | P1 |
| | screenshot-ocr | 截图OCR文字识别 | OpenAkita | P2 |
| | calendar-assist | 日程管理辅助 | OpenAkita | P2 |
| **创作辅助** | article-writer | 文章撰写(自媒体/博客) | Hermes | P0 |
| | video-script | 视频脚本草稿生成 | CoPaw | P1 |
| | mindmap-creator | 思维导图生成 | 现有种子 | P1 |
| | mermaid-creator | Mermaid图表生成 | 现有种子 | P1 |
| | canvas-creator | 画布创作 | 现有种子 | P2 |
| **Loop Engineering** | loop-engineering | Loop工程内化(已实现) | 自研 | ✅ 已完成 |
| | skill-auto-invent | 自动发明技能(Hermes机制) | Hermes | P0 |
| | skill-test-runner | 技能测试运行器 | OpenAkita | P1 |
| | skill-publisher | 技能发布到Gist/市场 | 现有 | P1 |

#### 3.2.2 OpenClaw & Hermes 生态兼容方案

**现状**：Nebula 已支持 agentskills.io + ClawHub + TeamSkillsHub 三源导入（`importer.rs`）,但：
- 代码中无 "OpenClaw" 协议级集成（仅 source 提及）
- 无 "Hermes" 技能协议提及
- 无 Hermes 式"自动发明技能"机制

**兼容方案**：

```
┌─────────────────────────────────────────────────────────────┐
│                SkillImporter 多协议兼容层                     │
│                                                              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐         │
│  │ agentskills │  │  OpenClaw   │  │   Hermes    │         │
│  │     .io     │  │  ClawHub    │  │  自发明机制  │         │
│  │  (已有✅)   │  │  (需适配)   │  │  (需新增)   │         │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘         │
│         │                │                │                 │
│         └────────────────┼────────────────┘                 │
│                          ▼                                  │
│              ┌───────────────────────┐                      │
│              │  SKILL.md 统一格式     │                      │
│              │  (YAML frontmatter +  │                      │
│              │   Markdown body)      │                      │
│              └───────────┬───────────┘                      │
│                          ▼                                  │
│              ┌───────────────────────┐                      │
│              │  SkillSpecValidator   │                      │
│              │  (三层校验已实现✅)   │                      │
│              └───────────────────────┘                      │
└─────────────────────────────────────────────────────────────┘
```

**OpenClaw 兼容（低难度）**：
- OpenClaw 使用 agentskills.io 兼容的 SKILL.md 格式（YAML frontmatter + Markdown body）,与 Nebula 现有格式**完全兼容**
- 现有 `ClawHub` source 已可拉取 ClawHub 社区技能
- **需补充**：在 SkillPanel UI 中标注"OpenClaw 兼容"badge,并在市场浏览中增加 OpenClaw 分类
- **需补充**：支持 OpenClaw 的 `/plugin marketplace add` 式命令行安装

**Hermes 兼容（中难度）**：
- Hermes 的 SKILL.md 格式与 Nebula **相同**（YAML frontmatter + Markdown body）
- **核心差异**：Hermes 有"Agent 自动发明技能"机制——当 Agent 发现某操作重复多次,自动创建 skill 文件存入 `~/.hermes/skills/`
- **需新增**：
  1. `SkillAutoInventor` 模块：监控 Agent 操作日志,检测重复模式（≥3次相同操作序列）,自动生成 SKILL.md 草稿
  2. 自动发明的技能 `trust_level = 0`,需用户审核后提升
  3. 技能存储路径：`~/.nebula/skills/auto-invented/`（对标 `~/.hermes/skills/`）
  4. 前端"自动发明技能"通知 + 审核面板

#### 3.2.3 技能安装/管理/调试流程设计

```
┌─────────────── 技能生命周期 ───────────────┐
│                                            │
│  发现 → 安装 → 审核 → 启用 → 使用 → 评分  │
│   │      │      │      │      │      │   │
│   ▼      ▼      ▼      ▼      ▼      ▼   │
│  市场浏览 一键导入 trust  能力声明 调用  ★ │
│  搜索     依赖检查  提升  沙箱执行  结果  │
│  推荐     冲突检测       审批门禁  反馈  │
│                                            │
│  ┌─ 调试工具 ──────────────────────────┐  │
│  │ • SkillInspector: 查看manifest+代码 │  │
│  │ • SkillTestRunner: 单技能测试运行   │  │
│  │ • SkillDebugger: 逐步执行+日志      │  │
│  │ • SkillProfiler: 性能分析           │  │
│  └────────────────────────────────────┘  │
└────────────────────────────────────────────┘
```

**关键技术实现要点**：

| 要点 | 实现方案 | 优先级 |
|------|---------|--------|
| 25+ 内置技能 | 按 P0/P1/P2 分批编写 SKILL.md,每个含 frontmatter + 指令 + 示例 | P0 |
| SkillAutoInventor | 新增 `src-tauri/src/skills/auto_inventor.rs`,操作日志分析+模式检测+草稿生成 | P0 |
| OpenClaw badge | SkillPanel UI 增加"OpenClaw 兼容"标签 + 市场分类 | P1 |
| 技能调试工具 | 新增 `SkillDebugger.tsx` + 后端 `skill_debug` 命令 | P1 |
| 沙箱多语言 | 扩展 `engine.rs` 语言白名单(当前仅Python),增加 Node.js/Shell | P2 |
| 远程技能市场 | 建立 `registry.nebula.ai` 集中式技能注册表服务 | P2 |

---

### 3.3 桌面端 UI/UX 升级

#### 3.3.1 对话窗口宽度与布局调整规范（参考 OpenAkita）

**对标分析**：
- **OpenAkita**：11 个功能面板全图形化,配置向导式上手,桌面 App 布局清晰
- **OpenAkita 借鉴点**：面板化设计、向导式首次配置、5分钟上手体验

**Nebula 现状问题**：
- `.msg { max-width: 80% }` 硬编码在 `src/styles/global.css` 第2446行
- `.chat-panel` 无显式宽度约束,超宽屏(3440px)对话框撑满,消息行过长
- 侧边栏 240px 固定不可调
- 无"隐藏侧边栏全屏对话"沉浸模式
- Settings 无"聊天宽度"配置项

**调整规范**：

| 元素 | 当前值 | 调整为 | 实现方式 |
|------|--------|--------|---------|
| 消息气泡 max-width | 80% 硬编码 | CSS变量 `--chat-msg-max-width` | `global.css` 第2446行 |
| 对话区域宽度 | 无约束 | `max-width: 1200px; margin: 0 auto` | `.chat-panel` 新增 |
| 宽度模式 | 无 | 窄(720px)/中(960px)/宽(1200px)/沉浸(100%) | Settings 配置项 |
| 侧边栏宽度 | 240px 固定 | 可拖拽 180-320px + 折叠 | `.sidebar` 新增 resize |
| 沉浸模式 | 无 | F11 快捷键隐藏侧边栏 | App.tsx 状态管理 |

#### 3.3.2 关键界面线框图描述

**界面一：模型配置中心（新增）**
```
┌──────────────────────────────────────────────────────────┐
│  Settings > 模型配置                          [测试全部]  │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ┌─ Provider 列表 ──────┐  ┌─ 配置详情 ──────────────┐  │
│  │                      │  │                          │  │
│  │  ● Ollama     🟢 23ms│  │  Provider 名称:          │  │
│  │  ● DeepSeek   🟢 1.2s│  │  [DeepSeek           ]  │  │
│  │  ● Anthropic  🔴 未配│  │                          │  │
│  │  ● OpenRouter 🟡 3s  │  │  API 地址:               │  │
│  │  + 添加 Provider     │  │  [https://api.deepseek.] │  │
│  │                      │  │                          │  │
│  └──────────────────────┘  │  API Key:                │  │
│                            │  [•••••••••••••••] [显示] │  │
│                            │                          │  │
│                            │  模型列表:                │  │
│                            │  ☑ deepseek-chat         │  │
│                            │  ☑ deepseek-coder        │  │
│                            │  [+ 自动拉取] [+ 手动]   │  │
│                            │                          │  │
│                            │  [测试连接]  [保存]      │  │
│                            └──────────────────────────┘  │
│                                                          │
│  ┌─ WorkType 路由 ────────────────────────────────────┐  │
│  │  Chat对话:      [DeepSeek ▼]                       │  │
│  │  Swarm蜂群:     [DeepSeek ▼]                       │  │
│  │  Evolution进化: [Ollama ▼] (本地强制)              │  │
│  │  MasterTask:    [Anthropic ▼]                      │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│  ┌─ 模型健康面板 ─────────────────────────────────────┐  │
│  │  Provider    延迟    成本/日   命中率   状态       │  │
│  │  Ollama      23ms    $0       5%       🟢          │  │
│  │  DeepSeek    1.2s    $0.12    35%      🟢          │  │
│  │  Anthropic   2.1s    $0.45    0%       🔴 未配     │  │
│  └────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

**界面二：对话区域宽度调整（4档）**
```
┌─ 窄模式 (720px) ──────────┐  ┌─ 中模式 (960px) ──────────┐
│ ┌─sidebar─┐ ┌─chat─────┐ │  │ ┌─sidebar─┐ ┌─chat──────┐ │
│ │ 180px   │ │ 720px    │ │  │ │ 240px   │ │ 960px     │ │
│ │         │ │ ←max-width│ │  │ │         │ │ ←max-width│ │
│ │         │ │  msg 80% │ │  │ │         │ │  msg 75%  │ │
│ └─────────┘ └──────────┘ │  │ └─────────┘ └───────────┘ │
└──────────────────────────┘  └──────────────────────────┘

┌─ 宽模式 (1200px) ─────────┐  ┌─ 沉浸模式 (100%) ─────────┐
│ ┌─sidebar─┐ ┌─chat──────┐│  │ ┌─chat (全屏) ──────────┐│
│ │ 240px   │ │ 1200px    ││  │ │ 100% width            ││
│ │         │ │ ←max-width││  │ │ msg max 800px居中     ││
│ │         │ │  msg 70%  ││  │ │ F11 退出              ││
│ └─────────┘ └───────────┘│  │ └───────────────────────┘│
└──────────────────────────┘  └──────────────────────────┘
```

**界面三：首次使用向导（新增，对标 OpenAkita）**
```
┌──────────────────────────────────────────────────────────┐
│            欢迎使用 Nebula (Step 1/4)                     │
├──────────────────────────────────────────────────────────┤
│                                                          │
│              🐍 Nebula - 你的第二大脑                     │
│                                                          │
│         "你无法信任一段你无法阅读的记忆"                    │
│                                                          │
│    Nebula 是本地优先的自主式知识型桌面 AI 伙伴             │
│                                                          │
│           [开始配置]    [稍后]                            │
│                                                          │
└──────────────────────────────────────────────────────────┘

         ↓ Step 2/4: 配置模型

┌──────────────────────────────────────────────────────────┐
│            配置你的 AI 模型 (Step 2/4)                    │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  选择你的首选模型:                                        │
│  ○ 本地 Ollama (免费,需已安装)                           │
│  ● DeepSeek (云端,性价比高)                              │
│  ○ Anthropic Claude (云端,高质量)                        │
│  ○ 自定义 API                                            │
│                                                          │
│  API Key: [••••••••••••••••••••••]                      │
│                                                          │
│  [测试连接] → 🟢 连接成功 (延迟: 1.2s)                   │
│                                                          │
│           [上一步]    [下一步]                            │
│                                                          │
└──────────────────────────────────────────────────────────┘

         ↓ Step 3/4: 选择技能  →  Step 4/4: 完成
```

#### 3.3.3 交互细节优化列表

| 编号 | 优化项 | 当前 | 优化为 | 优先级 |
|------|--------|------|--------|--------|
| U1 | 消息气泡宽度 | 80% 硬编码 | CSS变量+4档可配 | P0 |
| U2 | 对话区域宽度 | 无约束 | max-width 4档 | P0 |
| U3 | 侧边栏宽度 | 240px 固定 | 可拖拽 180-320px | P1 |
| U4 | 沉浸模式 | 无 | F11 快捷键 | P1 |
| U5 | 首次向导 | 无 | 4步向导(欢迎/模型/技能/完成) | P0 |
| U6 | 模型健康面板 | 无 | 延迟/成本/命中率/状态 | P1 |
| U7 | 技能市场badge | 无来源标注 | OpenClaw/Hermes/MCP badge | P1 |
| U8 | 对话框默认焦点 | 未确认 | 输入框自动聚焦 | P1 |
| U9 | 消息时间戳 | 显示 | 可切换显示/隐藏 | P2 |
| U10 | 代码块复制 | 无 | 一键复制按钮 | P1 |

---

## 4. 实施路线图 (Roadmap)

### 4.1 P0/P1/P2 需求优先级排序

**P0（立即执行，1-2周）—— 解决"不能用"问题**：

| 编号 | 任务 | 工作量 | 涉及模块 |
|------|------|--------|---------|
| P0-1 | 模型配置 UI 表单（Provider列表/地址/Key/测试） | M | 前端 ModelConfigPanel.tsx + 后端命令 |
| P0-2 | UnifiedDispatcher 默认启用 | S | Cargo.toml + dispatcher.rs |
| P0-3 | 对话框宽度 4 档可配 + CSS变量 | S | global.css + Settings.tsx |
| P0-4 | 首次使用向导（4步） | M | 新增 OnboardingWizard.tsx |
| P0-5 | 编写 10 个 P0 内置技能 SKILL.md | L | docs/skills/ |
| P0-6 | SkillAutoInventor（Hermes机制） | L | auto_inventor.rs |

**P1（近期执行，2-4周）—— 解决"不好用"问题**：

| 编号 | 任务 | 工作量 | 涉及模块 |
|------|------|--------|---------|
| P1-1 | 模型健康面板（延迟/成本/命中率） | M | ModelHealthPanel.tsx |
| P1-2 | 模型自动发现（/api/tags, /v1/models） | S | models_config.rs |
| P1-3 | Provider 热更新 | M | models_config.rs |
| P1-4 | 侧边栏可拖拽 + 折叠 | S | global.css + App.tsx |
| P1-5 | 沉浸模式 F11 | S | App.tsx |
| P1-6 | OpenClaw 兼容 badge + 命令行安装 | S | SkillPanel.tsx |
| P1-7 | 技能调试工具（Inspector/TestRunner/Debugger） | L | SkillDebugger.tsx |
| P1-8 | 编写 10 个 P1 内置技能 SKILL.md | L | docs/skills/ |
| P1-9 | 代码块复制按钮 + 输入框自动聚焦 | S | ChatPanel.tsx |

**P2（中期执行，1-2月）—— 解决"不够强"问题**：

| 编号 | 任务 | 工作量 | 涉及模块 |
|------|------|--------|---------|
| P2-1 | 远程技能市场 registry.nebula.ai | XL | 新建服务 |
| P2-2 | 沙箱多语言支持（Node.js/Shell） | M | engine.rs |
| P2-3 | Windows 沙箱 JobObject 内存 cap | M | engine.rs |
| P2-4 | 编写 5 个 P2 内置技能 SKILL.md | M | docs/skills/ |
| P2-5 | 技能更新检查（远端版本比对） | S | marketplace.rs |
| P2-6 | 11 面板全图形化（对标 OpenAkita） | XL | 前端全面重构 |

### 4.2 分阶段交付里程碑

```
Phase 1: 可见性提升 (Week 1-2)
├── P0-1 模型配置 UI ✅
├── P0-2 UnifiedDispatcher 默认 ✅
├── P0-3 对话框宽度可配 ✅
├── P0-4 首次使用向导 ✅
└── 交付: v2.1.0 "可见的 Nebula"

Phase 2: 生态破壁 (Week 3-6)
├── P0-5 10个P0内置技能 ✅
├── P0-6 SkillAutoInventor ✅
├── P1-6 OpenClaw兼容badge ✅
├── P1-7 技能调试工具 ✅
├── P1-8 10个P1内置技能 ✅
└── 交付: v2.2.0 "生态 Nebula"

Phase 3: 体验打磨 (Week 7-10)
├── P1-1 模型健康面板 ✅
├── P1-2 模型自动发现 ✅
├── P1-3 Provider热更新 ✅
├── P1-4 侧边栏可拖拽 ✅
├── P1-5 沉浸模式 ✅
├── P1-9 交互细节 ✅
└── 交付: v2.3.0 "顺滑 Nebula"

Phase 4: 规模化 (Month 3+)
├── P2-1 远程技能市场 ✅
├── P2-2 多语言沙箱 ✅
├── P2-6 11面板全图形化 ✅
└── 交付: v3.0.0 "全民 Nebula"
```

---

## 5. 风险评估与应对

### 5.1 技术风险

| 风险 | 概率 | 影响 | 预案 |
|------|------|------|------|
| UnifiedDispatcher 启用后回归 bug | 中 | 高 | 先在 `--features unified-dispatcher` 下跑完整测试套件(2576 tests),通过后再默认启用;保留环境变量 override 回退 |
| Provider 热更新导致连接泄漏 | 中 | 中 | reqwest::Client 用 Arc<RwLock> 包裹,更新时先 drain 旧 Client 的连接池;增加连接数监控 |
| SkillAutoInventor 误判(噪声模式) | 高 | 中 | 阈值设为 ≥5次相同操作序列(非3次);生成草稿 trust_level=0;用户审核后才启用;增加"忽略此模式"按钮 |
| 沙箱多语言安全风险 | 中 | 高 | Node.js/Shell 沙箱必须走 CapabilitySet 审批;Shell 命令白名单;Windows 用 JobObject 限制 |
| CSS 变量兼容性 | 低 | 低 | Preact+Vite 现代浏览器目标,CSS 变量兼容性无问题 |

### 5.2 生态风险

| 风险 | 概率 | 影响 | 预案 |
|------|------|------|------|
| OpenClaw/Hermes 协议变更 | 低 | 中 | SKILL.md 格式已标准化(YAML frontmatter),变更概率低;SkillSpecValidator 三层校验可拦截不合规技能 |
| 技能供应链攻击 | 中 | 高 | 导入技能 trust_level=0(已有);增加技能代码静态扫描(SAST);危险能力(exec/file-write/net)强制用户确认 |
| 技能市场冷启动 | 高 | 中 | 先内置 25+ 技能(P0/P1);SkillAutoInventor 自发明填充;支持从 GitHub Gist 一键导入 |
| 竞品快速跟进兼容策略 | 中 | 中 | 差异化在"五协议兼容"(agentskills.io+ClawHub+TeamHub+Hermes+MCP),竞品难以同时覆盖 |

### 5.3 体验风险

| 风险 | 概率 | 影响 | 预案 |
|------|------|------|------|
| 首次向导过于复杂 | 中 | 中 | 4步限制(欢迎/模型/技能/完成);每步<30秒;可跳过;DeepSeek 预填默认值 |
| 对话框宽度选择困难 | 低 | 低 | 默认"中模式"(960px),覆盖 80% 场景;设置项放 Settings>外观 |
| 模型配置术语晦涩 | 中 | 中 | 向导中使用通俗语言("性价比高"而非"DeepSeek-chat");tooltips 解释术语 |
| 功能面板过多(对标11面板) | 中 | 中 | 渐进式展示:默认 5 面板,高级模式解锁全部;面板可拖拽排序 |

---

## 附录A: 7专家研讨纪要

### A.1 专家1: AI基础设施专家

**观点**：
- 后端模型调度架构已非常成熟（ModelRouter + Gateway + Dispatcher + CostTracker + SemanticCache）,问题不在能力而在**可见性**
- UnifiedDispatcher 默认关闭是历史包袱,ADR-003 已设计完整,应果断启用
- 模型自动发现（/api/tags, /v1/models）是低成本高收益功能,应优先实现
- Provider 热更新虽有技术挑战（reqwest::Client 重建）,但对用户体验至关重要

**决议**：P0-1(配置UI) + P0-2(Dispatcher启用) + P1-2(自动发现) + P1-3(热更新) 列入路线图

### A.2 专家2: 插件生态架构师

**观点**：
- 技能系统三层架构（protocol/capability/executor）设计优秀,但**内容严重不足**是致命问题
- OpenClaw 兼容是**低垂果实**——格式相同,只需 UI 标注和命令行安装
- Hermes 的"自动发明技能"是**杀手锏功能**,必须实现,这是技能冷启动的关键
- 远程技能市场 registry.nebula.ai 是长期方向,但不应阻塞 P0

**决议**：P0-5(10个内置技能) + P0-6(SkillAutoInventor) + P1-6(OpenClaw badge) 列入路线图

### A.3 专家3: UX/UI交互设计师

**观点**：
- 对话框 80% 硬编码是**不可接受的刚性**,必须改为 CSS 变量 + 用户可配
- OpenAkita 的 11 面板全图形化是标杆,但 Nebula 不应盲目追求数量,应先做好核心面板
- 首次使用向导是**必须的**,对标 OpenAkita "5分钟上手"
- 沉浸模式(F11)是低成本高体验的功能

**冲突点**：11 面板 vs 开发成本
**决议**：P2-6(11面板) 推迟到 Phase 4;P0 优先做配置面板+向导+宽度可配

### A.4 专家4: 竞品分析师

**观点**：
- Nebula 的核心差异化是**8层记忆 + E2EE + Credits成本控制**,这是竞品都不具备的
- OpenAkita 的优势是"上手零门槛",Nebula 应学习其向导式体验
- CoPaw 的优势是"内置丰富技能",Nebula 应通过 25+ 内置技能 + 协议兼容追赶
- 不应与 OpenAkita 比拼工具数量(89个),而应强调**记忆深度+安全+成本**的差异化

**决议**：差异化策略聚焦"记忆深度碾压 + 成本控制唯一性 + 协议兼容破壁"

### A.5 专家5: 后端系统工程师

**观点**：
- models_config.rs 的 SSRF 防护已完善,无需额外安全改造
- keyring crate 跨平台 Key 存储已就绪,前端只需调用 Tauri 命令
- SkillAutoInventor 需要操作日志分析,建议复用现有 audit.rs 审计日志
- Provider 热更新的技术方案：Arc<RwLock<ModelsConfig>> + Client 重建,需注意连接池 drain

**决议**：技术方案可行,无阻塞项

### A.6 专家6: 产品经理

**观点**：
- P0 必须解决"用户看不到云端API配置"的感知问题,这是用户反馈最强烈的
- 首次使用向导是转化率关键,对标 OpenAkita 5分钟上手
- 技能冷启动用 SkillAutoInventor + 10个P0内置技能 双管齐下
- 不追求 P2 的 11 面板,先把 P0/P1 做扎实

**冲突裁决**：
- 生态开放性 vs 安全性 → 安全优先,导入技能 trust_level=0 不变
- UI 美观度 vs 开发成本 → P0 做核心,P2 做全面
- 工具数量 vs 质量 → 25个精选技能 > 89个平庸工具

**决议**：P0 聚焦可见性+向导+宽度+10技能+自发明;P1 聚焦面板+调试+OpenClaw

### A.7 专家7: 技术文档专家

**观点**：
- 优化建议文档需明确版本号(V1.0)、日期、可追溯
- 每个建议必须标注涉及文件路径和行号,便于工程执行
- 竞品信息不确定处已标注"需进一步调研",禁止编造
- 路线图需有明确里程碑和交付版本号

**决议**：文档结构规范,所有建议附文件路径,遵循 Constraints

---

## 附录B: 涉及文件索引

| 模块 | 文件路径 | 修改类型 |
|------|---------|---------|
| 模型配置UI(新增) | `src/components/ModelConfigPanel.tsx` | 新建 |
| 模型配置后端 | `src-tauri/src/llm/models_config.rs` | 修改(自动发现+热更新) |
| Dispatcher启用 | `src-tauri/Cargo.toml` + `src-tauri/src/llm/dispatcher.rs` | 修改(默认启用) |
| 模型健康面板(新增) | `src/components/ModelHealthPanel.tsx` | 新建 |
| 首次向导(新增) | `src/components/OnboardingWizard.tsx` | 新建 |
| 对话框宽度 | `src/styles/global.css` 第2446行 | 修改(CSS变量) |
| 宽度配置 | `src/components/Settings.tsx` | 修改(新增宽度配置项) |
| 侧边栏可调 | `src/styles/global.css` 第168行 + `src/App.tsx` | 修改 |
| 内置技能(新增) | `docs/skills/<skill-name>/SKILL.md` | 新建(25个) |
| 技能自发明(新增) | `src-tauri/src/skills/auto_inventor.rs` | 新建 |
| OpenClaw badge | `src/components/SkillPanel.tsx` | 修改 |
| 技能调试(新增) | `src/components/SkillDebugger.tsx` | 新建 |
| 沙箱多语言 | `src-tauri/src/skills/engine.rs` | 修改(P2) |

---

**文档结束**

本《智能体产品优化建议 V1.0》基于 7 专家综合设计会共识编制,所有建议均附工程可行性分析和文件路径标注,可直接作为下一迭代周期的执行依据。

**关键决策摘要**：
1. **模型配置**：后端已就绪,重点补前端 UI + 默认启用 UnifiedDispatcher
2. **技能生态**：25+ 内置技能 + SkillAutoInventor + OpenClaw/Hermes 协议兼容
3. **UI/UX**：对话框 4 档宽度可配 + 首次使用向导 + 沉浸模式
4. **差异化**：记忆深度 + E2EE + Credits 成本控制 + 五协议兼容

下一步：请技术总监/CEO审阅,确认后按 Phase 1-4 路线图执行。

# Nebula (nebula) v3.0 综合进化报告

## ——双报告融合 · 四大支柱 · 六大趋势 · 信任三原则

**版本**：v3.0（综合进化版）
**日期**：2026-07-03
**融合来源**：
- **报告 A**：`EXPERT_REVIEW_v3.0_INNOVATION.md`（7 专家审议 + 国内大厂趋势）
- **报告 B**：GLM-5.2 对话分析（OpenAkita 校准 + UI-TARS/CoPaw/LLM Wiki/Obsidian Skills 深度对标）
**审议基线**：`WHITEPAPER_v2.0.md` + `ROADMAP_v2.1.md` + `EXPERT_REVIEW_v2.1.md`
**竞品总数**：13 个（9 开源 + 4 国内大厂）
**任务总数**：68 个（去重合并后）

---

## 0. 执行摘要

本报告融合两份独立审议的成果，形成Nebula v3.0 的**统一进化蓝图**。

**核心结论**：Nebula应从"聊天工具"进化为**「省钱的自主式知识型桌面 AI 伙伴」**，差异化护城河是**「信任三原则」**——所有记忆必须可读、可编辑、可追溯，这是行业唯一。

**四维目标**（每个创新必须同时满足）：
- **更快**：平均响应 < 200ms（80% 本地 + 缓存命中）
- **更省钱**：月度 Token 成本降 90%（$30→$3）
- **更智能**：从"聊天"到"第二大脑"（Obsidian 兼容 + 知识图谱）
- **更贴合**：从"告诉你"到"帮你做"（OS-Controller + 视觉 + 6 级自主度）

**进化路线**：v2.3 省钱 → v2.4 知识 → v2.5 形象 → v2.6 可视 → v3.0 全自主

---

## 第一部分：双报告融合分析

### 1.1 报告来源与互补性

| 维度 | 报告 A（7 专家 + 大厂趋势） | 报告 B（GLM-5.2 对标分析） |
|------|---------------------------|--------------------------|
| **专家视角** | EA-1~EA-7 七角色分工 | 四大支柱（省钱/智能/贴合/快） |
| **竞品覆盖** | 9 开源 + 4 国内大厂 = 13 | 9 开源 + 4 国内大厂 = 13（重叠 8 个，各有独家） |
| **独有竞品** | Open WebUI、Dify、智谱 AutoGLM、Kimi、Cursor、Notion | OpenAkita（深度校准）、UI-TARS-desktop、CoPaw、LLM Wiki、Obsidian Skills |
| **趋势洞察** | 自主度滑块、Shadow Workspace、Credits 计费、24/7 Automations、多端同源 | 信任三原则、Event Stream 协议化、凭证加密卷分离、Proactive Engine |
| **任务粒度** | 6 大趋势 × 4 任务 = 24 个 | 4 支柱 × 38 个任务 |
| **License 分析** | 无 | 完整矩阵 |

### 1.2 覆盖维度对比

| 创新维度 | 报告 A | 报告 B | 融合结论 |
|---------|--------|--------|---------|
| **语义缓存** | L0.5 Semantic Cache（复用 LanceDB） | Prefix-Cache 适配层（多 provider） | **互补**：L0.5 做查询级缓存，Prefix-Cache 做 prompt 级缓存，叠加使用 |
| **费用管理** | ModelRouter + Credits Dashboard | CostEngine + TokenJuice 三级压缩 + 记忆成本标签 | **互补**：ModelRouter 做路由，TokenJuice 做压缩，Credits 做计费，三位一体 |
| **知识库** | Obsidian 兼容 + 文件夹索引 + 双向链接 + RAG 增强 | LLM Wiki 编译引擎 + 三视图 + 双向同步 + 溯源链 | **互补**：报告 A 重"输入"（RAG/索引），报告 B 重"输出"（Wiki 编译/可读性） |
| **OS 控制** | OS-Controller + 视觉三层 + Shadow Workspace | API+VLM 双模式 + Hybrid Browser + Remote Operator + AIO Sandbox | **互补**：报告 A 重"架构"（三层/Shadow），报告 B 重"策略"（双模式/三策略混合） |
| **桌面形象** | 悬浮球 + 浮动窗 + 全屏三形态 | 8 人格 + 情绪联动 + Proactive Engine + 语音 | **互补**：报告 A 重"形态"，报告 B 重"人格" |
| **工作流画布** | 蜂群画布（运行时可视化） | WorkflowCanvas（可编排+可执行） | **互补**：运行时可视化 + 设计时编排，两者都要 |
| **自动化** | Cron + Trigger + Watch + Automations | 异步长任务模式 + PlanEngine 联动 | **互补**：定时/触发 + 异步长任务 |
| **安全** | L4 价值层 + 审计 + 回滚 | AIO Sandbox + 凭证加密 + fail-closed + 12 trace | **互补**：报告 A 重"审批"，报告 B 重"隔离+可观测" |

### 1.3 独有贡献清单

**报告 A 独有**（必须吸收）：
1. **自主度滑块 L0-L5**（Cursor 启发）——从内联补全到全自主的连续谱
2. **Shadow Workspace**（Cursor Cloud Agent 启发）——Agent 后台工作，用户不阻塞
3. **Credits 计费模式**（Notion 启发）——信用不足自动暂停
4. **24/7 Automations**（Cron + Trigger + Watch）——无人值守
5. **多端同源**（CLI + PWA + API + 浏览器插件）
6. **三大工作场景闭环**（写作者/程序员/管理者）
7. **Open WebUI RAG**（9 向量库 + 9 文档提取引擎 + BM25 混合搜索）
8. **Dify 100+ 模型提供商** + Workflow DSL
9. **Gateway 守护进程模式**（OpenClaw 启发）
10. **SOUL.md / AGENTS.md / TOOLS.md 注入**（OpenClaw 启发）

**报告 B 独有**（必须吸收）：
1. **信任三原则**（可读/可编辑/可追溯）——产品哲学层升华
2. **OpenAkita MDRM 5 维关系图谱**（因果/时序/实体/层级/相似度）
3. **OpenAkita Organization Orchestration**（CEO/CTO/CFO 角色化 + deadlock detection）
4. **OpenAkita 反幻觉 [来源:工具] badge**
5. **UI-TARS Hybrid Browser Agent**（GUI+CDP+DOM 三策略混合）
6. **UI-TARS Event Stream 协议化** + EventStreamViewer
7. **UI-TARS AIO Agent Sandbox**（文件/网络/进程隔离）
8. **CoPaw 凭证加密卷分离**（DPAPI/Keychain/libsecret）
9. **CoPaw 异步长任务模式**（睡前目标，醒来见结果）
10. **LLM Wiki 编译引擎**（Karpathy 理念，知识编译非拼凑）
11. **Obsidian Skills 三层架构**（协议/能力/执行分层）
12. **记忆溯源链**（provenance 字段 + 修改历史）
13. **记忆双向同步**（用户编辑 ↔ AI 写入）
14. **TokenJuice 三级压缩**（脱敏 + 压缩 + 摘要）
15. **License 兼容性矩阵**

---

## 第二部分：综合竞品画像（13 个）

### 2.1 开源竞品（9 个）

| 竞品 | License | 核心优势 | 借鉴价值 | 借鉴边界 |
|------|---------|---------|---------|---------|
| **OpenClaw** | MIT | 20+ 渠道、语音唤醒、Live Canvas、Skills 生态、Gateway 守护进程、fail-closed 安全 | 渠道/守护进程/SOUL.md/fail-closed | ✅ 可代码级借鉴 |
| **Hermes/GPT-Runner** | MIT | `.gpt.md` 预设文件化、三端、团队共享、MoA 合议 | 预设文件化/MoA | ✅ 可代码级借鉴 |
| **Open WebUI** | MIT | RAG 9 向量库、5 插件类型、Channels、日历、Automations、企业认证、OpenTelemetry | RAG 管道/插件分层/企业特性 | ✅ 可代码级借鉴 |
| **Dify** | Apache 2.0 | 可视化 Workflow、100+ 模型、50+ 工具、LLMOps、BaaS API | 工作流编排/模型生态/LLMOps | ✅ 可代码级借鉴 |
| **Reasonix** | 未明确 | 语义缓存、费用管理、缓存命中率、思维树、反思式推理 | 缓存/费用/推理链 | ⚠️ 思路借鉴 |
| **OpenAkita** | AGPL-3.0 | MDRM 5 维图谱、Organization Orchestration、6-Layer sandbox、反幻觉 badge、12 trace span、8 Personas、Proactive Engine、89+ tools | 架构思路全面借鉴 | ❌ 仅思路，不可 fork |
| **OpenHuman** | MIT | 电脑管理、UI 自动化、TokenJuice、桌面吉祥物、Obsidian 导出、118+ OAuth | 电脑操作/压缩/吉祥物 | ✅ 可代码级借鉴 |
| **UI-TARS-desktop** | Apache 2.0 | VLM 视觉操作、Local+Remote Operator、Hybrid Browser、Event Stream、AIO Sandbox | OS-Controller 重构 | ✅ 可代码级借鉴 |
| **CoPaw** | 未明确 | 凭证加密卷、Docker 部署、skill-pool tags、异步任务 | 安全/部署/异步 | ⚠️ 需核实 license |

### 2.2 国内大厂竞品（4 个）

| 竞品 | 厂商 | 核心创新 | 借鉴价值 |
|------|------|---------|---------|
| **AutoClaw / GLM-PC** | 智谱 | "Every PC, 1 Minute"、50+ 步长链、CogAgent-9B 开源视觉模型 | 长链操作/视觉模型 |
| **Kimi K2.6** | 月之暗面 | Agent Swarm 集群、Deep Research、桌面端+浏览器插件 | 深度研究/多端 |
| **Cursor** | Anysphere | Cloud Agents、Automations、Design Mode、Marketplace、自主度滑块 | 自主度滑块/Shadow Workspace/自动化 |
| **Notion AI** | Notion | Custom Agents 24/7、Enterprise Search、Credits 计费 | 无人值守/ Credits /跨应用搜索 |

### 2.3 知识库生态（2 个）

| 项目 | 核心价值 | 借鉴点 |
|------|---------|--------|
| **Obsidian** | 双向链接、图谱可视化、本地 Markdown、社区插件、Canvas、Dataview | 双向链接语法/图谱视图/Dataview 查询 |
| **LLM Wiki**（Karpathy 理念） | AI 持续维护的结构化维基、知识"编译"非拼凑、index.md + log.md 自动维护 | Wiki 编译引擎/知识生长模式 |

---

## 第三部分：统一创新框架

### 3.1 产品哲学：信任三原则

> **核心宣言**：「你无法信任一段你无法阅读的记忆」
>
> Nebula的所有记忆必须**可读、可编辑、可追溯**——这是区别于黑盒 AI 的根本立场。

| 原则 | 含义 | 落地任务 |
|------|------|---------|
| **可读（Readable）** | 所有记忆以人类可读的 Markdown 渲染；LLM Wiki 编译输出；图谱/时间轴/Markdown 三视图 | T-E-01, T-B-02 |
| **可编辑（Editable）** | 用户可任意修改记忆，AI 写入与人类编辑双向同步，每次编辑记录版本 | T-B-03 |
| **可追溯（Traceable）** | 每条记忆携带 provenance（来源/时间/hash/修改链），决策可回溯到具体记忆 | T-B-04 |

**与竞品的根本差异**：
- OpenClaw/Hermes 的记忆是**黑盒向量库**——用户无法阅读
- OpenHuman 的记忆**可导出但单向**——AI 写入，用户只读
- Reasonix 的记忆是**Append-only 历史**——可追溯但不可编辑
- **Nebula的记忆是可读+可编辑+可追溯的"信任记忆"**——行业唯一

### 3.2 四大支柱 × 六大趋势 融合矩阵

```
                    ┌─────────────────────────────────────────┐
                    │          Nebula v3.0 创新框架              │
                    │                                          │
                    │   核心哲学：信任三原则（可读/可编辑/可追溯）  │
                    │                                          │
                    └──────────────────┬───────────────────────┘
                                       │
        ┌──────────────┬───────────────┼───────────────┬──────────────┐
        ▼              ▼               ▼               ▼              ▼
   ┌─────────┐  ┌──────────┐   ┌──────────┐   ┌──────────┐   ┌──────────┐
   │ 支柱一   │  │ 支柱二    │   │ 支柱三    │   │ 支柱四    │   │ 贯穿层   │
   │ 更省钱   │  │ 更智能    │   │ 更贴合    │   │ 更快      │   │          │
   │         │  │          │   │          │   │          │   │ 8层记忆   │
   │ CostEng │  │ LLM Wiki │   │ OS-Ctrl  │   │ 冷启动   │   │ L4价值层  │
   │ TokenJui│  │ 三视图    │   │ 视觉Agent │   │ 首响     │   │ E2EE同步  │
   │ ModelRtr│  │ 双向同步  │   │ 场景闭环  │   │ 桌面形象  │   │ 审计日志  │
   │ Credits │  │ 溯源链    │   │          │   │          │   │          │
   └────┬────┘  └────┬─────┘   └────┬─────┘   └────┬─────┘   └──────────┘
        │            │              │              │
        └────────────┴──────┬───────┴──────────────┘
                            │
                    ┌───────┴────────┐
                    │   六大趋势加持   │
                    │                │
                    │ 1.自主度滑块    │
                    │ 2.Shadow WS    │
                    │ 3.视觉驱动     │
                    │ 4.Credits计费  │
                    │ 5.24/7 Auto    │
                    │ 6.多端同源     │
                    └────────────────┘
```

### 3.3 差异化护城河（终极版）

| 维度 | OpenClaw | Open WebUI | Dify | 智谱 AutoGLM | Cursor | **Nebula v3.0** |
|------|---------|-----------|------|-------------|--------|---------------|
| **记忆深度** | 无层 | 持久记忆 | 无 | 无 | 代码索引 | **8 层 L0-L7** |
| **记忆可读性** | 黑盒 | 可导出 | 无 | 无 | 无 | **LLM Wiki + 三视图 + 双向同步** |
| **记忆可追溯** | 无 | 无 | 无 | 无 | 无 | **provenance + 版本控制** |
| **费用管理** | 无 | Usage | 无 | 无 | 按量 | **路由+压缩+Credits+预算** |
| **本地知识库** | 无 | RAG | RAG | 无 | 代码索引 | **Obsidian 兼容+图谱+Wiki编译** |
| **桌面形象** | 菜单栏 | Web UI | Web UI | Web UI | IDE | **悬浮球+8人格+情绪+语音** |
| **电脑操作** | 无 | Terminal | 无 | 50步长链 | Cloud Agent | **API+VLM双模+L4审批+Shadow** |
| **浏览器Agent** | 无 | 无 | 无 | 无 | 无 | **GUI+CDP+DOM三策略混合** |
| **安全深度** | DM pairing | RBAC | 无 | 无 | 无 | **L4价值层+AIO Sandbox+凭证加密** |
| **工作流可视化** | 无 | 无 | 画布(设计时) | 无 | 无 | **蜂群画布(运行时)+编排(设计时)** |
| **自主度** | 全自主 | 全自主 | 全自主 | 全自主 | 滑块 | **6级滑块 L0-L5** |
| **自动化** | 无 | Automations | 无 | 无 | Automations | **Cron+Trigger+Watch+异步长任务** |
| **协议层** | 无 | 无 | 无 | 无 | 无 | **Event Stream协议化+MCP三transport** |
| **多端** | 多渠道 | Web+Docker | Web+API | Web+App | Desktop+CLI+Slack+iOS | **Desktop+CLI+PWA+渠道+E2EE** |

---

## 第四部分：综合任务清单（68 个）

> 任务编号统一为 `T-E-*`（Evolution），按四大支柱 + 贯穿层组织。
> 优先级：P0（立即可做）> P1（关键路径）> P2（重要）> P3（增强）

### 4.1 支柱一：更省钱（14 个任务）

#### 核心模块：CostEngine + TokenJuice + ModelRouter + Credits

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-A-01 | **SemanticCache 层**（L0.5）：LlmGateway 入口加 `semantic_cache.check(embed(query))`，复用 LanceDB，cosine>0.92 直接返回，TTL 1h | P0 | S | A |
| T-E-A-02 | **TokenJuice 三级压缩**：L1 脱敏 + L2 压缩（HTML→MD/URL缩短/非ASCII）+ L3 摘要（旧对话 LLM 摘要替代原文），目标 -85% | P1 | M | B |
| T-E-A-03 | **ModelRouter 智能路由**：本地小模型分类（简单→Ollama/中等→DeepSeek/复杂→Claude），60% 走免费模型 | P1 | M | A |
| T-E-A-04 | **Prefix-Cache 适配层**：多 provider prompt caching（Anthropic/OpenAI API），长 system prompt 只计一次 | P1 | M | B |
| T-E-A-05 | **日预算限制**：`settings.json` 新增 `daily_budget_usd`，超限自动降级到 Ollama | P1 | S | A+B |
| T-E-A-06 | **Token 费用追踪**：每次 LLM 调用记录 input_tokens + output_tokens + model + cost | P0 | S | A+B |
| T-E-A-07 | **Credits Dashboard**：日/周/月趋势图、按 provider/任务/Agent 分桶、预算预警线、缓存命中率关联 | P1 | M | A+B |
| T-E-A-08 | **费用报告命令**：`nebula cost report` 输出本月各模型费用明细 | P2 | S | A |
| T-E-A-09 | **记忆成本标签**：每条记忆写入时记录 `ingest_cost`（向量化+LLM 抽取消耗） | P3 | S | B |
| T-E-A-10 | **缓存命中率仪表盘**：实时显示"省了多少 Token / 省了多少钱"，命中率<30% 报警 | P1 | S | A |
| T-E-A-11 | **智能预取**：用户打开文件时，预取该文件相关的历史对话缓存 | P2 | M | A |
| T-E-A-12 | **Automation Credits**：24/7 自动化任务独立计费，与对话费用分离 | P2 | M | A |
| T-E-A-13 | **费用数据加密存储**：与凭证加密卷分离联动（T-E-F-07） | P3 | S | B |
| T-E-A-14 | **Arena A/B 测试**：模型对比 + ELO 排行榜（借鉴 Dify/Open WebUI） | P3 | M | A |

### 4.2 支柱二：更智能（18 个任务）

#### 核心模块：LLM Wiki + Obsidian 兼容 + 可读记忆 + 推理链

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-B-01 | **LLM Wiki 编译引擎**：每次对话后 AI"编译"结构化 Markdown 笔记写入 `wiki/`，含 `[[双向链接]]` | P1 | L | B |
| T-E-B-02 | **可读记忆三视图**：①Markdown 视图（可编辑笔记）②图谱视图（3D 可视化）③时间轴视图（`/journey` 回放） | P1 | XL | B |
| T-E-B-03 | **记忆双向同步**：用户编辑→写回 SQLite+重新向量化+记录版本；AI 写入→更新 Markdown | P1 | L | B |
| T-E-B-04 | **记忆溯源链**：每条记忆携带 `provenance`（来源/时间/hash/修改链）+ 前端显示 `[来源:工具]` badge | P1 | M | B |
| T-E-B-05 | **双向链接 `[[]]` 语法**：EntityExtractor 新增链接生成 + 用户编辑时 `[[` 触发实体自动补全 | P2 | M | A+B |
| T-E-B-06 | **index.md + log.md 自动维护**：自动生成目录（按主题/时间/重要性）+ 更新日志 | P2 | S | B |
| T-E-B-07 | **知识图谱视图**：MemoryMap 升级为 Obsidian 风格力导向图（D3/PixiJS），支持 1000+ 节点 | P1 | L | A+B |
| T-E-B-08 | **Obsidian vault 兼容**：直接读取 `.obsidian/` 配置 + Markdown 双向同步 | P2 | M | A |
| T-E-B-09 | **文件夹监控索引**：监控指定目录，文件变更时自动 `SpongeEngine::absorb_file()` | P1 | M | A |
| T-E-B-10 | **`#` 命令注入**：ChatPanel 解析 `#filename` 触发文件吸收到对话 | P1 | S | A |
| T-E-B-11 | **BM25 + 向量混合搜索**（Hybrid Search）：解决纯向量在关键词精确匹配场景召回率低 | P0 | M | A |
| T-E-B-12 | **文档提取引擎**：集成 docling（Rust 友好，可 WASM 化），支持 PDF/DOCX/PPT | P1 | M | A |
| T-E-B-13 | **知识卡片**：AI 回复中 `[[实体]]` 可点击，弹出知识卡片（定义+关联+来源） | P2 | M | A |
| T-E-B-14 | **Dataview 式查询 DSL**：`FROM L3 WHERE kind=fact AND importance>0.7` 风格查询 | P2 | M | B |
| T-E-B-15 | **AI 自动整理 MOC**：三定时机制联动，每日按主题聚类生成"主题笔记"（Map of Content） | P2 | L | A+B |
| T-E-B-16 | **MDRM 5 维关系图谱**：扩展 CausalGraphEngine 为 5 维（因果/时序/实体/层级/相似度） | P2 | XL | B |
| T-E-B-17 | **ReasoningChain 结构体**：记录每步推理 `premise → inference → confidence → evidence` | P1 | M | A |
| T-E-B-18 | **思维树模式**：SwarmOrchestrator 多 Agent 各走一条推理路径，Negotiator 比较质量 | P2 | L | A |

### 4.3 支柱三：更贴合工作场景（20 个任务）

#### 核心模块：OS-Controller + 视觉 + 场景闭环 + 自动化 + 多端

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-C-01 | **OS-Controller 双模式**：①API 模式（UIAutomation/AT-SPI）②VLM 模式（截图+视觉识别），PlanEngine 自动选择 | P1 | XL | B |
| T-E-C-02 | **ScreenReader 截图理解**：集成 Qwen2.5-VL-3B 本地运行，`screenshot()` → 文字描述 | P1 | M | A+B |
| T-E-C-03 | **UiAutomator 抽象层**：Windows UIA / macOS AX / Linux AT-SPI 统一抽象 | P2 | XL | A |
| T-E-C-04 | **ActionExecutor**：click/type/open/switch 原子操作 + L4 审批闭环 | P2 | L | A |
| T-E-C-05 | **OS-Controller Sidecar**：独立进程运行，与主进程 IPC 通信 | P1 | L | A+B |
| T-E-C-06 | **Hybrid Browser Agent**：GUI 视觉点击 + CDP 协议（existing-session 复用）+ DOM 选择器，自动选最优 | P1 | XL | B |
| T-E-C-07 | **Remote Operator**：E2EE 加密通道远程控制另一台设备 | P3 | XL | B |
| T-E-C-08 | **Shadow Workspace**：任务在独立 git branch + 临时目录执行，不影响用户当前工作 | P1 | L | A |
| T-E-C-09 | **任务录屏回放**：记录 Agent 操作序列（文件修改+命令执行），用户可回放审查 | P2 | M | A |
| T-E-C-10 | **异步长任务模式**：用户描述目标后后台分步执行（跨小时/跨天），与 PlanEngine 联动 | P2 | L | A+B |
| T-E-C-11 | **操作录制回放**：记录用户操作序列 → AI 可回放"看一遍就会" | P2 | M | A |
| T-E-C-12 | **Design Mode**：用户在 UI 上画框/标注 → Agent 根据视觉提示操作 | P3 | L | A |
| T-E-C-13 | **工作场景模板库**：预置 Writer/Coder/Manager 三套场景模板 + 20+ 工作流模板 | P2 | M | A+B |
| T-E-C-14 | **剪贴板智能监听**：自动识别可操作内容（URL/代码/表格） | P2 | M | A |
| T-E-C-15 | **语音交互引擎**：Whisper.cpp 本地 STT + TTS + 唤醒词"Nebula" + 嘴型同步 | P2 | XL | A+B |
| T-E-C-16 | **一键导出**：对话/报告/代码 → Markdown/PDF/DOCX/Git | P2 | M | A |
| T-E-C-17 | **IM 扫码绑定**：Feishu/WeCom/DingTalk（中国市场优先） | P2 | L | B |
| T-E-C-18 | **OAuth 集成层**：Gmail/Notion/GitHub/Feishu/Calendar 5 个首批 | P2 | XL | B |
| T-E-C-19 | **多端协同**：CLI（clap）+ PWA + API 网关（gRPC+REST）+ 浏览器插件 | P2 | XL | A+B |
| T-E-C-20 | **Docker 部署**：headless 模式 + 数据卷/密钥卷分离 | P3 | M | B |

### 4.4 支柱四：更快（10 个任务）

#### 核心模块：性能优化 + 桌面形象 + Proactive

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-D-01 | **冷启动 < 3s 工程**：cached model metadata + lazy slash-command + React.lazy + SQLite 迁移并行 + L0Cache 预热 | P1 | M | A+B |
| T-E-D-02 | **首响 < 500ms 工程**：流式 IPC + prefix cache + Ollama 预热 + 首 token 优先 | P1 | M | B |
| T-E-D-03 | **桌面悬浮球**：Tauri 2.0 多窗口透明悬浮窗 + 状态指示器（空闲/思考/执行/通知） | P1 | M | A+B |
| T-E-D-04 | **8 人格系统**：管家/Jarvis/助手/女友/男友/技术专家/商务/家庭，表情随 L5 情绪联动 | P2 | XL | B |
| T-E-D-05 | **Proactive Engine**：主动问候/任务跟进/闲聊/晚安，频率自适应 + 每日"学习汇报" | P2 | L | B |
| T-E-D-06 | **文件拖拽 + 右键集成**：拖拽文件到悬浮球自动 absorb + 系统右键"问Nebula" | P2 | M | A |
| T-E-D-07 | **浮动进度窗**：长任务显示进度条 + 中断按钮，不阻塞用户 | P2 | S | A |
| T-E-D-08 | **WebGL 引擎复用**：MemoryMap + WorkflowCanvas 共用 PixiJS，1000+ 节点 60fps | P2 | XL | B |
| T-E-D-09 | **UI 性能基准 CI**：1000/5000/10000 节点 fps 基线，回归报警 | P2 | M | B |
| T-E-D-10 | **多 Agent 并行流式渲染**：每个 Agent 输出独立流式渲染 + 工具调用计时统计 | P2 | M | B |

### 4.5 贯穿层：蜂群 + 安全 + 协议 + 自动化 + 自主度（6 个任务组）

#### 蜂群与协作

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-S-01 | **Agent 角色专业化**：恢复 Coder/Writer/Reviewer 角色，Dify 模式（system_prompt + tool_set + knowledge_scope） | P0 | M | A |
| T-E-S-02 | **LLM Function Calling**：蜂群支持工具调用（当前仅文本协商） | P0 | L | A |
| T-E-S-03 | **DynamicAgentPool 重构**：`Arc<tokio::sync::Mutex<>>` + 数量按复杂度动态调整 | P1 | M | A |
| T-E-S-04 | **MoA 一等公民**：Negotiator 升级为多模型合议（多 provider 投票） | P1 | L | B |
| T-E-S-05 | **deadlock detection**：AgentBus 检测循环等待 + LLM 仲裁打破 | P2 | M | B |
| T-E-S-06 | **Organization Orchestration**：CEO/CTO/CFO 角色化 + blackboard 共享 | P3 | XL | B |

#### 工作流可视化

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-S-10 | **WorkflowCanvas 可编排画布**：React Flow 拖拽编排（设计时），节点类型：Memory/Skill/Agent/LLM/Condition/Loop | P1 | XL | B |
| T-E-S-11 | **蜂群运行时画布**：SwarmEvent 实时渲染为节点+连线（运行时可视化） | P1 | L | A |
| T-E-S-12 | **节点交互**：点击 Agent 节点查看输出、拖拽连线修改顺序、右键增删 Agent | P2 | M | A |
| T-E-S-13 | **工作流模板**：保存执行图为 YAML 模板，下次复用 | P2 | M | A |
| T-E-S-14 | **执行回放**：记录 SwarmEvent 时间线，支持回放/快进 | P2 | M | A |

#### 安全与可观测

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-S-20 | **exec fail-closed**：exec approvals 超时默认拒绝（OpenClaw） | P0 | S | B |
| T-E-S-21 | **assemble_context ACL**：未授权内容不进 prompt context（OpenClaw） | P0 | S | B |
| T-E-S-22 | **AIO Sandbox**：升级 WASM 为 all-in-one 隔离（文件 chroot + 网络白名单 + 进程命名空间） | P2 | XL | B |
| T-E-S-23 | **凭证加密卷分离**：DPAPI/Keychain/libsecret，与 settings.json 解耦 | P1 | M | B |
| T-E-S-24 | **文件快照回滚**：Skill 执行前快照工作区，失败后回滚（file_write 类） | P2 | M | B |
| T-E-S-25 | **12 trace span types**：扩展为 chat/swarm/skill/memory/llm/reflect/acl/plan/crdt/sidecar/channel/export | P1 | M | B |
| T-E-S-26 | **Event Stream 协议化**：SwarmEvent 升级为协议（type/payload/trace_id/timestamp）+ EventStreamViewer | P1 | L | B |
| T-E-S-27 | **trusted diagnostics channels**：诊断信息走独立可信通道（OpenClaw） | P1 | S | B |
| T-E-S-28 | **标注+持续改进**：用户对 AI 回复标注好/坏，回流到微调数据集（Dify） | P2 | M | A |
| T-E-S-29 | **OpenTelemetry 原生集成**：tracing span 覆盖全链路（chat→LLM→memory→swarm） | P1 | M | A |

#### 协议与集成

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-S-30 | **MCP `tools/list` + `tools/call` 补完**：当前是桩，必须真实实现 | P0 | M | A |
| T-E-S-31 | **MCP SSE transport**：新增 SseTransport 变体（stdio/HTTP/SSE 三种） | P1 | M | B |
| T-E-S-32 | **MCP stdio 子进程管理**：自动发现并启动本地 MCP 服务器 | P1 | M | A |
| T-E-S-33 | **OpenAPI 工具服务器**：自动解析 OpenAPI 3.0 spec 生成 Tool 定义 | P1 | L | A |
| T-E-S-34 | **MCPO（MCP over HTTP）**：允许远程 MCP 服务器 | P2 | M | A |
| T-E-S-35 | **5 层插件模型**：Filter/Action/Pipe/Tool/Skill 分层（Open WebUI） | P0 | L | A |
| T-E-S-36 | **SkillEngine 三层架构**：协议层 / 能力层 / 执行层 分离（Obsidian Skills） | P2 | L | B |
| T-E-S-37 | **skill-pool tags**：Skill 结构新增 tags + 按标签过滤 | P3 | S | B |
| T-E-S-38 | **可视化生成 Skills 套件**：canvas-creator / mermaid-creator / mindmap-creator | P2 | M | B |
| T-E-S-39 | **SOUL.md / AGENTS.md / TOOLS.md 注入**：用户自定义 AI 人格（OpenClaw） | P1 | M | A |
| T-E-S-40 | **OpenAI 兼容层**：vLLM/LMStudio/OpenRouter 直接接入 | P1 | M | A |
| T-E-S-41 | **models.json 动态配置**：用户自行添加提供商 | P1 | S | A |
| T-E-S-42 | **VectorStore trait 抽象**：支持切换 LanceDB/Qdrant/ChromaDB | P2 | L | A |
| T-E-S-43 | **SQLite 加密（SQLCipher）**：与 E2EE 同步配合 | P1 | M | A |
| T-E-S-44 | **StorageBackend trait**：支持本地/S3/WebDAV | P2 | L | A |
| T-E-S-45 | **ClawHub 双向兼容**：技能导出为 ClawHub 格式 | P2 | M | A |
| T-E-S-46 | **技能发布命令**：`nebula skill publish` 到社区市场 | P2 | M | A |

#### 自动化与自主度

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-S-50 | **自主度滑块 L0-L5**：L0 内联补全 / L1 定向编辑 / L2 对话 / L3 Plan / L4 蜂群 / L5 后台自动化 | P0 | L | A |
| T-E-S-51 | **Level 0 内联补全**：ChatPanel 输入框 AI 建议补全（本地小模型，零成本） | P0 | M | A |
| T-E-S-52 | **Level 1 定向编辑**：选中文字 + 快捷键 → AI 局部改写 | P1 | M | A |
| T-E-S-53 | **Cron 定时任务引擎**：`cron.rs` + SQLite 存储 + Sidecar 执行 | P1 | L | A |
| T-E-S-54 | **事件触发器**：文件监听 + 消息监听 + Webhook 接收 | P2 | M | A |
| T-E-S-55 | **条件监控 Watch**：网页抓取 + 系统指标 + 日历事件 | P2 | M | A |
| T-E-S-56 | **Automation 模板**：预置日报/周报/费用报告模板 | P2 | S | A |
| T-E-S-57 | **后台执行通知**：系统通知 + 悬浮球状态变化 | P2 | S | A |
| T-E-S-58 | **Calendar 组件**：月/周/日视图 + AI Function Calling 管理日程 | P1 | M | A |
| T-E-S-59 | **统一收件箱**：所有渠道消息汇入 ChatPanel（OpenClaw） | P0 | M | A |

#### 基础设施

| 任务 ID | 描述 | 优先级 | 复杂度 | 来源 |
|---------|------|--------|--------|------|
| T-E-S-60 | **Gateway 守护进程**：`nebula gateway` 子命令 + 系统服务注册 | P1 | L | A |
| T-E-S-61 | **SidecarManager 自动 start_all**：bootstrap 时自动启动 + gRPC HealthCheck | P1 | M | A |
| T-E-S-62 | **`nebula doctor` 健康检查**：一键诊断配置/权限/连接 | P2 | S | A |
| T-E-S-63 | **三定时机制**：Consolidation（03:00）+ Self-check（12:00）+ Retrospection（21:00） | P1 | L | B |
| T-E-S-64 | **反幻觉 [来源:工具] badge**：记忆来源标记 + 一致性 belt 警示 | P1 | M | B |

---

## 第五部分：统一进化路线图

### 5.1 阶段规划

| 阶段 | 主题 | 核心任务 | 预期效果 |
|------|------|---------|---------|
| **v2.1**（当前） | 记忆闭环 | T-S1 全部 | 57% → 75% |
| **v2.2** | 协议+安全 | Stage 2a/2b + T-E-S-20~29 | 75% → 88% |
| **v2.3** | **省钱+低门槛革命** | T-E-A-01~07 + T-E-S-50~52 | Token 降 70%，自主度 L0-L1 |
| **v2.4** | **知识革命** | T-E-B-01~18 | 从"聊天"到"第二大脑" |
| **v2.5** | **形象+后台革命** | T-E-D-01~07 + T-E-C-08~10 | 悬浮球 + Shadow Workspace |
| **v2.6** | **可视+视觉革命** | T-E-S-10~14 + T-E-C-01~06 | 蜂群画布 + 视觉 Agent |
| **v3.0** | **全自主革命** | T-E-C-13~20 + T-E-S-53~59 | 24/7 自动化 + 多端 + 场景闭环 |

### 5.2 P0 任务清单（立即可做，不依赖 Stage）

| 任务 ID | 描述 | 复杂度 |
|---------|------|--------|
| T-E-A-01 | SemanticCache 层（L0.5） | S |
| T-E-A-06 | Token 费用追踪 | S |
| T-E-B-11 | BM25 + 向量混合搜索 | M |
| T-E-S-01 | Agent 角色专业化 | M |
| T-E-S-02 | LLM Function Calling | L |
| T-E-S-20 | exec fail-closed | S |
| T-E-S-21 | assemble_context ACL | S |
| T-E-S-30 | MCP tools/list + tools/call 补完 | M |
| T-E-S-35 | 5 层插件模型 | L |
| T-E-S-50 | 自主度滑块 L0-L5 框架 | L |
| T-E-S-51 | Level 0 内联补全 | M |
| T-E-S-59 | 统一收件箱 | M |

### 5.3 量化目标

| 维度 | 当前 | v2.3 目标 | v3.0 目标 |
|------|------|----------|----------|
| **平均响应时间** | 2-5s | <1s（40% 缓存命中） | <200ms（80% 本地） |
| **月度 Token 成本** | ~$30 | ~$9（降 70%） | ~$3（降 90%） |
| **日活跃次数** | 3-5 次 | 10-15 次（悬浮球） | 30-50 次（OS-Controller） |
| **知识覆盖** | 仅对话 | +本地文件 | +全工作场景 |
| **可操作范围** | 仅文本 | +文件操作 | +电脑操作 |
| **自主度等级** | 仅 L4 | L0-L4 | L0-L5 |
| **自动化任务** | 0 | 0 | 5+ 个定时/触发 |
| **可用终端** | 仅桌面 | +CLI | +CLI+PWA+渠道 |
| **记忆可读性** | 黑盒 | Markdown 视图 | 三视图+双向同步 |
| **记忆可追溯** | 无 | provenance 字段 | 完整溯源链 |

---

## 第六部分：License 合规与生态策略

### 6.1 License 兼容性矩阵

| 对标项目 | License | 与 nebula(MIT) 兼容 | 借鉴边界 |
|---------|---------|------------------------|---------|
| OpenClaw | MIT | ✅ | 可代码级借鉴 |
| Hermes | MIT | ✅ | 可代码级借鉴 |
| Open WebUI | MIT | ✅ | 可代码级借鉴 |
| Dify | Apache 2.0 | ✅ | 可代码级借鉴（保留 NOTICE） |
| UI-TARS-desktop | Apache 2.0 | ✅ | 可代码级借鉴（保留 NOTICE） |
| OpenHuman | MIT | ✅ | 可代码级借鉴 |
| Reasonix | 未明确 | ⚠️ | 思路借鉴，需核实 |
| CoPaw | 未明确 | ⚠️ | 思路借鉴，需核实 |
| OpenAkita | **AGPL-3.0** | ❌ | **仅思路借鉴，不可代码 fork** |
| Obsidian Skills | GPL-3.0 | ⚠️ | 思路借鉴，不可代码 fork |
| LLM Wiki 理念 | 公开理念 | ✅ | 自由借鉴 |

### 6.2 生态策略

| 生态 | 策略 |
|------|------|
| **MCP 生态** | 补完 tools/list + tools/call，兼容 stdio/HTTP/SSE 三 transport |
| **ClawHub 生态** | 双向兼容，技能可导入导出 |
| **agentskills.io** | 对齐 16 categories 工具分类 |
| **Obsidian 生态** | vault 兼容，30M 用户零迁移成本 |
| **OpenAPI 生态** | 自动从 spec 生成 Tool 定义 |
| **Dify DSL** | WorkflowSpec YAML 互操作 |

---

## 第七部分：行动建议

### 7.1 立即可做（本周启动，不依赖任何 Stage）

**安全加固组**（2 人天）：
- T-E-S-20 exec fail-closed
- T-E-S-21 assemble_context ACL

**省钱先锋组**（3 人天）：
- T-E-A-01 SemanticCache 层（复用 LanceDB，零新增依赖）
- T-E-A-06 Token 费用追踪

**记忆增强组**（3 人天）：
- T-E-B-11 BM25 + 向量混合搜索
- T-E-B-10 `#` 命令注入

**协议补完组**（5 人天）：
- T-E-S-30 MCP tools/list + tools/call 真实实现
- T-E-S-59 统一收件箱

### 7.2 v2.3 省钱革命（Stage 1 完成后）

**核心交付**：
- T-E-A-01~07（CostEngine + TokenJuice + ModelRouter + Credits）
- T-E-S-50~52（自主度滑块 L0-L1）

**预期效果**：Token 成本降 70%，用户获得"内联补全"轻量模式

### 7.3 文档更新建议

基于本综合报告，建议更新以下文档：
1. **`ROADMAP_v2.2.md`**：新增 Stage 7（四大支柱创新）章节，纳入 68 个任务
2. **`WHITEPAPER_v3.0.md`**：正式定义"信任三原则"+ 四大支柱 + 六大趋势
3. **`EXPERT_AGENTS_v2.2.md`**：正式定义 EA-6/EA-7 角色矩阵
4. **`ARCHITECTURE.md`**：补充 OS-Controller 双模式 / Event Stream 协议 / AIO Sandbox 架构

### 7.4 优先级决策矩阵

```
                    高价值
                      │
           P0区       │       P1区
      ┌───────────────┼───────────────┐
      │ T-E-S-20      │ T-E-A-02      │
      │ T-E-S-21      │ T-E-A-03      │
      │ T-E-A-01      │ T-E-B-01      │
      │ T-E-A-06      │ T-E-B-02      │
      │ T-E-B-11      │ T-E-C-01      │
      │ T-E-S-30      │ T-E-C-06      │
      │ T-E-S-35      │ T-E-S-10      │
      │ T-E-S-50      │ T-E-S-53      │
      │ T-E-S-51      │ T-E-S-60      │
 低成本├───────────────┼───────────────┤高成本
      │              │               │
           P3区       │       P2区
      │              │ T-E-B-16      │
      │              │ T-E-C-03      │
      │              │ T-E-D-04      │
      │              │ T-E-S-22      │
      │              │ T-E-S-06      │
      └───────────────┴───────────────┘
                    低价值
```

---

## 附录 A：双报告任务映射表

| 报告 A 任务 | 报告 B 任务 | 统一任务 ID | 说明 |
|------------|------------|------------|------|
| EA-1 SemanticCache | T-S8-A-03 Prefix-Cache | T-E-A-01 + T-E-A-04 | 互补，两者都做 |
| EA-1 缓存仪表盘 | T-S8-A-04 费用仪表盘 | T-E-A-07 + T-E-A-10 | 合并为 Credits Dashboard |
| EA-3 ModelRouter | T-S8-A-01 CostEngine | T-E-A-03 + T-E-A-05 | 合并，路由+预算 |
| EA-3 Token 追踪 | T-S8-A-06 费用加密 | T-E-A-06 + T-E-A-13 | 合并 |
| EA-2 文件夹索引 | T-S8-E-01 LLM Wiki | T-E-B-01 + T-E-B-09 | 互补，输入+输出 |
| EA-2 双向链接 | T-S8-E-02 双向链接 | T-E-B-05 | 合并 |
| EA-2 知识图谱 | T-S8-B-02 三视图 | T-E-B-02 + T-E-B-07 | 合并 |
| EA-2 Obsidian 兼容 | — | T-E-B-08 | 报告 A 独有 |
| EA-2 RAG 增强 | T-S8-B-11 BM25 | T-E-B-11 + T-E-B-12 | 合并 |
| EA-2 推理链 | — | T-E-B-17 + T-E-B-18 | 报告 A 独有 |
| EA-4 OS-Controller | T-S8-F-01 双模式 | T-E-C-01 | 合并，报告 B 重构为双模式 |
| EA-4 ScreenReader | T-S8-F-01 VLM | T-E-C-02 | 合并 |
| EA-4 UiAutomator | — | T-E-C-03 | 报告 A 独有 |
| — | T-S8-F-03 Hybrid Browser | T-E-C-06 | 报告 B 独有 |
| — | T-S8-F-02 Remote Operator | T-E-C-07 | 报告 B 独有 |
| EA-5 悬浮球 | T-S8-D-03 桌面吉祥物 | T-E-D-03 + T-E-D-04 | 合并，形态+人格 |
| EA-5 插件生态 | T-S8-E-04 三层架构 | T-E-S-35 + T-E-S-36 | 合并 |
| EA-6 蜂群画布 | T-S8-B-01 WorkflowCanvas | T-E-S-10 + T-E-S-11 | 互补，设计时+运行时 |
| EA-6 多渠道 | T-S8-C-04 IM 绑定 | T-E-C-17 + T-E-S-59 | 合并 |
| EA-6 语音 | T-S8-C-02 语音引擎 | T-E-C-15 | 合并 |
| EA-6 日历自动化 | — | T-E-S-58 | 报告 A 独有 |
| EA-7 MCP 补完 | T-S7-F-01 SSE transport | T-E-S-30 + T-E-S-31 | 合并 |
| EA-7 OpenAPI | — | T-E-S-33 | 报告 A 独有 |
| 趋势 自主度滑块 | — | T-E-S-50~52 | 报告 A 独有 |
| 趋势 Shadow WS | T-S8-F-06 异步任务 | T-E-C-08 + T-E-C-10 | 互补 |
| 趋势 Credits | T-S8-A-05 预算 | T-E-A-05 + T-E-A-12 | 合并 |
| 趋势 24/7 Auto | — | T-E-S-53~57 | 报告 A 独有 |
| 趋势 多端 | T-S8-C-05 多端协同 | T-E-C-19 | 合并 |
| — | T-S8-F-04 Event Stream | T-E-S-26 | 报告 B 独有 |
| — | T-S8-F-05 AIO Sandbox | T-E-S-22 | 报告 B 独有 |
| — | T-S8-F-07 凭证加密 | T-E-S-23 | 报告 B 独有 |
| — | T-S8-B-04 溯源链 | T-E-B-04 | 报告 B 独有 |
| — | T-S8-B-03 双向同步 | T-E-B-03 | 报告 B 独有 |
| — | T-S8-E-05 可视化 Skills | T-E-S-38 | 报告 B 独有 |
| — | T-S8-D-04 Proactive | T-E-D-05 | 报告 B 独有 |

---

## 附录 B：一句话定位演进

| 版本 | 定位 |
|------|------|
| v1.0 | 本地优先的 AI Agent 桌面应用 |
| v2.0 | 带 8 层记忆的本地 AI Agent |
| **v3.0** | **省钱的自主式知识型桌面 AI 伙伴**——它记得你的一切知识（可读/可编辑/可追溯），帮你操作电脑（API+VLM 双模式+L4 审批），替你省 Token 钱（智能路由+三级压缩+Credits），6 级自主度按需选择（L0 补全→L5 无人值守），24/7 自动化（Cron+Trigger+Watch），而且一直陪在你桌面上（悬浮球+8 人格+语音）。 |

---

**报告结束**。

本综合报告融合两份独立审议成果，覆盖 13 个竞品、6 大趋势、68 个任务，所有创新均以**可量化的用户价值**为驱动，核心哲学是**「信任三原则」**——可读、可编辑、可追溯。建议作为 `ROADMAP_v2.2.md` 后续规划的核心参考，按 v2.3→v2.4→v2.5→v2.6→v3.0 顺序迭代。

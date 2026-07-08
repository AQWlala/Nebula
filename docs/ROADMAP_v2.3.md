# Nebula · 生产路线图 v2.3

**版本**：v2.3（精简版，聚焦未完成任务）
**日期**：2026-07-08
**作者**：Solo Developer
**前置文档**：
- `ROADMAP_v2.1.md`（Stage 1-6 工程闭环，已全部完成）
- `ROADMAP_v2.2.md`（v2.3 前身，含已完成任务的完整 commit 描述，仅供历史追溯）
- `WHITEPAPER_v2.0.md`（Stage 1-6 设计权威）

---

## 0. v2.2 → v2.3 变更说明

**变更动机**：v2.2 文档 534 行，已完成任务占 70% 篇幅（每条带 commit hash + 实现细节），模型每次读取消耗大量上下文 token。v2.3 精简已完成任务为单行索引，聚焦未完成任务。

**变更范围**：
| 范围 | v2.2 | v2.3 |
|------|------|------|
| 已完成任务 | 详细描述（commit + 测试数 + 实现细节） | 单行索引（ID + 标题 + 完成日期） |
| 未完成任务 | 按支柱分组，散落各处 | 按优先级（P1/P2/P3）分组集中展示 |
| 技术债务 | §4 完整保留 | §4 完整保留（紧凑表） |
| 旧"立即可做"建议 | 已过时（P0 全部完成） | 移除，替换为 v2.3 推进节奏 |
| License 矩阵 | §5 | 保留（§6） |
| 附录 | §7 | 精简为 §7 |

**任务编号体系**：
- `T-S<阶段>-<组>-<序号>`（Stage 1-6，v2.1，全部完成）
- `T-E-<支柱>-<序号>`（Stage 7 创新支柱，A=省钱/B=智能/C=贴合/D=快/S=贯穿层）
- `T-E-L-<序号>`（Loop Engineering 内化，L=Loop）
- `T-D-<领域>-<序号>`（技术债务，F=前端/B=后端/C=CI/T=测试/S=安全）

---

## 1. 当前状态总览

| 支柱 | 已完成 | 未完成 | 进度 |
|------|--------|--------|------|
| A 省钱（T-E-A-01~14） | 14 | 0 | ✅ 100% |
| B 智能（T-E-B-01~18） | 17 | 1 | 94% |
| C 贴合（T-E-C-01~20） | 9 | 11 | 45% |
| D 快（T-E-D-01~10） | 6 | 4 | 60% |
| S 贯穿层（T-E-S-01~63） | 41 | 12 | 77% |
| Loop Engineering（T-E-L-01~08b） | 5 | 5 | 50% |
| **合计 T-E-*** | **92** | **33** | **74%** |
| 技术债务（T-D-*） | 0 | 34 | 0% |

**Stage 7 P0 阶段**：✅ 全部完成（12/12）
**Loop 阶段一**（最小可用 Loop）：✅ 全部完成（T-E-L-01/02/03）
**Loop 阶段二**（信号源+模板+可视化）：1/3 完成（T-E-L-05 待标记），剩余 T-E-L-04 / T-E-L-08a
**Loop 阶段三**（成本+可观测+设计）：1/3 完成（T-E-L-06 待提交），剩余 T-E-L-07 / T-E-L-08b

---

## 2. 已完成任务索引（精简，仅 ID + 标题 + 日期）

### 2.1 支柱 A 省钱（14/14 ✅）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-A-01 | SemanticCache 层（L0.5） | 2026-07-03 |
| T-E-A-02 | TokenJuice 三级压缩 | 2026-07-03 |
| T-E-A-03 | ModelRouter 智能路由 | 2026-07-03 |
| T-E-A-04 | Prefix-Cache 适配层 | 2026-07-03 |
| T-E-A-05 | 日预算限制 | 2026-07-03 |
| T-E-A-06 | Token 费用追踪 | 2026-07-03 |
| T-E-A-07 | Credits Dashboard | 2026-07-03 |
| T-E-A-08 | 费用报告命令 | 2026-07-03 |
| T-E-A-09 | 记忆成本标签 | 2026-07-04 |
| T-E-A-10 | 缓存命中率仪表盘 | 2026-07-03 |
| T-E-A-11 | 智能预取 | 2026-07-04 |
| T-E-A-12 | Automation Credits | 2026-07-04 |
| T-E-A-13 | 费用数据加密存储 | 2026-07-04 |
| T-E-A-14 | Arena A/B 测试 | 2026-07-04 |

### 2.2 支柱 B 智能（17/18 ✅）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-B-01 | LLM Wiki 编译引擎 | 2026-07-04 |
| T-E-B-02 | 可读记忆三视图 | 2026-07-07 |
| T-E-B-03 | 记忆双向同步 | 2026-07-04 |
| T-E-B-04 | 记忆溯源链 | 2026-07-03 |
| T-E-B-05 | 双向链接 `[[]]` 语法 | 2026-07-04 |
| T-E-B-06 | index.md + log.md 自动维护 | 2026-07-04 |
| T-E-B-07 | 知识图谱视图 | 2026-07-07 |
| T-E-B-08 | Obsidian vault 兼容 | 2026-07-07 |
| T-E-B-09 | 文件夹监控索引 | 2026-07-03 |
| T-E-B-10 | `#` 命令注入 | 2026-07-03 |
| T-E-B-11 | BM25 + 向量混合搜索 | 2026-07-03 |
| T-E-B-12 | 文档提取引擎 | 2026-07-03 |
| T-E-B-13 | 知识卡片 | 2026-07-04 |
| T-E-B-14 | Dataview 式查询 DSL | 2026-07-04 |
| T-E-B-16 | MDRM 5 维关系图谱 | 2026-07-07 |
| T-E-B-17 | ReasoningChain 结构体 | 2026-07-03 |
| T-E-B-18 | 思维树模式 | 2026-07-04 |

### 2.3 支柱 C 贴合（9/20 ✅）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-C-02 | ScreenReader 截图理解 | 2026-07-04 |
| T-E-C-08 | Shadow Workspace | 2026-07-07 |
| T-E-C-09 | 任务录屏回放 | 2026-07-07 |
| T-E-C-10 | 异步长任务模式 | 2026-07-07 |
| T-E-C-13 | 工作场景模板库 | 2026-07-04 |
| T-E-C-14 | 剪贴板智能监听 | 2026-07-03 |
| T-E-C-16 | 一键导出 | 2026-07-04 |
| T-E-C-17 | IM 扫码绑定 | 2026-07-04 |
| T-E-C-20 | Docker 部署 | 2026-07-04 |

### 2.4 支柱 D 快（6/10 ✅）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-D-01 | 冷启动优化 3s | 2026-07-03 |
| T-E-D-02 | 首响 < 500ms | 2026-07-03 |
| T-E-D-03 | 桌面悬浮球 | 2026-07-03 |
| T-E-D-06 | 文件拖拽 + 右键集成 | 2026-07-04 |
| T-E-D-07 | 浮动进度窗 | 2026-07-03 |
| T-E-D-10 | 多 Agent 并行流式渲染 | 2026-07-04 |

### 2.5 支柱 S 贯穿层（35/48 ✅，含 Stage 7 P0 全部完成）

Stage 7 P0 批次 1-4（12/12 ✅，2026-07-03 全部完成）：
T-E-S-20（exec fail-closed）/ T-E-S-21（assemble_context ACL）/ T-E-A-01 / T-E-A-06 / T-E-B-11 / T-E-S-01 / T-E-S-30 / T-E-S-02 / T-E-S-35 / T-E-S-50 / T-E-S-51 / T-E-S-59

其他已完成（按 ID 升序）：
T-E-S-03（DynamicAgentPool）/ T-E-S-04（DynamicAgentPool 扩展）/ T-E-S-05（deadlock detection）/ T-E-S-23（凭证加密卷分离）/ T-E-S-24（文件快照回滚）/ T-E-S-25（12 trace span types）/ T-E-S-27（trusted diagnostics）/ T-E-S-28（标注+持续改进）/ T-E-S-29（OpenTelemetry 原生集成）/ T-E-S-36（SkillEngine 三层架构）/ T-E-S-37（skill-pool tags 扩展）/ T-E-S-38（可视化生成 Skills）/ T-E-S-40（OpenAI 兼容层）/ T-E-S-41（models.json 动态配置）/ T-E-S-42（VectorStore trait）/ T-E-S-43（MCP stdio supervisor）/ T-E-S-44（StorageBackend trait）/ T-E-S-45（ClawHub 双向兼容）/ T-E-S-46（技能发布命令）/ T-E-S-47（Skill hot-reload）/ T-E-S-48（OpenAPI Tool Server）/ T-E-S-49（MCPO Streamable HTTP）/ T-E-S-52（Doctor 健康检查）/ T-E-S-54（Background notifications）/ T-E-S-55（Condition watch）/ T-E-S-57（Event triggers）/ T-E-S-61（soul.md injection）/ T-E-S-62（Sqlite + Sqlcipher）

> 完整 commit hash + 测试详情见 `ROADMAP_v2.2.md` 对应章节或 `git log --grep "T-E-S-"`。

### 2.6 Loop Engineering（5/9 ✅，2 个待标记）

| 任务 ID | 标题 | 完成日期 |
|---------|------|---------|
| T-E-L-01 | MasterAgent Loop 执行模式 | 2026-07-08 |
| T-E-L-02 | CronTask 扩展 | 2026-07-08 |
| T-E-L-03 | ReviewerAgent 升级为 CheckerAgent | 2026-07-08 |
| T-E-L-05 | Loop 模板库 | ⚠️ 实现已完成 3 commits，待标记 DONE |
| T-E-L-06 | Loop 预算管理 + 安全防护 | ⚠️ 实现已完成，待提交 push |

---

## 3. 未完成任务清单（按优先级分组）

### 3.1 P1 优先级（10 个）

| 任务 ID | 标题 | 复杂度 | 依赖 | 所属 Wave |
|---------|------|--------|------|-----------|
| T-E-C-01 | OS-Controller 双模式（API+VLM） | XL | T-S6-A-01a | Wave 4 |
| T-E-C-05 | OS-Controller Sidecar | L | T-S4-B-03 | Wave 4 |
| T-E-C-06 | Hybrid Browser Agent | XL | 无 | Wave 4 |
| T-E-S-10 | WorkflowCanvas 可编排画布 | XL | 无 | Wave 4 |
| T-E-S-11 | 蜂群运行时画布 | L | T-S1-B-02 | Wave 4 |
| T-E-S-26 | Event Stream 协议化 | L | T-S1-B-02 | Wave 4 |
| T-E-S-53 | Cron 定时任务引擎 | L | T-S4-B-03 | Wave 5 |
| T-E-S-58 | Calendar 组件 | M | T-E-S-02 | Wave 5 |
| T-E-S-60 | Gateway 守护进程 | L | T-S4-B-03 | Wave 3 |
| T-E-S-63 | 三定时机制 | L | 无 | Wave 5 |

### 3.2 P2 优先级（19 个）

| 任务 ID | 标题 | 复杂度 | 依赖 |
|---------|------|--------|------|
| T-E-B-15 | AI 自动整理 MOC | L | T-E-S-63 |
| T-E-C-03 | UiAutomator 抽象层 | XL | 无 |
| T-E-C-04 | ActionExecutor | L | T-E-C-03 |
| T-E-C-11 | 操作录制回放 | M | T-E-C-04 |
| T-E-C-15 | 语音交互引擎 | XL | 无 |
| T-E-C-18 | OAuth 集成层（5 服务） | XL | 无 |
| T-E-C-19 | 多端协同 | XL | 无 |
| T-E-D-04 | 8 人格系统 | XL | T-E-D-03 |
| T-E-D-05 | Proactive Engine | L | T-E-S-63 |
| T-E-D-08 | WebGL 引擎复用 | XL | T-S5-B-02 |
| T-E-D-09 | UI 性能基准 CI | M | T-E-D-08 |
| T-E-S-12 | 节点交互 | M | T-E-S-11 |
| T-E-S-13 | 工作流模板 | M | T-E-S-10 |
| T-E-S-14 | 执行回放 | M | T-E-S-11 |
| T-E-S-22 | AIO Sandbox | XL | T-S2-A-01c |
| T-E-S-56 | Automation 模板 | S | T-E-S-53 |
| T-E-L-04 | GitHub MCP 连接器（pull-only） | L | T-E-C-18 |
| T-E-L-05 | Loop 模板库（待标记 DONE） | M | T-E-L-01 |
| T-E-L-08a | Loop 运行时阶段环 | M | T-E-S-11 |

### 3.3 P3 优先级（5 个）

| 任务 ID | 标题 | 复杂度 | 依赖 |
|---------|------|--------|------|
| T-E-C-07 | Remote Operator | XL | T-E-C-05 |
| T-E-C-12 | Design Mode | L | T-E-C-04 |
| T-E-D-04+ | （已含于 P2） | - | - |
| T-E-S-06 | Organization Orchestration | XL | T-E-S-04 |
| T-E-L-07 | Loop 审计日志 | S | T-E-L-01 |
| T-E-L-08b | Loop 设计节点 | XL | T-E-S-10 |

---

## 4. 技术债务（19 个 T-D-* 任务，全部未开始）

> **来源**：代码质量审计（2026-07-08），覆盖 Rust 后端、前端 TypeScript、CI/构建配置、测试覆盖、安全配置五大维度。
> **任务编号**：`T-D-<领域>-<序号>`（F=前端 / B=后端 / C=CI配置 / T=测试 / S=安全配置）

### 4.1 P0 严重问题（12 个，必须修）

| 任务 ID | 描述 | 领域 | 复杂度 |
|---------|------|------|--------|
| T-D-B-15 | **🔴 构建阻塞：digest crate 版本冲突**（sha2 v0.11 用 digest 0.11.3，hkdf v0.13 期望 digest 0.10.7）→ 统一版本，修复 `src/sync/e2ee.rs:195` 和 `src/im/webhook.rs:26` | 后端 | S |
| T-D-T-04 | **🔴 CI 集成测试被跳过**：Windows CI `cargo nextest run --lib` 因 STATUS_ENTRYPOINT_NOT_FOUND 只跑单元测试，25 个集成测试文件 + 2 个 e2e 测试不执行 → 隔离 Tauri 依赖测试或添加 Linux runner 跑完整套件 | 测试 | M |
| T-D-B-01 | tracing_setup.rs 8 路组合爆炸 → builder pattern 缩减为 1 个 | 后端 | S |
| T-D-B-06 | **lib.rs 3,333 行巨型文件**（257 个 Tauri 命令）→ 按领域拆分为 < 300 行入口 + 子模块 | 后端 | L |
| T-D-B-07 | **1,805 panic 点**（1,361 unwrap + 377 expect + 67 panic）→ < 50，消除桌面随机闪退 | 后端 | L |
| T-D-B-08 | **gRPC wire 非标准**（自定义 JSON framing shim）→ 迁移到 tonic 标准协议，grpcurl 可连接 | 后端 | L |
| T-D-B-09 | **渠道路由层断路**：3 个空操作适配器（WebChat/Telegram/Discord send() 返回空 Ok(())）+ ChannelAdapter trait 设计缺陷（&mut self 与 Arc<> 不兼容）+ InboxManager::send_reply 回信丢失 | 后端 | M |
| T-D-F-01 | tauri.ts 单文件 3190 行/108KB → 按领域拆分 | 前端 | M |
| T-D-C-01 | CI 仅 Windows → 恢复 macOS+Linux matrix 或明确记录决策 | CI | M |
| T-D-F-02 | ESLint 配置不存在 → 新增 flat config | 前端 | S |
| T-D-S-01 | cargo audit 忽略 14 个安全建议 → 逐项评估 + 跟踪机制 | 安全 | M |
| T-D-C-06 | **关键功能开关默认关闭**（channels/mcp/headless/rest-api/self-evolution/evolution-engine）→ 默认开启或明确记录决策 | CI | S |

### 4.2 P1 重要问题（21 个）

| 任务 ID | 描述 | 领域 | 复杂度 |
|---------|------|------|--------|
| T-D-B-10 | **Skill 生态补齐**：无技能自动发现（SKILL.md 热加载）+ agentskills.io 规范字段缺失 + TeamSkillsHub 导入返回 stub + 无 Eligibility 检查（bins/env/config/os 4 维） | 后端 | M |
| T-D-B-11 | **EvolutionEngine 断路**：evolution_run 命令未实现（注释标记 "left for future iteration"）+ EvolutionWorker 仅调用 PromptSelfMutator 不跑完整 4 阶段管道 + 无 LLM 反馈循环 | 后端 | L |
| T-D-B-12 | **无自托管 Web 静态服务**：REST API 仅提供 JSON 端点，无内建 HTTP 静态文件服务（对标 OpenClaw Web Admin / Hermes Dashboard） | 后端 | S |
| T-D-B-13 | **无系统服务注册**：缺少 systemd/launchd/Windows Service 注册（对标 OpenClaw Gateway 守护进程） | 后端 | M |
| T-D-B-14 | **Sidecar 3/5 服务骨架化**：仅 Memory/Swarm/LLM 有处理器，Os-Controller/Reflection 骨架化 | 后端 | M |
| T-D-F-03 | 重复代码提取（renderMarkdown / downloadBlob） | 前端 | S |
| T-D-F-04 | 硬编码中文字符串 → 迁移到 i18n key | 前端 | M |
| T-D-F-05 | i18n 类型不安全 → 基于 zh-CN.json 类型推导 | 前端 | S |
| T-D-F-06 | cancelled 布尔反模式 → AbortController | 前端 | S |
| T-D-B-02 | bootstrap.rs 1113 行单函数 → 拆分 bootstrap phase | 后端 | L |
| T-D-B-03 | std::mem::forget(h) 泄露 JoinHandle → 保存 + panic hook | 后端 | S |
| T-D-B-04 | memory/ 40+ 子文件平铺 → 按职能分组 | 后端 | M |
| T-D-B-05 | features 死 feature 清理（custom-protocol 等） | 后端 | S |
| T-D-T-01 | vitest 覆盖率阈值过低（30%/20%/25%/30%） | 测试 | M |
| T-D-T-02 | 核心文件零测试（bootstrap/gateway/dispatcher/app_config） | 测试 | L |
| T-D-T-03 | E2E 测试接入 CI（Playwright） | 测试 | M |
| T-D-C-02 | Vite/Vitest 配置重复 → 统一单配置源 | CI | S |
| T-D-C-03 | Prettier 配置不存在 → 新增 .prettierrc | CI | S |
| T-D-C-04 | tsconfig 禁用 noUnusedLocals/Parameters → 分阶段开启 | CI | S |
| T-D-C-05 | Dockerfile 缺 HEALTHCHECK/非 root/多架构 | CI | M |
| T-D-C-07 | **incremental = false**（Rust 1.96.1 rmeta encoder ICE 规避）→ 跟踪 rustc 修复进度，恢复增量编译以加速开发迭代 | CI | S |

### 4.3 P1 仓库卫生（1 个，来自外部审查 2026-07-08）

| 任务 ID | 描述 | 领域 | 复杂度 |
|---------|------|------|--------|
| T-D-O-01 | **IMPROVEMENT_PLAN_v1.0.md 过时文件清理**：v1.0 基线规划文档，已被 ROADMAP v2.1/v2.2/v2.3 取代，但仍被 git 跟踪。删除或移至 `docs/archive/`。同步检查项目根是否有其他过时文档被跟踪。 | 仓库卫生 | S |

### 4.4 技术债务推进原则

1. **不阻塞功能开发**：与功能任务并行，P0 债务优先在 Wave 间隙处理
2. **分批消化**：每个 Wave 结束后评估债务状态
3. **测试先行**：拆分重构类任务（如 T-D-B-02）必须先补测试
4. **安全优先**：T-D-S-01 和 T-D-B-03 涉及安全，优先处理
5. **仓库卫生**：T-D-O-01 与 T-D-F-02（ESLint）一起做，清理过时文档 + 建立代码门禁

---

## 5. 外部审查交叉验证（2026-07-08）

> **来源**：外部智能体对 ROADMAP_v2.2 的代码级交叉验证报告（`D:\tmp\ROADMAP_REVIEW.md`），对 12 个最高优先级已完成任务逐一代码验证。

### 5.1 审查结论

- **项目完成度**：~74%（v2.3 校准后），高于早期 IMPROVEMENT_PLAN 估算的 25%
- **代码质量**：`cargo check` 0 警告，`npm run typecheck` 0 错误，107 前端测试全绿
- **核心瓶颈**：① 工作流可视化全缺（T-E-S-10~14）② 后端核心文件零测试（bootstrap/gateway/dispatcher）③ 技术债务 34 项零处理

### 5.2 审查发现的 v2.3 修正

| 问题 | 修正 |
|------|------|
| v2.3 §1 C 支柱统计错误（3/14 → 9/20） | ✅ 已修正 §1 和 §2.3 |
| v2.3 §2.3 遗漏 5 个已完成 C 任务（C-02/08/09/10/20） | ✅ 已补全 |
| T-E-L-05/06 实际已完成但 ROADMAP 未标记 | ⚠️ 待标记（见 §6.1 立即收尾） |
| IMPROVEMENT_PLAN_v1.0.md 过时但 git 跟踪 | ✅ 新增 T-D-O-01 任务 |
| DEVELOPMENT_PROPOSAL_v2.0 §1.1 CR-01 digest 冲突构建阻塞 | ✅ 新增 T-D-B-15 任务（P0） |
| DEVELOPMENT_PROPOSAL_v2.0 §1.1 CR-02 CI 集成测试跳过 | ✅ 新增 T-D-T-04 任务（P0） |
| DEVELOPMENT_PROPOSAL_v2.0 §1.2 HI-05 incremental=false | ✅ 新增 T-D-C-07 任务（P1） |
| DEVELOPMENT_PROPOSAL_v2.0 §1.2 HI-01 lib.rs 3333 行 | ✅ 新增 T-D-B-06 任务（P0） |
| DEVELOPMENT_PROPOSAL_v2.0 §1.2 1805 panic 点 | ✅ 新增 T-D-B-07 任务（P0） |
| DEVELOPMENT_PROPOSAL_v2.0 §1.3 gRPC 非标准/渠道断路/Skill 缺口/Evolution 断路/Web 服务缺失/系统服务缺失/Sidecar 骨架/功能开关关闭 | ✅ 新增 T-D-B-08~14 + T-D-C-06（共 8 项） |

### 5.3 审查建议的执行顺序（与 v2.3 §6 推进节奏对照）

审查报告建议的执行顺序与 v2.3 §6 基本一致，差异点：
- 审查建议 P2 任务 T-E-D-09（UI 性能基准 CI）放 P3，v2.3 保留在 P2（因依赖 T-E-D-08 未完成）
- 审查建议先做技术债务清扫（7 个 S 复杂度项），v2.3 §6 Wave 1 已采纳此思路

---

## 6. v2.3 推进节奏建议

### 6.1 立即收尾（本周内）

1. **标记 T-E-L-05 为 DONE**（核实 3 commits 已落地，更新 ROADMAP）
2. **提交 T-E-L-06 工作区变更**（11 改 + 3 新增，含 ROADMAP 修改）并 push 触发 CI
3. **关闭 finish-te-l-03-checker-agent spec**（核心目标已通过 T-E-L-06 达成）
4. **🔴 T-D-B-15 digest crate 版本冲突修复**（构建阻塞，必须最先解决）
5. **🔴 T-D-T-04 CI 集成测试恢复**（25 个集成测试文件不执行是回归风险真空）

### 6.2 Wave 1（v2.3 地基修复 + 技术债务 P0 清算）

> **对齐 DEVELOPMENT_PROPOSAL_v2.0 Phase 0**：功能任务暂停，全力做债务清理。

**Phase 0 - 构建阻塞修复（W1）**：
- 🔴 T-D-B-15 digest crate 版本冲突（S，最先）
- 🔴 T-D-T-04 CI 集成测试恢复（M）

**Phase 0 - 严重质量修复（W2-3）**：
- T-D-B-07 1805 panic 点 → < 50（L，最大工程量）
- T-D-B-06 lib.rs 3333 行拆分（L，与 T-D-B-07 同步做）
- T-D-B-02 bootstrap.rs 1113 行拆分（L，测试先行）
- T-D-F-01 tauri.ts 3190 行拆分（M）
- T-D-B-01 tracing_setup.rs 8 路重构（S）
- T-D-B-08 gRPC wire 标准化（L，迁移到 tonic）
- T-D-B-09 渠道路由断路修复（M）

**Phase 0 - CI 与配置（W4）**：
- T-D-F-02 ESLint 配置（S，最简单）
- T-D-S-01 cargo audit 14 项评估（M）
- T-D-C-01 CI 跨平台决策（M）
- T-D-C-06 功能开关默认开启决策（S）
- T-D-T-01 vitest 覆盖率提升（M）
- T-D-O-01 过时文档清理（S）

### 6.3 Wave 2（v2.4 知识革命收尾）

- T-E-B-15 AI 自动整理 MOC（依赖 T-E-S-63，需先做 T-E-S-63）
- T-E-S-63 三定时机制（P1，Wave 5 任务但被 T-E-B-15/T-E-D-05 依赖，提前）

### 6.4 Wave 3（v2.5 形象+后台革命）

- T-E-S-60 Gateway 守护进程
- T-E-D-04 8 人格系统
- T-E-D-05 Proactive Engine（依赖 T-E-S-63）

### 6.5 Wave 4（v2.6 可视+视觉革命）

- T-E-S-10 WorkflowCanvas（P1，XL，最大工程量）
- T-E-S-11 蜂群运行时画布
- T-E-S-26 Event Stream 协议化
- T-E-C-01 OS-Controller 双模式
- T-E-C-05 OS-Controller Sidecar
- T-E-C-06 Hybrid Browser Agent
- T-E-L-08a Loop 运行时阶段环（依赖 T-E-S-11）

### 6.6 Wave 5（v3.0 全自主革命）

- T-E-S-53 Cron 定时任务引擎
- T-E-S-58 Calendar 组件
- T-E-C-18 OAuth 集成层
- T-E-C-19 多端协同
- T-E-L-04 GitHub MCP（依赖 T-E-C-18）

### 6.7 Loop Engineering 阶段三（P3，可延后）

- T-E-L-07 Loop 审计日志
- T-E-L-08b Loop 设计节点（依赖 T-E-S-10）

---

## 7. License 合规矩阵

| 对标项目 | License | 与 Nebula(MIT) 兼容 | 借鉴边界 |
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

---

## 8. 附录

### 8.1 来源标记说明

- **来源 A**：报告 A（`EXPERT_REVIEW_v3.0_INNOVATION.md`，7 专家 + 大厂趋势）
- **来源 B**：报告 B（GLM-5.2 对话分析，OpenAkita 校准 + UI-TARS/CoPaw/LLM Wiki 深度对标）
- **来源 A+B**：双报告共同提出，互补合并
- **来源 Loop 内化**：Loop Engineering 公开资料内化（`docs/skills/loop-engineering/`），7 专家评审通过

### 8.2 配套文档

- `docs/ROADMAP_v2.1.md`（Stage 1-6 工程闭环，已全部完成）
- `docs/ROADMAP_v2.2.md`（v2.3 前身，含已完成任务的完整 commit 描述，仅供历史追溯）
- `docs/COMPREHENSIVE_EVOLUTION_v3.0.md`（创新审议综合报告）
- `docs/skills/loop-engineering/NEBULA_LOOP_DESIGN.md`（Loop Engineering 设计权威）

### 8.3 依赖关系速查

- Stage 1-6（v2.1）已全部完成 ✅，是 Stage 7 的基础
- T-E-L-* 系列依赖 Wave 3 的 T-E-C-10（已完成）
- T-E-L-08a 依赖 T-E-S-11（蜂群运行时画布，未完成）
- T-E-L-08b 依赖 T-E-S-10（WorkflowCanvas，未完成）
- T-E-B-15 依赖 T-E-S-63（三定时机制，未完成）
- T-E-D-05 依赖 T-E-S-63（同上）
- T-E-L-04 依赖 T-E-C-18（OAuth 集成层，未完成）

---

**文档结束**。

v2.3 是聚焦未完成任务的精简版路线图。已完成任务的 commit hash 和实现细节请查 `ROADMAP_v2.2.md` 或 `git log --grep "T-E-"`。后续版本推进以本文档 §3（未完成任务）和 §5（推进节奏）为准。

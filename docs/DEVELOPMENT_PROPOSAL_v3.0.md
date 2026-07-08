# Nebula 全面修复与迭代开发建议书 v3.0

**依据哲学**：信任三原则（可读·可编辑·可追溯）+ Open Mythos 开放神话  
**核心宣言**：你无法信任一段你无法阅读的记忆  
**基线版本**：v2.0.0（ROADMAP_v2.3 状态，74% 完成）  
**制定日期**：2026-07-08  
**前序文档**：`DEVELOPMENT_PROPOSAL_v2.0.md`（v2.0 基线）  
**配套文档**：`WHITEPAPER_v3.1.md`（设计权威）、`ROADMAP_v2.3.md`（未完成任务）、`PRODUCTION_TASK_TRACKER.md`（M0a-M7b 完成记录）

---

## 0. v2.0 → v3.0 变更说明

### 0.1 变更动机

v2.0 建议书基于代码审查和文档分析制定，但存在以下不足：

1. **未深入代码级审查**：缺陷列表主要来自文档推断，缺少对关键模块（AppState/Swarm/Evolution/Writing）的代码级验证
2. **缺少蜂群智能体架构升级**：未纳入"2 自定义主智能体 + 6 通用子智能体"的架构演进方向
3. **写作场景缺口未识别**：未发现 writing/ 模块与白皮书承诺的严重落差
4. **AppState 膨胀风险未评估**：45+ Arc 字段的巨型结构体未列入缺陷清单

### 0.2 v3.0 新增内容

| 范围 | v2.0 | v3.0 |
|------|------|------|
| 缺陷来源 | 文档推断 + 外部审查 | **+ 代码级审查（AppState/Swarm/Evolution/Writing）** |
| 架构演进 | 无 | **+ 蜂群智能体架构升级（2 主 + 6 子）** |
| 写作场景 | 未涉及 | **+ 写作场景深度补齐（自媒体/长篇小说）** |
| 任务规划 | Phase 0-3 | **+ Phase 0-4（新增蜂群架构升级阶段）** |
| 验收标准 | 通用指标 | **+ 场景化验收（写作工作流端到端验证）** |

---

## 1. 项目哲学基线（Open Mythos）

### 1.1 信任三原则（v3.0 核心哲学）

| 原则 | 含义 | 代码落地 | 缺口 |
|------|------|---------|------|
| **可读（Readable）** | 所有记忆以人类可读的 Markdown 渲染 | ✅ Memory Inspector / LLM Wiki / 三视图 | ⚠️ AppState 45+ 字段不可读；bootstrap 1113 行不可读 |
| **可编辑（Editable）** | 用户可任意修改记忆，AI 写入与人类编辑双向同步 | ✅ 双向同步 + provenance | ⚠️ Agent 角色硬编码在 Rust 中，不可运行时编辑 |
| **可追溯（Traceable）** | 每条记忆携带 provenance，决策可回溯 | ✅ version_control + provenance | ⚠️ 进化日志仅在 evolution-engine feature 下可用 |

### 1.2 v2.0 五哲学

| 哲学 | 落地状态 | 缺口 |
|------|---------|------|
| 记忆是 AI 的灵魂（6 层 L0-L5） | ✅ 完成 | 无 |
| 模式对用户不可见（AI 自动判断） | ✅ 完成 | 无 |
| 价值对齐前置（L4 价值层） | ✅ 完成 | 无 |
| 本地优先（E2EE + 私钥不出设备） | ✅ 完成 | ⚠️ E2EE 仍为单棘轮，无前向保密 |
| 可观测可审计（Prometheus + OpenTelemetry） | ⚠️ 基础完成 | 核心文件零测试，可观测性覆盖有缺口 |

### 1.3 四大支柱落地状态

| 支柱 | 完成度 | 关键缺口 |
|------|--------|---------|
| 💰 更省钱 | 100% | 无 |
| 🧠 更智能 | 94% | T-E-B-15 AI 自动整理 MOC |
| 🔧 更贴合 | 45% | OS-Controller 双模式、Hybrid Browser、OAuth、语音交互 |
| ⚡ 更快 | 60% | 8 人格系统、Proactive Engine、WebGL 优化 |

**Stage 7 总进度：74%（92/125 任务完成）** · 技术债务：0%（20 项全未开始）

---

## 2. 项目缺陷全面审查

### 2.1 🔴 P0 严重缺陷（阻塞构建/运行/安全）

| ID | 缺陷 | 影响 | 发现来源 |
|----|------|------|---------|
| **CR-01** | `digest` crate 版本冲突：`sha2 v0.11` vs `hkdf v0.13` | `cargo check --features grpc,channels` 编译失败 | 代码审查 |
| **CR-02** | Windows CI 集成测试被跳过（STATUS_ENTRYPOINT_NOT_FOUND） | 25 个集成测试文件不执行，回归风险高 | CI 审查 |
| **CR-03** | `cargo audit` 14 个安全建议被忽略（continue-on-error） | 已知漏洞未被跟踪修复 | CI 审查 |
| **CR-04** | AppState 膨胀：45+ Arc 字段巨型结构体 | 任何子系统增删改都要修改此文件，违反开闭原则 | 代码审查（v3.0 新增） |

### 2.2 🟡 P1 严重问题（影响质量/生产力/架构健康）

| ID | 缺陷 | 影响 | 发现来源 |
|----|------|------|---------|
| **HI-01** | `tauri.ts` 单文件 3190 行/108KB | 维护困难，违反"可读"哲学 | 外部审查 |
| **HI-02** | `bootstrap.rs` 1113 行单函数 | 业务逻辑不可读不可测 | 外部审查 |
| **HI-03** | 前端测试覆盖率低（行 35.5%，函数 22.9%） | 17+ 组件零测试 | 外部审查 |
| **HI-04** | CI 仅 Windows runner | 跨平台兼容性无保障 | 外部审查 |
| **HI-05** | `incremental = false`（Rust ICE 工作区） | 每次全量重建 | 外部审查 |
| **HI-06** | `memory/` 40+ 子文件平铺无分组 | 模块内聚性差 | 外部审查 |
| **HI-07** | 核心文件零测试（bootstrap/gateway/dispatcher/app_config） | 关键代码无测试保护 | 外部审查 |
| **HI-08** | Dockerfile 缺 HEALTHCHECK/非 root/多架构 | 生产部署不规范 | 外部审查 |
| **HI-09** | AgentKind 枚举标记 Deprecated 但仍大量使用 | 语义矛盾，维护困惑 | 代码审查（v3.0 新增） |
| **HI-10** | writing/ 模块过于单薄（2 文件） | 与白皮书"28 场景模板"承诺严重不符 | 代码审查（v3.0 新增） |
| **HI-11** | swarm/agents 6 角色偏编程场景 | 与"双主控+蜂群 worker"和写作场景不匹配 | 代码审查（v3.0 新增） |
| **HI-12** | `master-orchestrator` 无运行时开关 | 与 ADR-004 设计不一致 | Feature Flag 审计 |
| **HI-13** | ARCHITECTURE.md 品牌残留与数字过时 | 文档与代码不一致 | 文档审查（v3.0 新增） |

### 2.3 🟠 P2 中等缺陷（影响可维护性/可扩展性）

| ID | 缺陷 | 影响 |
|----|------|------|
| **MI-01** | ESLint 配置不存在 | 前端代码风格无强制保障 |
| **MI-02** | Prettier 配置不存在 | 格式化无统一标准 |
| **MI-03** | `tsconfig` 禁用 `noUnusedLocals/Parameters` | 死代码易堆积 |
| **MI-04** | Vite/Vitest 配置重复 | 配置维护双倍成本 |
| **MI-05** | `tracing_setup.rs` 8 路组合爆炸 | 日志配置过度复杂 |
| **MI-06** | 硬编码中文字符串散落前端 | i18n 不完整 |
| **MI-07** | 前端 `cancelled` 布尔反模式 | 竞态条件隐患 |
| **MI-08** | `std::mem::forget(h)` 泄露 JoinHandle | 资源泄漏风险 |
| **MI-09** | 死 feature 未清理（custom-protocol 等） | 配置污染 |
| **MI-10** | `IMPROVEMENT_PLAN_v1.0.md` 过时但仍被 git 跟踪 | 文档误导风险 |
| **MI-11** | gRPC JSON framing shim 永久化 | 限制外部集成能力（v3.0 新增） |
| **MI-12** | E2EE 仍为单棘轮 | 无前向保密（v3.0 新增） |

### 2.4 🔵 功能缺口（未完成的高级特性）

| 支柱 | 未完成数 | 关键缺口 |
|------|---------|---------|
| **C 贴合** | 11 (45%) | OS-Controller 双模式、Hybrid Browser、OAuth、语音 |
| **D 快** | 4 (60%) | 8 人格系统、Proactive Engine、WebGL |
| **S 贯穿** | 12 (77%) | WorkflowCanvas、蜂群画布、Event Stream、Cron |
| **B 智能** | 1 (94%) | AI 自动整理 MOC |
| **Loop** | 5 (50%) | GitHub MCP、运行时阶段环 |

### 2.5 🟣 架构演进缺口（v3.0 新增）

| ID | 缺口 | 影响 | 优先级 |
|----|------|------|--------|
| **AE-01** | 无"主智能体"概念：当前 Swarm 是扁平调度，缺少"2 个自定义角色主智能体 + 6 通用子智能体"的分层架构 | 无法支持"自媒体写作/长篇小说"等场景化分工 | P1 |
| **AE-02** | Agent 角色硬编码：6 个 Agent 的 system_prompt/tool_set/knowledge_scope 写死在 Rust 代码中 | 用户无法运行时自定义角色，违反"可编辑"哲学 | P1 |
| **AE-03** | 无场景化写作策略：writing/ 仅 2 文件，无自媒体/长篇小说的差异化写作管线 | 白皮书承诺的"场景闭环"无法落地 | P1 |
| **AE-04** | 进化仅限 prompt/技能/SOUL.md：缺"基因级"进化（策略权重/适应度函数/受控变异） | 主智能体无法根据场景反馈自主优化策略 | P2 |
| **AE-05** | 无主→子任务分派协议：MasterOrchestrator 拆解 DAG 后直接 fan-out，缺少"主智能体按场景分派子智能体"的语义 | 写作场景无法实现"搜索/整理/审查"的分工协作 | P1 |

---

## 3. 改进原则与优先级策略

### 3.1 核心理念映射

每项改进任务须至少符合一条 Nebula 哲学：

| 哲学 | 改进任务筛选条件 |
|------|----------------|
| **可读** | 减少单块文件、增加注释文档、提升代码组织 |
| **可编辑** | 使配置/参数可外部修改、增加回滚能力、角色可运行时编辑 |
| **可追溯** | 增加审计日志、provenance、可观测性 |
| **本地优先** | 不引入强制性外部依赖 |
| **可观测** | 增加指标、追踪、日志覆盖 |

### 3.2 优先级分类

| 优先级 | 定义 | 处理时机 |
|--------|------|---------|
| **P0** | 阻塞构建/运行，或导致数据损失/安全漏洞 | 立即处理 |
| **P1** | 严重降低开发效率/代码质量/架构演进能力 | 本迭代处理 |
| **P2** | 影响架构健康/可维护性 | 下个迭代处理 |
| **P3** | 功能增强/新特性 | 按路线图推进 |

### 3.3 依赖关系

```
Phase 0（地基修复，2-3 周）
  ├─ CR-01~CR-04（阻塞修复）
  ├─ HI-01~HI-08（质量修补）
  └─ MI-01~MI-10（健康改进）
        │
Phase 1（质量闭环 + 架构准备，4-6 周）
  ├─ 技术债务 P0 清算
  ├─ 核心文件测试补齐
  ├─ CI 跨平台恢复
  ├─ 前端质量重构
  └─ AppState 分组重构（为 Phase 3 做准备）
        │
Phase 2（功能补齐，6-8 周）
  ├─ 支柱 C/D/S 未完成任务（按 ROADMAP_v2.3 Wave 推进）
  ├─ Loop Engineering 内化
  └─ 端到端体验优化
        │
Phase 3（蜂群架构升级，4-6 周）  ← v3.0 新增
  ├─ 主智能体概念引入（PrimaryAgent）
  ├─ 场景化角色配置（自媒体/长篇小说）
  ├─ 主→子任务分派协议
  ├─ 基因级进化机制
  └─ 写作场景深度补齐
        │
Phase 4（创新扩展，长期）
  └─ v3.0 全自主革命目标
```

---

## 4. Phase 0：地基修复（2-3 周）

### 4.1 P0-A：构建阻塞修复

#### A-1：digest crate 版本冲突（CR-01）

| 字段 | 内容 |
|------|------|
| **任务** | 修复 `sha2 v0.11` 与 `hkdf v0.13` 的 digest 版本不兼容 |
| **根因** | `sha2:0.11` 使用 `digest:0.11`，但 `hkdf:0.13` 期望 `digest:0.10` trait |
| **方案** | 方案 A：降级 `sha2` 到 `=0.10`（需验证 `aes-gcm` 兼容性）<br>方案 B：升级 `hkdf` 到支持 `digest 0.11` 的版本<br>方案 C：`[patch]` digest 到统一版本 |
| **涉及文件** | `Cargo.toml`、`src/sync/e2ee.rs:195`、`src/im/webhook.rs:26` |
| **验收** | `cargo check --features grpc,channels --lib` 通过，`cargo check --features grpc,channels --tests` 通过，E2EE 加密/解密单元测试通过 |
| **复杂度** | S |

#### A-2：Windows CI 集成测试（CR-02）

| 字段 | 内容 |
|------|------|
| **任务** | 恢复 CI 上集成测试的执行 |
| **方案** | 方案 A：隔离 Tauri 依赖测试到独立 binary<br>方案 B：`#[cfg(not(target_os = "windows"))]` 跳过<br>方案 C：添加 macOS/Linux runner |
| **涉及文件** | `.github/workflows/test.yml` |
| **验收** | CI 中至少一个平台运行全部集成测试（25+ 测试文件），且结果可见于 CI 报告 |
| **复杂度** | M |

#### A-3：cargo audit 安全建议追踪（CR-03）

| 字段 | 内容 |
|------|------|
| **任务** | 逐项评估 14 个被忽略的 cargo audit 建议，建立追踪机制 |
| **方案** | 1. 逐项评估能否升级/替换<br>2. 无法立即修复的写入 `SECURITY_ADVISORIES.md` 追踪表<br>3. CI 改为分两组：已知(allow) + 新(deny) |
| **验收** | 14 项全部有评估结果（已修复/已记录+追踪），新增漏洞会触发 CI 告警 |
| **复杂度** | M |

#### A-4：AppState 分组重构（CR-04）

| 字段 | 内容 |
|------|------|
| **任务** | 将 45+ Arc 字段按子系统分组为子结构体 |
| **方案** | 新增 `MemorySubsystems`/`SwarmSubsystems`/`SecuritySubsystems`/`UiSubsystems`/`InfraSubsystems`，AppState 只持有 5-6 个 `Arc<SubSystems>` |
| **涉及文件** | `app_state.rs`、`lib.rs`、所有 `commands/*.rs` |
| **验收** | AppState 字段数 < 10，各 SubSystems 内聚完整，`cargo check` 通过，所有命令功能无变化 |
| **复杂度** | L |

### 4.2 P0-B：严重质量问题修复

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| HI-01 | tauri.ts 拆分 | 按 8+ 领域拆分（chat/memory/skill/swarm/work/writing/editor/sync/os + types） | 原 tauri.ts < 100 行（barrel re-export），各模块 < 500 行 | M |
| HI-02 | bootstrap.rs 拆分 | 拆为 5-8 个 phase 函数 | 各 phase 函数 < 200 行 | L |
| HI-03 | 前端覆盖率提升 | 为 10+ 零测试组件添加基础测试 | 覆盖率阈值 50/40/40/50 | L |
| HI-04 | CI 跨平台评估 | 添加 Linux runner | CI 矩阵 windows + linux | M |
| HI-09 | AgentKind Deprecated 清理 | 统一为 Generic + 场景化角色配置（为 Phase 3 做准备） | 无 Deprecated 标记的活跃使用 | M |
| HI-10 | writing/ 模块补齐 | 新增场景模板引擎 + 自媒体/长篇小说模板 | 28 个场景模板可用 | L |
| HI-13 | ARCHITECTURE.md 更新 | 品牌统一为 Nebula，数字更新为最新 | 文档与代码一致 | S |

### 4.3 P0-C：可维护性问题修复

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| MI-01 | 添加 ESLint flat 配置 | 新增 `eslint.config.mjs` | `npm run lint` 通过 | S |
| MI-02 | 添加 Prettier 配置 | 新增 `.prettierrc` + `.prettierignore` | `npm run format` 可靠运行 | S |
| MI-03 | tsconfig 死代码检测 | 分阶段启用 `noUnusedLocals`/`noUnusedParameters` | `npm run typecheck` 0 错误 | S |
| MI-04 | Vite/Vitest 配置统一 | 提取公共配置到 `vite.shared.ts` | 单配置源生效 | S |
| MI-05 | tracing_setup.rs 重构 | Builder pattern 替代 8 路组合爆炸 | 配置代码减少 50%+ | S |
| MI-06 | 硬编码字符串迁移 | 扫描前端提取未 i18n 的中文字符串 | 新增 key + 翻译 | M |
| MI-07 | cancelled 布尔反模式 | 替换为 AbortController | clippy 无相关警告 | S |
| MI-08 | JoinHandle 泄漏修复 | 保存到 struct + 添加 panic hook | clippy 无相关警告 | S |
| MI-09 | 死 feature 清理 | 审计 Cargo.toml 移除无效 feature | `cargo check` 通过 | S |
| MI-10 | 过时文档清理 | `IMPROVEMENT_PLAN_v1.0.md` 移至 `docs/archive/` | 根目录无过时规划文档 | S |

### 4.4 Phase 0 验收标准

```
□ cargo check --features grpc,channels --lib 通过（CR-01）
□ cargo check --features grpc,channels --tests 通过
□ CI 中至少一个平台跑完全部集成测试（CR-02）
□ cargo audit 14 项全部评估完成（CR-03）
□ AppState 字段数 < 10，5-6 个 SubSystems 就绪（CR-04）
□ tauri.ts < 100 行 + 8+ 领域模块就绪（HI-01）
□ bootstrap.rs 各 phase < 200 行（HI-02）
□ 前端覆盖率 ≥ 50/40/40/50（HI-03）
□ CI 至少 windows + linux 双平台（HI-04）
□ writing/ ≥ 28 个场景模板（HI-10）
□ ESLint + Prettier + tsconfig 严格模式就绪
□ 所有 MI 项修复完成
□ cargo clippy --features grpc,channels -- -D warnings 通过
□ cargo fmt --all -- --check 通过
□ ARCHITECTURE.md 品牌统一 + 数字更新
```

---

## 5. Phase 1：质量闭环 + 架构准备（4-6 周）

### 5.1 P1-A：技术债务清算

| 任务 | 文件 | 当前测试数 | 目标测试数 |
|------|------|-----------|-----------|
| bootstrap 测试 | `bootstrap.rs` | 0 | 5+（各 phase 独立测试） |
| gateway 测试 | `llm/gateway.rs` | 0 | 8+（断路器/降级/路由） |
| dispatcher 测试 | `llm/dispatcher.rs` | 0 | 5+（请求分发/错误处理） |
| app_config 测试 | `app_config.rs` | 0 | 5+（环境变量解析） |

### 5.2 P1-B：CI/CD 安全门禁

| 门禁项 | 当前 | 目标 |
|--------|------|------|
| clippy 门前 | 有 | `-D warnings` 全绿 |
| fmt 门前 | 有 | `--check` 全绿 |
| audit 门前 | continue-on-error | 分两组：已知(allow) + 新(deny) |
| coverage 门前 | 30/20/25/30 | 50/40/40/50 |
| typecheck 门前 | 有 | 0 error |
| E2E 门前 | 无 | 2+ 冒烟 |

### 5.3 P1-C：架构准备（为 Phase 3 蜂群升级铺路）

| 任务 | 方案 | 验收 | 复杂度 |
|------|------|------|--------|
| Agent 角色配置外部化 | 新增 `AgentRoleConfig` JSON 文件 + 运行时加载 | 编辑 JSON 即改变 Agent 行为，无需重编译 | M |
| master-orchestrator 运行时开关 | 新增 `MASTER_ORCHESTRATOR_ENABLED: AtomicBool` | feature on 时可运行时关闭 | S |
| PrimaryAgent trait 设计 | 新增 `PrimaryAgent` trait（decompose/delegate/synthesize） | trait 定义 + 空实现编译通过 | M |
| WorkerCapability 扩展 | 新增 `WriteShort`/`WriteLong`/`Search`/`Outline`/`Review`/`Polish`/`Archive` | 7 个写作场景能力枚举 | S |

### 5.4 Phase 1 验收标准

```
□ bootstrap.rs ≥ 5 个单元测试，覆盖率 > 60%
□ gateway.rs ≥ 8 个测试，覆盖率 > 50%
□ dispatcher.rs ≥ 5 个测试
□ app_config.rs ≥ 5 个测试
□ CI 三平台 cargo check 全绿
□ Docker HEALTHCHECK + non-root user
□ Playwright 2+ 用户场景 E2E 测试
□ ChatPanel < 300 行
□ i18n 类型安全 + 完整覆盖
□ 覆盖率门槛 50/40/40/50
□ Agent 角色配置可外部编辑
□ PrimaryAgent trait 定义就绪
□ WorkerCapability 写作场景枚举就绪
□ npm run typecheck 0 error
□ npm run lint 0 error
```

---

## 6. Phase 2：功能补齐（6-8 周）

> 按 ROADMAP_v2.3 Wave 推进，与 v2.0 建议书一致，此处精简。

### 6.1 Wave 3（v2.5 形象+后台革命）

| 任务 | 目标 | 验收 |
|------|------|------|
| T-E-S-60 Gateway 守护进程 | 进程守护 + 自动重启 | crash 后 5s 内自动重启 |
| T-E-D-04 8 人格系统 | 8 种人格 + 表情联动 | 人格切换生效 |
| T-E-D-05 Proactive Engine | 主动问候/任务跟进 | 每日主动交互 1-2 次 |

### 6.2 Wave 4（v2.6 可视+视觉革命）

| 任务 | 目标 | 验收 |
|------|------|------|
| T-E-S-10 WorkflowCanvas | React Flow 拖拽编排 | 可拖拽创建/编辑工作流 |
| T-E-S-11 蜂群运行时画布 | Agent 实时运行可视化 | Agent 节点状态实时更新 |
| T-E-S-26 Event Stream 协议化 | SwarmEvent 升级协议 | type/payload/trace_id/timestamp 完整 |
| T-E-C-01 OS-Controller 双模式 | API + VLM 双模式 | click/type 两种模式可用 |
| T-E-C-05 OS-Controller Sidecar | 独立进程运行 | Sidecar 启动/停止正常 |
| T-E-C-06 Hybrid Browser Agent | GUI + CDP + DOM 三策略 | 可打开网页并执行操作 |

### 6.3 Wave 5（v3.0 全自主革命）

| 任务 | 目标 | 验收 |
|------|------|------|
| T-E-S-53 Cron 定时任务引擎 | 三计时机制 | 定时任务自动执行 |
| T-E-S-58 Calendar 组件 | 日历 + 日程管理 | 可查看/创建日程 |
| T-E-S-63 三定时机制 | 每日/周/月定时任务 | consolidation 按计划执行 |
| T-E-C-18 OAuth 集成层 | 5 个核心 OAuth 服务 | 3+ 服务授权可用 |
| T-E-C-19 多端协同 | Desktop + CLI + PWA + 渠道 | 多渠道消息统一收件箱 |

### 6.4 Phase 2 验收标准

```
□ OS-Controller 双模式可执行 click/type
□ Hybrid Browser Agent 可操作网页
□ WorkflowCanvas 可拖拽编排
□ 蜂群运行时画布实时显示 Agent 状态
□ Cron 三定时正常执行
□ 8 人格切换生效
□ Proactive Engine 每日主动交互
□ OAuth 3+ 服务授权可用
□ 所有功能配套测试通过
□ CI 全绿
```

---

## 7. Phase 3：蜂群架构升级（4-6 周）← v3.0 新增

### 7.1 设计目标

将现有"扁平蜂群"升级为"2 自定义主智能体 + 6 通用子智能体"的分层蜂群架构，支持场景化写作工作流。

### 7.2 架构演进

```
当前架构（扁平蜂群）：
  用户 → SwarmOrchestrator → 2-6 GenericAgent（并行）

目标架构（分层蜂群）：
  用户选择场景
    ├→ PrimaryAgent-A（自媒体写作专家）
    │    ├→ SearchAgent（资料搜索/热点追踪）
    │    ├→ OutlineAgent（大纲生成/结构规划）
    │    ├→ DraftAgent（初稿撰写/段落扩展）
    │    ├→ ReviewAgent（质量审查/风格校验）
    │    ├→ PolishAgent（润色优化/SEO/排版）
    │    └→ ArchiveAgent（资料整理/知识归档）
    │
    └→ PrimaryAgent-B（长篇小说写作专家）
         ├→ SearchAgent（设定搜索/读者偏好）
         ├→ OutlineAgent（章节拆分/人物设定）
         ├→ DraftAgent（章节初稿/多章节并行）
         ├→ ReviewAgent（人物一致性/剧情连贯性）
         ├→ PolishAgent（文笔润色/统一风格）
         └→ ArchiveAgent（人物关系图/伏笔清单）
```

### 7.3 任务清单

#### 7.3.1 核心架构（3 个任务）

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| **AE-01** | PrimaryAgent 实现 | 新增 `swarm/agents/primary_agent.rs`，实现 `decompose()`/`delegate()`/`synthesize()` 三核心方法，持有 6 个子智能体引用 | PrimaryAgent 可接收任务、分解、分派、集合整理 | L |
| **AE-02** | 场景化角色配置 | 新增 `PrimaryGene` 结构体（role_id/strategy_weights/evolution_generation/fitness_score/domain_config），2 个预置配置：social_media / novel | 编辑 JSON 即创建新主智能体角色 | M |
| **AE-05** | 主→子任务分派协议 | 新增 `swarm/delegation.rs`，定义 `DelegatedTask`/`SubTaskType` 枚举，PrimaryAgent 通过 AgentBus 分派 | 主智能体可将任务按类型分派给对应子智能体 | M |

#### 7.3.2 子智能体重定义（1 个任务）

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| **HI-11** | 6 子智能体角色重定义 | 将现有 Coder/Writer/Reviewer/Researcher/Planner/Generic 重定义为 SearchAgent/OutlineAgent/DraftAgent/ReviewAgent/PolishAgent/ArchiveAgent，更新 system_prompt + tool_set + knowledge_scope | 6 个子智能体按写作场景分工，可通过 WorkerCapability 路由 | M |

#### 7.3.3 基因级进化（1 个任务）

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| **AE-04** | GeneMutator 实现 | 新增 `evolution/gene_mutator.rs`，基于 OutcomeLedger 的适应度信号，对 PrimaryGene.strategy_weights 执行受控变异（±2.5%），适应度下降时回滚 | 主智能体策略权重随使用自动优化，回滚机制可用 | M |

#### 7.3.4 写作场景深度实现（2 个任务）

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| **AE-03** | 自媒体写作场景 | PrimaryAgent(social_media) 完整工作流：搜索热点→生成标题+大纲→撰写初稿→审查→润色+SEO→归档 | 端到端：输入主题→输出完整自媒体文章 | L |
| **AE-03b** | 长篇小说写作场景 | PrimaryAgent(novel) 完整工作流：世界观+人物设定→章节大纲→多章节并行初稿→一致性审查→润色→归档 | 端到端：输入题材→输出多章节小说初稿 | XL |

### 7.4 写作场景工作流详细设计

#### 自媒体写作工作流

```
用户: "帮我写一篇关于AI趋势的公众号文章"
  │
  ▼ PrimaryAgent(social_media).delegate()
  │
  ├→ SearchAgent: "搜索2026年AI趋势热点数据"
  ├→ SearchAgent: "搜索竞品文章的标题风格"
  │
  ▼ (搜索结果返回)
  │
  ├→ OutlineAgent: "生成3个备选标题+文章大纲"
  │
  ▼ (用户选择标题和大纲)
  │
  ├→ DraftAgent: "按大纲撰写初稿(1500字)"
  │
  ▼ (初稿完成)
  │
  ├→ ReviewAgent: "审查逻辑连贯性/事实准确性/平台适配度"
  ├→ PolishAgent: "润色+排版+添加SEO关键词"
  │
  ▼ PrimaryAgent.synthesize() → 最终文章
  │
  └→ ArchiveAgent: "将本次写作经验沉淀到L3记忆"
```

**验收标准**：
- 输入"AI趋势"主题，5 分钟内输出 1500 字完整文章
- 文章包含标题、摘要、正文、SEO 关键词
- 搜索结果被正确引用（provenance 标记）
- 写作经验自动沉淀到记忆

#### 长篇小说写作工作流

```
用户: "帮我写一部玄幻小说，30章"
  │
  ▼ PrimaryAgent(novel).delegate()
  │
  ├→ OutlineAgent: "生成世界观+人物设定+30章大纲"
  │
  ▼ (大纲确认)
  │
  ├→ SearchAgent: "搜索玄幻小说常见设定/套路/读者偏好"
  ├→ ArchiveAgent: "整理人物关系图/时间线/伏笔清单"
  │
  ▼ (并行写作 — 关键创新点)
  │
  ├→ DraftAgent-1: "写第1-5章初稿"
  ├→ DraftAgent-2: "写第6-10章初稿"
  ├→ DraftAgent-3: "写第11-15章初稿"
  │
  ▼ (各章节初稿完成)
  │
  ├→ ReviewAgent: "审查人物一致性/剧情连贯性/伏笔回收"
  ├→ PolishAgent: "润色文笔/统一风格"
  │
  ▼ PrimaryAgent.synthesize() → 完整小说
  │
  └→ ArchiveAgent: "更新人物关系图/伏笔状态"
```

**验收标准**：
- 输入"玄幻小说"题材，输出世界观+人物+30 章大纲
- 多个 DraftAgent 可并行写不同章节
- ReviewAgent 检测到人物/剧情不一致时标记警告
- 人物关系图和伏笔清单随写作进度自动更新

### 7.5 Phase 3 验收标准

```
□ PrimaryAgent trait 实现完整（decompose/delegate/synthesize）
□ 2 个预置主智能体角色可用（social_media / novel）
□ 6 个子智能体按写作场景分工
□ 主→子任务分派协议通过 AgentBus 工作
□ GeneMutator 可根据适应度信号优化策略权重
□ 自媒体写作端到端工作流可用
□ 长篇小说写作端到端工作流可用
□ 多个 DraftAgent 可并行写不同章节
□ 写作经验自动沉淀到记忆
□ 所有新增代码有配套测试
□ cargo check + cargo clippy + npm run typecheck 全绿
```

---

## 8. Phase 4：创新扩展（长期）

### 8.1 全自主革命（v3.0 目标）

| Wave | 版本 | 核心任务 | 验收指标 |
|------|------|---------|---------|
| Wave 1 | v2.3 | 地基修复 + 技术债务清算 | Phase 0+1 验收通过 |
| Wave 2 | v2.4 | AI 自动整理 MOC + 三定时 | T-E-B-15 / T-E-S-63 完成 |
| Wave 3 | v2.5 | 8 人格 + Proactive Engine | 日活跃 10-15 次 |
| Wave 4 | v2.6 | WorkflowCanvas + OS-Controller | Agent 行为可视化 |
| Wave 5 | v3.0 | Cron + OAuth + 多端协同 | 无人值守自动化 |

### 8.2 差异化护城河巩固

| 护城河 | 巩固措施 | 验收 |
|--------|---------|------|
| 最深记忆 | 记忆 6 层 → 8 层（L6-L7） | L6(知识蒸馏) + L7(不变记忆) 实现 |
| 最强安全 | AIO Sandbox 完整实现 | bwrap/seatbelt/MIC 三平台隔离 |
| 可审计可回滚 | 进化日志 + 段落级回滚全面集成 | 所有记忆操作可回滚 |
| 场景化蜂群 | 2+ 自定义主智能体 + 6 通用子智能体 | 自媒体/长篇小说场景闭环 |

---

## 9. 全面验收标准矩阵

### 9.1 代码质量验收

| 指标 | 当前 | Phase 0 | Phase 1 | Phase 2 | Phase 3 |
|------|------|---------|---------|---------|---------|
| AppState 字段数 | 45+ | < 10 | < 10 | < 10 | < 10 |
| 危险 panic 点 | 35 | < 30 | < 20 | < 10 | < 5 |
| Rust 警告数 | 20+ | < 10 | < 5 | 0 | 0 |
| 前端覆盖率(行) | 35.5% | 40% | 50% | 60% | 65% |
| 前端覆盖率(函数) | 22.9% | 35% | 45% | 55% | 60% |
| Rust 单测数 | 993 | 1000+ | 1100+ | 1300+ | 1400+ |
| E2E 测试数 | 2 | 2 | 5+ | 10+ | 15+ |

### 9.2 架构健康验收

| 指标 | 当前 | Phase 3 目标 |
|------|------|-------------|
| 主智能体概念 | 无 | 2+ 自定义角色 |
| 子智能体分工 | 偏编程场景 | 6 个写作场景角色 |
| 角色可编辑性 | 硬编码 Rust | JSON 外部配置 |
| 进化粒度 | prompt/技能/SOUL.md | + 基因级（策略权重） |
| 写作场景 | 2 文件骨架 | 自媒体+长篇小说完整工作流 |
| 主→子分派 | 无 | DelegatedTask 协议 |

### 9.3 哲学一致性验收

| 哲学 | 检查项 | 验收方式 |
|------|--------|---------|
| **可读** | 无 > 500 行单函数 | `wc -l` 扫描 |
| **可读** | 无 > 2000 行单文件 | glob + wc 扫描 |
| **可读** | AppState 字段 < 10 | 代码审查 |
| **可编辑** | Agent 角色可通过 JSON 编辑 | 集成测试验证 |
| **可编辑** | SOUL.md 注入生效 | 集成测试验证 |
| **可追溯** | 关键操作有 audit log | OutcomeLedger 查询 |
| **可追溯** | provenance 字段完整 | memory 表结构检查 |
| **本地优先** | 无强制联网路径 | `cargo check --no-default-features` |
| **可观测** | 关键路径有 tracing span | trace 导出检查 |

---

## 10. 风险登记

| 风险 | 严重度 | 概率 | 缓解措施 |
|------|--------|------|----------|
| digest 升级破坏 E2EE | 🔴 致命 | 低 | 独立 branch 测试 + E2EE 加密/解密验证 |
| AppState 重构引入回归 | 🟡 高 | 中 | 每拆分一步运行 `cargo check` + 全量测试 |
| 子智能体重定义破坏 gRPC 兼容 | 🟡 高 | 中 | 保留旧 AgentKind 作为 gRPC 兼容层 |
| 写作场景工作流复杂度超预期 | 🟡 中 | 高 | 先实现自媒体（单篇文章），再扩展长篇小说 |
| 基因级进化策略漂移 | 🟡 中 | 中 | 受控变异（±2.5%）+ 人类审批网关 + 回滚 |
| bus factor = 1（单人开发） | 🟡 高 | 100% | ADR + CHANGELOG + PRODUCTION_TASK_TRACKER 全程文档化 |
| Phase 2 工作量低估 | 🟡 中 | 中 | 按 Wave 优先级裁剪，先做 P1 后做 P2 |
| CI 跨平台 Runner 费用 | 🟡 中 | 高 | Linux 用免费额度，macOS 用最小配置 |

---

## 11. 版本发布计划

| 版本 | 内容 | 预估时间 |
|------|------|---------|
| **v2.0.2** | Phase 0 地基修复 | Week 3 |
| **v2.1.0** | Phase 1 质量闭环 + 架构准备 | Week 9 |
| **v2.2.0** | Phase 2 Wave 3-4（OS-Controller + WorkflowCanvas） | Week 15 |
| **v2.3.0** | Phase 2 Wave 5 + Phase 3 蜂群架构升级 | Week 21 |
| **v3.0.0** | Phase 4 全自主版本 | Week 26+ |

---

## 12. 每周验收节奏

| 周 | Phase | 核心交付 | 验收指标 |
|----|-------|---------|---------|
| W1 | P0 | CR-01 digest 修复 + CR-02 CI 集成测试恢复 | `cargo check` 通过，CI 集成测试运行 |
| W2 | P0 | CR-03 audit + HI-01 tauri.ts 拆分 + HI-13 文档更新 | audit 追踪表完成，tauri.ts < 100 行 |
| W3 | P0 | CR-04 AppState 重构 + HI-02 bootstrap 拆分 | AppState < 10 字段，phase < 200 行 |
| W4 | P0 | HI-03 覆盖提升 + HI-04 CI 跨平台 + MI 项 | 覆盖率 > 40%，双平台 CI |
| W5 | P1 | 核心文件测试补齐 + Agent 角色外部化 | bootstrap/gateway/dispatcher 测试就绪 |
| W6 | P1 | CI 强化 + Docker 修复 + E2E 入门 | HEALTHCHECK, non-root, E2E 2+ |
| W7 | P1 | PrimaryAgent trait + WorkerCapability 扩展 | trait 定义 + 写作枚举就绪 |
| W8 | P1 | 前端质量重构 + 覆盖率门槛提升 | ChatPanel < 300, 50/40/40/50 |
| W9 | P1 | Phase 1 验收 | 全部验收标准通过 |
| W10-15 | P2 | 功能补齐（按 Wave 3-5 推进） | 各 Wave 验收通过 |
| W16 | P3 | PrimaryAgent + 场景化角色 + 分派协议 | 主智能体可分解/分派/集合 |
| W17 | P3 | GeneMutator + 子智能体重定义 | 基因级进化可用 |
| W18 | P3 | 自媒体写作场景端到端 | 输入主题→输出文章 |
| W19 | P3 | 长篇小说写作场景端到端 | 输入题材→输出多章节初稿 |
| W20-21 | P3 | 集成测试 + 性能优化 + Phase 3 验收 | 全部验收标准通过 |
| W22+ | P4 | 全自主革命 | 按 Wave 验收 |

---

## 13. 立即行动项（今天）

| 顺序 | 行动 | 预期耗时 |
|------|------|---------|
| 1 | **修复 digest crate 冲突**（CR-01） | ~4h |
| 2 | **创建 cargo audit 追踪表**，逐项评估 14 个忽略项 | ~2h |
| 3 | **提交 CI 跨平台配置变更**，添加 Linux runner | ~2h |
| 4 | **启动 tauri.ts 拆分**，创建 `api/chat.ts` + `api/memory.ts` | ~4h |
| 5 | **删除/归档** 过时文档（IMPROVEMENT_PLAN_v1.0.md） | ~0.5h |

---

## 14. 任务总表

### 14.1 Phase 0 任务总表（27d）

| 编号 | 对应缺陷 | 优先级 | 复杂度 | 工时 |
|------|---------|--------|--------|------|
| P0-A-1 | CR-01 digest 冲突 | P0 | S | 1d |
| P0-A-2 | CR-02 CI 集成测试 | P0 | M | 3d |
| P0-A-3 | CR-03 audit 追踪 | P0 | M | 2d |
| P0-A-4 | CR-04 AppState 重构 | P0 | L | 5d |
| P0-B-1 | HI-01 tauri.ts 拆分 | P0 | M | 3d |
| P0-B-2 | HI-02 bootstrap 拆分 | P0 | L | 5d |
| P0-B-3 | HI-03 前端覆盖提升 | P0 | L | 5d |
| P0-B-4 | HI-04 CI 跨平台 | P0 | M | 3d |
| P0-B-5 | HI-09 AgentKind 清理 | P0 | M | 2d |
| P0-B-6 | HI-10 writing/ 补齐 | P0 | L | 5d |
| P0-B-7 | HI-13 文档更新 | P0 | S | 0.5d |
| P0-C-01~10 | MI-01~10 修复 | P0 | S/M | 5d |

### 14.2 Phase 1 任务总表（22d）

| 编号 | 描述 | 优先级 | 复杂度 | 工时 |
|------|------|--------|--------|------|
| P1-A-1 | bootstrap 测试补齐 | P1 | M | 3d |
| P1-A-2 | gateway 测试补齐 | P1 | M | 3d |
| P1-A-3 | dispatcher 测试补齐 | P1 | M | 2d |
| P1-A-4 | app_config 测试补齐 | P1 | S | 1d |
| P1-B-1 | CI 门禁强化 | P1 | S | 1d |
| P1-C-1 | Agent 角色配置外部化 | P1 | M | 3d |
| P1-C-2 | master-orchestrator 运行时开关 | P1 | S | 1d |
| P1-C-3 | PrimaryAgent trait 设计 | P1 | M | 2d |
| P1-C-4 | WorkerCapability 扩展 | P1 | S | 1d |
| P1-D-1 | ChatPanel 拆分 | P1 | M | 2d |
| P1-D-2 | i18n 类型安全 | P1 | M | 2d |
| P1-D-3 | Docker HEALTHCHECK + non-root | P1 | S | 1d |
| P1-D-4 | Playwright E2E 接入 CI | P1 | M | 2d |

### 14.3 Phase 2 任务总表（~150d，按 Wave 裁剪）

> 与 ROADMAP_v2.3 §3 对齐，此处不重复。详见 ROADMAP_v2.3.md §3.1-3.3。

### 14.4 Phase 3 任务总表（~30d）

| 编号 | 对应缺口 | 复杂度 | 工时 |
|------|---------|--------|------|
| P3-A-1 | AE-01 PrimaryAgent 实现 | L | 8d |
| P3-A-2 | AE-02 场景化角色配置 | M | 4d |
| P3-A-3 | AE-05 主→子分派协议 | M | 4d |
| P3-A-4 | HI-11 子智能体重定义 | M | 3d |
| P3-A-5 | AE-04 GeneMutator | M | 4d |
| P3-A-6 | AE-03 自媒体写作场景 | L | 5d |
| P3-A-7 | AE-03b 长篇小说写作场景 | XL | 8d |

---

## 15. 总工时估算

| Phase | 工时 | 日历时间 | 主要风险 |
|-------|------|---------|---------|
| **Phase 0** 地基修复 | 27d | 2-3 周 | digest 升级破坏 E2EE |
| **Phase 1** 质量闭环 + 架构准备 | 22d | 4-6 周 | AppState 重构引入回归 |
| **Phase 2** 功能补齐 | ~150d | 6-8 周 | 工作量低估，需按 Wave 裁剪 |
| **Phase 3** 蜂群架构升级 | ~30d | 4-6 周 | 写作场景复杂度超预期 |
| **Phase 4** 创新扩展 | 持续 | 6+ 月 | 竞品追赶 |

---

> **Nebula 承诺**：你的知识，如星云般不断演化。
>
> 这份建议书不是一张完美的蓝图，而是一份务实的航海图——它承认现状（74% 完成度 + 20 项技术债务 + 架构演进缺口），尊重哲学（信任三原则），并给出可执行的修复路径。
>
> 每一行代码都应为**可读、可编辑、可追溯**而存在。

**文档结束。**
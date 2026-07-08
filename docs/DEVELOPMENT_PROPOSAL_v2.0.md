# Nebula 全面修复与迭代开发建议书 v2.0

**依据哲学**：「信任三原则」——可读·可编辑·可追溯  
**核心宣言**：你无法信任一段你无法阅读的记忆  
**基线版本**：v2.0.0（代码库当前状态）  
**制定日期**：2026-07-08  
**配套文档**：`WHITEPAPER_v3.1.md`（设计权威）、`ROADMAP_v2.3.md`（未完成任务）、`IMPROVEMENT_PLAN_v1.0.md`（前序规划）

---

## 0. 项目哲学基线（Open Mythos）

### 0.1 信任三原则（v3.0 核心哲学）

| 原则 | 含义 | 当前落地状态 |
|------|------|-------------|
| **可读（Readable）** | 所有记忆以人类可读的 Markdown 渲染；LLM Wiki 编译输出；三视图可用 | ✅ 基础落地，Memory Inspector / LLM Wiki / 三视图已实现 |
| **可编辑（Editable）** | 用户可任意修改记忆，AI 写入与人类编辑双向同步 | ✅ 基础落地，双向同步 + provenance 字段完成 |
| **可追溯（Traceable）** | 每条记忆携带 provenance，决策可回溯 | ✅ 基础落地，version_control + provenance 实现 |

### 0.2 v2.0 五哲学

| 哲学 | 当前状态 |
|------|---------|
| 记忆是 AI 的灵魂（8 层 L0-L7） | ✅ 完成 |
| 模式对用户不可见（AI 自动判断） | ✅ 完成 |
| 价值对齐前置（L4 价值层） | ✅ 完成 |
| 本地优先（E2EE + 私钥不出设备） | ✅ 完成 |
| 可观测可审计（Prometheus + OpenTelemetry） | ⚠️ 基础完成，指标覆盖有缺口 |

### 0.3 四大支柱落地状态

| 支柱 | 目标 | 完成度 |
|------|------|--------|
| 💰 **更省钱**（Cost/Tokens 优化） | 月度成本 $30→$3 | 100% |
| 🧠 **更智能**（记忆可读+Wiki） | 记忆从黑盒变白盒 | 94% |
| 🔧 **更贴合**（OS-Controller+场景） | 覆盖完整工作链路 | 45% |
| ⚡ **更快**（性能+桌面形象） | 冷启动 3s / 首响 500ms | 60% |

**Stage 7 总进度：74%（92/125 任务完成）** · 技术债务：0%（19 项全未开始）

---

## 1. 当前项目缺陷与风险

### 1.1 🔴 严重缺陷（阻碍构建/运行，必须立即处理）

| ID | 缺陷 | 影响 | 发现来源 |
|----|------|------|---------|
| **CR-01** | `digest` crate 版本冲突：`sha2 v0.11`(digest v0.11.3) vs `hkdf v0.13`(digest v0.10.7)，破坏 `src/sync/e2ee.rs:195` 和 `src/im/webhook.rs:26` | `cargo check --features grpc,channels` 编译失败，blocking | 代码审查 |
| **CR-02** | Windows CI 集成测试被跳过：`cargo nextest run --lib` 因 STATUS_ENTRYPOINT_NOT_FOUND 只跑单元测试，25 个集成测试文件 + 2 个 e2e 测试不执行 | 集成测试覆盖真空，回归风险高 | CI 配置审查 |
| **CR-03** | `cargo audit` 14 个安全建议被忽略（continue-on-error），无追踪机制 | 已知漏洞未被跟踪修复 | CI 配置审查 |

### 1.2 🟡 严重问题（影响质量/生产力）

| ID | 缺陷 | 影响 |
|----|------|------|
| **HI-01** | `tauri.ts` 单文件 3190 行 / 108KB，20+ 领域混合 | 维护困难，违反"可读"哲学 |
| **HI-02** | `bootstrap.rs` 1113 行单函数 | 业务逻辑不可读不可测，违反"可读" |
| **HI-03** | 前端测试覆盖率低（行 35.5%，函数 22.91%） | 17+ 组件零测试，重构无安全网 |
| **HI-04** | CI 仅 Windows runner，无 macOS/Linux | 跨平台兼容性无保障 |
| **HI-05** | `incremental = false`（Rust 1.96.1 ICE 工作区） | 每次全量重建，开发迭代慢 |
| **HI-06** | `memory/` 40+ 子文件平铺无分组 | 模块内聚性差，违反"可读" |
| **HI-07** | 核心文件零测试（bootstrap/gateway/dispatcher/app_config） | 最关键的启动/调度代码无测试保护 |
| **HI-08** | Dockerfile 缺 HEALTHCHECK、非 root 运行、多架构支持 | 生产部署不规范 |

### 1.3 🟠 中等缺陷（影响架构健康/可维护性）

| ID | 缺陷 | 影响 |
|----|------|------|
| **MI-01** | ESLint 配置不存在 | 前端代码风格无强制保障 |
| **MI-02** | Prettier 配置不存在 | 格式化无统一标准 |
| **MI-03** | `tsconfig` 禁用了 `noUnusedLocals/Parameters` | 死代码易堆积 |
| **MI-04** | Vite/Vitest 配置重复 | 配置维护双倍成本 |
| **MI-05** | `tracing_setup.rs` 8 路组合爆炸 | 日志配置过度复杂 |
| **MI-06** | 硬编码中文字符串散落前端 | i18n 不完整 |
| **MI-07** | 前端 `cancelled` 布尔反模式 | 竞态条件隐患 |
| **MI-08** | `std::mem::forget(h)` 泄露 JoinHandle | 资源泄漏风险 |
| **MI-09** | 死 feature 未清理（custom-protocol 等） | 配置污染 |
| **MI-10** | `IMPROVEMENT_PLAN_v1.0.md` 过时但仍被 git 跟踪 | 文档误导风险 |

### 1.4 🔵 功能缺口（未完成的高级特性）

| 支柱 | 未完成任务数 | 关键缺口 |
|------|-------------|---------|
| **C 贴合** | 11 个 (45%) | OS-Controller 双模式、Hybrid Browser Agent、OAuth 集成、语音交互 |
| **D 快** | 4 个 (60%) | 8 人格系统、Proactive Engine、WebGL 性能优化 |
| **S 贯穿** | 12 个 (77%) | WorkflowCanvas、蜂群画布、Event Stream 协议化、Cron 定时引擎 |
| **B 智能** | 1 个 (94%) | AI 自动整理 MOC |
| **Loop** | 5 个 (50%) | GitHub MCP 连接器、Loop 运行时阶段环 |

---

## 2. 改进原则与优先级策略

### 2.1 核心理念映射

每项改进任务须至少符合一条 Nebula 哲学：

| 哲学 | 改进任务筛选条件 |
|------|----------------|
| **可读** | 减少单块文件、增加注释文档、提升代码组织 |
| **可编辑** | 使配置/参数可外部修改、增加回滚能力 |
| **可追溯** | 增加审计日志、provenance、可观测性 |
| **本地优先** | 不引入强制性外部依赖 |
| **可观测** | 增加指标、追踪、日志覆盖 |

### 2.2 优先级分类

| 优先级 | 定义 | 处理时机 |
|--------|------|---------|
| **P0** | 阻塞构建/运行，或导致数据损失/安全漏洞 | 立即处理 |
| **P1** | 严重降低开发效率/代码质量 | 本迭代处理 |
| **P2** | 影响架构健康/可维护性 | 下个迭代处理 |
| **P3** | 功能增强/新特性 | 按路线图推进 |

### 2.3 依赖关系

```
Phase 0（地基修复，2-3周）
  ├─ CR-01~CR-03（阻塞修复）
  ├─ HI-01~HI-04（质量修补）
  └─ MI-01~MI-10（健康改进）
        │
Phase 1（质量闭环，4-6周）
  ├─ 技术债务 P0 清算
  ├─ 核心文件测试补齐
  ├─ CI 跨平台恢复
  └─ 前端质量重构
        │
Phase 2（功能补齐，6-8周）
  ├─ 支柱 C/D/S 未完成任务
  ├─ Loop Engineering 内化
  └─ 端到端体验优化
        │
Phase 3（创新扩展，长期）
  └─ v3.0 全自主革命目标
```

---

## 3. Phase 0：地基修复（2-3 周）

### 3.1 P0-A：构建阻塞修复

#### A-1：digest crate 版本冲突（CR-01）

| 字段 | 内容 |
|------|------|
| **任务** | 修复 `sha2 v0.11` 与 `hkdf v0.13` 的 digest 版本不兼容 |
| **根因** | `sha2:0.11` 使用 `digest:0.11`，但 `hkdf:0.13` 期望 `digest:0.10` trait |
| **方案** | 方案 A：降级 `sha2` 到 `=0.10`（需验证 `aes-gcm` 依赖兼容性）<br>方案 B：升级 `hkdf` 到支持 `digest 0.11` 的版本（查 `hkdf:0.16+` 是否兼容）<br>方案 C：在 `Cargo.toml` 中 `[patch]` digest 到统一版本 |
| **涉及文件** | `Cargo.toml`（dependencies），`src/sync/e2ee.rs:195`，`src/im/webhook.rs:26` |
| **验收** | `cargo check --features grpc,channels --lib` 通过，`cargo check --features grpc,channels --tests` 通过 |
| **类型** | P0 / S 复杂度 |

#### A-2：Windows CI 集成测试（CR-02）

| 字段 | 内容 |
|------|------|
| **任务** | 恢复 CI 上集成测试的执行，解决 STATUS_ENTRYPOINT_NOT_FOUND |
| **方案** | 方案 A：将 Tauri 相关测试隔离到独立 binary 避免 COM 冲突<br>方案 B：使用 `#[cfg(not(target_os = "windows"))]` 跳过 Tauri 依赖测试<br>方案 C：添加 macOS/Linux runner 跑完整测试套件 |
| **涉及文件** | `.github/workflows/test.yml` |
| **验收** | CI 中至少一个平台运行全部集成测试（25+ 测试文件），且结果可见于 CI 报告 |
| **类型** | P0 / M 复杂度 |

#### A-3：cargo audit 安全建议追踪（CR-03）

| 字段 | 内容 |
|------|------|
| **任务** | 逐项评估 14 个被忽略的 cargo audit 建议，建立追踪机制 |
| **方案** | 1. 逐项评估能否升级/替换<br>2. 无法立即修复的写入 `SECURITY_ADVISORIES.md` 追踪表<br>3. CI 中改为 `continue-on-error: false`（或分两组：已知+新） |
| **验收** | 14 项全部有评估结果（已修复/已记录+追踪），新增漏洞会触发 CI 告警 |
| **类型** | P0 / M 复杂度 |

### 3.2 P0-B：严重质量问题修复

#### B-1：tauri.ts 按领域拆分（HI-01）

| 字段 | 内容 |
|------|------|
| **任务** | 将 3190 行 tauri.ts 拆分为 8+ 领域模块 |
| **方案** | `api/chat.ts` / `api/memory.ts` / `api/skill.ts` / `api/swarm.ts` / `api/work.ts` / `api/writing.ts` / `api/editor.ts` / `api/sync.ts` / `api/os.ts` + `types.ts` |
| **验收** | 原 tauri.ts < 100 行（barrel re-export），拆分后各模块 < 500 行，`cargo check` + `npm run typecheck` 通过 |
| **类型** | P0 / M 复杂度 |

#### B-2：bootstrap.rs 拆分（HI-02）

| 字段 | 内容 |
|------|------|
| **任务** | 将 1113 行 bootstrap 单函数拆分为 5-8 个 phase 函数 |
| **方案** | `bootstrap_config()` / `bootstrap_storage()` / `bootstrap_llm()` / `bootstrap_memory()` / `bootstrap_swarm()` / `bootstrap_commands()` / `bootstrap_grpc()` / `bootstrap_headless()` |
| **验收** | 各 phase 函数 < 200 行，`cargo check` 通过，原有功能无变化 |
| **类型** | P0 / L 复杂度 |

#### B-3：前端测试覆盖率提升（HI-03）

| 字段 | 内容 |
|------|------|
| **任务** | 将覆盖率阈值从 30%/20%/25%/30% 提升至 50%/40/40/50 |
| **方案** | 为 10+ 个零测试组件添加基础渲染+交互测试（ArenaPanel, Dashboard, DagCanvas, FloatingBall, KnowledgeCardDialog 等） |
| **验收** | `npm run test:coverage` 达到新阈值，CI coverage check pass |
| **类型** | P0 / L 复杂度 |

#### B-4：CI 跨平台评估（HI-04）

| 字段 | 内容 |
|------|------|
| **任务** | 恢复至少一个非 Windows runner（macOS 或 Linux） |
| **方案** | 最小：添加 Linux runner 仅跑 `cargo check --no-default-features`（无 Tauri 依赖）<br>完整：Linux runner 跑完整测试套件（无 `--lib` 限制） |
| **验收** | CI 矩阵至少包含 windows + linux，linux 上运行完整 cargo test |
| **类型** | P0 / M 复杂度 |

### 3.3 P0-C：可维护性问题修复

| ID | 任务 | 方案 | 验收 | 复杂度 |
|----|------|------|------|--------|
| MI-01 | 添加 ESLint flat 配置 | 新增 `eslint.config.mjs`，与现有 `.eslintrc` 对齐 | `npm run lint` 通过，CI lint job pass | S |
| MI-02 | 添加 Prettier 配置 | 新增 `.prettierrc` + `.prettierignore` | `npm run format` 可靠运行 | S |
| MI-03 | tsconfig 死代码检测 | 分阶段启用 `noUnusedLocals` / `noUnusedParameters` | `npm run typecheck` 0 错误 | S |
| MI-04 | Vite/Vitest 配置统一 | 提取公共配置到 `vite.shared.ts` | 单配置源生效 | S |
| MI-05 | tracing_setup.rs 重构 | Builder pattern 替代 8 路组合爆炸 | 配置代码减少 50%+ | S |
| MI-06 | 硬编码字符串迁移 | 扫描前端提取未 i18n 的中文字符串 | 新增 key + 翻译 | M |
| MI-07 | cancelled 布尔反模式 | 替换为 AbortController | `cargo clippy` 无相关警告 | S |
| MI-08 | JoinHandle 泄漏修复 | 保存到 struct + 添加 panic hook | `cargo clippy` 无相关警告 | S |
| MI-09 | 死 feature 清理 | 审计 Cargo.toml 移除无效 feature | `cargo check` 通过 | S |
| MI-10 | 过时文档清理 | 将 `IMPROVEMENT_PLAN_v1.0.md` 移至 `docs/archive/` | 根目录无过时规划文档 | S |
| MI-11 | memory/ 模块分组 | `memory/l0_cache/`, `memory/orchestrator/`, `memory/acl/` 等 | 模块目录结构清晰 | M |

### 3.4 Phase 0 验收标准

```
□ cargo check --features grpc,channels --lib 通过（CR-01 修复）
□ cargo check --features grpc,channels --tests 通过
□ CI 中至少一个平台跑完全部集成测试（CR-02 修复）
□ cargo audit 14 项全部评估完成（CR-03 修复）
□ tauri.ts < 100 行 + 8+ 领域模块就绪（HI-01 修复）
□ bootstrap.rs 各 phase < 200 行（HI-02 修复）
□ 前端覆盖率 ≥ 50/40/40/50（HI-03 修复）
□ CI 至少 windows + linux 双平台（HI-04 修复）
□ ESLint + Prettier + tsconfig 严格模式就绪
□ 所有 MI 项修复完成
□ cargo clippy --features grpc,channels -- -D warnings 通过
□ cargo fmt --all -- --check 通过
```

---

## 4. Phase 1：质量闭环（4-6 周）

### 4.1 P1-A：技术债务清算

#### A-1：核心文件测试补齐（T-D-T-02）

| 任务 | 文件 | 当前测试数 | 目标测试数 |
|------|------|-----------|-----------|
| bootstrap 测试 | `bootstrap.rs` | 0 | 5+（各 phase 独立测试） |
| gateway 测试 | `llm/gateway.rs` | 0 | 8+（断路器/降级/路由） |
| dispatcher 测试 | `grpc/dispatcher.rs` | 0 | 5+（请求分发/错误处理） |
| app_config 测试 | `app_config.rs` | 0 | 5+（环境变量解析） |

#### A-2：CI 强化（T-D-C-01~05）

| 任务 | 验收 |
|------|------|
| macOS + Linux CI runner 恢复 | 三平台 `cargo check` 全绿 |
| Docker HEALTHCHECK + 非 root | `docker inspect` 显示健康状态 |
| Playwright E2E 接入 CI | 2+ 个用户场景 E2E 测试 |

#### A-3：前端质量重构（T-D-F-01~06）

| 任务 | 验收 |
|------|------|
| ChatPanel 拆分 847 行 → < 300 行 | `wc -l` 验证 |
| i18n 类型安全 + 完整覆盖 | 基于 zh-CN.json 推导类型 |
| 响应式布局 | 移动端 375px 可用 |

### 4.2 P1-B：CI/CD 安全门禁

| 门禁项 | 当前 | 目标 |
|--------|------|------|
| clippy 门前 | 有 | `-D warnings` 全绿 |
| fmt 门前 | 有 | `--check` 全绿 |
| audit 门前 | continue-on-error | 分两组：已知(allow) + 新(deny) |
| coverage 门前 | 30/20/25/30 | 50/40/40/50 |
| typecheck 门前 | 有 | 0 error |
| E2E 门前 | 无 | 2+ 冒烟 |

### 4.3 Phase 1 验收标准

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
□ npm run typecheck 0 error
□ npm run lint 0 error
□ npm run format:check 通过
```

---

## 5. Phase 2：功能补齐（6-8 周）

### 5.1 P2-A：支柱 C 贴合（11 个未完成任务）

按 ROADMAP_v2.3 Wave 4 顺序执行：

| Wave | 任务 | 目标 | 验收 |
|------|------|------|------|
| W4-1 | **T-E-C-01** OS-Controller 双模式 | API + VLM 双模式操作电脑 | 可通过 API 和截图两种模式执行 click/type |
| W4-2 | **T-E-C-05** OS-Controller Sidecar | 独立进程运行 OS-Controller | Sidecar 启动/停止正常 |
| W4-3 | **T-E-C-06** Hybrid Browser Agent | GUI + CDP + DOM 三策略混合 | 可打开网页并执行操作 |
| W4-4 | **T-E-C-15** 语音交互引擎 | 语音输入 + TTS 输出 | 悬浮球语音按钮可用 |
| W4-5 | **T-E-C-18** OAuth 集成层 | 5 个核心 OAuth 服务 | Gmail/GitHub/Notion/Obsidian/MS 授权可用 |
| W4-6 | **T-E-C-19** 多端协同 | Desktop + CLI + PWA + 渠道 | 多渠道消息统一收件箱 |

### 5.2 P2-B：支柱 S 贯穿层（12 个未完成任务）

| Wave | 任务 | 目标 | 验收 |
|------|------|------|------|
| W4-7 | **T-E-S-10** WorkflowCanvas | React Flow 拖拽编排，5 种节点类型 | 可拖拽创建/编辑工作流 |
| W4-8 | **T-E-S-11** 蜂群运行时画布 | Agent 实时运行可视化 | Agent 节点状态实时更新 |
| W4-9 | **T-E-S-26** Event Stream 协议化 | SwarmEvent 升级协议 | type/payload/trace_id/timestamp 完整 |
| W4-10 | **T-E-S-53** Cron 定时任务引擎 | 三计时机制 | 03:00/12:00/21:00 自动执行 |
| W4-11 | **T-E-S-58** Calendar 组件 | 日历 + 日程管理 | 可查看/创建日程 |
| W4-12 | **T-E-S-60** Gateway 守护进程 | 进程守护 + 自动重启 | crash 后 5s 内自动重启 |
| W4-13 | **T-E-S-63** 三定时机制 | 每日/周/月定时任务 | consolidation 按计划执行 |

### 5.3 P2-C：支柱 D 快（4 个未完成任务）

| Wave | 任务 | 目标 | 验收 |
|------|------|------|------|
| W3-1 | **T-E-D-04** 8 人格系统 | 8 种人格 + 表情联动 | 人格切换生效，表情随之变化 |
| W3-2 | **T-E-D-05** Proactive Engine | 主动问候/任务跟进/闲聊 | 每日主动交互 1-2 次 |
| W4-14 | **T-E-D-08** WebGL 渲染优化 | 图谱 1000 节点 60fps | PixiJS 基准测试通过 |
| W4-15 | **T-E-D-09** UI 性能基准 CI | 首次渲染/重绘/内存基准 | CI step 可对比性能趋势 |

### 5.4 P2-D：支柱 B 智能收尾

| 任务 | 目标 | 验收 |
|------|------|------|
| **T-E-B-15** AI 自动整理 MOC | MOC 自动生成 | 每周自动整理知识库 MOC |
| **T-E-S-63**（与 P2-B 共享） | 三定时机制 | 为 T-E-B-15 提供定时触发 |

### 5.5 P2-E：Loop Engineering 内化（T-E-L-04/07/08a/08b）

| 任务 | 目标 | 验收 |
|------|------|------|
| **T-E-L-04** GitHub MCP 连接器 | pull-only，读取 Issue/PR/Code | `gh issue list` 等价 |
| **T-E-L-07** Loop 审计日志 | 每次 Loop 执行记录 | OutcomeLedger 可查询 |
| **T-E-L-08a** Loop 运行时阶段环 | 依赖 T-E-S-11 | 蜂群画布中显示 Loop 状态 |
| **T-E-L-08b** Loop 设计节点 | 依赖 T-E-S-10 | WorkflowCanvas 支持 Loop 节点 |

### 5.6 Phase 2 验收标准

```
□ OS-Controller 双模式可执行 click/type
□ Hybrid Browser Agent 可操作网页
□ OAuth 5 服务授权可用
□ WorkflowCanvas 可拖拽编排
□ 蜂群运行时画布实时显示 Agent 状态
□ Cron 三定时正常执行
□ 8 人格切换生效
□ Proactive Engine 每日主动交互
□ 图谱 1000 节点 60fps
□ GitHub MCP 连接器可用
□ Loop 审计日志可查询
□ 三定时 + MOC 自动整理完成
□ 所有功能配套测试通过
□ CI 全绿
```

---

## 6. Phase 3：创新扩展（长期，按 Wave 推进）

### 6.1 P3-A：全自主革命（v3.0 目标）

| Wave | 版本 | 核心任务 | 验收指标 |
|------|------|---------|---------|
| Wave 1 | v2.3 | 地基修复 + 技术债务清算 | Phase 0+1 验收通过 |
| Wave 2 | v2.4 | AI 自动整理 MOC + 三定时 | T-E-B-15 / T-E-S-63 完成 |
| Wave 3 | v2.5 | 8 人格 + Proactive Engine | 日活跃 10-15 次 |
| Wave 4 | v2.6 | WorkflowCanvas + OS-Controller | Agent 行为可视化 |
| Wave 5 | v3.0 | Cron + OAuth + 多端协同 | 无人值守自动化 |

### 6.2 P3-B：差异化护城河巩固

| 护城河 | 巩固措施 | 验收 |
|--------|---------|------|
| 最深记忆 | 记忆 6 层 → 8 层（L6-L7） | L6(知识蒸馏) + L7(不变记忆) 实现 |
| 最强安全 | AIO Sandbox 完整实现 | bwrap/seatbelt/MIC 三平台隔离 |
| 可审计可回滚 | 进化日志 + 段落级回滚全面集成 | 所有记忆操作可回滚 |

---

## 7. 全面验收标准矩阵

### 7.1 代码质量验收

| 指标 | 当前 | Phase 0 目标 | Phase 1 目标 | Phase 2 目标 |
|------|------|-------------|-------------|-------------|
| 危险 panic 点 | 35 | < 30 | < 20 | < 10 |
| Rust 警告数 | 20+ | < 10 | < 5 | 0 |
| TypeScript 错误 | 0 | 0 | 0 | 0 |
| ESLint 错误 | ? | 0 | 0 | 0 |
| 前端覆盖率(行) | 35.5% | 40% | 50% | 60% |
| 前端覆盖率(函数) | 22.9% | 35% | 45% | 55% |
| 前端测试文件 | 16 | 16+ | 25+ | 35+ |
| Rust 单测数 | 993 | 1000+ | 1100+ | 1300+ |
| Rust 集成测试 | 142 | 142 | 180+ | 220+ |
| E2E 测试数 | 2 | 2 | 5+ | 10+ |

### 7.2 CI/CD 验收

| 门禁 | 当前 | Phase 0 | Phase 1 | Phase 2 |
|------|------|---------|---------|---------|
| cargo check | ❌ 失败 | ✅ 全绿 | ✅ 全绿 | ✅ 全绿 |
| cargo clippy | ✅ 通过 | ✅ -D warnings | ✅ -D warnings | ✅ -D warnings |
| cargo format | ✅ 通过 | ✅ | ✅ | ✅ |
| cargo audit | ⚠️ 忽略14 | ✅ 追踪 | ✅ 分两组 | ✅ 全绿 |
| npm typecheck | ✅ 通过 | ✅ | ✅ | ✅ |
| npm lint | ❌ 不存在 | ✅ 存在 | ✅ 通过 | ✅ -D warnings |
| npm test:coverage | ⚠️ 35% | ✅ 40% | ✅ 50% | ✅ 60% |
| Playwright E2E | ❌ 不存在 | ❌ | ✅ 2+ | ✅ 5+ |
| 跨平台 CI | ❌ 仅 Windows | ❌ | ✅ win+lin | ✅ win+lin+mac |

### 7.3 安全验收

| 领域 | 当前 | 目标 |
|------|------|------|
| cargo audit 已忽略 | 14 | 0（或全部追踪） |
| Docker non-root | ❌ | ✅ |
| injection_guard 覆盖率 | 100% (13/13 已修复) | 保持 100% |
| SSRF 覆盖率 | 100% (13/13 已修复) | 保持 100% |
| MemoryAcl deny-all | ✅ | ✅ 保持 |
| E2EE 加密密钥轮换 | ✅ | ✅ 保持 |
| AIO Sandbox | ❌ 未实现 | Phase 3 目标 |

### 7.4 哲学一致性验收

| 哲学 | 检查项 | 验收方式 |
|------|--------|---------|
| **可读** | 无 > 500 行单函数 | `wc -l` 扫描 |
| **可读** | 无 > 2000 行单文件 | glob + wc 扫描 |
| **可读** | 关键模块有模块级文档注释 | `grep "^//!"` 扫描 |
| **可编辑** | 配置可通过环境变量覆盖 | 检查 AppConfig |
| **可编辑** | SOUL.md 注入生效 | 集成测试验证 |
| **可追溯** | 关键操作有 audit log | 检查 OutcomeLedger |
| **可追溯** | provenance 字段完整 | memory 表结构检查 |
| **本地优先** | 无强制联网路径 | `cargo check --no-default-features` |
| **可观测** | 关键路径有 tracing span | trace 导出检查 |

---

## 8. 风险登记

| 风险 | 严重度 | 概率 | 缓解措施 |
|------|--------|------|---------|
| digest 升级破坏 E2EE | 🔴 致命 | 低 | 先在独立 branch 测试，集成验证 E2EE 加密/解密 |
| 模块拆分引入回归 | 🟡 高 | 中 | 每拆分一步运行 `cargo check` + 集成测试 |
| CI 跨平台 Runner 费用 | 🟡 中 | 高 | Linux 用免费额度，macOS 用最小配置 |
| 前端重构破坏用户交互 | 🟡 高 | 中 | E2E 测试 + 视觉回归测试覆盖 |
| bus factor = 1（单人开发） | 🟡 高 | 100% | ADR + CHANGELOG 全程文档化（已部分实施） |
| Phase 2 工作量低估 | 🟡 中 | 中 | 按 Wave 优先级裁剪，先做 P1 后做 P2 |
| 竞品在 Phase 2 期间追赶 | 🟡 中 | 中 | 聚焦差异化护城河，不追赶通用功能 |

---

## 9. 版本发布计划

| 版本 | 内容 | 预估日期 |
|------|------|---------|
| **v2.0.1** | Phase 0 地基修复 | Week 3 |
| **v2.1.0** | Phase 1 质量闭环 | Week 9 |
| **v2.2.0** | Phase 2 功能补齐（C 支柱+S 贯穿层） | Week 15 |
| **v2.3.0** | Phase 2 功能补齐（D 支柱+B 智能+Loop） | Week 17 |
| **v3.0.0** | Phase 3 全自主版本 | Week 26+ |

---

## 10. 每周验收节奏

| 周 | Phase | 核心交付 | 验收指标 |
|----|-------|---------|---------|
| W1 | P0 | CR-01 digest 修复 + CR-02 CI 集成测试恢复 | `cargo check` 通过，CI 集成测试运行 |
| W2 | P0 | CR-03 cargo audit + HI-01 tauri.ts 拆分 | audit 追踪表完成，tauri.ts < 100 行 |
| W3 | P0 | HI-02 bootstrap 拆分 + HI-03 覆盖提升 | phase 函数 < 200 行，覆盖率 > 40% |
| W4 | P0 | HI-04 CI 跨平台 + MI 项全部修复 | 双平台 CI，所有 MI 项 close |
| W5 | P1 | 核心文件测试补齐 | bootstrap/gateway/dispatcher 测试就绪 |
| W6 | P1 | CI 强化 + Docker 修复 + E2E 入门 | HEALTHCHECK, non-root, E2E 2+ |
| W7 | P1 | 前端质量重构 | ChatPanel < 300, i18n 类型安全 |
| W8 | P1 | 覆盖率门槛提升 + lint/formatter 门禁 | 50/40/40/50, CI 全绿 |
| W9 | P1 | Phase 1 验收 | 全部验收标准通过 |
| W10 | P2 | OS-Controller 双模式 | API 模式 click/type 可用 |
| W11 | P2 | Hybrid Browser Agent | 网页操作 3+ 场景 |
| W12 | P2 | WorkflowCanvas | 拖拽编排工作流 |
| W13 | P2 | 蜂群运行时画布 | Agent 实时可视化 |
| W14 | P2 | Cron 定时引擎 + 三定时 | 定时任务自动执行 |
| W15 | P2 | 8 人格系统 + Proactive | 人格切换生效，主动交互 1-2 次/天 |
| W16 | P2 | OAuth 集成层 | 3+ 服务授权可用 |
| W17 | P2 | Loop Engineering 内化 | GitHub MCP + 审计日志 |
| W18+ | P3 | 全自主革命 | 按 Wave 验收 |

---

## 11. 附录：任务追踪速查表

### 11.1 Phase 0 任务总表

| 任务编号 | 对应缺陷 | 优先级 | 复杂度 | 预估工时 | 状态 |
|---------|---------|--------|--------|---------|------|
| P0-A-1 | CR-01 digest 冲突 | P0 | S | 1d | ⏳ |
| P0-A-2 | CR-02 CI 集成测试 | P0 | M | 3d | ⏳ |
| P0-A-3 | CR-03 audit 追踪 | P0 | M | 2d | ⏳ |
| P0-B-1 | HI-01 tauri.ts 拆分 | P0 | M | 3d | ⏳ |
| P0-B-2 | HI-02 bootstrap 拆分 | P0 | L | 5d | ⏳ |
| P0-B-3 | HI-03 前端覆盖提升 | P0 | L | 5d | ⏳ |
| P0-B-4 | HI-04 CI 跨平台 | P0 | M | 3d | ⏳ |
| P0-C-01~11 | MI-01~11 修复 | P0 | S/M | 5d | ⏳ |

### 11.2 Phase 1 任务总表

| 任务编号 | 描述 | 优先级 | 复杂度 | 预估工时 |
|---------|------|--------|--------|---------|
| P1-A-1 | bootstrap 测试补齐 | P1 | M | 3d |
| P1-A-2 | gateway 测试补齐 | P1 | M | 3d |
| P1-A-3 | dispatcher 测试补齐 | P1 | M | 2d |
| P1-A-4 | app_config 测试补齐 | P1 | S | 1d |
| P1-A-5 | CI 三平台恢复 | P1 | M | 3d |
| P1-A-6 | Docker HEALTHCHECK + non-root | P1 | S | 1d |
| P1-A-7 | Playwright E2E 接入 CI | P1 | M | 2d |
| P1-A-8 | ChatPanel 拆分 | P1 | M | 2d |
| P1-A-9 | i18n 类型安全 | P1 | M | 2d |
| P1-A-10 | 响应式布局 | P1 | M | 3d |
| P1-B-1 | CI 门禁强化 | P1 | S | 1d |

### 11.3 Phase 2 任务总表

| 任务编号 | 对应 T-E-* | 复杂度 | 预估工时 |
|---------|-----------|--------|---------|
| P2-A-1 | T-E-C-01 OS-Controller 双模式 | XL | 15d |
| P2-A-2 | T-E-C-05 Sidecar | L | 8d |
| P2-A-3 | T-E-C-06 Hybrid Browser | XL | 12d |
| P2-A-4 | T-E-C-15 语音交互 | XL | 10d |
| P2-A-5 | T-E-C-18 OAuth 集成 | XL | 12d |
| P2-A-6 | T-E-C-19 多端协同 | XL | 10d |
| P2-B-1 | T-E-S-10 WorkflowCanvas | XL | 12d |
| P2-B-2 | T-E-S-11 蜂群画布 | L | 8d |
| P2-B-3 | T-E-S-26 Event Stream | L | 5d |
| P2-B-4 | T-E-S-53 Cron 引擎 | L | 6d |
| P2-B-5 | T-E-S-58 Calendar | M | 4d |
| P2-B-6 | T-E-S-60 守护进程 | L | 5d |
| P2-B-7 | T-E-S-63 三定时 | L | 5d |
| P2-C-1 | T-E-D-04 8 人格 | XL | 10d |
| P2-C-2 | T-E-D-05 Proactive | L | 6d |
| P2-C-3 | T-E-D-08 WebGL 优化 | XL | 8d |
| P2-C-4 | T-E-D-09 UI 基准 | M | 3d |
| P2-D-1 | T-E-B-15 MOC 自动整理 | L | 5d |
| P2-E-1 | T-E-L-04 GitHub MCP | L | 5d |
| P2-E-2 | T-E-L-07 审计日志 | S | 2d |
| P2-E-3 | T-E-L-08a 运行时阶段环 | M | 4d |
| P2-E-4 | T-E-L-08b 设计节点 | XL | 8d |

---

## 12. 总工时估算

| Phase | 工时 | 日历时间 | 主要风险 |
|-------|------|---------|---------|
| **Phase 0** 地基修复 | 27d | 2-3 周 | digest 升级破坏 E2EE |
| **Phase 1** 质量闭环 | 22d | 4-6 周 | 单块拆分引入回归 |
| **Phase 2** 功能补齐 | ~150d | 6-8 周 | 工作量低估，需按 Wave 裁剪 |
| **Phase 3** 创新扩展 | 持续 | 6+ 月 | 竞品追赶 |

> **注意**：Phase 2 估算是极限值，实际执行需根据 Wave 优先级裁剪。建议优先完成 Wave 3-4 核心任务（OS-Controller / WorkflowCanvas / 人格系统），Wave 5 任务（OAuth / 多端协同）可延后。

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

> **Nebula 承诺**：你的知识，如星云般不断演化。
>
> 这份建议书不是一张完美的蓝图，而是一份务实的航海图——它承认现状（74% 完成度 + 20 项技术债务），尊重哲学（信任三原则），并给出可执行的修复路径。
>
> 每一行代码都应为**可读、可编辑、可追溯**而存在。

**文档结束。**

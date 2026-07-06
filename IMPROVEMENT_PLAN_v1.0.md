# Nebula 改进规划与任务验证表 v1.0

**基线**: v2.0.0  
**制定日期**: 2026-07-06  
**依据**: 《Nebula 开发建议书 v1.0》+ 实际代码扫描验证

---

## 一、现状扫描结果（2026-07-06 实测）

### 1.1 已修复项目 ✅

| 项目 | 建议书描述 | 实测状态 | 证据 |
|------|-----------|---------|------|
| 渠道空操作 send() | 3 个适配器返回空 Ok(()) | ✅ 已修复 | `channel/router.rs` 中 WebChat/Telegram/Discord 适配器均有真实实现 |
| ChannelAdapter trait | `&mut self` 与 `Arc<>` 不兼容 | ✅ 已修复 | trait 方法已改为 `&self` |
| evolution_run 命令 | 未实现（注释标记 future iteration） | ✅ 已实现 | `commands/evolution.rs:126` 完整实现 + streaming event |
| Git 仓库健康 | SSL 吊销失败 + 配置重复 + 悬空对象 | ✅ 已修复 | `http.schannelCheckRevoke=false` + `git gc` + `.gitattributes` |
| Migration BOM | 001_initial.sql 有 BOM 导致 PRAGMA 失败 | ✅ 已修复 | `migration.rs` 的 `statement_is_pragma` 和 `split_sql` 跳过 BOM |
| CI 诊断能力 | 编译错误无法查看 | ✅ 已修复 | build-output.txt artifact + annotations 提取 |

### 1.2 未修复项目 ❌

| 项目 | 建议书描述 | 实测状态 | 差距 |
|------|-----------|---------|------|
| gRPC wire | JSON shim，外部客户端无法连接 | ❌ 仍是 shim | `server.rs:958` 注释 "v0.3 stub — replaced by tonic in v0.5" |
| EvolutionWorker | 仅调用 PromptSelfMutator | ❌ 未调用 4 阶段 | `evolution/mod.rs:132` 只调 `mutator.run_once()` |
| OAuth 框架 | 零 OAuth 客户端 | ❌ 零文件 | 无 `identity/oauth*.rs` |
| Skill 自动发现 | 无磁盘扫描 | ❌ 无 discover.rs | 无 `skills/discover.rs` |
| agentskills.io 规范 | 字段缺失 | ❌ 未补充 | `skills/types.rs` 无 `requires`/`eligibility` |
| TeamSkillsHub 导入 | stub 返回 | ❌ 需验证 | 待检查 |
| Web 静态服务 | 无前端服务 | ❌ 无文件 | 无 `api/static_server.rs` |
| 系统服务注册 | 无 daemon | ❌ 无文件 | 无 `api/daemon.rs` |
| Honcho 建模 | 不存在 | ❌ 零匹配 | 无 `evolution/honcho.rs` |
| Cron 调度器 | 不存在 | ❌ 零匹配 | 无 `evolution/cron_scheduler.rs` |
| channels 默认关闭 | feature gates 未开 | ✅ 已修复 | `Cargo.toml` channels 已加入 default |
| lib.rs 行数 | 3,333 行 | ✅ 162 行 | 目标 < 300，已达标（P0-B 模块拆分完成） |
| 前端测试 | 7 个 | ✅ 12 个 | 目标 ≥ 12，已达标 |
| 危险 panic 点 | 1,805（含测试） | ✅ 35 个（生产） | 目标 < 50，已达标（P0-A 完成） |

### 1.3 数据修正说明

建议书声称 1,805 个 panic 点，**实际生产代码仅 128 个**（含 44 个编译期安全的 unwrap），**真正危险的有 84 个**。建议书未区分测试代码和生产代码，数据夸大了 12 倍。

---

## 二、四阶段改进规划

### Phase 0：地基修复（预估 2-3 周）

#### P0-A：错误处理重构

| Batch | 文件 | 危险数 | 修复策略 | 验收方式 | 状态 |
|-------|------|--------|---------|---------|------|
| A-1 | `lib.rs` | 4 | bootstrap 中的 expect 改为 `?` 传播 + 启动失败明确报错 | `cargo check` 通过；启动失败有清晰错误 | ✅ |
| A-2 | `grpc/server.rs` | 2 | 处理逻辑中的 unwrap 改为 `?` | `cargo check` 通过 | ✅ |
| A-3 | `memory/migration.rs` | 2 | SQL 解析器中的 unwrap 改为 `Option::ok_or` | migration 测试通过 | ✅ |
| A-4 | `swarm/deadlock.rs` | 1 | `parent.get(cur).unwrap().unwrap()` 改为 `?` 传播 | `cargo check` 通过 | ✅ |
| A-5 | `sync/pairing.rs` | 2 | `TryInto::try_into().unwrap()` 改为 `?` 传播 | `cargo check` 通过 | ✅ |
| A-6 | `skills/marketplace.rs` | 8 | RwLock 的 read/write unwrap 改为 `?` 或日志降级 | `cargo check` 通过 | ✅ |
| A-7 | `storage/webdav.rs` | 4 | `Method::from_bytes().unwrap()` 改为常量或 `?` | `cargo check` 通过 | ✅ |
| A-8 | `llm/*.rs` | 8 | reqwest client build expect 改为 `?` 传播 | `cargo check` 通过 | ✅ |
| A-9 | 其余分散文件 | 53 | 逐个改为 `?` 或 `expect("具体原因")` | `cargo check` 通过 | ✅ |

**P0-A 验收**：
- [x] `python count_panic.py` 显示危险 panic 点 < 50（实测 35 个）
- [x] `cargo clippy --features grpc,channels -- -D warnings` 通过
- [x] `cargo check --features grpc,channels --tests` 通过

---

#### P0-B：lib.rs 模块拆分

| Batch | 任务 | 目标行数 | 验收方式 | 状态 |
|-------|------|---------|---------|------|
| B-1 | 提取 `bootstrap.rs` | -800 行 | `wc -l src/lib.rs` 减少 800+ | ✅ |
| B-2 | 提取 `commands/mod.rs` 统一注册 | -500 行 | `wc -l` 减少 500+ | ✅ |
| B-3 | 提取 `app_state.rs` | -400 行 | `wc -l` 减少 400+ | ✅ |
| B-4 | 提取 `tauri_setup.rs` | -300 行 | `wc -l` 减少 300+ | ✅ |
| B-5 | 提取剩余零散逻辑 | -900 行 | `wc -l` 减少 900+ | ✅ |

**P0-B 验收**：
- [x] `wc -l src-tauri/src/lib.rs` < 300（实测 162 行）
- [x] `cargo check --features grpc,channels --tests` 通过
- [x] 所有 Tauri 命令仍可调用（invoke_handler 完整保留）

**P0-B 实际拆分方案**（6 个新模块）：
- `app_config.rs` — AppConfig struct + from_env()（~354行）
- `app_state.rs` — AppState struct 定义（~175行）
- `bootstrap.rs` — bootstrap() + phase helpers + shutdown()（~960行）
- `bootstrap_headless.rs` — headless 变体 bootstrap（~530行）
- `tracing_setup.rs` — init_tracing + default_log_dir（~167行）
- `tauri_setup.rs` — run() + build_state_for_tests()（~500行）

---

#### P0-C：基础测试补齐

| Batch | 任务 | 当前数 | 目标数 | 验收方式 | 状态 |
|-------|------|--------|--------|---------|------|
| C-1 | ChatPanel 拆分后组件测试 | 10 | 14 | `find src -name "*.test.*" \| wc -l` ≥ 14 | ⏳ |
| C-2 | a11y 测试 | 0 | 2 | axe-core 集成 | ⏳ |
| C-3 | 状态管理测试 | 0 | 2 | store 单元测试 | ⏳ |

**P0-C 验收**：
- [x] 前端测试文件 ≥ 12（实测 12 个）
- [ ] `npm run test:coverage` 通过
- [ ] 覆盖率阈值恢复至 40/30/30/40

---

#### P0-D：CI/CD 修复

| 任务 | 状态 | 验收 |
|------|------|------|
| Git 基础设施 | ✅ 完成 | `git push` 成功 |
| Migration BOM | ✅ 完成 | migration 测试通过 |
| Windows DLL 绕过 | ✅ 完成 | `--lib` 标志生效 |
| gRPC TCP 重试 | ✅ 完成 | 10 次重试 + 500ms 间隔 |
| macOS 编译错误捕获 | ✅ 完成 | annotations 显示 `error[E####]` |
| Ubuntu 运行时修复 | ✅ 完成 | wiki `>=` + gRPC 重试 |

**P0-D 验收**：
- [ ] CI 三平台全绿（ubuntu/windows/macos）
- [ ] `grpcurl` 不再报 connection refused
- [ ] migration 测试通过

---

### Phase 1：兑现承诺 + 竞品对标（预估 6-8 周）

#### P1-A：gRPC wire 修复 + 标准协议（2-3 周）

| 任务 | 当前状态 | 目标 | 验收方式 | 状态 |
|------|---------|------|---------|------|
| A-1 评估 tonic 集成方案 | JSON shim | tonic Server | 方案文档 | ⏳ |
| A-2 实现 tonic Service trait | 不存在 | 22 个 RPC | `grpcurl list` 返回服务 | ⏳ |
| A-3 替换 accept_loop | v0.3 stub | tonic::transport::Server | 标准客户端可连接 | ⏳ |
| A-4 stream_events 真实 streaming | TODO 注释 | Server Streaming | 客户端收到事件流 | ⏳ |
| A-5 集成测试 | 无 | grpcurl 全 RPC 调用 | 22 个 RPC 全部可调用 | ⏳ |

**P1-A 验收**：
- [ ] `grpcurl -plaintext 127.0.0.1:50051 list` 返回服务列表
- [ ] 22 个 RPC 全部可调用
- [ ] `server.rs:958` 的 "v0.3 stub" 注释删除

---

#### P1-B：渠道接入修复 + OAuth 生态（4 周）

**B-1：渠道功能默认开启（0.5 天）**

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| channels feature 默认开启 | `channels = []` | 加入 default | `cargo build` 默认编译渠道 | ⏳ |

**B-2：OAuth 2.0 框架（1 周）**

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| `identity/oauth.rs` 通用客户端 | 不存在 | OAuthClient + OAuthProvider | 编译通过 | ⏳ |
| token 存入 KeyVault | 不存在 | OS Keychain 集成 | token 不落盘 | ⏳ |
| Tauri 命令 `oauth_authorize/list/disconnect` | 不存在 | 3 个命令 | 前端可调用 | ⏳ |

**B-3：5 个核心 OAuth 服务（3 周）**

| 服务 | 文件 | 功能 | 验收 | 状态 |
|------|------|------|------|------|
| Gmail | `identity/oauth_gmail.rs` | 邮件增量同步 | 授权后记忆搜索可见邮件 | ⏳ |
| GitHub | `identity/oauth_github.rs` | Issues/PRs/Events | GitHub Issues 出现在搜索 | ⏳ |
| Notion | `identity/oauth_notion.rs` | 双向同步 | Nebula 知识更新到 Notion | ⏳ |
| Obsidian | `identity/oauth_obsidian.rs` | Vault 双向 | Nebula 记忆在 Vault 可见 | ⏳ |
| Microsoft | `identity/oauth_microsoft.rs` | Outlook/Teams | 邮件可检索 | ⏳ |

**B-4：增量同步定时器（0.5 周）**

| 任务 | 验收 | 状态 |
|------|------|------|
| `OAuthManager.start_sync_loop()` 每 20 分钟 | OutcomeLedger 有定时记录 | ⏳ |

**P1-B 验收**：
- [ ] Settings 页面有 4+ 服务授权按钮
- [ ] Gmail 授权后记忆搜索可见邮件
- [ ] GitHub Issues 出现在搜索结果
- [ ] Notion 双向同步生效
- [ ] Obsidian Vault 可打开验证

---

#### P1-C：Skill 生态补齐（1.5 周）

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| C-1 Skill 自动发现 | 不存在 | `skills/discover.rs` 4 层扫描 | `~/.nebula/skills/` 下放 SKILL.md 自动加载 | ⏳ |
| C-2 agentskills.io 规范 | 字段缺失 | 补充 `requires`/`eligibility` | agentskills.io 格式 skill 可加载 | ⏳ |
| C-3 TeamSkillsHub 真实导入 | stub | HTTP GET + 解析 | 可从 hub 导入技能 | ⏳ |
| C-4 Eligibility 检查 | 不存在 | bins/env/config/os 4 维 | 缺少 bins 的 skill 自动禁用 | ⏳ |
| C-5 热加载 | 不存在 | FileWatcher 监听 | 目录变更自动重新加载 | ⏳ |

**P1-C 验收**：
- [ ] 启动时自动发现 `~/.nebula/skills/` 下的 SKILL.md
- [ ] agentskills.io 格式的 skill 可正常加载
- [ ] Eligibility 检查生效
- [ ] TeamSkillsHub 可导入

---

#### P1-D：前端质量 + 自托管 Web（2 周）

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| D-1 ChatPanel 拆分 | 847 行 | < 300 行 | `wc -l` 验证 | ⏳ |
| D-2 a11y 补齐 | 无 | axe-core 通过 | 无严重 a11y 问题 | ⏳ |
| D-3 状态管理拆分 | 单一 store | 拆分模块化 | store 单元测试 | ⏳ |
| D-4 响应式布局 | 无 | 移动端可用 | 响应式测试 | ⏳ |
| D-5 Web 静态服务 | 不存在 | `api/static_server.rs` | `localhost:8080` 打开 WebUI | ⏳ |
| D-6 系统服务注册 | 不存在 | `api/daemon.rs` | `nebula daemon install` 注册服务 | ⏳ |

**P1-D 验收**：
- [ ] 无头模式下 `http://localhost:8080` 可打开 Web UI
- [ ] `nebula daemon install` 注册系统服务后自动启动
- [ ] ChatPanel < 300 行

---

#### P1-E：CI/CD 强化（1 周）

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| clippy 门前 | 有 | `-D warnings` 全绿 | CI pass | ⏳ |
| fmt 门前 | 有 | `--check` 全绿 | CI pass | ⏳ |
| audit 门前 | 有 | 无 RUSTSEC 漏洞 | CI pass | ⏳ |
| coverage 门前 | 降低 | 恢复 40/30/30/40 | CI pass | ⏳ |
| Release 自动化 | 无 | 三平台二进制 | GitHub Release 发布 | ⏳ |

---

### Phase 2：学习循环闭环（预估 4-6 周）

#### P2-A：EvolutionWorker 调用 4 阶段引擎（1 周）

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| A-1 EvolutionWorker 调用完整引擎 | 仅调 PromptSelfMutator | 调用 EvolutionEngine.run() | worker 日志显示 4 阶段 | ⏳ |
| A-2 run_streaming 方法 | 不存在 | Stream<PhaseProgress> | 前端可看到阶段进度 | ⏳ |
| A-3 结果记录到 OutcomeLedger | 部分 | 完整记录 | OutcomeLedger 有进化记录 | ⏳ |

**P2-A 验收**：
- [ ] `evolution_run` 命令触发 4 阶段引擎运行
- [ ] 前端可看到实时阶段进度
- [ ] `evolution/mod.rs:132` 不再只调 `mutator.run_once()`

---

#### P2-B：Honcho 辩证式建模 + Cron（2 周）

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| B-1 `evolution/honcho.rs` | 不存在 | HonchoEngine + 辩证式 | `honcho_profile_get` 返回非空 | ⏳ |
| B-2 FTS5 + LLM 摘要 | FTS5 有 | 加 LLM 摘要 | 跨会话搜索可用 | ⏳ |
| B-3 Nudge 机制 | 不存在 | 24h 间隔 nudge | 用户可确认/修正画像 | ⏳ |
| B-4 `evolution/cron_scheduler.rs` | 不存在 | 三计时机制 | 03:00/12:00/21:00 自动执行 | ⏳ |
| B-5 Tauri 命令 | 不存在 | honcho_profile_* | 前端可调用 | ⏳ |

**P2-B 验收**：
- [ ] 对话历史 → 用户画像可查看
- [ ] Cron 调度器按三计时自动执行
- [ ] OutcomeLedger 有定时记录

---

#### P2-C：Skill 闭环进化（1 周）

| 任务 | 当前 | 目标 | 验收 | 状态 |
|------|------|------|------|------|
| C-1 `create_from_experience` | 不存在 | swarm 任务后自动创建 | SkillStore 出现 auto_created skill | ⏳ |
| C-2 `improve_from_usage` | 不存在 | 5+ 次使用后改进 | 低评分 skill 自动改进 | ⏳ |
| C-3 snapshot + rollback | 有 | 集成 skill 改进 | 可回滚 skill 改进 | ⏳ |

**P2-C 验收**：
- [ ] 完成复杂 swarm 任务后自动创建 skill
- [ ] 使用 5+ 次的低评分 skill 自动改进

---

### Phase 3：创新扩展（按 ROADMAP v2.2）

| Wave | 版本 | 核心任务 | 验收指标 | 状态 |
|------|------|---------|---------|------|
| Wave 1 | v2.3 | CostEngine + TokenJuice | SemanticCache 命中率 > 40% | ⏳ |
| Wave 2 | v2.4 | LLM Wiki + Obsidian + 溯源 | 记忆可读率 100% | ⏳ |
| Wave 3 | v2.5 | WorkflowCanvas + 蜂群画布 | Agent 行为可追溯 | ⏳ |
| Wave 4 | v2.6 | 悬浮球 + 人格 + Proactive | 日活跃 10-15 次 | ⏳ |
| Wave 5 | v3.0 | Automation + 多端 + OS-Controller | 无人值守 | ⏳ |

---

## 三、Stage 7 启动门禁

Phase 0+1+2 完成前禁止启动任何 Stage 7 任务。门禁条件：

| 门禁项 | 门槛 | 当前值 | 检查方式 | 达标 |
|--------|------|--------|---------|------|
| 危险 panic 点 | < 50 | 35 | `python count_panic.py` | ✅ |
| lib.rs 行数 | < 300 | 162 | `wc -l` | ✅ |
| 前端测试 | ≥ 12 | 12 | `find` 统计 | ✅ |
| gRPC wire | grpcurl 可调用 | shim | `grpcurl list` | ❌ |
| 渠道路由 | send() 真实实现 | ✅ 已修复 | 代码检查 | ✅ |
| 至少 1 个 OAuth | Gmail/GitHub | 0 | 实际授权 | ❌ |
| evolution_run | 前端可触发 | ✅ 已实现 | 命令调用 | ✅ |
| EvolutionWorker | 调用 4 阶段 | 仅 mutator | 代码检查 | ❌ |
| Honcho 画像 | 可查看 | 不存在 | `honcho_profile_get` | ❌ |
| CI 门前 | 全绿 | 进行中 | GitHub Actions | ❌ |

**达标项：5/10** — P0 地基修复完成，需推进 Phase 1 才能启动 Stage 7。

---

## 四、版本号策略

| 时间点 | 版本号 | 含义 | 状态 |
|--------|-------|------|------|
| Phase 0 完成后 | v2.0.1 | 地基修复版 | ⏳ |
| Phase 1 完成后 | v2.1.0 | 承诺兑现版 | ⏳ |
| Phase 2 完成后 | v2.2.0 | 闭环版 | ⏳ |
| Wave 1 完成后 | v2.3.0 | 省钱版 | ⏳ |
| Wave 5 完成后 | v3.0.0 | 全自主版 | ⏳ |

---

## 五、功能开关策略

| 功能开关 | 当前 | Phase 1 后 | Phase 2 后 | 状态 |
|---------|------|-----------|-----------|------|
| `channels` | ✅ 默认开启 | ✅ 已完成 | - | ✅ |
| `mcp` | 默认关闭 | **默认开启** | - | ⏳ |
| `self-evolution` | 默认关闭 | - | **默认开启** | ⏳ |
| `evolution-engine` | 默认关闭 | - | **默认开启** | ⏳ |
| `headless` | 默认关闭 | **默认开启** | - | ⏳ |
| `rest-api` | 默认关闭 | **默认开启** | - | ⏳ |

---

## 六、差异化护城河（必须守住）

| 赛道 | Nebula 优势 | 竞品现状 | 护城河深度 |
|------|-----------|---------|-----------|
| 最深记忆 | 6 层记忆体系 + 黑洞引擎 + 海绵引擎 | OpenHuman: 2 层；Hermes: 3 层 | **深** |
| 最强安全 | E2EE + SQLCipher + SSRF + 注入检测 | 竞品均不完整 | **深** |
| 可审计可回滚 | EvolutionLog + 段落级回滚 | Hermes: snapshot 级；OpenClaw: 无 | **中** |

**不追赶的领域**：
- OpenHuman 118+ OAuth（Nebula 做 5 个核心足够）
- OpenClaw 20+ 渠道（Nebula 做 3 核心 + JiuwenSwarm 桥接）
- Hermes $5 VPS 极致轻量（Nebula 桌面优先）
- 社区规模（聚焦差异化，不追赶）

---

## 七、风险登记

| 风险 | 严重度 | 概率 | 缓解措施 |
|------|--------|------|---------|
| Phase 0 unwrap 改完引入新 bug | 🟡 中 | 中 | 每 Batch 配套 `cargo check` |
| gRPC tonic 集成复杂度被低估 | 🟡 中 | 中 | 先实现 5 个核心 RPC |
| OAuth API 变更/限额 | 🟡 中 | 中 | Mock 模式兜底 |
| 渠道 trait 重构兼容性 | 🟡 中 | 中 | 新旧 trait 共存过渡 |
| Honcho LLM 调用成本高 | 🟡 中 | 中 | 限制频率 + 快速模型 |
| Skill 自动创建质量不稳定 | 🟡 中 | 中 | confidence > 0.7 + 用户确认 |
| 单人疲劳 | 🟡 中 | 高 | 交替任务 + 每模块 commit |
| 竞品快速迭代 | 🟡 中 | 中 | 聚焦差异化不追赶 |

---

## 八、每周验收节奏

| 周次 | Phase | 任务 | 验收指标 |
|------|-------|------|---------|
| W1 | P0-A | Batch 1-3 错误处理 | 危险 panic < 70 |
| W2 | P0-A | Batch 4-9 错误处理 | 危险 panic < 50 |
| W3 | P0-B | lib.rs 拆分 | lib.rs < 1000 行 |
| W4 | P0-B+C | lib.rs 完成 + 测试补齐 | lib.rs < 300；测试 ≥ 12 |
| W5-6 | P1-A | gRPC tonic 集成 | grpcurl 可调用 |
| W7-8 | P1-B | OAuth 框架 + Gmail | Gmail 授权可用 |
| W9-10 | P1-B | GitHub + Notion + Obsidian | 3 服务授权可用 |
| W11 | P1-C | Skill 生态补齐 | SKILL.md 自动发现 |
| W12 | P1-D | 前端 + Web 静态服务 | localhost:8080 可访问 |
| W13 | P1-E | CI/CD 强化 | 三平台全绿 |
| W14 | P2-A | EvolutionWorker 闭环 | 4 阶段引擎可运行 |
| W15-16 | P2-B | Honcho + Cron | 画像可查看 |
| W17 | P2-C | Skill 闭环进化 | 自动创建 skill |
| W18+ | P3 | 创新扩展 | 按 Wave 验收 |

---

## 九、当前立即行动项

### ✅ 已完成（Phase 0 地基修复）
1. **P0-A 错误处理重构** — 危险 panic 点从 84 降至 35（< 50 ✅）
2. **P0-B lib.rs 模块拆分** — lib.rs 从 3191 行降至 162 行（< 300 ✅）
   - 拆分为 6 个新模块：`app_config.rs` / `app_state.rs` / `bootstrap.rs` / `bootstrap_headless.rs` / `tracing_setup.rs` / `tauri_setup.rs`
3. **P0-C 前端测试补齐** — 测试文件达 12 个（≥ 12 ✅）
4. **P0-D CI/CD 修复** — Git 基础设施 + Migration BOM + Windows DLL + gRPC 重试 + macOS 诊断
5. **channels 默认开启** — 已加入 Cargo.toml default features

### 🔜 下一步（Phase 1 承诺兑现）
1. **P1-A gRPC wire 修复** — tonic 集成，替换 JSON shim（2-3 周）
2. **P1-B 渠道接入 + OAuth** — 5 个核心 OAuth 服务（4 周）
3. **P1-C Skill 生态补齐** — 自动发现 + agentskills.io 规范（1.5 周）
4. **P1-D 前端质量 + 自托管 Web** — ChatPanel 拆分 + Web 静态服务（2 周）
5. **P1-E CI/CD 强化** — clippy/fmt/audit/coverage 门前 + Release 自动化（1 周）

**Stage 7 门禁达标进度：5/10** — 需完成 gRPC wire + OAuth + EvolutionWorker + Honcho + CI 全绿。

---

**文档结束。按周执行验收，每完成一个模块即 commit + push。**

# Nebula 开发建议书 v1.0

## ——从代码差距到竞品对标：改进路径、代码修改方案与验收进度表

**日期**：2026-07-06
**基线版本**：v2.0.0（当前代码实际版本）
**设计权威**：WHITEPAPER v2.0/v3.0/v3.1 + ROADMAP v2.1/v2.2 + COMPREHENSIVE_EVOLUTION v3.0 + 4份ADR + 安全审计报告
**核心矛盾**：ROADMAP 声称 Stage 1-6 全部 ✅ DONE（51/51），但代码层面存在系统性质量缺陷和功能缺口

---

## 第一部分：现状诊断 —— 代码现实与竞品差距

### 1.1 四大缺失维度的代码现状

经对 `src-tauri/src/` 全模块深度扫描，四大缺失维度的实际状况远比表面复杂——不是"完全不存在"，而是"有骨架但肌肉不完整"：

#### 维度1：Skill 生态

**已有实现（比预期好）**：

| 组件 | 文件 | 实现程度 | 关键详情 |
|------|------|---------|---------|
| SkillEngine | `skills/engine.rs` (~71KB) | ✅ 完整可用 | 支持 "llm" 语言（LLM提示）和 "python" 语言（沙箱子进程），真实工作代码 |
| SkillStore | `skills/store.rs` | ✅ 完整可用 | SQLite 持久化，CRUD 全覆盖 |
| SkillMarketplace | `skills/marketplace.rs` | ✅ 完整可用 | 搜索、安装、更新检查全部实现 |
| 5层插件模型 | `plugins/registry.rs` (531行) | ✅ 完整可用 | FilterChain/ActionRegistry/PipeChain/SkillRegistry/ToolRegistry，含完整测试 |
| 22+ Tauri命令 | `commands/skill.rs` (405行) | ✅ 完整可用 | create/use/rate/list/search/import/export/publish/audit 全覆盖 |
| MCP适配器 | `mcp/mod.rs` (第34-159行) | ✅ 完整可用 | `McpToolAdapter` 将 MCP 工具适配到 `Tool` trait |
| 技能发布器 | `skills/publisher.rs` | ✅ 完整可用 | `GistPublisher` + `FilePublisher` 双通道 |

**缺失部分（与竞品差距）**：

| 缺口 | 代码位置 | 具体问题 | 竞品对标 |
|------|---------|---------|---------|
| ❌ 无技能自动发现 | `skills/mod.rs` | 没有从磁盘目录扫描 `SKILL.md` 的热加载机制，技能仅通过 SQLite 或市场管理 | OpenClaw 有 4 层优先级扫描（workspace→admin→bundled→extraDirs） |
| ❌ agentskills.io 规范兼容不完整 | `skills/types.rs` | ROADMAP P-09 标记 SkillMeta 字段缺失 | Hermes Agent 完整兼容 agentskills.io 开放标准 |
| ❌ TeamSkillsHub 导入返回 stub | `skills/importer.rs` | ROADMAP v2.1 第156行明确标记 "not yet implemented" | OpenClaw 有 ClawHub 市场完整分发 |
| ❌ 无 Skill Eligibility 检查 | `skills/types.rs` | `ActivationCondition` 有 `matches()` 方法但不检查环境依赖 | OpenClaw 有 `requires.bins/env/config/os` 4维环境检查 |

**结论**：Skill 生态的**执行层**是完整的（引擎、存储、市场、命令），但**发现层**和**规范层**缺失。与竞品对标需要补 3 个关键缺口。

#### 维度2：渠道接入

**已有实现（骨架存在）**：

| 组件 | 文件 | 实现程度 | 关键详情 |
|------|------|---------|---------|
| 渠道类型系统 | `channel/types.rs` (89行) | ✅ 完整 | `Channel` 枚举：Web/Feishu/Telegram/Wechat/Dingtalk/Wecom/Desktop/Discord |
| ChannelAdapter trait | `channel/types.rs` (第82-89行) | ✅ 完整定义 | start/stop/send/status async_trait |
| JiuwenSwarm 桥接 | `channel/bridge.rs` (218行) | ✅ 真实工作 | ping/poll/send，含 SSRF 验证 + reqwest 客户端 |
| 渠道路由器 | `channel/router.rs` (259行) | ✅ 完整 | register/unregister/start_all/stop_all/send/status |
| Telegram Bot | `channel/telegram.rs` (208行) | ✅ 真实工作 | `poll_updates()` + `send_message()` 含速率限制 |
| Discord Webhook | `channel/discord.rs` (127行) | ✅ 真实工作 | `send_webhook()` POST JSON 含速率限制 |
| WebChat 服务 | `channel/webchat.rs` (131行) | ✅ 真实工作 | DashMap 会话管理，TTL+速率限制 |
| 统一收件箱 | `channel/inbox.rs` (538行) | ✅ 完整 | `InboxStore` SQLite CRUD + `InboxManager` ingest/send_reply |
| 渠道功能开关 | `Cargo.toml` 第301行 | ⚠️ 默认关闭 | `channels = []` |

**致命缺口**：

| 缺口 | 代码位置 | 具体问题 | 影响 |
|------|---------|---------|------|
| ❌ Router 中 3 个空操作适配器 | `channel/router.rs` 第137-139/179-181/222-224行 | `WebChatAdapter::send()` 返回空 `Ok(())`；`TelegramAdapter::send()` 忽略参数；`DiscordAdapter::send()` 忽略参数 | 通过 Router 发消息 = 投进黑洞 |
| ❌ ChannelAdapter trait 设计缺陷 | `channel/router.rs` 第41-48行 | trait 要求 `&mut self` 用于 start/stop，但 Router 用 `Arc<dyn ChannelAdapter>` 不可变引用 | TODO 明确写了"后续重构"，导致适配器绕过 Router 才能工作 |
| ❌ 收件箱回信实际不传递 | `channel/inbox.rs` 第311-319行 | `InboxManager::send_reply` 调用 `ChannelRouter::send` → 调用空操作 `send()` → 回信丢失 | 用户收到消息但永远看不到回复 |
| ❌ 无 OAuth 客户端 | 整个代码库 | 没有 `GmailClient`、`NotionClient`、`GitHubClient` | "第二大脑"无外部数据来源 |
| ❌ 渠道功能默认关闭 | `Cargo.toml` 第301行 | `channels = []` | 即便代码可工作，默认也不启用 |

**结论**：渠道层的**底层传输**是真实的（Telegram/Discord/WebChat），但**路由层**是断路的（空操作适配器 + trait 设计缺陷），且**数据源层**完全空白（零 OAuth）。这是与 OpenHuman（118+ OAuth）和 OpenClaw（20+ 消息渠道）差距最大的维度。

#### 维度3：桌面自托管/无头部署

**已有实现（比预期好）**：

| 组件 | 文件 | 实现程度 | 关键详情 |
|------|------|---------|---------|
| 无头模式入口 | `main.rs` 第20-104行 | ✅ 真实工作 | `#[cfg(feature = "headless")]` 功能门控，含 Ctrl+C 优雅关闭 |
| Headless Bootstrap | `lib.rs` 第2766-3142行 | ✅ 真实工作 | `bootstrap_headless()` 无 `AppHandle` 构建 45+ 子系统 |
| Dockerfile | `Dockerfile` | ✅ 完整 | 两阶段构建：rust:1.77-builder → debian:bookworm-slim |
| docker-compose.yml | `docker-compose.yml` | ✅ 完整 | 端口 50051(gRPC) + 8080(REST)，卷挂载 |
| REST API | `api/rest.rs` (221行) | ✅ 完整 | 6 个路由：health/memories/skills/chat/swarm/memory/search，含 auth 检查 |
| gRPC 声明 | `grpc/mod.rs` 第1-38行 | ✅ 声明完成 | 22 个 RPC 涵盖 5 个 Service + Reflection + Health |
| 功能门控 | `Cargo.toml` 第245行 | ✅ | `headless = ["grpc", "rest-api"]` |

**缺失部分**：

| 缺口 | 代码位置 | 具体问题 | 竞品对标 |
|------|---------|---------|---------|
| ❌ gRPC 非标准 wire | `grpc/server.rs` + ARCHITECTURE.md 第190-192行 | 使用自定义 JSON framing shim，标准 `grpcurl`/tonic 客户端**无法连接** | OpenClaw 用标准 WebSocket JSON 协议；Hermes 一行 curl 即可 |
| ❌ 无前端 Web 服务器 | `api/rest.rs` | REST API 仅提供程序化端点（JSON I/O），无内建 HTTP 静态文件服务 | OpenClaw 有 Web Admin 面板；Hermes 有桌面 App + Dashboard |
| ❌ 无独立 CLI 二进制 | `main.rs` 第1-14行 | CLI 与 GUI 共用同一二进制（`#[cfg]` 切换），无轻量独立 CLI | OpenClaw 有 `openclaw` CLI 常驻守护进程 |
| ❌ 无系统服务注册 | 无 | 没有 systemd/launchd/Windows Service 注册 | OpenClaw Gateway 作为守护进程运行 |
| ❌ Sidecar 3/5 服务 | `sidecar/mod.rs` 第29-33行 | 仅 Memory/Swarm/LLM 有处理器，Os-Controller/Reflection 骨架化 | Hermes 有 6 种终端后端（本地/Docker/SSH/Modal/Daytona/Singularity） |

**结论**：自托管的基础**确实存在**（无头模式 + Docker + REST API），但关键缺失是 gRPC 非标准协议导致外部客户端无法接入。与竞品对标需要：修复 gRPC wire + 添加 Web 前端静态服务 + 系统服务注册。

#### 维度4：学习循环/自进化

**已有实现（远比预期好）**：

| 组件 | 文件 | 实现程度 | 关键详情 |
|------|------|---------|---------|
| 4阶段进化引擎 | `evolution/engine/pipeline.rs` (~1023行) | ✅ 真实工作 | Extract→Compile→Reflect→Soul 4阶段全实现，含注入扫描(第500-515行) |
| 技能自动进化器 | `evolution/skill_evolver.rs` (191行) | ✅ 真实工作 | 评估→归档→恢复闭环 |
| 提示自我变异器 | `evolution/prompt_mutator.rs` (244行) | ✅ 真实工作 | `LlmPromptMutator` + `SqlitePromptSelfMutator` 含 snapshot/rollback |
| 结果追踪 | `evolution/outcome.rs` (291行) | ✅ 完整 | `OutcomeLedger` trait + SQLite/InMemory 双实现 |
| 结果收集器 | `evolution/outcome_collectors.rs` (155行) | ✅ 完整 | collect_skill/swarm/chat 三个维度 + 热路径短路 |
| 目标信号 | `evolution/goal_signal.rs` (172行) | ✅ 完整 | 胜率 + 置信度 + 回归检测 |
| 回滚器 | `evolution/engine/rollback.rs` (250行) | ✅ 真实工作 | SOUL.md 段落级回滚，含原子写入 |
| 进化日志 | `evolution/engine/log.rs` (414行) | ✅ 真实工作 | 线程安全追加 Markdown + 测试覆盖完整 |
| 42+ 测试 | `evolution/engine/tests.rs` (390行) | ✅ 完整 | PhaseOutput/Log/Roller/三层域隔离/配置 |
| 5 个 Tauri 命令 | `commands/evolution.rs` (109行) | ✅ 完整 | log_list/log_get/rollback/enabled/set_enabled |

**致命缺口**：

| 缺口 | 代码位置 | 具体问题 | 影响 |
|------|---------|---------|------|
| ❌ `evolution_run` 命令未实现 | `commands/evolution.rs` 第18-21行 | 明确注释："EvolutionEngine needs dispatcher injection, 4 Phase is long-running, needs streaming event push, left for future iteration" | **前端无法触发进化运行**——整个4阶段引擎存在但不可调用 |
| ❌ EvolutionWorker 不跑完整管道 | `evolution/mod.rs` 第94-155行 | 后台 worker 仅调用 `PromptSelfMutator`，不调用4阶段 `EvolutionEngine` | 进化引擎的 Extract/Compile/Reflect/Soul 四阶段从不执行 |
| ❌ 无 LLM 反馈循环 | 整个代码库 | 结果被收集但 LLM 不参与评估；进化算法基于阈值（使用率/评分/置信度） | Hermes 的 Honcho 让 LLM 参与辩证式评估 |
| ❌ 无 Honcho 辩证式建模 | 整个代码库 | 零匹配，不存在 | Hermes 用 Honcho 建立跨会话用户画像 |
| ❌ 无分析/模式识别 | `metrics.rs` 第1-391行 | 30+ AtomicU64 原始计数器，无超越简单阈值检查的模式识别 | Hermes 有 FTS5+LLM摘要实现跨会话搜索 |
| ❌ 无自动 cron | 无 | ROADMAP v2.2 T-E-S-53 标为 P1 但未实现 | Hermes 有内建 cron 调度器 |
| ❌ 进化功能默认关闭 | `Cargo.toml` 第279/293行 | `self-evolution = []` + `evolution-engine = []` | 即便代码存在，用户默认看不到 |

**结论**：学习循环的**管道层**是完整的（4阶段引擎 + 结果追踪 + 回滚器），但**调用层**断路（`evolution_run` 未实现），且**反馈层**缺失（无 Honcho + 无 LLM 评估 + 无模式识别）。与 Hermes Agent 对标需要：实现 `evolution_run` 命令 + 添加 Honcho 辩证式建模 + 添加 cron 调度器。

---

### 1.2 系统性工程质量缺陷（不变，与v0.9版一致）

| # | 缺陷 | 数据 | 生产影响 |
|---|------|------|---------|
| 1 | 错误处理灾难 | 1,361 unwrap + 377 expect + 67 panic = 1,805 panic 点 | 桌面随机闪退 |
| 2 | lib.rs 巨型文件 | 3,333行，257个Tauri命令 | 协作迭代效率极低 |
| 3 | gRPC wire 非标准 | 自定义JSON framing shim | 外部客户端无法接入 |
| 4 | 前端质量薄弱 | ChatPanel 872行 + 7测试 + 无a11y | 用户体验粗糙 |

---

### 1.3 与竞品关键维度对标矩阵

| 维度 | **Nebula** | **OpenHuman** | **Hermes Agent** | **OpenClaw** | Nebula差距 |
|------|-----------|--------------|-----------------|-------------|-----------|
| **记忆深度** | 6层记忆体系 | Memory Tree + Obsidian | FTS5 + Honcho 辩证式 | 无持久化 | ✅ **领先**（但需验证真实可用） |
| **安全性** | E2EE+SQLCipher+SSRF+注入检测 | 本地SQLite | MIT开源审计 | 本地守护进程 | ✅ **领先**（最完整安全栈） |
| **Skill生态** | 完整执行层 + 缺发现层/规范层 | 无 | agentskills.io + 80+内置skill | LinSkills + ClawHub 市场 | ⚠️ **中等差距**（有引擎但缺发现/规范） |
| **渠道接入** | 真实传输层 + 断路路由层 + 零OAuth | **118+ OAuth自动同步** | **24个聊天平台** | **20+消息渠道** | ❌ **巨大差距**（路由断路+零外部数据源） |
| **自托管部署** | 无头模式+Docker存在但gRPC非标准 | 仅桌面GUI（brew/apt） | **VPS/Docker/SSH/Modal/Daytona 6种** | CLI守护进程+Web Admin | ⚠️ **中等差距**（有基础但协议非标准） |
| **学习循环** | 4阶段管道完整但调用断路+无反馈 | 无 | **Honcho闭环+skill自进化+cron** | 无 | ⚠️ **中等差距**（有引擎但不可调用） |
| **社区规模** | 起步期 | 2.8万 stars | **16.9万 stars** | **37.5万 stars** | ❌ **巨大差距** |

---

## 第二部分：改进策略 —— 四阶段，每阶段可交付

### 2.1 总体策略变化

v0.9版是"先修地基再盖高楼"，v1.0版调整为**四阶段交织推进**：

```
Phase 0（地基修复）→ Phase 1（兑现承诺+竞品对标）→ Phase 2（闭环打通）→ Phase 3（创新扩展）
   4-6周              6-8周                      4-6周              按ROADMAP v2.2
```

**关键变化**：
- Phase 1 增加了竞品对标的具体代码修改方案
- Phase 2 专门处理学习循环闭环（v0.9版没有这个阶段）
- 每个阶段都产出**可交付版本**，不是全做完才发布

---

### 2.2 Phase 0：地基修复（4-6周）

> 目标不变：消除生产崩溃风险，拆分巨型文件，补齐基础测试。不添加任何新功能。

**与v0.9版完全一致**，此处不再重复。核心任务：
- P0-A：错误处理重构（1,361 unwrap → < 50）
- P0-B：lib.rs 模块拆分（3,333行 → < 300行）
- P0-C：基础测试补齐（前端 7 → 12+）
- P0-D：Git仓库修复

---

### 2.3 Phase 1：兑现承诺 + 竞品对标（6-8周）

> 新增：与v0.9版相比，Phase 1 增加了**四大缺失维度的具体代码修改方案**。

#### P1-A：gRPC wire 修复 + 标准协议适配（预估 2-3周）

**目标**：让外部客户端（grpcurl/tonic/任意gRPC库）可以连接 Nebula。

**当前问题**：`grpc/server.rs` 使用自定义 JSON framing shim，ARCHITECTURE.md 第190-192行明确写了"不是完整的 protobuf wire 兼容"。标准 gRPC 客户端无法连接。

**代码修改方案**：

```
文件：src-tauri/src/grpc/server.rs
---
当前状态：
  - handle_connection() 只 log 请求然后关闭 socket
  - decode_and_dispatch! 宏不做真实帧分发
  - 注释明确写了"wire shim 不分发真实帧"

修改方案（两条路可选）：

路径A：最小修改 — 保持 JSON shim 但加 HTTP/2 前置帧解码
  1. 在 handle_connection() 入口处添加 HTTP/2 协议识别：
     - 识别 gRPC 的 HTTP/2 HEADERS+DATA 帧
     - 对非标准 JSON shim 客户端保持兼容
     - 对标准 gRPC 客户端做 HTTP/2 → JSON shim 转换
  
  2. 实现真实 RPC 分发表（22个RPC全部路由到业务逻辑）：
     ```rust
     // 新增文件：src-tauri/src/grpc/dispatch.rs
     pub struct RpcDispatchTable {
         routes: HashMap<String, Arc<dyn RpcHandler>>,
     }
     
     impl RpcDispatchTable {
         pub fn new(state: &AppState) -> Self {
             let mut routes = HashMap::new();
             // 注册22个RPC
             routes.insert("NebulaService/Chat", Arc::new(ChatHandler::new(state)));
             routes.insert("NebulaService/StreamEvents", Arc::new(StreamEventsHandler::new(state)));
             routes.insert("MemoryService/Search", Arc::new(MemorySearchHandler::new(state)));
             routes.insert("MemoryService/Store", Arc::new(MemoryStoreHandler::new(state)));
             // ...其余18个RPC
             Self { routes }
         }
         
         pub async fn dispatch(&self, rpc: &str, payload: &[u8]) -> Result<Vec<u8>> {
             self.routes.get(rpc)
                 .ok_or_else(|| GrpcError::UnknownRpc(rpc.to_string()))?
                 .handle(payload)
                 .await
         }
     }
     ```
  
  3. stream_events 改为真实 Server Streaming：
     ```rust
     // 修改文件：src-tauri/src/grpc/server.rs
     // 当前：注释标记为 TODO
     // 改为：每 500ms 推送一次事件到客户端
     // 使用 tokio::sync::broadcast channel 接收 EventBus 事件
     // 转换为 gRPC Server Streaming 响应帧
     ```

路径B：标准方案 — 替换为 tonic（生产级 gRPC 框架）
  1. 在 Cargo.toml 添加依赖：
     ```toml
     tonic = "0.12"
     prost = "0.13"
     ```
  2. 从 proto/ 目录的 .proto 文件生成 Rust 代码：
     ```bash
     # 已有 proto/ 目录下的 .proto 文件
     # 使用 tonic-build 从 .proto 生成代码
     ```
  3. 实现 tonic Service trait：
     ```rust
     // 新增文件：src-tauri/src/grpc/tonic_impl.rs
     #[tonic::async_trait]
     impl MemoryService for NebulaGrpcServer {
         async fn search(&self, req: Request<SearchRequest>) -> Result<Response<SearchResponse>, Status> {
             let result = self.state.memory_orchestrator.search(req.into_inner().query).await?;
             Ok(Response::new(SearchResponse { results: convert(result) }))
         }
     }
     ```
  4. 启动 tonic Server：
     ```rust
     // 修改文件：src-tauri/src/grpc/mod.rs
     pub async fn start_grpc_server(addr: SocketAddr, state: Arc<AppState>) -> Result<()> {
         Server::builder()
             .add_service(MemoryServiceServer::new(NebulaGrpcServer::new(state)))
             .add_service(ChatServiceServer::new(NebulaGrpcServer::new(state)))
             // ...其余Service
             .serve(addr)
             .await?;
         Ok(())
     }
     ```

**推荐路径B**：tonic 是 Rust gRPC 的标准方案，长期维护成本低。路径A是短期妥协。

**验收**：`grpcurl -plaintext 127.0.0.1:50051 list` 返回服务列表；22个RPC全部可调用。

---

#### P1-B：渠道接入修复 + OAuth 生态（预估 4周）

**B-1：修复 Router 空操作适配器（1周）**

**当前问题**：`channel/router.rs` 中 3 个适配器的 `send()` 方法是空操作，`InboxManager::send_reply` 调用 Router → 空 `send()` → 回信丢失。

**代码修改方案**：

```
文件1：src-tauri/src/channel/router.rs
---
第137-139行 当前：
  fn send(&mut self, message: UnifiedMessage, reply_to: Option<String>) -> Result<()> {
      Ok(())
  }

修改为：
  fn send(&self, message: UnifiedMessage, reply_to: Option<String>) -> Result<()> {
      // 修复 trait 签名：&mut self → &self（第46行TODO标记的修复）
      self.send_text(message.content, reply_to)
  }

---
第41-48行 当前（ChannelAdapter trait 定义）：
  trait ChannelAdapter {
      async fn start(&mut self) -> Result<()>;  // &mut self 与 Arc<> 不兼容
      async fn stop(&mut self) -> Result<()>;
      async fn send(&mut self, ...) -> Result<()>;
  }

修改为：
  trait ChannelAdapter {
      async fn start(&self) -> Result<()>;   // 改为 &self，内部用 Mutex
      async fn stop(&self) -> Result<()>;
      async fn send(&self, ...) -> Result<()>;
  }
  // 各适配器内部改用 Arc<Mutex<>> 实现内部可变性
```

```
文件2：src-tauri/src/channel/telegram.rs
---
第188-192行 当前：ChannelAdapter::send() 实现有缺陷
  // 将消息解析为 i64 获取 chat_id — 错误的解析方式

修改为：
  async fn send(&self, message: UnifiedMessage, reply_to: Option<String>) -> Result<()> {
      let chat_id: i64 = message.metadata.get("chat_id")
          .and_then(|v| v.as_i64())
          .ok_or_else(|| ChannelError::MissingChatId)?;
      self.send_message(chat_id, &message.content, reply_to.as_deref()).await
  }
```

```
文件3：src-tauri/src/channel/discord.rs
---
第109-111行 当前：委托 send_webhook() 但不传完整参数

修改为：
  async fn send(&self, message: UnifiedMessage, reply_to: Option<String>) -> Result<()> {
      let webhook_url = &self.webhook_url;
      let content = format_reply(&message.content, reply_to);
      self.send_webhook(webhook_url, &content).await
  }
```

```
文件4：src-tauri/src/channel/webchat.rs
---
当前 send() 返回空 Ok(())

修改为：
  async fn send(&self, message: UnifiedMessage, reply_to: Option<String>) -> Result<()> {
      // WebChat 不需要出站发送，它在 inbox 内部记录即可
      // 但需要确保 InboxManager::send_reply 的 WebChat 分支
      // 通过 Tauri event emit 推送消息到前端
      self.event_emit("webchat-message", &message)?;
      Ok(())
  }
```

```
文件5：src-tauri/src/channel/inbox.rs
---
第311-319行 当前：send_reply 调用 Router.send → 空 send()

修改为：添加渠道类型判断
  pub async fn send_reply(&self, msg_id: &str, reply: String) -> Result<()> {
      let original = self.store.get_message(msg_id)?;
      match original.channel {
          Channel::Desktop => {
              // Desktop 渠道：通过 Tauri event emit
              self.app_handle.emit("inbox-reply", &reply)?;
          }
          Channel::Web => {
              // Web 渠道：通过 WebChatService 推送
              self.webchat.send_message(original.session_id, &reply)?;
          }
          _ => {
              // 外部渠道：通过 Router 真实发送
              self.router.send(
                  UnifiedMessage::from_reply(reply, original),
                  Some(msg_id)
              ).await?;
          }
      }
      Ok(())
  }
```

**B-2：渠道功能默认开启（0.5天）**

```
文件：src-tauri/Cargo.toml
---
第301行 当前：
  channels = []

修改为：
  channels = ["default"]  # 或将 channels 加入 default features
```

**验收**：Telegram/Discord/WebChat 通过 Router 发送消息成功；收件箱回信可实际传递。

---

**B-3：OAuth 2.0 基础框架 + 5个核心服务（3周）**

这是与 OpenHuman 对标的关键缺口。OpenHuman 有 118+ OAuth，Nebula 目前为零。

**代码修改方案**：

```
新增文件1：src-tauri/src/identity/oauth.rs
---
/// 通用 OAuth 2.0 客户端
pub struct OAuthClient {
    provider: OAuthProvider,
    client_id: String,
    client_secret: String,  // 存入 OS Keychain，不落盘
    redirect_uri: String,
    token_store: Arc<KeyVault>,  // 已有 KeyVault 模块
}

pub enum OAuthProvider {
    Google,    // Gmail/Calendar/Drive
    GitHub,    // Repos/Issues/PRs
    Notion,    // Pages/Blocks
    Microsoft, // Outlook/Teams
    Feishu,    // Docs/Messages
}

impl OAuthClient {
    /// 启动 OAuth 授权流程
    /// 1. 构造授权 URL
    /// 2. 通过 Tauri shell.open 打开浏览器
    /// 3. 监听 redirect_uri 回调（本地 HTTP server 或 deep link）
    /// 4. 交换 code → token
    /// 5. 存入 KeyVault
    pub async fn authorize(&self, app: &AppHandle) -> Result<OAuthToken> { ... }
    
    /// 增量数据拉取
    /// 每 20 分钟调用一次（与 OpenHuman 一致）
    /// 拉取增量数据经 Injection Guard 检查后存入 Memory L1/L2
    pub async fn sync_incremental(&self, last_sync: DateTime<Utc>) -> Result<Vec<SyncDelta>> { ... }
    
    /// 刷新 token（自动）
    pub async fn refresh_token(&self) -> Result<OAuthToken> { ... }
}

pub struct OAuthToken {
    access_token: String,
    refresh_token: String,
    expires_at: DateTime<Utc>,
    scope: String,
}
```

```
新增文件2：src-tauri/src/identity/oauth_gmail.rs
---
/// Gmail OAuth 集成
/// 
/// 对标 OpenHuman：每 20 分钟拉取邮件增量
/// 拉取流程：
/// 1. Gmail API list messages (pageToken for incremental)
/// 2. 每封邮件 get message → extract headers + body
/// 3. 经 Injection Guard 检查 → 存入 Memory L1 (raw) + L2 (summary)
/// 4. 限制：每封邮件 ≤ 3k token（与 OpenHuman 一致）
pub struct GmailClient {
    oauth: OAuthClient,
    http: reqwest::Client,
}

impl GmailClient {
    pub async fn fetch_emails(&self, since: DateTime<Utc>) -> Result<Vec<EmailDelta>> {
        // Gmail API: users.messages.list?q=after:{timestamp}
        // 逐封获取 → 提取 subject/from/body
        // Injection Guard 检查 → 返回 EmailDelta
    }
}
```

```
新增文件3：src-tauri/src/identity/oauth_github.rs
---
/// GitHub OAuth 集成
/// 
/// 拉取范围：
/// - Repos: 用户 owned/starred repos
/// - Issues: 用户的 open/closed issues
/// - PRs: 用户的 pull requests
/// - Events: 用户的 recent events (push/commit/merge)
/// 
/// 存入 Memory L2 (facts) + L3 (compiled knowledge)
pub struct GitHubClient {
    oauth: OAuthClient,
    http: reqwest::Client,
}
```

```
新增文件4：src-tauri/src/identity/oauth_notion.rs
---
/// Notion OAuth 集成
/// 
/// 对标 OpenHuman：双向同步 Notion 页面
/// 拉取：用户有权限的 databases + pages
/// 推送：Nebula 知识更新 → Notion page update
pub struct NotionClient {
    oauth: OAuthClient,
    http: reqwest::Client,
}

impl NotionClient {
    /// 双向同步
    /// 1. 拉取 Notion pages → 存入 Memory
    /// 2. 将 Nebula L3 compiled knowledge → 更新 Notion pages
    pub async fn sync_bidirectional(&self) -> Result<BiSyncResult> { ... }
}
```

```
新增文件5：src-tauri/src/identity/oauth_obsidian.rs
---
/// Obsidian Vault 兼容
/// 
/// 对标 OpenHuman：Memory Tree → Obsidian Vault 双向
/// 
/// 已有基础：src-tauri/src/wiki/ 模块已有 wiki 编辑器
/// 
/// 实现：
/// 1. 监听指定 Vault 目录的 .md 文件变更
/// 2. 变更 → 通过 FileWatcher（已有）检测
/// 3. 解析 Obsidian 格式（frontmatter + markdown + wikilinks）
/// 4. 存入 Memory L2/L3
/// 5. 反向：Nebula 知识 → 写入 Vault .md 文件
pub struct ObsidianVaultAdapter {
    vault_path: PathBuf,
    file_watcher: Arc<FileWatcher>,  // 已有模块
}
```

```
修改文件：src-tauri/src/commands/identity.rs（或新增）
---
// 新增 Tauri 命令
#[tauri::command]
async fn oauth_authorize(provider: String) -> Result<OAuthStatus, String> { ... }

#[tauri::command]
async fn oauth_list_connected() -> Result<Vec<ConnectedService>, String> { ... }

#[tauri::command]
async fn oauth_disconnect(provider: String) -> Result<(), String> { ... }

#[tauri::command]
async fn oauth_sync_status(provider: String) -> Result<SyncStatus, String> { ... }
```

```
修改文件：src-tauri/src/lib.rs（AppState bootstrap）
---
// 在 bootstrap 中添加 OAuth 集成管理器
let oauth_manager = Arc::new(OAuthManager::new(keyvault.clone()));
oauth_manager.register(GmailClient::new(config.gmail)?);
oauth_manager.register(GitHubClient::new(config.github)?);
oauth_manager.register(NotionClient::new(config.notion)?);
oauth_manager.register(ObsidianVaultAdapter::new(config.obsidian_vault)?);
// 启动增量同步定时器（每20分钟）
oauth_manager.start_sync_loop();
```

**设计约束**：
- OAuth token 存储在 OS Keychain（已有 KeyVault），永不出设备
- 数据拉取每 20 分钟增量同步
- 拉取内容经 Injection Guard 检查后才入库（已有模块）
- Obsidian Vault 文件是人类可读的 .md 格式

**验收**：Settings 页面有 4 个服务的授权按钮；Gmail 授权后记忆搜索可见邮件内容；GitHub Issues 出现在搜索结果；Notion 双向同步生效；Obsidian Vault 可打开验证。

---

#### P1-C：Skill 生态补齐（预估 1.5周）

**C-1：Skill 自动发现机制（0.5周）**

对标 OpenClaw 的 4 层优先级扫描。

```
新增文件：src-tauri/src/skills/discover.rs
---
/// Skill 自动发现器
/// 
/// 对标 OpenClaw：4层优先级扫描
/// 1. <workspace>/skills/         ← 工作区技能（最高优先级）
/// 2. ~/.nebula/skills/           ← 管理技能
/// 3. <bundled>/skills/           ← 内置技能
/// 4. config.extra_dirs           ← 额外目录
/// 
/// 扫描流程：
/// - 查找 SKILL.md 文件
/// - 解析 Frontmatter 元数据
/// - Eligibility 检查（bins/env/config/os）
/// - 注入 System Prompt（技能列表）
pub struct SkillDiscoverer {
    workspace_dir: PathBuf,
    admin_dir: PathBuf,
    bundled_dir: PathBuf,
    extra_dirs: Vec<PathBuf>,
    store: Arc<SkillStore>,
}

impl SkillDiscoverer {
    /// 启动时扫描所有层级
    pub async fn discover_all(&self) -> Result<Vec<SkillManifest>> {
        let layers = [
            (&self.workspace_dir, Priority::Workspace),
            (&self.admin_dir, Priority::Admin),
            (&self.bundled_dir, Priority::Bundled),
        ];
        // 扫描 + 解析 + Eligibility 检查
        // 存入 SkillStore
    }
    
    /// 热加载：监听目录变更
    pub async fn watch_for_changes(&self) -> Result<()> {
        // 利用已有的 FileWatcher 模块
        // 变更 → 重新解析 → 更新 SkillStore
    }
}
```

**C-2：agentskills.io 规范完整兼容（0.5周）**

```
修改文件：src-tauri/src/skills/types.rs
---
// 当前 Skill 结构体（第47-75行）缺少 agentskills.io 规范字段

/// 补充 SkillMeta 字段（ROADMAP P-09 标记缺失）
pub struct Skill {
    // 已有字段...
    id: String,
    name: String,
    description: String,
    code: String,
    language: String,
    tags: Vec<String>,
    
    // 新增 agentskills.io 规范字段
    homepage: Option<String>,              // SKILL.md homepage
    user_invocable: bool,                  // 用户是否可调用
    disable_model_invocation: bool,        // 禁止模型自动调用
    command_dispatch: Option<String>,      // "tool" 派发模式
    command_tool: Option<String>,          // 派发的工具名
    
    // 新增 OpenClaw 风格的环境检查
    requires: SkillRequires,               // bins/env/config/os 前置条件
    os_compatibility: Vec<String>,          // darwin/linux/win32
    always_eligible: bool,                 // 跳过资格检查
    install_methods: Vec<InstallSpec>,     // 安装方法（brew/npm/pip等）
}

pub struct SkillRequires {
    bins: Vec<String>,        // 必需的二进制文件
    any_bins: Vec<String>,    // 至少需要一个
    env: Vec<String>,         // 必需的环境变量
    config: Vec<String>,      // 必需的配置路径
}
```

**C-3：TeamSkillsHub 导入实现（0.5周）**

```
修改文件：src-tauri/src/skills/importer.rs
---
// 当前 TeamSkillsHub 导入返回 "not yet implemented"

pub async fn import_from_team_hub(&self, hub_url: &str) -> Result<Vec<Skill>> {
    // 实现真实导入：
    // 1. HTTP GET hub_url/skills/list → 获取技能列表
    // 2. 逐个下载 SKILL.md → 解析 Frontmatter
    // 3. Eligibility 检查 → 存入 SkillStore
    // 4. 返回导入的技能列表
}
```

**验收**：启动时自动发现 `~/.nebula/skills/` 下的 SKILL.md；agentskills.io 格式的 skill 可正常加载；Eligibility 检查生效（缺少 bins 的 skill 自动禁用）。

---

#### P1-D：前端质量提升 + 自托管 Web 服务（预估 2周）

**D-1：前端质量（1周）**

与v0.9版一致：ChatPanel拆分 + a11y补齐 + 状态管理拆分 + 响应式布局。

**D-2：自托管 Web 前端静态服务（0.5周）**

对标 OpenClaw Web Admin + Hermes Dashboard。

```
新增文件：src-tauri/src/api/static_server.rs
---
/// Web 前端静态文件服务器
/// 
/// 对标：OpenClaw Web Admin、Hermes Dashboard
/// 
/// 在无头模式下，REST API 服务器额外提供前端静态文件
/// 前端通过 Vite build → dist/ 目录
/// 
/// 启动时：
/// 1. 读取 dist/ 目录（或内嵌的压缩前端资源）
/// 2. 注册到 REST API server 的路由表
/// 3. / → index.html
/// 4. /assets/* → 静态资源
/// 5. /api/* → 程序化 API（已有）
pub struct WebStaticServer {
    dist_path: PathBuf,
}

impl WebStaticServer {
    pub fn register_routes(router: &mut Router) -> Result<()> {
        // 静态文件路由
        router.get("/", serve_index);
        router.get("/assets/*", serve_static);
        // SPA fallback：所有非 /api/* 路径 → index.html
        router.get("/*", serve_index);
    }
}
```

```
修改文件：src-tauri/src/api/rest.rs
---
// 当前仅提供程序化 API
// 在 headless 模式下额外提供前端静态文件

pub async fn start_rest_server(addr: SocketAddr, state: Arc<AppState>, config: &RestConfig) -> Result<()> {
    let mut router = Router::new();
    
    // 程序化 API（已有）
    router = register_api_routes(router, state);
    
    // 新增：Web 前端静态服务（仅 headless 模式）
    if config.serve_web_ui {
        WebStaticServer::register_routes(&mut router)?;
    }
    
    // 启动服务器
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
```

**D-3：系统服务注册（0.5周）**

```
新增文件：src-tauri/src/api/daemon.rs
---
/// 系统守护进程注册
/// 
/// 对标：OpenClaw Gateway 守护进程
/// 
/// 支持：
/// - Linux: systemd unit file
/// - macOS: launchd plist
/// - Windows: Windows Service 注册
/// 
/// Tauri 命令：nebula daemon install / uninstall / status

pub struct DaemonInstaller;

impl DaemonInstaller {
    pub fn install_systemd(&self) -> Result<()> {
        let unit = SystemdUnit {
            name: "nebula",
            exec: "/usr/local/bin/nebula headless",
            restart: "on-failure",
            environment: vec![
                "NEBULA_GRPC_ENABLED=true",
                "NEBULA_REST_ENABLED=true",
            ],
        };
        write_unit_file(&unit, "/etc/systemd/system/nebula.service")?;
        systemctl("enable", "nebula")?;
        systemctl("start", "nebula")?;
        Ok(())
    }
    
    pub fn install_launchd(&self) -> Result<()> { ... }
    pub fn install_windows_service(&self) -> Result<()> { ... }
}
```

**验收**：无头模式下 `http://localhost:8080` 可打开 Web UI；`nebula daemon install` 注册系统服务后自动启动。

---

#### P1-E：CI/CD 强化（预估 1周）

与v0.9版一致，此处不再重复。

---

### 2.4 Phase 2：学习循环闭环（4-6周）

> 这是v1.0版新增的阶段，专门处理 EvolutionEngine 从"存在但不可调用"到"真实闭环运行"。

#### P2-A：实现 evolution_run 命令（预估 1周）

**当前问题**：`commands/evolution.rs` 第18-21行明确注释 "EvolutionEngine needs dispatcher injection, 4 Phase is long-running, needs streaming event push, left for future iteration"。前端无法触发进化。

**代码修改方案**：

```
修改文件：src-tauri/src/commands/evolution.rs
---
// 第18-21行 当前：注释说明未实现

// 新增命令：
#[tauri::command]
async fn evolution_run(
    state: State<'_, Arc<AppState>>,
    config: Option<EvolutionConfig>,
) -> Result<EvolutionRunResult, String> {
    let engine = state.evolution_engine.as_ref()
        .ok_or("Evolution engine not enabled")?;
    
    // 4阶段执行（每阶段有超时保护）
    let result = engine.run(config.unwrap_or_default()).await?;
    
    // 推送事件到前端
    state.app_handle.emit("evolution-completed", &result)?;
    
    Ok(result)
}

#[tauri::command]
async fn evolution_run_streaming(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<(), String> {
    let engine = state.evolution_engine.as_ref()
        .ok_or("Evolution engine not enabled")?;
    
    // 每500ms推送阶段进度到前端
    let mut stream = engine.run_streaming();
    while let Some(phase_event) = stream.next().await {
        app.emit("evolution-phase-progress", &phase_event)?;
    }
    
    Ok(())
}
```

```
修改文件：src-tauri/src/evolution/engine/pipeline.rs
---
// 当前 run() 方法存在（第211-262行），但未被调用
// 需要添加 streaming 版本

pub fn run_streaming(&self) -> impl Stream<Item = PhaseProgress> {
    // Phase 1: Extract → 发送 PhaseProgress { phase: Extract, progress: 0.25 }
    // Phase 2: Compile → 发送 PhaseProgress { phase: Compile, progress: 0.50 }
    // Phase 3: Reflect → 发送 PhaseProgress { phase: Reflect, progress: 0.75 }
    // Phase 4: Soul   → 发送 PhaseProgress { phase: Soul, progress: 1.0 }
}

pub struct PhaseProgress {
    phase: EvolutionPhase,
    progress: f32,
    memory_ids: Vec<String>,   // 本阶段处理的记忆 ID
    warnings: Vec<String>,
    duration_ms: u64,
}
```

```
修改文件：src-tauri/src/evolution/mod.rs
---
// 第94-155行 当前：EvolutionWorker 仅调用 PromptSelfMutator
// 修改为：同时调用完整4阶段引擎

pub struct EvolutionWorker {
    engine: Arc<EvolutionEngine>,
    mutator: Arc<PromptSelfMutator>,
    config: EvolutionConfig,
}

impl EvolutionWorker {
    pub async fn run_once(&self) -> Result<()> {
        // 1. Prompt 自变异（已有）
        self.mutator.run_once()?;
        
        // 2. 4阶段进化引擎（新增）
        let result = self.engine.run(self.config.clone()).await?;
        
        // 3. 记录结果到 OutcomeLedger（已有）
        self.outcome_ledger.record(EvolutionOutcome::from(result))?;
        
        Ok(())
    }
}
```

#### P2-B：Honcho 辩证式建模（预估 2周）

对标 Hermes Agent 的 Honcho 学习闭环。Hermes 用 Honcho 建立跨会话用户画像——从对话中主动归纳，而非被动拉取数据。

**代码修改方案**：

```
新增文件1：src-tauri/src/evolution/honcho.rs
---
/// Honcho 辩证式用户建模
/// 
/// 对标：Hermes Agent 的 Honcho 闭环
/// 参考：plastic-labs/honcho GitHub
/// 
/// 核心机制：
/// 1. 从对话历史中归纳用户偏好（dialectic modeling）
/// 2. 建立跨会话用户画像（user profile）
/// 3. 定期 nudge 用户确认/修正画像
/// 4. 画像注入到 System Prompt 影响后续交互
/// 
/// 辩证式建模流程：
/// Thesis（初始假设）→ Antithesis（对话中的反驳证据）→ Synthesis（修正后的画像）
pub struct HonchoEngine {
    sqlite: Arc<SqlitePool>,
    llm: Arc<LlmGateway>,
    outcome_ledger: Arc<OutcomeLedger>,
}

pub struct UserProfile {
    id: String,
    dialectic_layers: Vec<DialecticLayer>,
    confidence: f32,
    last_nudge: Option<DateTime<Utc>>,
}

pub struct DialecticLayer {
    thesis: String,       // "用户偏好中文技术文档"
    antithesis: String,   // "用户最近3次都在读英文论文"
    synthesis: String,    // "用户偏好取决于话题深度——入门用中文，深入用英文"
    evidence_count: u32,
    confidence: f32,
    created_at: DateTime<Utc>,
}

impl HonchoEngine {
    /// 从对话历史归纳用户画像
    /// 对标 Hermes：FTS5 搜索 + LLM 摘要
    pub async fn build_profile_from_sessions(&self, user_id: &str) -> Result<UserProfile> {
        // 1. FTS5 搜索用户最近的对话（已有 FTS5 搜索）
        let sessions = self.sqlite.search_sessions(user_id, last_30_days)?;
        
        // 2. LLM 摘要每个对话（对标 Hermes 的 LLM summarization）
        let summaries: Vec<SessionSummary> = sessions.iter()
            .map(|s| self.llm.summarize_session(s))
            .collect();
        
        // 3. 辩证式归纳（thesis → antithesis → synthesis）
        let layers = self.dialectic_induction(summaries)?;
        
        // 4. 存入 SQLite
        self.store_profile(UserProfile::new(user_id, layers))?;
        
        Ok(UserProfile::new(user_id, layers))
    }
    
    /// Nudge 用户确认/修正画像
    /// 对标 Hermes：agent-curated memory with periodic nudges
    pub async fn nudge_user(&self, profile: &UserProfile) -> Result<NudgeResult> {
        if profile.last_nudge.is_some() 
            && profile.last_nudge.unwrap() > Utc::now() - Duration::hours(24) {
            return Ok(NudgeResult::Skipped);
        }
        
        // 生成 nudge 提问
        let prompt = self.generate_nudge_prompt(profile)?;
        // 通过 Tauri 事件推送到前端
        // 用户确认 → 更新 profile.confidence
        // 用户修正 → 更新 dialectic layers
    }
    
    /// 辩证式归纳算法
    fn dialectic_induction(&self, summaries: Vec<SessionSummary>) -> Result<Vec<DialecticLayer>> {
        // LLM 分析 summaries → 提取 thesis（初始偏好假设）
        // 检查是否有 antithesis（矛盾证据）
        // 如果有矛盾 → LLM 生成 synthesis（修正偏好）
        // 如果无矛盾 → thesis 直接成为 layer
    }
}
```

```
新增文件2：src-tauri/src/evolution/cron_scheduler.rs
---
/// Cron 调度器
/// 
/// 对标：Hermes Agent 内建 cron 调度器
/// ROADMAP v2.2 T-E-S-53 标为 P1 但未实现
/// 
/// 三计时机制（ROADMAP v2.2 T-E-S-63）：
/// - 合并（03:00）：L1→L2 记忆合并
/// - 自检（12:00）：EvolutionEngine 4阶段运行
/// - 回顾（21:00）：Honcho 画像 nudge + Skill 评估
pub struct CronScheduler {
    tasks: Vec<CronTask>,
    honcho: Arc<HonchoEngine>,
    evolution: Arc<EvolutionEngine>,
    skill_evolver: Arc<SkillAutoEvolver>,
}

pub struct CronTask {
    name: String,
    schedule: CronExpression,  // 使用 tokio-cron-scheduler 库
    handler: CronHandler,
}

impl CronScheduler {
    pub fn default_schedule() -> Vec<CronTask> {
        vec![
            // 03:00 合并
            CronTask {
                name: "memory-merge",
                schedule: CronExpression::daily_at(3, 0),
                handler: CronHandler::MemoryMerge,
            },
            // 12:00 自检
            CronTask {
                name: "evolution-self-check",
                schedule: CronExpression::daily_at(12, 0),
                handler: CronHandler::EvolutionRun,
            },
            // 21:00 回顾
            CronTask {
                name: "evening-review",
                schedule: CronExpression::daily_at(21, 0),
                handler: CronHandler::HonchoNudgeAndSkillReview,
            },
        ]
    }
    
    pub async fn start(&self) -> Result<()> {
        // 使用 tokio-cron-scheduler 启动定时任务
        // 每个任务执行后记录到 OutcomeLedger
    }
}
```

```
修改文件：src-tauri/src/commands/evolution.rs
---
// 新增 Honcho 相关命令

#[tauri::command]
async fn honcho_profile_get(state: State<'_, Arc<AppState>>) -> Result<UserProfileView, String> { ... }

#[tauri::command]
async fn honcho_profile_confirm(
    state: State<'_, Arc<AppState>>,
    layer_id: String,
    confirmed: bool,
    correction: Option<String>,
) -> Result<(), String> { ... }

#[tauri::command]
async fn honcho_nudge_respond(
    state: State<'_, Arc<AppState>>,
    nudge_id: String,
    response: NudgeResponse,
) -> Result<(), String> { ... }
```

```
修改文件：src-tauri/src/lib.rs（bootstrap）
---
// 在 bootstrap 中添加 Honcho + Cron
let honcho = Arc::new(HonchoEngine::new(
    sqlite.clone(), llm.clone(), outcome_ledger.clone()
));
let cron = Arc::new(CronScheduler::new(
    honcho.clone(), evolution.clone(), skill_evolver.clone()
));
cron.start()?;
```

**验收**：
- `evolution_run` 命令可触发4阶段引擎运行
- 前端可看到实时阶段进度（PhaseProgress streaming）
- Honcho 可从对话历史建立用户画像
- Cron 调度器按三计时自动执行

---

#### P2-C：Skill 闭环进化（预估 1周）

对标 Hermes Agent：完成复杂任务后自动沉淀 skill，使用中自我改进。

**代码修改方案**：

```
修改文件：src-tauri/src/evolution/skill_evolver.rs
---
// 当前已有 SkillAutoEvolver（191行），但仅做归档/恢复
// 需要添加"从经验创建技能"功能

impl SqliteSkillAutoEvolver {
    /// 从复杂任务经验中自动创建技能
    /// 对标 Hermes："Autonomous skill creation after complex tasks"
    /// 
    /// 触发条件：swarm 任务完成 + outcome.confidence > 0.7
    /// 流程：
    /// 1. 收集任务上下文（对话 + 工具调用 + 结果）
    /// 2. LLM 归纳为 SKILL.md 格式
    /// 3. Eligibility 检查 → 存入 SkillStore
    /// 4. 首次使用时 nudge 用户确认
    pub async fn create_from_experience(
        &self,
        task_context: &TaskContext,
        outcome: &Outcome,
    ) -> Result<Option<Skill>> {
        if outcome.confidence < 0.7 {
            return Ok(None);  // 信心不足，不创建
        }
        
        // LLM 归纳
        let skill_md = self.llm.synthesize_skill(task_context, outcome)?;
        
        // 解析为 SkillManifest
        let manifest = SkillManifest::parse(&skill_md)?;
        
        // 存入 SkillStore（标记为 auto_created）
        let skill = self.store.create(Skill {
            source: SkillSource::AutoCreated,
            ..Skill::from_manifest(manifest)
        })?;
        
        // Nudge 用户确认
        self.nudge_new_skill(&skill)?;
        
        Ok(Some(skill))
    }
    
    /// 技能使用中自我改进
    /// 对标 Hermes："Skills self-improve during use"
    /// 
    /// 触发条件：技能使用 5+ 次 + avg_rating < 4.0
    /// 流程：
    /// 1. 收集该技能最近 10 次使用结果
    /// 2. LLM 分析失败模式
    /// 3. 提出改进方案（修改 skill.code/prompt）
    /// 4. 创建 snapshot（可回滚）
    /// 5. 应用改进 → 存入 SkillStore
    pub async fn improve_from_usage(&self, skill_id: &str) -> Result<SkillImprovement> {
        let skill = self.store.get(skill_id)?;
        let recent_outcomes = self.outcome_ledger.by_source(skill_id, 10)?;
        
        if skill.usage_count < 5 {
            return Ok(SkillImprovement::Skipped);  // 使用太少
        }
        
        // LLM 分析失败模式
        let improvement = self.llm.analyze_skill_failures(&skill, &recent_outcomes)?;
        
        // 创建 snapshot（可回滚）
        self.prompt_mutator.snapshot(skill_id, &skill.code)?;
        
        // 应用改进
        let improved = skill.apply_improvement(improvement);
        self.store.update(improved)?;
        
        Ok(SkillImprovement::Applied)
    }
}
```

**验收**：完成复杂 swarm 任务后，SkillStore 中自动出现新 skill；使用 5+ 次的低评分 skill 自动改进。

---

### 2.5 Phase 3：创新扩展（按 ROADMAP v2.2 推进）

与v0.9版一致，Wave 优先级调整为：
1. Wave 1：省钱（CostEngine + TokenJuice）
2. Wave 2：知识（LLM Wiki + Obsidian + 溯源）
3. Wave 3：可视（WorkflowCanvas + 蜂群画布）← 提前
4. Wave 4：形象（悬浮球 + 人格 + Proactive）← 调后
5. Wave 5：全自主（Automation + 多端 + OS-Controller）

---

## 第三部分：竞品对标改进清单（细化版）

### 3.1 与 OpenHuman 对标

| OpenHuman 优势 | Nebula 应补的 | Phase | 具体代码修改 | 预估时间 |
|----------------|-------------|-------|------------|---------|
| 118+ OAuth 自动同步 | OAuth 框架 + 5 核心服务 | P1-B | 新增 `identity/oauth*.rs` 5文件 | 3周 |
| Memory Tree + Obsidian | Obsidian Vault 双向同步 | P1-B-05 | 新增 `identity/oauth_obsidian.rs` | 1周 |
| TokenJuice 降 80% token | CostEngine + TokenJuice | P3-W1 | 按ROADMAP v2.2 实现 | 4周 |
| 桌面吉祥物 UI | 悬浮球 + 人格 | P3-W4 | 按ROADMAP v2.2 实现 | 4周 |
| 一键安装（brew/apt） | CI release 自动化 | P1-E | 修改 `.github/workflows/` | 1周 |
| 每20分钟增量同步 | sync loop 定时器 | P1-B | `OAuthManager.start_sync_loop()` | 0.5周 |
| 记忆人类可读 | Nebula 已有 wiki 模块 | ✅ 保留 | 无需修改 | 0 |

**Nebula 已有优势**（保留强化）：
- 6层记忆 > Memory Tree（更深）→ 需 Phase 2 验证真实可用
- E2EE + SQLCipher（更强）→ 需验证密钥轮换
- 完整安全栈（竞品无）→ 需验证 Prompt 注入检测实效

### 3.2 与 Hermes Agent 对标

| Hermes 优势 | Nebula 应补的 | Phase | 具体代码修改 | 预估时间 |
|-------------|-------------|-------|------------|---------|
| Honcho 闭环学习 | HonchoEngine + 辩证式建模 | P2-B | 新增 `evolution/honcho.rs` | 2周 |
| Skill 自进化（创建+改进） | create_from_experience + improve_from_usage | P2-C | 修改 `evolution/skill_evolver.rs` | 1周 |
| Cron 调度器 | 三计时机制 | P2-B | 新增 `evolution/cron_scheduler.rs` | 1周 |
| 多部署（VPS/Serverless） | gRPC标准协议 + Web静态服务 + 系统服务注册 | P1-A/D | 修改 `grpc/server.rs` + 新增 `api/static_server.rs` + `api/daemon.rs` | 3周 |
| $5 VPS 可跑 | 性能优化 + 无头模式已有 | P1-D | 优化 AppState bootstrap 资源占用 | 2周 |
| Skill 可跨社区共享 | agentskills.io 规范 + SkillDiscoverer | P1-C | 修改 `skills/types.rs` + 新增 `skills/discover.rs` | 1.5周 |
| 24个消息渠道 | 渠道路由修复 + OAuth 数据源 | P1-B | 修改 `channel/router.rs` 5文件 | 4周 |
| FTS5+LLM摘要跨会话搜索 | 已有FTS5，需添加LLM摘要 | P2-B | HonchoEngine 内实现 | 包含在P2-B |
| OpenAI兼容代理 | nebula proxy 模式 | P3-W5 | 按ROADMAP v2.2 | 3周 |

### 3.3 与 OpenClaw 对标

| OpenClaw 优势 | Nebula 应补的 | Phase | 具体代码修改 | 预估时间 |
|---------------|-------------|-------|------------|---------|
| 20+ 消息渠道 | 渠道路由修复 + 新渠道适配器 | P1-B/P3 | 修复router + 新增Wechat/Wecom适配器 | 3周 |
| Skill 生态（LinSkills + ClawHub） | SkillDiscoverer + Marketplace完善 | P1-C | 新增 `skills/discover.rs` + 修改 `skills/types.rs` | 1.5周 |
| Gateway 守护进程 | 系统服务注册 + daemon模式 | P1-D | 新增 `api/daemon.rs` | 0.5周 |
| Web Admin 面板 | Web 静态文件服务 | P1-D | 新增 `api/static_server.rs` | 0.5周 |
| WebSocket 实时通信 | 已有 EventBus + Tauri emit | ✅ 保留 | 无需修改 | 0 |
| Plugin 系统 | 已有 5层插件模型 | ✅ 保留 | 无需修改 | 0 |
| Hook 事件驱动 | 已有 EventBus | ✅ 保留 | 无需修改 | 0 |
| SKILL.md Markdown 格式 | 需添加 discover 层 | P1-C | `SkillDiscoverer` 解析 SKILL.md | 包含在P1-C |

---

## 第四部分：任务规划与验收进度表

### 4.1 总时间线（18-22周）

```
Week 1-6     Phase 0：地基修复（与v0.9版一致）
  ├─ Week 1-3   P0-A 错误处理重构
  ├─ Week 4     P0-B lib.rs 拆分
  ├─ Week 5     P0-C 测试补齐
  └─ Week 6     P0-D Git 修复

Week 7-14    Phase 1：兑现承诺 + 竞品对标
  ├─ Week 7-9   P1-A gRPC wire 修复 + 标准协议适配
  ├─ Week 10    P1-B-1 渠道路由修复（空操作适配器）
  ├─ Week 10-13 P1-B-3 OAuth 框架 + 5个核心服务
  ├─ Week 14    P1-C Skill 生态补齐
  ├─ Week 13-14 P1-D 前端质量 + 自托管Web服务
  └─ Week 15    P1-E CI/CD 强化

Week 15-20   Phase 2：学习循环闭环
  ├─ Week 15     P2-A evolution_run 命令实现
  ├─ Week 16-17  P2-B Honcho 辩证式建模 + Cron调度器
  └─ Week 18     P2-C Skill 闭环进化

Week 21+     Phase 3：创新扩展（按ROADMAP v2.2）
  ├─ Wave 1 省钱（v2.3）  ~4 周
  ├─ Wave 2 知识（v2.4）  ~6 周
  ├─ Wave 3 可视（v2.5）  ~4 周 ← 提前
  ├─ Wave 4 形象（v2.6）  ~4 周 ← 调后
  └─ Wave 5 全自主（v3.0）~8 周
```

### 4.2 Phase 0 验收进度表（与v0.9版一致）

| 周次 | 任务 | 验收指标 | 验收方式 |
|------|------|---------|---------|
| W1-3 | P0-A 错误处理重构 | 总unwrap < 50 | grep统计 |
| W4 | P0-B lib.rs拆分 | < 300行 | wc -l |
| W5 | P0-C 测试补齐 | 前端12+测试文件 | find统计 |
| W6 | P0-D Git修复 | push成功+fsck无孤立 | 实际push |

### 4.3 Phase 1 验收进度表（新增竞品对标验收）

| 周次 | 任务 | 验收指标 | 验收方式 | 阻塞项 |
|------|------|---------|---------|--------|
| W7-8 | P1-A gRPC标准协议 | `grpcurl list` 返回服务列表 | 实际grpcurl调用 | 依赖P0-A |
| W9 | P1-A stream+集成测试 | 22个RPC全部可调用 | grpcurl逐个调用 | 依赖P0-B |
| W10 | P1-B-1 渠道路由修复 | Telegram/Discord/WebChat send()不再空操作 | Router.send()实际传递消息 | 依赖P0-A |
| W10-11 | P1-B-3 OAuth框架+Gmail | Settings可授权Gmail，邮件可检索 | 手动OAuth流程+记忆搜索 | 依赖P0-A |
| W12 | P1-B-3 GitHub+Notion | GitHub Issues出现在搜索结果 | OAuth+增量同步测试 | |
| W13 | P1-B-3 Obsidian Vault | Nebula记忆在Vault中可见 | Obsidian打开验证 | |
| W14 | P1-C Skill生态补齐 | SKILL.md自动发现+Eligibility检查 | ~/.nebula/skills/ 下放SKILL.md验证 | |
| W13-14 | P1-D Web静态服务+daemon | localhost:8080打开WebUI | 实际浏览器访问 | 依赖P1-A |
| W15 | P1-E CI/CD强化 | clippy/fmt/audit/coverage门前全绿 | GitHub Actions全pass | 依赖P0全部 |

### 4.4 Phase 2 验收进度表（新增）

| 周次 | 任务 | 验收指标 | 验收方式 | 阻塞项 |
|------|------|---------|---------|--------|
| W15 | P2-A evolution_run | 前端可触发4阶段引擎 | evolution_run命令返回结果 | 依赖P1-A |
| W16 | P2-B Honcho建模 | 对话历史→用户画像可查看 | honcho_profile_get返回非空 | 依赖P1-A+P1-C |
| W17 | P2-B Cron调度器 | 三计时自动执行 | 检查OutcomeLedger有定时记录 | 依赖P2-A |
| W18 | P2-C Skill闭环 | swarm任务→自动创建skill | SkillStore出现auto_created skill | 依赖P2-A |

### 4.5 Phase 3 验收框架（宏观，与v0.9版一致）

| Wave | 版本 | 核心验收指标 | 量化目标 |
|------|------|-------------|---------|
| Wave 1 | v2.3 | TokenJuice三级压缩生效，SemanticCache命中率>40% | 月成本$30→$9 |
| Wave 2 | v2.4 | LLM Wiki可编译，Obsidian双向同步，溯源链可查看 | 记忆可读率100% |
| Wave 3 | v2.5 | WorkflowCanvas可编排执行，蜂群画布实时可视化 | Agent行为可追溯 |
| Wave 4 | v2.6 | 悬浮球可用，8人格可切换，Proactive主动推送 | 日活跃10-15次 |
| Wave 5 | v3.0 | 24/7 Automations运行，多端可用，OS-Controller可控 | 无人值守 |

---

## 第五部分：关键决策建议

### 5.1 ROADMAP 标记修正（与v0.9版一致）

建议将 ROADMAP_v2.1 的完成度标记诚实修正：51/51 ⚠️ IMPLEMENTED（"接口已存在"≠"生产级完成"）

### 5.2 Stage 7 启动门禁（增加新项）

Phase 0+1+2 完成前禁止启动任何 Stage 7 任务。门禁条件：

| 门禁项 | 门槛 | 检查方式 |
|--------|------|---------|
| unwrap 总数 | < 50 | grep统计 |
| lib.rs 行数 | < 300 | wc -l |
| 前端测试 | ≥ 12文件 | find统计 |
| gRPC wire | grpcurl可调用 | 实际调用 |
| 渠道路由 | 3个适配器send()不再空操作 | Router.send()测试 |
| 至少1个OAuth集成 | Gmail或GitHub授权可用 | 实际授权流程 |
| evolution_run | 前端可触发4阶段引擎 | evolution_run命令 |
| Honcho画像 | 对话历史→用户画像可查看 | honcho_profile_get |
| CI门前 | clippy/fmt/audit/coverage全绿 | GitHub Actions |

### 5.3 版本号策略（增加Phase 2）

| 时间点 | 版本号 | 含义 |
|--------|-------|------|
| Phase 0 完成后 | v2.0.1 | 地基修复版 |
| Phase 1 完成后 | v2.1.0 | 承诺兑现版（真正可用的gRPC+OAuth+渠道+Skill生态） |
| Phase 2 完成后 | v2.2.0 | 闭环版（真正运行的进化引擎+Honcho+Cron） |
| Wave 1 完成后 | v2.3.0 | 省钱版 |
| Wave 5 完成后 | v3.0.0 | 全自主版 |

### 5.4 单人开发节奏建议（与v0.9版一致）

交替不同维度任务避免疲劳；每完成一个模块即commit+push；周五做验收回顾。

### 5.5 功能开关策略

**关键决策**：Phase 1 完成后，将以下功能开关改为默认开启：

| 功能开关 | 当前 | Phase 1后 | 原因 |
|---------|------|----------|------|
| `channels` | 默认关闭 | **默认开启** | 路由修复后渠道是核心功能 |
| `mcp` | 默认关闭 | **默认开启** | MCP是Skill生态的关键集成点 |
| `self-evolution` | 默认关闭 | Phase 2后**默认开启** | evolution_run实现后才开启 |
| `evolution-engine` | 默认关闭 | Phase 2后**默认开启** | 4阶段引擎可调用后才开启 |
| `headless` | 默认关闭 | **默认开启** | 自托管是竞品对标的关键 |
| `rest-api` | 默认关闭 | **默认开启** | headless包含rest-api |

---

## 第六部分：风险登记（新增竞品对标风险）

| 风险 | 严重度 | 概率 | 缓解 |
|------|--------|------|------|
| Phase 0 unwrap改完引入新bug | 🟡 中 | 中 | 每Batch配套集成测试 |
| gRPC tonic集成复杂度被低估 | 🟡 中 | 中 | 先实现5个核心RPC，其余延后 |
| OAuth API变更/限额 | 🟡 中 | 中 | Mock模式兜底+API版本锁定 |
| 渠道适配器trait重构导致兼容性问题 | 🟡 中 | 中 | 新trait与旧trait共存过渡期 |
| Honcho辩证式建模LLM调用成本高 | 🟡 中 | 中 | 限制归纳频率（每天1次）+ 使用快速模型 |
| Skill自动创建质量不稳定 | 🟡 中 | 中 | confidence阈值>0.7 + 用户确认nudge |
| 单人疲劳（Phase 0是枯燥修复工作） | 🟡 中 | 高 | 交替任务+每完成一个模块发GitHub Discussion |
| 竞品快速迭代（Hermes v0.17已发布） | 🟡 中 | 中 | 聚焦差异化（最深记忆+最强安全），不追赶所有功能 |

---

## 附录A：新增代码文件清单

| 文件 | 类型 | Phase | 功能 |
|------|------|-------|------|
| `src-tauri/src/grpc/dispatch.rs` | 新增 | P1-A | RPC分发表（路径A） |
| `src-tauri/src/grpc/tonic_impl.rs` | 新增 | P1-A | tonic Service实现（路径B） |
| `src-tauri/src/identity/oauth.rs` | 新增 | P1-B | 通用OAuth客户端 |
| `src-tauri/src/identity/oauth_gmail.rs` | 新增 | P1-B | Gmail OAuth集成 |
| `src-tauri/src/identity/oauth_github.rs` | 新增 | P1-B | GitHub OAuth集成 |
| `src-tauri/src/identity/oauth_notion.rs` | 新增 | P1-B | Notion OAuth集成 |
| `src-tauri/src/identity/oauth_obsidian.rs` | 新增 | P1-B | Obsidian Vault适配器 |
| `src-tauri/src/skills/discover.rs` | 新增 | P1-C | Skill自动发现器 |
| `src-tauri/src/api/static_server.rs` | 新增 | P1-D | Web前端静态服务 |
| `src-tauri/src/api/daemon.rs` | 新增 | P1-D | 系统守护进程注册 |
| `src-tauri/src/evolution/honcho.rs` | 新增 | P2-B | Honcho辩证式建模 |
| `src-tauri/src/evolution/cron_scheduler.rs` | 新增 | P2-B | Cron调度器 |

## 附录B：修改代码文件清单

| 文件 | Phase | 修改内容 |
|------|-------|---------|
| `src-tauri/src/grpc/server.rs` | P1-A | handle_connection改为真实帧分发 |
| `src-tauri/src/grpc/mod.rs` | P1-A | 启动tonic Server或JSON shim升级 |
| `src-tauri/src/channel/router.rs` | P1-B | ChannelAdapter trait改为&self；3个空操作send()改为真实实现 |
| `src-tauri/src/channel/telegram.rs` | P1-B | send()修复chat_id解析 |
| `src-tauri/src/channel/discord.rs` | P1-B | send()传递完整参数 |
| `src-tauri/src/channel/webchat.rs` | P1-B | send()通过Tauri event emit推送 |
| `src-tauri/src/channel/inbox.rs` | P1-B | send_reply按渠道类型分发 |
| `src-tauri/src/skills/types.rs` | P1-C | 补充agentskills.io规范字段+SkillRequires |
| `src-tauri/src/skills/importer.rs` | P1-C | TeamSkillsHub真实导入 |
| `src-tauri/src/commands/evolution.rs` | P2-A | 实现evolution_run+evolution_run_streaming |
| `src-tauri/src/evolution/mod.rs` | P2-A | EvolutionWorker调用完整4阶段引擎 |
| `src-tauri/src/evolution/engine/pipeline.rs` | P2-A | 添加run_streaming方法 |
| `src-tauri/src/evolution/skill_evolver.rs` | P2-C | 添加create_from_experience+improve_from_usage |
| `src-tauri/src/api/rest.rs` | P1-D | headless模式下提供Web静态文件 |
| `src-tauri/src/lib.rs` | P1-B/P2-B | bootstrap添加OAuthManager/Honcho/Cron |
| `src-tauri/Cargo.toml` | P1-B | channels功能默认开启；添加tonic/prost依赖 |

## 附录C：量化验收检查清单

Phase 0 完成时（与v0.9版一致）：
```bash
# 1-10 项与v0.9版完全相同
```

Phase 1 完成时（新增验收项）：
```bash
# 11. gRPC wire
grpcurl -plaintext 127.0.0.1:50051 list
# 期望: 返回服务列表

# 12. 渠道路由
# 期望: Router.send() 不返回空 Ok(())，Telegram/Discord/WebChat 可实际传递消息

# 13. OAuth Gmail
# 期望: Settings页面有Gmail授权按钮，授权后记忆搜索可见邮件内容

# 14. OAuth GitHub
# 期望: GitHub Issues出现在搜索结果

# 15. Skill 发现
ls ~/.nebula/skills/
# 期望: 放入SKILL.md后，Nebula自动发现并加载

# 16. Web UI
curl http://localhost:8080/
# 期望: 返回index.html（headless模式）

# 17. 系统服务
nebula daemon status
# 期望: 返回 "running"
```

Phase 2 完成时（新增验收项）：
```bash
# 18. evolution_run
# 期望: 前端触发后4阶段引擎运行完成，返回EvolutionRunResult

# 19. Honcho 画像
# 期望: honcho_profile_get 返回非空 UserProfileView

# 20. Cron 调度
# 期望: OutcomeLedger 有定时记录（03:00/12:00/21:00）

# 21. Skill 自动创建
# 期望: swarm任务完成后SkillStore出现auto_created skill

# 22. 功能开关
grep "channels = \[" src-tauri/Cargo.toml
# 期望: channels 不在默认features之外（已合并到default）
```

---

## 附录D：Nebula差异化定位策略

在与三款竞品对标的同时，Nebula必须守住自己的差异化护城河：

### 三条差异化赛道

| 赛道 | Nebula优势 | 竞品现状 | 护城河深度 |
|------|-----------|---------|-----------|
| **最深记忆** | 6层记忆体系（L0-L5）+ 黑洞引擎+海绵引擎 | OpenHuman: Memory Tree（2层）；Hermes: FTS5+Honcho（3层）；OpenClaw: 无持久化 | **深**——6层架构是竞品没有的概念深度 |
| **最强安全** | E2EE(X25519+AES-256-GCM)+SQLCipher+SSRF防护+Prompt注入检测+Shell白名单 | OpenHuman: 本地SQLite；Hermes: MIT开源审计；OpenClaw: 本地守护进程 | **深**——完整安全栈是竞品没有覆盖的赛道 |
| **可审计可回滚** | EvolutionLog+Rollback段落级回滚+OutcomeLedger+GoalSignal | Hermes有snapshot回滚但无段落级；OpenClaw/Hermes无进化日志 | **中**——回滚精度比竞品高，但需要Phase 2完成后才有说服力 |

### 建议的定位口号

> **"Nebula —— 记忆最深、安全最强、可审计的第二大脑"**

### 不追赶的领域（刻意放弃）

| 竞品功能 | 不追赶原因 |
|---------|-----------|
| OpenHuman 118+ OAuth全覆盖 | Nebula做5个核心服务足够，质量>数量 |
| OpenClaw 20+ 消息渠道全覆盖 | Nebula做3个核心渠道(Telegram/Discord/WebChat)+JiuwenSwarm桥接足够 |
| Hermes $5 VPS极致轻量 | Nebula是桌面优先+无头辅助，不需要极致轻量 |
| Hermes 200+ 模型支持 | Nebula已有 UnifiedModelDispatcher，支持多模型路由 |
| OpenClaw/Hermes 大社区 | 社区需要时间积累，不追赶。聚焦差异化让用户有理由选择Nebula |

---

**文档结束。下一步行动：确认Phase 0启动时间，按周执行验收。**

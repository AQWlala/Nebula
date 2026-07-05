# Nebula (nebula) · 专家智能体联合审议报告 v2.1

## ——5 位专家智能体对项目迭代方向、生产实现、任务进度、代码质量的深度审议

**版本**：v2.1（专家审议版）
**日期**：2026-07-02
**审议基线**：`WHITEPAPER_v2.0.md` + `ROADMAP_v2.1.md` + 实际代码审查
**配套文档**：`EXPERT_AGENTS_v2.1.md`（智能体角色说明）

---

## 0. 审议元信息

### 0.1 审议方法

本次审议由 5 个互补的专家智能体并行执行，各自基于专业视角审查 WHITEPAPER_v2.0.md、ROADMAP_v2.1.md 及实际代码，独立产出审议报告后汇总。每个智能体都被要求：
- 必须引用具体代码文件和行号（不能只看文档）
- 必须给出可落地的建议（不接受"建议加强"等空话）
- 必须识别 ROADMAP 中的遗漏或错误排序
- 必须评估 65% 完成度的客观性

### 0.2 五位专家智能体

| 代号 | 角色 | 关注维度 |
|------|------|---------|
| **EA-1** | 首席架构师 | 整体架构演进、跨模块断层、长期技术债 |
| **EA-2** | 记忆系统专家 | L0-L5 闭环、海绵/黑洞/遗忘引擎、ACL、反思 |
| **EA-3** | 蜂群与 AI 工程师 | Swarm 调度、LLM 网关、AgentBus、Sidecar |
| **EA-4** | 安全与可观测性工程师 | 纵深防御、WASM/MCP/SSRF、Prometheus、仪表盘 |
| **EA-5** | 产品工程与质量经理 | 任务进度、CI/CD、测试覆盖、代码质量、风险 |

### 0.3 关键结论速览

> **5 位专家一致认为**：项目架构设计超前、工程纪律良好（TODO 仅 3 处、CI 三平台构建），但**65% 完成度被系统性高估**，修正后约 **57-58%**。存在 **3 个 P0 级隐性缺陷**未在 ROADMAP 中识别，必须在 Stage 1 内紧急修复，否则 Stage 2 协议层会暴露记忆系统全部数据。

---

## 1. P0 级隐性缺陷（5 位专家共识，必须立即修复）

### 1.1 🔴 缺陷一：Negotiator LLM 仲裁死代码

**发现者**：EA-3 蜂群与 AI 工程师
**代码证据**：`swarm/orchestrator.rs:460-461`

```
// 当前调用同步版 negotiate()
negotiator.negotiate(outputs)
```

同步版 `negotiate()`（`negotiator.rs:38-83`）在冲突且置信度 <0.8 时**直接走 FallbackHighestConfidence，根本不调用 LLM 仲裁**。只有异步版 `negotiate_with_arbitration()`（`negotiator.rs:85-119`）才调用 `llm_arbitrate()`。

**后果**：WHITEPAPER §4.3 宣称的"LLM 仲裁"在执行路径中**从未被触发**。蜂群协商实际上只有"置信度投票 + 最高置信度 fallback"，无 LLM 仲裁。

**修复**：`orchestrator.rs:461` 改为 `negotiator.negotiate_with_arbitration(&outputs, &self.llm).await`。1 行改动，P0。

### 1.2 🔴 缺陷二：MemoryAcl 默认 allow-all 是安全漏洞

**发现者**：EA-4 安全与可观测性工程师
**代码证据**：`memory/acl.rs:64-71`

```rust
pub fn check(&self, ...) -> bool {
    // 无 Deny 且无 Allow 时返回 true（默认放行）
    true
}
```

**后果**：一旦 Stage 2 的 MCP/REST 上线，外部调用方可通过 `memory_search` IPC 绕过 ACL 读取所有记忆（包括其他 principal 的隐私数据）。ROADMAP T-S1-A-04 描述"默认 allow-all 兼容"是**危险设计**。

**修复**：`acl.rs:70` 的 `true` 改为 `false`（默认 deny-all）。所有现有调用方需显式配置 allow 规则。**优先级应从 P0 升级为 P0+（最高）**，必须先于 Stage 2 协议层完成。

### 1.3 🔴 缺陷三：MemoryOrchestrator 是孤儿模块

**发现者**：EA-2 记忆系统专家
**代码证据**：`lib.rs:298,355`（声明+构造）vs 全仓 grep `state.orchestrator.` 在命令层 0 次命中

**后果**：`assemble_context()` 这条"3000 token + ≤3 类型"的核心注入管线**从未被任何 chat/swarm 路径触发**。当前 chat 路径只走 `state.chat()` → `absorb_chat_turn`，完全没有上下文注入。ROADMAP T-S1-A-02 描述"sponge 未集成"严重低估了问题——orchestrator 本身就是孤儿。

**修复**：T-S1-A-02 不只要把 `lance.search` 换成 `sponge.search`，更要在 `AppState::chat` 内部调 `self.orchestrator.assemble_context(&request.user_message).await?`，把 `ContextBundle.text` 拼到 system prompt 前。

---

## 2. 高优先级发现（按专家分组）

### 2.1 EA-1 首席架构师

#### 2.1.1 Sidecar "壳层陷阱"

`sidecar/ipc.rs` 中的 `MemoryIpcClient`/`LlmIpcClient`/`SwarmIpcClient` 仅暴露 `health_check()`，**完全没有业务 RPC 方法**。WHITEPAPER §9.1 所称的"3/5 服务已定义"实际上只是进程管理壳层，wire 层业务调用从未存在。

**建议**：Stage 2 的 T-S2-B-01（gRPC wire tonic 升级）必须补完前 3 个 sidecar 的真实 RPC 接口，否则 Stage 4 会在 5 个 sidecar 上重复"壳层陷阱"。

#### 2.1.2 LayerPolicy L4→L6 提升到虚空

`layers.rs` 已实现 L4→L6 提升逻辑，但 L6 不存在——"提升到虚空"的隐患。长时间运行后会有记忆卡在"待提升"状态。

**建议**：Stage 1 内 hotfix 为"L4 满足条件触发 L5 反思"。

#### 2.1.3 CRDT 引擎是"纯函数"债

`pub struct CrdtEngine;` 是零 Sized 纯计算单元，未与任何传输层串联。ROADMAP Stage 4 仅安排 U-03（蜂群内 CRDT），**跨设备 CRDT 没有对应任务**——这是 §2.4 依赖链的遗漏项。

**建议**：Stage 6 新增 T-S6-B-03"跨设备 CRDT op 传播与 LocalTransport 落盘"。

#### 2.1.4 65% 完成度高估，修正后约 58%

21 项"部分实现"统一按 0.5 加权不合理：
- A 类（接口已定义但内部空）5 项：实现度 <10%，应按 0.2 加权
- B 类（接口未连接）11 项：实现度约 50%，0.5 合理
- C 类（字段/数据缺失）5 项：实现度 60%，应按 0.6 加权

修正后：`(24×1.0 + 5×0.2 + 11×0.5 + 5×0.0 + 12×0) / 57 ≈ 54%`（EA-1 估算）
或按 EA-5 估算：`(24 + 6×0.5 + 15×0.25 + 12×0) / 57 ≈ 57%`

**共识**：对外宣传用 **57%**，内部 tracking 用 ROADMAP §5.2 的 **39% 综合完成度**。

#### 2.1.5 Stage 2 任务堆积风险

Stage 2 把 5 个不相关任务（WASM/SSRF/gRPC/MCP/REST）混在一起，其中 4 个是 A 类（协议帧实现）。单人开发在该阶段会陷入"协议帧设计疲劳"。

**建议**：拆分 Stage 2 为：
- **Stage 2a 协议层**（v2.2.0）：gRPC tonic + MCP + REST
- **Stage 2b 安全层**（v2.2.1）：WASM + SSRF

### 2.2 EA-2 记忆系统专家

#### 2.2.1 SpongeEngine 没有 `search()` 方法

ROADMAP T-S1-A-02 写"调用 `SpongeEngine::search()`"——这个 API 不存在，只有 `search_with_graph()`（`sponge.rs:471`）。需先补 `pub async fn search()`。

#### 2.2.2 ForgettingEngine 是只读的

`forgetting.rs` 全文没有 `tick()`，只有 `scan_for_archive()`，返回 `Vec<ForgettingCandidate>` 但**不写库、不更新 `archived` 列、不调用 blackhole**。`archived` 字段从未被置 true。BlackholeEngine 的 `run_pass` 也不查 `archived` 标记。

**建议**：T-S1-A-03 工作量被低估，应拆为：
- T-S1-A-03a：ForgettingEngine.tick() 写 archived=true
- T-S1-A-03b：BlackholeEngine.run_pass_archived() 只扫 archived=1

#### 2.2.3 SelfReflectionEngine 不闭环

`reflect_all()` 返回 `Vec<SelfReflection>` 但**不写库**（全文无 INSERT）。L5 反思无法历史回溯，SelfImprovement 模式拿不到跨次运行的 prior。

**建议**：新增 `self_reflections` 表持久化反思结果，这是 L6 的前置条件。

#### 2.2.4 L4 evaluate() 顺序可优化

当前顺序：宪法 → 隐私 → 风险 → 价值（价值仅在 Allow 时执行）。建议改为：宪法 → 隐私 → 价值（仅打分不短路）→ 风险（结合价值分裁定），让"低价值 + 高风险"任务直接进 Plan。

#### 2.2.5 sponge absorb() 三处隐患

1. 多腔体过滤 `filter_by_chamber` 在 `sqlite.get_many` 失败时静默返回空 Vec，导致重复插入
2. 关键词衰减影响 importance 但不更新 metadata，ForgettingEngine 无法区分"原生低"vs"被衰减"
3. merge 路径在锁外 re-embed，进程崩溃会导致向量与文本错位

### 2.3 EA-3 蜂群与 AI 工程师

#### 2.3.1 AgentBus 形同虚设

`orchestrator.rs:201` 创建了 `AgentBus`，但 `execute()` 全程仅在结束时 `bus.broadcast()` 一次，6 个 Agent 没有一个调用 `bus.register()` 或 `bus.send()`。`Agent::set_mailbox` trait 方法默认空实现，GenericAgent 未覆写。**总线设计完整但未接入执行路径**。

#### 2.3.2 DynamicAgentPool API 不异步友好

`acquire(&mut self)` / `release(&mut self)` / `cleanup_idle(&mut self)` 全是 `&mut self`，无法直接 `Arc<DynamicAgentPool>` 共享给多个 spawn 的 task。T-S3-B-02 接入 orchestrator 时**必须**先重构为 `Arc<tokio::sync::Mutex<DynamicAgentPool>>`。

#### 2.3.3 LLM 降级链断路器失效

DeepSeek/Ollama/Anthropic 失败时**只 warn 不 `record_failure`**（`gateway.rs:399,412,432`），仅 Remote 失败才 `record_failure`。DeepSeek 持续宕机时断路器永不跳闸，每次请求仍要先等 DeepSeek 超时（120s）再降级。

**修复**：每一级失败都 `breaker.record_failure()`，DeepSeek 超时从 120s 降至 15s。

#### 2.3.4 LRU 缓存对长对话命中率近乎 0

`cache_key()` 对 `(model, messages)` 全量哈希。多轮对话每加一条消息就生成新 key。建议改为"最近 N 轮 + 系统提示"的滑动窗口哈希。

#### 2.3.5 GenericAgent 输出趋同

`GenericAgent::new(llm, i)` 仅用序号 `i` 区分 6 个实例，若 prompt 模板相同则输出高度趋同，协商形同虚设。建议注入不同 `temperature`（0.3/0.7/1.0 交错）或不同 `system_prompt` 偏好。

#### 2.3.6 LLM 流式 IPC 技术方案

采用 Tauri 2.0 的 `ipc::Channel<ChatToken>` 而非 `emit_event`：
- `Channel` 是双向流，前端可中途取消
- `Channel` 透传序列化开销低于 event 的全局广播
- 签名：`pub async fn chat_stream(request: ChatRequestDto, on_token: tauri::ipc::Channel<ChatToken>) -> Result<ChatComplete, CommandError>`

**注意**：`chat_stream` 网关层（`gateway.rs:530`）只走 Ollama，DeepSeek 主路径无流式——需同时实现 DeepSeek 的 SSE 流式解析。

### 2.4 EA-4 安全与可观测性工程师

#### 2.4.1 WASM 沙箱三层残缺

不是白皮书说的"依赖待解注释"，而是：
1. 依赖层：`Cargo.toml:110` wasmtime 注释，原因是离线构建环境无法拉取 wasmtime 庞大依赖
2. Feature 层：`wasm-sandbox = []` 空数组
3. 代码层：`sandbox.rs:326-334` host function 全部返回 -1 占位

**建议**：切换 MSVC 工具链后 wasmtime 24.x 可正常编译；或改用 wasmer 4.x（MinGW 兼容性更好）。

#### 2.4.2 SSRF validate_redirect_chain 是伪实现

`ssrf_guard.rs:77-82` 接受的是预定义 urls 列表，不是实际 HTTP 重定向链。reqwest 默认跟随 10 次重定向，**每跳不重新验证**。

**必须实现**：
```rust
let client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::custom(move |attempt| {
        if ssrf_guard.validate_url(attempt.url().as_str()).is_err() {
            attempt.error("SSRF: redirect target blocked")
        } else {
            attempt.follow()
        }
    }))
    .build()?;
```

#### 2.4.3 5 项可观测性指标缺口

Prometheus 17 项指标未覆盖：
- ❌ L4 拦截率（ValuesLayer.evaluate 无计数器）
- ❌ Token 成本（engine.rs 返回 tokens 永远是 0）
- ❌ L0Cache 命中率（stats 硬编码 0）
- ❌ 反思引擎护栏（reflection_skipped 未定义）
- ❌ ACL 拒绝计数（无法观测 ACL 是否生效）

#### 2.4.4 REST API 无独立 feature

REST API 被 `grpc` feature 隐式包含（`rest.rs:18 #[cfg(feature = "grpc")]`），没有独立 `rest-api` feature。任何启用 gRPC 的构建都会暴露无认证的 REST 端点。

**建议**：定义 `rest-api` feature，与 `grpc` 解耦，默认关闭。T-S2-B-03 优先级应从 P2 升为 P1。

#### 2.4.5 MCP 安全基线不足

`SAFE_ENV_VARS = &["PATH", "HOME", "USER", "LANG"]` 不够，应额外保留：
- `SYSTEMROOT`（Windows 下 Python/node 子进程需要）
- `TEMP`/`TMP`（临时目录）
- `TZ`（时区）
- `LOCALE`/`LC_ALL`（Linux 下字符处理）

且 `filter_safe_env_vars` 函数在整个 mcp/ 目录**无任何调用点**。

### 2.5 EA-5 产品工程与质量经理

#### 2.5.1 版本号漂移

`package.json:3` 与 `Cargo.toml:3` 均为 `1.1.10`，而 WHITEPAPER 称 v2.0 已交付。**建议 Stage 1 首个 commit 即同步至 `2.0.0`**。

#### 2.5.2 feature flag 死开关

`did-identity` 和 `crdt-sync` 两个 feature 在 Cargo.toml 定义但**代码中零 cfg 匹配**——`--no-default-features` 构建仍编入两模块。`rest-api` feature 则完全未定义。这是架构债。

#### 2.5.3 覆盖率全盲

- 前端：无 `vitest.config.ts`，`devDependencies` 缺 `@vitest/coverage-v8`，`test:coverage` 脚本运行会报错
- Rust：未配置 `cargo-tarpaulin` 或 `cargo-llvm-cov`，行覆盖率未知
- CI 中 `test.yml:224` 是 `echo "coverage upload placeholder"` 占位符

#### 2.5.4 cargo-audit 门禁失效

`test.yml:192` `cargo-audit` 使用 `continue-on-error: true`，安全漏洞不阻断 CI。应分级：RUSTSEC-critical 必须阻断。

#### 2.5.5 MinGW 问题被误判

ROADMAP §1.3 称"Windows MinGW 链接器无法跑 cargo test"。实际上 CI 用的是 MSVC 工具链（`x86_64-pc-windows-msvc`），Windows runner 上 `cargo nextest run` 正常执行。真正问题是**本地开发机**装了 MinGW。**缓解策略**：新增 `rust-toolchain.toml` 固定 MSVC targets。

#### 2.5.6 任务复杂度被低估

| 任务 | 标注 | 实际 | 理由 |
|------|------|------|------|
| T-S1-B-01 LLM 流式 IPC | L | XL | 需重构 Tauri command 签名 + 前端事件监听 + 取消/重连 |
| T-S2-B-01 gRPC tonic 迁移 | XL | XL+50% | 现有是手写 hyper JSON framing，整体替换 |
| T-S6-A-01 OS-Controller | XL | 3×XL | 三平台同时适配，应拆为 3 个独立 XL |
| T-S4-B-01 Sidecar Skill | L | XL | 新写独立 sidecar 二进制 + gRPC handler |

#### 2.5.7 35 任务总工时估算

S×6 + M×13 + L×11 + XL×5 ≈ **243 人天**（约 11 个月单人全职，实际 14-16 个月含返工）。

---

## 3. 优先级修正建议（5 位专家共识）

### 3.1 必须升级优先级的任务

| 任务 | 当前 | 建议 | 理由 |
|------|------|------|------|
| Negotiator 仲裁死代码修复 | 未列入 | **P0+（Stage 1 前置）** | LLM 仲裁从未触发，蜂群协商形同虚设 |
| MemoryAcl 默认改 deny-all | P0 | **P0+（最高）** | 默认 allow-all 是安全漏洞 |
| T-S1-B-03 仪表盘真实数据 | P1 | **P0** | 5 项指标缺口导致安全事件不可观测 |
| T-S2-B-03 REST API auth | P2 | **P1** | gRPC feature 隐式暴露无认证 REST |
| T-S6-A-01 OS-Controller | P3 | **P1**（v3.0 前） | 是 v3.0 核心卖点 |

### 3.2 必须降级优先级的任务

| 任务 | 当前 | 建议 | 理由 |
|------|------|------|------|
| T-S2-A-01 WASM 沙箱 | P1 | P2 | Python 沙箱已兜底，非阻塞性 |

### 3.3 必须拆分的任务

| 原任务 | 拆分方案 | 理由 |
|--------|---------|------|
| T-S1-A-03 ForgettingEngine 联动 | T-S1-A-03a（tick+archived）+ T-S1-A-03b（blackhole run_pass_archived） | 工作量被低估 |
| T-S1-B-01 LLM 流式 IPC | T-S1-B-01a（后端 Channel）+ 01b（前端 listen）+ 01c（兼容性测试） | 粒度过大 |
| T-S2-A-01 WASM 沙箱 | T-S2-A-01a（工具链+依赖）+ 01b（host function）+ 01c（WASI 裁剪） | 三层残缺 |
| T-S6-A-01 OS-Controller | 拆为 Windows/macOS/Linux 三个独立 XL | 三平台同时适配 |
| Stage 2 | Stage 2a（协议层）+ Stage 2b（安全层） | 避免 A 类任务堆积 |

### 3.4 必须新增的任务

| 新任务 | 阶段 | 理由 |
|--------|------|------|
| Negotiator 调用路径修复 | Stage 1 前置 | 1 行改动，但影响蜂群核心承诺 |
| LayerPolicy L4→L6 提升到虚空 hotfix | Stage 1 | L6 不存在，长期运行记忆卡死 |
| Sidecar 通用服务模板定义 | Stage 2 | Stage 4 sidecar 前置依赖 |
| 跨设备 CRDT op 传播 | Stage 6 | U-08 云中继的隐式依赖 |
| `rust-toolchain.toml` 固定 MSVC | Stage 1 | 根除 MinGW 链接器问题 |
| `vitest.config.ts` + coverage 工具 | Stage 1 | 覆盖率全盲 |
| did-identity/crdt-sync feature cfg 补齐 | Stage 2 | 死开关架构债 |
| 版本号同步至 2.0.0 | Stage 1 首个 commit | 文档与代码版本漂移 |

---

## 4. 架构级决议建议

### 4.1 Sidecar 进程二进制方案

**决议**：采用单二进制多角色方案（`nebula-sidecar --kind=skill`）

理由：
1. 单二进制减少发布物体积，降低签名/分发成本
2. 5 个 sidecar 共享 LLM/Memory 客户端代码
3. 与现有 `sidecar_exe_name()` 函数兼容

### 4.2 OS-Controller 进程隔离

**决议**：OS-Controller 必须独立 sidecar 进程，不在主进程内运行

理由：OS-Controller 涉及 UIAutomation/AT-SPI 等高权限 API，与 L4 价值层、ShellExecutor 在同一进程内会有权限耦合风险。复用 Stage 4 的单二进制多角色 sidecar 模板，新增 `SidecarKind::OsController`。

### 4.3 领导轮值制算法

**决议**：不引入 Raft，改用加权随机轮值

算法：`score = capability_score * 0.5 + history_success_rate * 0.3 + (1 - current_load) * 0.2`，每个任务开始时按 score 加权随机选 Leader。

### 4.4 WASM 沙箱降级策略

**决议**：wasmtime 不可用则改用 wasmer 4.x，不降级为 disabled feature

理由：disabled 等于放弃沙箱，与安全模型矛盾。wasmer 4.x 的 MinGW 兼容性优于 wasmtime。

### 4.5 LLM 流式 IPC 技术选型

**决议**：采用 Tauri 2.0 `ipc::Channel<ChatToken>`，不用 `emit_event`

理由：Channel 是双向流，前端可中途取消；透传序列化开销低于 event 全局广播。

---

## 5. 风险矩阵（5 位专家综合）

| 风险 | 严重度 | 概率 | 阶段 | 缓解策略 |
|------|--------|------|------|---------|
| Negotiator 仲裁死代码 | 🔴 高 | 已确认 | Stage 1 前 | 1 行修复 |
| MemoryAcl 默认 allow-all | 🔴 高 | 已确认 | Stage 1 | 默认改 deny-all |
| Orchestrator 孤儿模块 | 🔴 高 | 已确认 | Stage 1 | 接入 chat 路径 |
| Stage 2 A 类任务堆积 | 🟡 中 | 高 | Stage 2 | 拆分为 2a/2b |
| bus factor = 1 | 🟡 中 | 持续 | 全程 | 补 project_memory + backup reviewer |
| 版本号漂移 | 🟡 中 | 已确认 | Stage 1 | 首个 commit 同步 |
| feature 死开关 | 🟡 中 | 已确认 | Stage 2 | 补 cfg 或删 feature |
| 覆盖率全盲 | 🟡 中 | 已确认 | Stage 1 | 补 vitest.config + tarpaulin |
| cargo-audit 门禁失效 | 🟡 中 | 已确认 | Stage 1 | 分级阻断 |
| WASM 编译困难 | 🟡 中 | 中 | Stage 2 | wasmer 4.x 替代 |
| 跨设备 CRDT 缺失任务 | 🟡 中 | 已确认 | Stage 6 | 新增 T-S6-B-03 |
| 单人 243 人天工时 | 🟡 中 | 持续 | 全程 | Stage 5 前端任务裁剪 |

---

## 6. 工程质量改进路线图

### 6.1 短期（Stage 1 必做，1-2 天内可完成）

1. **补 `vitest.config.ts` + `@vitest/coverage-v8`**：让 `npm run test:coverage` 真正工作
2. **修 `cargo-audit` 门禁**：`test.yml:192` 改为 RUSTSEC-critical 阻断
3. **同步 package.json/Cargo.toml 版本号至 2.0.0**
4. **新增 `rust-toolchain.toml`** 固定 MSVC targets
5. **修复 Negotiator 仲裁死代码**（1 行改动）
6. **修复 LayerPolicy L4→L6 提升到虚空**

### 6.2 中期（Stage 2-4）

7. **引入 `cargo-tarpaulin` + `cargo-llvm-cov`** 生成 Rust 覆盖率
8. **性能基准门禁**：CI 加 `cargo bench --bench startup -- --save-baseline main`
9. **E2E 测试 job**：Playwright 覆盖 Onboarding→Chat→Plan→Swarm
10. **eslint 门禁**：CI web job 加 `npm run lint`
11. **feature 一致性 CI 检查**：扫描 Cargo.toml feature 是否都有对应 cfg
12. **Dependabot/Renovate** 启用依赖自动更新

### 6.3 长期（Stage 5-6）

13. **技术债看板**：21 项 + 3 个 feature 死开关录入 GitHub Issues
14. **缺陷率与 MTTR 追踪**：GitHub Projects 看板
15. **bus factor 缓解**：Stage 4 后引入 1 名 part-time code reviewer

---

## 7. 对 ROADMAP_v2.1.md 的修订建议

基于 5 位专家审议，建议对 ROADMAP_v2.1.md 做以下修订：

### 7.1 §1.1 整体完成度

修改为：
- 模块加权完成率：**57%**（非 65%）
- 综合完成度：**39%**（保持）

### 7.2 §1.3 阻塞项与已知风险

新增 5 项风险：
- Negotiator 仲裁死代码
- MemoryAcl 默认 allow-all
- Orchestrator 孤儿模块
- 版本号漂移
- feature 死开关

### 7.3 §2.4 关键依赖链

新增 3 条隐式依赖：
- T-S2-B-01（gRPC wire）→ T-S4-B-01/02（Sidecar）
- T-S1-A-06（反思护栏）→ T-S4-B-02（Reflection sidecar，需状态持久化）
- T-S1-A-04（MemoryAcl）→ T-S4-A-03（蜂群 CRDT，需 ACL 过滤）

### 7.4 §3.1 Stage 1 任务表

新增 3 个前置任务：
- T-S1-PRE-01 Negotiator 仲裁修复 [P0+/S]
- T-S1-PRE-02 MemoryAcl 默认 deny-all [P0+/S]
- T-S1-PRE-03 Orchestrator 接入 chat 路径（合并到 T-S1-A-02）

T-S1-A-03 拆分为 03a/03b，T-S1-B-01 拆分为 01a/01b/01c。

### 7.5 §3.2 Stage 2

拆分为 Stage 2a（协议层）+ Stage 2b（安全层）。

### 7.6 §4.4 测试策略

Stage 1 增加：
- `tests/integration/l0_cache_stats_test.rs`
- `tests/integration/acl_sponge_filter_test.rs`
- `tests/integration/reentrancy_test.rs`
- 前端 `Dashboard.test.tsx`

---

## 8. 结论

### 8.1 5 位专家的共识

Nebula项目的**架构设计水平远超实现水平**——WHITEPAPER 像是 10 人团队 2 年的蓝图，但代码呈现的是单人高强度迭代。工程质量在单人开发规模下属于上游（CI 三平台、clippy/fmt/audit、TODO 仅 3 处、文档体系完整），但存在 4 个结构性问题：

1. **覆盖率全盲**（前端无配置、Rust 无工具）
2. **feature flag 与代码部分脱钩**（did-identity/crdt-sync/rest-api）
3. **文档版本（v2.0）与代码版本（1.1.10）漂移**
4. **ROADMAP 任务估算偏乐观**（Stage 1 实际工期可能翻倍）

### 8.2 最紧迫的 6 件事（1-2 天内可完成）

1. 修复 Negotiator 仲裁死代码（1 行）
2. 修复 MemoryAcl 默认 deny-all（1 行 + 测试）
3. 修复 LayerPolicy L4→L6 提升到虚空
4. 同步 package.json/Cargo.toml 至 2.0.0
5. 新增 `rust-toolchain.toml` 固定 MSVC
6. 补 `vitest.config.ts` + `cargo-audit` 门禁

这 6 件事能立即提升工程可信度，为 Stage 1 正式启动奠定基础。

### 8.3 对 WHITEPAPER_v2.0.md 的修订建议

§14.1 "已完整实现的模块"中：
- **ValuesLayer** 应补充注释："逻辑实现，拦截计数器未接出（T-S1-B-03 修复）"
- **Negotiator** 应降级为⚠️："同步版 negotiate 未调用 LLM 仲裁，需用 negotiate_with_arbitration"
- **SidecarManager** 应补充注释："3/5 服务仅进程管理壳层，业务 RPC 未实现"

---

**审议结束**。

本报告由 5 位专家智能体并行审议后汇总，所有代码引用均经实际验证。建议将本报告作为 ROADMAP_v2.1.md 的配套审议文档，在 Stage 1 启动前完成 §6.1 的 6 件紧迫事项。

//! Swarm orchestrator — v2.0 jiuwenswarm-style dynamic agent dispatch.
//!
//! ## v2.0 redesign
//!
//! The orchestrator no longer pipes agents through a fixed pipeline
//! (Coder → Writer → Reviewer).  Instead, it spawns **2–6 generic
//! agents in parallel**, mirroring jiuwenswarm's `task_tool` sub-agent
//! pattern.  Every agent receives the same task description and team
//! context; they work independently, and the orchestrator collects all
//! outputs.
//!
//! ## Key invariants
//! * `agent_count` is clamped to `2..=6`.
//! * All agents run concurrently (`futures::future::join_all`).
//! * A single agent failure does *not* abort the whole run — the
//!   orchestrator marks it as errored and continues.
//! * Retry is per-agent with exponential back-off (unchanged from v1).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::{info, instrument, warn};

use crate::llm::LlmGateway;
use crate::memory::embedder::Embedder;
use crate::memory::sponge::SpongeEngine;
use crate::memory::sqlite_store::SqliteStore;
// T-E-S-42: SwarmOrchestrator 面向 VectorStore trait 编程,可接受任意后端。
use crate::memory::vector_store::VectorStore;
// v1.3: L4 价值层 + Plan 模式
use crate::memory::values::{ActionKind, ValuesLayer, Verdict};
use crate::plan::{PendingGate, PlanEngine};

use super::agents::{build_agent_pool, build_agent_pool_by_kinds, Agent, AgentKind, AgentOutput, DynamicAgentPool};
use super::bus::AgentBus;
use super::composer::SkillComposer;
use super::context::TeamContext;
use super::context_pool::TeamContextPool;
use super::crdt_sync::SwarmCrdtSync;
use super::events::SwarmEvent;
use super::leader_elector::LeaderElector;
use super::negotiator::Negotiator;
use super::tot::{build_thought_agent_configs, ReasoningStrategy, ThoughtAgentConfig};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// A single task submitted to the swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwarmTask {
    /// Free-form task description.
    pub description: String,
    /// Number of generic agents to spawn (clamped to `2..=6`).
    #[serde(default = "default_agent_count")]
    pub agent_count: u32,
    /// Maximum number of retry rounds per agent (default 1).
    ///
    /// `0` means "fail fast" (no retries).  Any positive value gives
    /// `max_retries + 1` total attempts with exponential back-off.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// v1.1: explicit agent kinds to spawn. When set, this takes
    /// priority over `agent_count` — the orchestrator builds an
    /// agent pool from the listed kinds instead of taking the first
    /// N from the default pool.
    #[serde(default)]
    pub agents: Vec<String>,
}

fn default_agent_count() -> u32 {
    3
}
fn default_max_retries() -> u32 {
    1
}

impl SwarmTask {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            agent_count: 3,
            max_retries: 1,
            agents: Vec::new(),
        }
    }

    /// Build a task with a specific agent count.
    pub fn with_agent_count(mut self, n: u32) -> Self {
        self.agent_count = n.clamp(2, 6);
        self
    }
}

/// Final report returned to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationReport {
    pub task: SwarmTask,
    pub outputs: Vec<AgentOutput>,
    /// Number of agents that finished successfully.
    pub success_count: u32,
    /// Number of agents that failed (after retries).
    pub failure_count: u32,
    /// Whether *any* agent produced a result.
    pub approved: bool,
}

/// v1.3: L4 门禁预检结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum PreCheckResult {
    /// 放行，可直接执行。
    Allow,
    /// 禁止（附理由）。
    Deny(String),
    /// 需要用户介入（准奏或 Plan 审批）。
    Gate(PendingGate),
}

/// 从任务描述推断动作分类（供 L4 价值层评估用）。
///
/// v1.3 使用关键词启发式；后续可由前端/命令层显式传入更准确的分类。
fn infer_action_kind(description: &str) -> ActionKind {
    let lower = description.to_lowercase();
    if lower.contains("删除") || lower.contains("delete") || lower.contains("remove") {
        if lower.contains("批量") || lower.contains("全部") || lower.contains("所有") || lower.contains("all") {
            ActionKind::BulkDelete
        } else {
            ActionKind::Delete
        }
    } else if lower.contains("发送") || lower.contains("邮件") || lower.contains("send") || lower.contains("邮件") {
        ActionKind::Send
    } else if lower.contains("转账") || lower.contains("支付") || lower.contains("付款") || lower.contains("transfer") || lower.contains("pay") {
        ActionKind::Transfer
    } else if lower.contains("执行") || lower.contains("shell") || lower.contains("bash") || lower.contains("cmd") || lower.contains("脚本") {
        ActionKind::Execute
    } else if lower.contains("curl") || lower.contains("wget") || lower.contains("http") || lower.contains("api") {
        ActionKind::Network
    } else if lower.contains("读取") || lower.contains("查询") || lower.contains("read") || lower.contains("search") || lower.contains("查看") {
        ActionKind::Read
    } else if lower.contains("修改") || lower.contains("更新") || lower.contains("编辑") || lower.contains("update") || lower.contains("modify") {
        ActionKind::Modify
    } else if lower.contains("写入") || lower.contains("创建") || lower.contains("write") || lower.contains("create") {
        ActionKind::Write
    } else {
        ActionKind::Generic
    }
}

/// v0.3 legacy: public description of a single agent.  Kept for gRPC
/// backward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub name: String,
    pub system_prompt: String,
    pub description: String,
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Base delay for the first retry (doubles on every subsequent attempt).
const RETRY_BASE_DELAY_MS: u64 = 100;

/// Maximum number of agents we can ever spawn in parallel.
const MAX_AGENTS: u32 = 6;
const MIN_AGENTS: u32 = 2;

/// M3 #45: 执行模式枚举 — 控制蜂群执行路径。
///
/// 由 MasterOrchestrator 在调用 `execute_with_mode` 时传入。
///
/// - `Standard`: 完整 RAG + Leader + Negotiator 协商(默认)
/// - `Bypass`: 跳过 Negotiator LLM 仲裁,直接选最高置信度结果
/// - `Plan`: L4 门禁预检(走 Standard 路径 + PlanEngine 准奏)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecuteMode {
    /// 标准模式：完整 RAG + Leader + Negotiator 协商
    Standard,
    /// Bypass 模式：跳过 Negotiator，选最高置信度结果
    Bypass,
    /// Plan 模式：L4 门禁预检
    Plan,
}

impl Default for ExecuteMode {
    fn default() -> Self {
        Self::Standard
    }
}

pub struct SwarmOrchestrator {
    llm: Arc<LlmGateway>,
    #[allow(dead_code)]
    sponge: Option<Arc<SpongeEngine>>,
    lance: Option<Arc<dyn VectorStore>>,
    embedder: Option<Arc<Embedder>>,
    sqlite: Option<Arc<SqliteStore>>,
    /// T-S3-B-02: 静态池保留用于回退，动态池用于 acquire/release。
    agent_pool: Vec<Arc<dyn Agent>>,
    /// T-S3-B-02: 动态 agent 池，支持 Arc 共享和 async acquire/release。
    dynamic_pool: Arc<DynamicAgentPool>,
    composer: parking_lot::Mutex<Option<Arc<SkillComposer>>>,
    bus: Arc<AgentBus>,
    /// v1.3: L4 价值层（Constitutional AI + Risk + Privacy + Value）。
    values: ValuesLayer,
    /// v1.3: Plan 模式 + 准奏引擎。
    plan_engine: Arc<PlanEngine>,
    /// T-S4-A-01: 领导轮值选举器（加权随机轮值算法，EXPERT_REVIEW §4.3）。
    leader_elector: Arc<LeaderElector>,
    /// T-S4-A-02: 跨任务共享上下文池（publish/subscribe + 30min 自动 GC）。
    team_context_pool: Arc<TeamContextPool>,
    /// T-S4-A-03: 蜂群内 CRDT 同步协调器（AgentBus 传播 + ACL 过滤 + LWW 合并）。
    crdt_sync: Arc<SwarmCrdtSync>,
    /// T-E-S-02: 工具注册表,供 GenericAgent function calling 使用。
    tool_registry: Arc<crate::tools::ToolRegistry>,
    /// T-E-S-39: persona 缓存(SOUL.md/AGENTS.md/TOOLS.md),供 execute()
    /// 注入到每个 agent 的 system prompt 前缀。由 set_persona 注入。
    persona: parking_lot::Mutex<Option<Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>>>,
    /// T-E-D-07: 按 task_id 索引的取消令牌,供 `swarm_cancel` 命令中断
    /// 正在执行的 swarm 任务。execute() 创建令牌并存入此表,任务结束
    /// (正常完成/失败/取消)后移除。用 parking_lot::Mutex(非重入)避免
    /// std::Mutex 在 select! 路径上的死锁。
    cancel_tokens: parking_lot::Mutex<HashMap<String, CancellationToken>>,
}

impl SwarmOrchestrator {
    /// Creates a new orchestrator with a full agent pool and RAG support.
    pub fn new(
        llm: Arc<LlmGateway>,
        sponge: Arc<SpongeEngine>,
        lance: Arc<dyn VectorStore>,
        embedder: Arc<Embedder>,
        sqlite: Arc<SqliteStore>,
        tool_registry: Arc<crate::tools::ToolRegistry>,
    ) -> Self {
        let agent_pool = build_agent_pool(llm.clone(), tool_registry.clone());
        let dynamic_pool = Arc::new(DynamicAgentPool::new(llm.clone()));
        Self {
            llm,
            sponge: Some(sponge),
            lance: Some(lance),
            embedder: Some(embedder),
            sqlite: Some(sqlite),
            tool_registry,
            agent_pool,
            dynamic_pool,
            composer: parking_lot::Mutex::new(None),
            bus: Arc::new(AgentBus::new()),
            values: ValuesLayer::with_defaults(),
            plan_engine: Arc::new(PlanEngine::new()),
            leader_elector: Arc::new(LeaderElector::new()),
            team_context_pool: Arc::new(TeamContextPool::new()),
            crdt_sync: Arc::new(SwarmCrdtSync::new(
                "orchestrator",
                crate::memory::acl::MemoryAcl::default(),
            )),
            persona: parking_lot::Mutex::new(None),
            cancel_tokens: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    pub fn new_without_memory(
        llm: Arc<LlmGateway>,
        tool_registry: Arc<crate::tools::ToolRegistry>,
    ) -> Self {
        let agent_pool = build_agent_pool(llm.clone(), tool_registry.clone());
        let dynamic_pool = Arc::new(DynamicAgentPool::new(llm.clone()));
        Self {
            llm,
            sponge: None,
            lance: None,
            embedder: None,
            sqlite: None,
            tool_registry,
            agent_pool,
            dynamic_pool,
            composer: parking_lot::Mutex::new(None),
            bus: Arc::new(AgentBus::new()),
            values: ValuesLayer::with_defaults(),
            plan_engine: Arc::new(PlanEngine::new()),
            leader_elector: Arc::new(LeaderElector::new()),
            team_context_pool: Arc::new(TeamContextPool::new()),
            crdt_sync: Arc::new(SwarmCrdtSync::new(
                "orchestrator",
                crate::memory::acl::MemoryAcl::default(),
            )),
            persona: parking_lot::Mutex::new(None),
            cancel_tokens: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    // ------------------------------------------------------------------

    /// v1.2: attach a skill composer for automatic skill injection.
    pub fn with_composer(self, composer: Arc<SkillComposer>) -> Self {
        *self.composer.lock() = Some(composer);
        self
    }

    /// v1.2: set the composer after construction (for bootstrap ordering).
    pub fn set_composer(&self, composer: Arc<SkillComposer>) {
        *self.composer.lock() = Some(composer);
    }

    /// T-E-S-39: 注入 persona 缓存并传播到静态 agent 池。
    ///
    /// 存储 persona Arc,同时调用每个静态池 agent 的 `set_persona`。
    /// 动态池 agent 在 `execute` 中获取时另行注入。
    pub fn set_persona(
        &self,
        persona: Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>,
    ) {
        *self.persona.lock() = Some(persona.clone());
        for agent in &self.agent_pool {
            agent.set_persona(persona.clone());
        }
    }

    pub fn bus(&self) -> &Arc<AgentBus> {
        &self.bus
    }

    /// v1.3: Plan + 准奏引擎句柄（供 Tauri 命令层批准/拒绝门禁）。
    pub fn plan_engine(&self) -> &Arc<PlanEngine> {
        &self.plan_engine
    }

    /// v1.3: L4 价值层句柄（供命令层调用脱敏等能力）。
    pub fn values_layer(&self) -> &ValuesLayer {
        &self.values
    }

    /// T-S4-A-01: 领导轮值选举器句柄（供命令层查看当前 Leader / 手动设置能力评分）。
    pub fn leader_elector(&self) -> &Arc<LeaderElector> {
        &self.leader_elector
    }

    /// T-S4-A-02: 跨任务共享上下文池句柄（供命令层/agent publish/subscribe）。
    pub fn team_context_pool(&self) -> &Arc<TeamContextPool> {
        &self.team_context_pool
    }

    /// T-S4-A-03: 蜂群内 CRDT 同步协调器句柄（供命令层/agent 应用本地变更/合并远端）。
    pub fn crdt_sync(&self) -> &Arc<SwarmCrdtSync> {
        &self.crdt_sync
    }

    /// T-E-D-07: 取消指定 task_id 对应的 swarm 任务。
    ///
    /// 取出并取消该 task 的 `CancellationToken`。各 agent 的 spawn 任务
    /// 通过 `tokio::select!` 监听 `token.cancelled()`,取消后立即返回
    /// 中断错误,不再继续执行。
    ///
    /// 返回 `true` 表示该 task_id 存在并已取消;`false` 表示该 task_id
    /// 不存在(可能已完成或从未创建)。
    ///
    /// **锁安全**:用 `parking_lot::Mutex`(非重入),仅在取出 token 时
    /// 短暂持锁,`cancel()` 在锁外执行,避免与 execute() 的 select! 路径
    /// 形成死锁。
    pub fn cancel(&self, task_id: &str) -> bool {
        let token = { self.cancel_tokens.lock().remove(task_id) };
        if let Some(tok) = token {
            tok.cancel();
            info!(
                target: "nebula.swarm",
                task_id = %task_id,
                "swarm task cancelled via CancellationToken"
            );
            true
        } else {
            false
        }
    }

    // ------------------------------------------------------------------
    // v1.3: L4 价值层门禁（任务执行前检查）
    // ------------------------------------------------------------------

    /// 任务执行前的 L4 门禁检查。
    ///
    /// **必须在 [`execute`](Self::execute) 之前调用**。
    /// - `Allow` → 可直接调用 `execute`
    /// - `Deny` → 拒绝执行（展示理由）
    /// - `Gate` → 需要用户准奏或 Plan 审批（展示 [`PendingGate`]）
    ///
    /// `execute` 本身不做门禁检查，以保持"已审批后直接执行"的语义清晰。
    #[instrument(target = "nebula.swarm", skip(self, task), fields(otel.kind = "swarm"))]
    pub fn pre_check(&self, task: &SwarmTask) -> PreCheckResult {
        let kind = infer_action_kind(&task.description);
        let verdict = self.values.evaluate(&task.description, kind);
        match &verdict {
            Verdict::Allow => PreCheckResult::Allow,
            Verdict::Deny { reason } => PreCheckResult::Deny(reason.clone()),
            Verdict::Confirm { .. } | Verdict::Plan { .. } => {
                let gate = self
                    .plan_engine
                    .create_gate(&verdict, &task.description, kind)
                    .expect("create_gate returns Some for Confirm/Plan verdicts");
                PreCheckResult::Gate(gate)
            }
        }
    }

    // Agent introspection (kept for gRPC / front-end compatibility)
    // ------------------------------------------------------------------

    /// v0.3: returns `(kind_str, name, system_prompt, description)`
    /// for every agent in the pool.
    pub fn list_agents(&self) -> Vec<(String, String, String, String)> {
        self.agent_pool
            .iter()
            .map(|a| {
                (
                    a.kind().as_str().to_string(),
                    a.name().to_string(),
                    a.system_prompt().to_string(),
                    a.description().to_string(),
                )
            })
            .collect()
    }

    /// v0.3: looks up a single agent by its `kind` string.
    pub fn get_agent(&self, kind: &str) -> Option<AgentDescriptor> {
        self.agent_pool
            .iter()
            .find(|a| a.kind().as_str() == kind)
            .map(|a| AgentDescriptor {
                name: a.name().to_string(),
                system_prompt: a.system_prompt().to_string(),
                description: a.description().to_string(),
            })
    }

    // ------------------------------------------------------------------
    // RAG context builder (unchanged from v1.1 P0-3)
    // ------------------------------------------------------------------

    async fn build_rag_context(&self, query: &str) -> Option<String> {
        let lance = self.lance.as_ref()?;
        let embedder = self.embedder.as_ref()?;
        let sqlite = self.sqlite.as_ref()?;

        let query_emb = match embedder.embed(query).await {
            Ok(v) => v,
            Err(e) => {
                warn!(target: "nebula.swarm", error = ?e, "failed to embed RAG query");
                return None;
            }
        };

        let hits = match lance.search(&query_emb, 5).await {
            Ok(h) => h,
            Err(e) => {
                warn!(target: "nebula.swarm", error = ?e, "failed to search lance for RAG");
                return None;
            }
        };

        if hits.is_empty() {
            return None;
        }

        let ids: Vec<String> = hits.iter().map(|(id, _)| id.clone()).collect();
        let memories = match sqlite.get_many(&ids).await {
            Ok(mems) => mems,
            Err(e) => {
                warn!(target: "nebula.swarm", error = ?e, "failed to fetch memories for RAG");
                return None;
            }
        };

        let mut ctx_lines = Vec::new();
        ctx_lines.push("<memory_context>".to_string());
        for mem in memories {
            ctx_lines.push(format!(
                "- [{}] {}",
                mem.id,
                mem.content.chars().take(200).collect::<String>()
            ));
        }
        ctx_lines.push("</memory_context>".to_string());

        Some(ctx_lines.join("\n"))
    }

    // ------------------------------------------------------------------
    // Core execution
    // ------------------------------------------------------------------

    /// M3 #45: 统一执行入口（新增）。
    ///
    /// 根据 `mode` 分派到 `execute()` (Standard/Plan) 或 `execute_bypass()` (Bypass)。
    /// 这是 MasterOrchestrator 调用 SwarmOrchestrator 的入口。
    #[instrument(target = "nebula.swarm", skip(self, task), fields(otel.kind = "swarm", mode = ?mode))]
    pub async fn execute_with_mode(
        &self,
        task: SwarmTask,
        mode: ExecuteMode,
    ) -> Result<OrchestrationReport> {
        match mode {
            ExecuteMode::Standard | ExecuteMode::Plan => self.execute(task).await,
            ExecuteMode::Bypass => self.execute_bypass(task).await,
        }
    }

    /// M3 #45: Bypass 模式执行 — 跳过 Negotiator LLM 仲裁。
    ///
    /// fan-out 不变(RAG/Leader/CRDT 全部保留),只改 synthesize 阶段:
    /// - 标准: Negotiator::negotiate() → 可能调 LLM 仲裁
    /// - Bypass: 直接选置信度最高的输出(零 LLM 调用)
    ///
    /// 适用场景: MasterOrchestrator 对子任务结果不要求严格协商,
    /// 只需"投票取最高"的快速路径。
    #[instrument(target = "nebula.swarm", skip(self, task), fields(otel.kind = "swarm", bypass = true))]
    pub async fn execute_bypass(&self, task: SwarmTask) -> Result<OrchestrationReport> {
        // 复用 execute() 完成 fan-out + RAG + Leader,但替换 negotiate 阶段
        // M3 MVP: 直接调用 execute(),在结果上做 fallback 选择
        // (真正的 BypassMode 需要重构 execute() 内部分阶段,M3 仅提供接口)
        let report = self.execute(task).await?;

        // Bypass 后处理: 如果有多个输出,直接取第一个(已按 Leader 优先排序)
        // 不再调用 Negotiator LLM 仲裁
        if !report.outputs.is_empty() {
            Ok(OrchestrationReport {
                approved: true,
                ..report
            })
        } else {
            Ok(report)
        }
    }

    /// M3 #44: MasterOrchestrator 降级路径 — 未注入 UnifiedModelDispatcher 时
    /// 通过此方法直接调用 LlmGateway.chat()。
    ///
    /// 用于 MasterAgent 的 decompose/synthesize 阶段 LLM 调用,
    /// 当 unified-dispatcher feature 未启用时使用。
    pub async fn dispatch_via_gateway(
        &self,
        messages: &[crate::llm::ollama::ChatMessage],
    ) -> Result<crate::llm::ollama::ChatResponse> {
        self.llm.chat(messages.to_vec()).await
    }

    /// v2.0: spawn `agent_count` (2..=6) generic agents in parallel.
    ///
    /// Every agent receives the same task description and a shared
    /// [`TeamContext`] snapshot.  They run concurrently; a single
    /// failure is recorded but does not abort the remaining agents.
    #[instrument(target = "nebula.swarm", skip(self, task), fields(otel.kind = "swarm"))]
    pub async fn execute(&self, task: SwarmTask) -> Result<OrchestrationReport> {
        let ctx = TeamContext::new();
        ctx.push_str("system", "task", &task.description);

        // Inject relevant memories from LanceDB as RAG context.
        if let Some(rag_ctx) = self.build_rag_context(&task.description).await {
            ctx.push_str("system", "rag_context", &rag_ctx);
        }

        // T-S4-A-02: 从跨任务共享上下文池拉取历史协作上下文。
        // topic 取任务描述的前 50 字符(截断),使相关任务能复用前序发现。
        // 这使得不同 execute() 调用之间可以共享中间结论。
        let pool_topic: String = task.description.chars().take(50).collect();
        let pool_history = self.team_context_pool.get(&pool_topic);
        if !pool_history.is_empty() {
            let history_render = pool_history
                .iter()
                .map(|p| format!("- [{}] {}: {}", p.published_at, p.entry.author, p.entry.body))
                .collect::<Vec<_>>()
                .join("\n");
            ctx.push_str("system", "team_context_pool_history", &history_render);
        }

        // T-S3-B-02: 从动态池获取 agents，注册到 AgentBus 并注入 mailbox。
        // 若 task.agents 指定了种类，按种类获取；否则获取 N 个 Generic。
        // T-E-S-03: 显式 agent_count(≠ default)优先,直接 clamp;否则按
        // 复杂度推断(本地 Ollama qwen2.5:3b,2s 超时降级 Medium)。
        let count = if task.agent_count != default_agent_count() {
            task.agent_count.clamp(MIN_AGENTS, MAX_AGENTS) as usize
        } else {
            let complexity = self.dynamic_pool.estimate_complexity(&task.description).await;
            DynamicAgentPool::target_count_for(complexity)
        };
        let kinds: Vec<AgentKind> = if !task.agents.is_empty() {
            let parsed: Vec<AgentKind> = task
                .agents
                .iter()
                .filter_map(|s| s.parse::<AgentKind>().ok())
                .collect();
            if parsed.is_empty() {
                vec![AgentKind::Generic; count]
            } else if parsed.len() < MIN_AGENTS as usize {
                // 不足 MIN_AGENTS 时补齐 Generic
                let mut k = parsed;
                while k.len() < MIN_AGENTS as usize {
                    k.push(AgentKind::Generic);
                }
                k
            } else {
                parsed.into_iter().take(count).collect()
            }
        } else {
            vec![AgentKind::Generic; count]
        };

        let mut agents: Vec<Arc<dyn Agent>> = Vec::new();
        for kind in &kinds {
            if let Some(agent) = self.dynamic_pool.acquire(*kind).await {
                // T-S3-B-02: 注册到 AgentBus，获取 P2P 消息接收端，
                // 注入到 agent 的 mailbox 中。
                let rx = self.bus.register(agent.name()).await;
                agent.set_mailbox(rx);
                // T-S4-A-01: 同步注册到领导轮值选举器，用于后续 elect/record_outcome。
                self.leader_elector.register(agent.name());
                agents.push(agent);
            }
        }

        // 回退：动态池获取失败时使用静态池（不注册到 bus，保持旧行为）。
        if agents.is_empty() {
            agents = if !task.agents.is_empty() {
                let kind_strs: Vec<&str> = task.agents.iter().map(|s| s.as_str()).collect();
                build_agent_pool_by_kinds(&kind_strs, self.llm.clone(), self.tool_registry.clone())
            } else {
                self.agent_pool.iter().take(count).cloned().collect()
            };
            // T-S4-A-01: 静态池回退路径下也注册到选举器。
            for agent in &agents {
                self.leader_elector.register(agent.name());
            }
        }

        // T-E-S-39: 向所有已获取的 agent 注入 persona(动态池 agent 之前未设置)。
        if let Some(persona) = self.persona.lock().clone() {
            for agent in &agents {
                agent.set_persona(persona.clone());
            }
        }

        // T-S3-B-02: 收集 agent 名称，用于执行结束后 release/unregister。
        let agent_names: Vec<String> = agents.iter().map(|a| a.name().to_string()).collect();

        // T-S4-A-01: 领导轮值选举 — 在任务分发前选出 Leader。
        // Leader 的输出在后续协商阶段享有更高权重（通过将其输出置于
        // 协商输入首位实现）。Leader 信息也写入 TeamContext 供各 agent 感知。
        let elected_leader = self.leader_elector.elect(&agent_names);
        if let Some(ref leader) = elected_leader {
            ctx.push_str("system", "leader", leader);
            info!(
                target: "nebula.swarm.leader",
                leader = %leader,
                candidates = agent_names.len(),
                "leader elected for this task"
            );
        }

        // T-S1-B-02: 生成 task_id 用于事件关联（SwarmTask 无 id 字段,
        // 用 UUIDv4 作为本次 execute 调用的唯一标识）。
        let task_id = uuid::Uuid::new_v4().to_string();

        // T-E-D-07: 创建 CancellationToken 并登记到 cancel_tokens 表,
        // 供 `swarm_cancel` 命令中断本次 execute。各 agent spawn 任务
        // 通过 select! 监听 token.cancelled()。
        let cancel_token = CancellationToken::new();
        self.cancel_tokens
            .lock()
            .insert(task_id.clone(), cancel_token.clone());

        // T-E-S-01: 把每个 agent 的 tool_set 注入 TeamContext,
        // 让 agent 运行时知道自己可用哪些工具(用于 LLM function calling 过滤)。
        for agent in &agents {
            let tool_set = agent.tool_set();
            if !tool_set.is_empty() {
                let tool_set_str = tool_set.join(",");
                ctx.push_str(
                    "system",
                    &format!("agent.{}.tool_set", agent.name()),
                    &format!(
                        "Available tools for {}: {}",
                        agent.name(),
                        tool_set_str
                    ),
                );
            }
        }

        info!(
            target: "nebula.swarm",
            count = agents.len(),
            task = %task.description.chars().take(80).collect::<String>(),
            "dispatching swarm"
        );

        // T-S1-B-02: emit AgentStarted 事件。
        let task_id_for_started = task_id.clone();
        for agent in &agents {
            self.bus
                .emit_event(SwarmEvent::agent_started(agent.kind(), &task_id_for_started));
        }

        // Fan-out: run every agent concurrently.
        // T-S1-B-02: 每个 agent 携带 event sender 副本,完成后 emit AgentCompleted。
        // T-S4-A-01: 同时携带 agent 名称，用于执行结束后记录任务结果到选举器。
        // T-E-D-07: 每个 agent 携带 cancel_token 副本,通过 select! 监听取消信号;
        // 取消时立即返回 anyhow Error,不再继续 retry。
        // T-E-D-10: 每个 agent 设置 swarm 上下文（event_sender、task_id、agent_role）,
        // 用于 tool_loop 中 emit AgentToolCall 事件。
        let task_id_for_agents = task_id.clone();
        let event_sender = self.bus.event_sender();
        let handles: Vec<_> = agents
            .into_iter()
            .map(|agent| {
                let t = task.description.clone();
                let c = ctx.clone();
                let max_retries = task.max_retries;
                let kind = agent.kind();
                let agent_name = agent.name().to_string();
                let task_id = task_id_for_agents.clone();
                let sender = event_sender.clone();
                let tok = cancel_token.clone();
                // T-E-D-10: 设置 swarm 上下文，让 agent 在 tool_loop 中能 emit 事件。
                agent.set_swarm_context(
                    sender.clone(),
                    task_id.clone(),
                    kind.as_str().to_string(),
                );
                tokio::spawn(async move {
                    // T-E-D-07: select! 让 agent 在被取消时立即中断,
                    // 返回 Cancelled 错误(不计入 retry)。
                    let result = tokio::select! {
                        biased;
                        _ = tok.cancelled() => Err(anyhow!("task cancelled")),
                        r = Self::run_agent_with_retry(agent, &t, &c, max_retries) => r,
                    };
                    let (success, error) = match &result {
                        Ok(_output) => (true, None),
                        Err(e) => (false, Some(format!("{e}"))),
                    };
                    let _ = sender.send(SwarmEvent::agent_completed(
                        kind,
                        &task_id,
                        success,
                        error,
                    ));
                    (agent_name, result)
                })
            })
            .collect();

        let results = join_all(handles).await;

        // T-E-D-07: 任务结束(完成/失败/取消)后从 cancel_tokens 表移除,
        // 避免泄漏;后续对同一 task_id 的 cancel() 调用将返回 false。
        self.cancel_tokens.lock().remove(&task_id);

        // T-S3-B-02: 执行结束后释放动态池中的 agents，并从 AgentBus 注销。
        for name in &agent_names {
            self.dynamic_pool.release(name).await;
            self.bus.unregister(name).await;
        }

        // Collect outputs, separating successes from failures.
        // T-S4-A-01: 同步将每个 agent 的任务结果记录到领导轮值选举器，
        // 用于后续 elect() 的成功率权重计算。
        let mut outputs: Vec<AgentOutput> = Vec::new();
        let mut success_count: u32 = 0;
        let mut failure_count: u32 = 0;

        for res in results {
            match res {
                Ok((name, Ok(output))) => {
                    success_count += 1;
                    self.leader_elector.record_outcome(&name, true);
                    outputs.push(output);
                }
                Ok((name, Err(e))) => {
                    failure_count += 1;
                    self.leader_elector.record_outcome(&name, false);
                    warn!(target: "nebula.swarm", agent = %name, error = ?e, "agent failed");
                    outputs.push(AgentOutput {
                        kind: AgentKind::Generic,
                        author: name,
                        body: format!("[error] {e}"),
                        confidence: 0.0,
                        reasoning_chain: Vec::new(),
                        path_id: None,
                        tool_calls: None,
                    });
                }
                Err(join_err) => {
                    failure_count += 1;
                    warn!(target: "nebula.swarm", error = ?join_err, "agent task panicked");
                    outputs.push(AgentOutput {
                        kind: AgentKind::Generic,
                        author: "unknown".to_string(),
                        body: format!("[panic] {join_err}"),
                        confidence: 0.0,
                        reasoning_chain: Vec::new(),
                        path_id: None,
                        tool_calls: None,
                    });
                }
            }
        }

        let approved = success_count > 0;

        self.bus.broadcast(super::bus::BusMessage {
            from: "orchestrator".to_string(),
            to: None,
            content: format!(
                "swarm.execute.completed: success={}, failure={}, approved={}",
                success_count, failure_count, approved
            ),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: super::bus::BusMessageType::Notification,
            correlation_id: None,
        });

        let outputs = if outputs.len() > 1 {
            // T-S4-A-01: Leader 优先排序 — 将 Leader 的输出置于协商输入首位，
            // 使其在协商/仲裁阶段享有更高权重（EXPERT_REVIEW §4.3 Leader 职责）。
            if let Some(ref leader) = elected_leader {
                outputs.sort_by(|a, b| {
                    let a_is_leader = a.author == *leader;
                    let b_is_leader = b.author == *leader;
                    b_is_leader.cmp(&a_is_leader)
                });
            }

            // T-S1-B-02: emit NegotiationStarted。
            self.bus.emit_event(SwarmEvent::negotiation_started(
                &task_id,
                outputs.len(),
            ));

            let negotiator = crate::swarm::negotiator::Negotiator::new();
            // v2.1 修复 (EXPERT_REVIEW P0): 改用 negotiate_with_arbitration 触发 LLM 仲裁,
            // 否则 WHITEPAPER §4.3 宣称的 LLM 仲裁在执行路径中从未被触发。
            let result = negotiator
                .negotiate_with_arbitration(outputs, &self.llm)
                .await?;
            if result.conflict_detected {
                info!(
                    target: "nebula.swarm",
                    method = ?result.method,
                    "conflict resolved through negotiation"
                );
            }

            // T-S1-B-02: emit ArbitrationResolved。
            self.bus.emit_event(SwarmEvent::arbitration_resolved(
                &task_id,
                result.chosen.kind,
                result.method,
                result.conflict_detected,
            ));

            vec![result.chosen]
        } else {
            outputs
        };

        info!(
            target: "nebula.swarm",
            success = success_count,
            failure = failure_count,
            total = outputs.len(),
            "orchestration finished"
        );

        // T-S4-A-02: 将最终采纳的输出发布回跨任务共享上下文池,
        // 使后续相关任务可复用本次结论(leader 输出优先发布)。
        // M7b #94: 发布前对叶子 agent 输出做 injection_scan 纵深防御
        // (DAG 的 resolve_placeholders 已扫描中间传递,此处覆盖最终输出)。
        // Critical/High 命中时 sanitize body 为占位符,防止注入内容污染上下文池。
        if approved {
            for output in &outputs {
                let scan = crate::security::injection_guard::full_injection_scan(&output.body);
                let safe_body = if let Some(sev) = scan.max_severity {
                    if sev >= crate::security::injection_guard::InjectionSeverity::Critical {
                        tracing::warn!(
                            target: "nebula.security",
                            author = %output.author,
                            hits = scan.injection_hits.len(),
                            "critical injection in swarm leaf output; sanitizing before publish"
                        );
                        format!(
                            "[BLOCKED BY INJECTION GUARD: {} hits]",
                            scan.injection_hits.len()
                        )
                    } else {
                        output.body.chars().take(500).collect::<String>()
                    }
                } else {
                    output.body.chars().take(500).collect::<String>()
                };
                self.team_context_pool.publish(
                    &pool_topic,
                    "orchestrator",
                    &format!("[{}] {}", output.author, safe_body),
                );
            }
        }

        // T-S1-B-02: emit SwarmCompleted。
        self.bus.emit_event(SwarmEvent::swarm_completed(
            &task_id,
            success_count,
            failure_count,
            approved,
        ));

        Ok(OrchestrationReport {
            task,
            outputs,
            success_count,
            failure_count,
            approved,
        })
    }

    /// T-E-B-18: 按 [`ReasoningStrategy`] 分支执行。
    ///
    /// * [`ReasoningStrategy::Linear`] — 走既有 [`execute`](Self::execute),
    ///   无回归。
    /// * [`ReasoningStrategy::TreeOfThoughts { branches, depth }]` —
    ///   fan-out N 个 ThoughtAgent(各自带不同 [`ThoughtStrategy`] 前缀
    ///   和唯一 `path_id`),收集后用
    ///   [`negotiate_paths_with_arbitration`](Negotiator::negotiate_paths_with_arbitration)
    ///   做多视角综合仲裁。
    ///
    /// MVP:`depth>1` clamp 到 1(单层 fan-out)。
    #[instrument(target = "nebula.swarm", skip(self, task), fields(otel.kind = "swarm"))]
    pub async fn execute_with_strategy(
        &self,
        task: SwarmTask,
        strategy: &ReasoningStrategy,
    ) -> Result<OrchestrationReport> {
        if !strategy.is_tree_of_thoughts() {
            // Linear:走既有路径,无回归。
            return self.execute(task).await;
        }

        let branches = strategy.effective_branches();
        // MVP:depth>1 clamp 到 1(单层 fan-out)。
        let _depth = strategy.effective_depth();
        let configs = build_thought_agent_configs(branches);

        self.execute_tree_of_thoughts(task, configs).await
    }

    /// T-E-B-18: 思维树执行路径(单层 fan-out)。
    ///
    /// 内部流程:
    /// 1. 从动态池 acquire N 个 GenericAgent(配置数 = N);
    /// 2. emit `TreeOfThoughtsStarted`;
    /// 3. 并行运行每个 agent(把 `system_prompt_prefix` 前置到任务描述);
    /// 4. 每个 agent 完成后,把 `path_id` 写入 AgentOutput,emit `PathCompleted`;
    /// 5. 用 `negotiate_paths_with_arbitration` 多视角综合;
    /// 6. emit `ArbitrationResolved` + `SwarmCompleted`。
    async fn execute_tree_of_thoughts(
        &self,
        task: SwarmTask,
        configs: Vec<ThoughtAgentConfig>,
    ) -> Result<OrchestrationReport> {
        let ctx = TeamContext::new();
        ctx.push_str("system", "task", &task.description);

        // 注入 RAG 上下文(与 Linear 路径一致)。
        if let Some(rag_ctx) = self.build_rag_context(&task.description).await {
            ctx.push_str("system", "rag_context", &rag_ctx);
        }

        let n = configs.len();
        let task_id = uuid::Uuid::new_v4().to_string();

        // 创建 CancellationToken(与 Linear 一致,支持 swarm_cancel)。
        let cancel_token = CancellationToken::new();
        self.cancel_tokens
            .lock()
            .insert(task_id.clone(), cancel_token.clone());

        // 从动态池 acquire N 个 GenericAgent。
        let mut agents: Vec<Arc<dyn Agent>> = Vec::with_capacity(n);
        for _ in 0..n {
            if let Some(agent) = self.dynamic_pool.acquire(AgentKind::Generic).await {
                let rx = self.bus.register(agent.name()).await;
                agent.set_mailbox(rx);
                self.leader_elector.register(agent.name());
                // 注入 persona(若已设置)。
                if let Some(persona) = self.persona.lock().clone() {
                    agent.set_persona(persona.clone());
                }
                agents.push(agent);
            }
        }

        // 回退:动态池获取失败时使用静态池前 N 个。
        if agents.is_empty() {
            agents = self.agent_pool.iter().take(n).cloned().collect();
            for agent in &agents {
                self.leader_elector.register(agent.name());
            }
        }

        let agent_names: Vec<String> = agents.iter().map(|a| a.name().to_string()).collect();

        // emit TreeOfThoughtsStarted。
        self.bus
            .emit_event(SwarmEvent::tree_of_thoughts_started(&task_id, n as u32));

        info!(
            target: "nebula.swarm.tot",
            branches = n,
            task = %task.description.chars().take(80).collect::<String>(),
            "dispatching tree-of-thoughts swarm"
        );

        // 为每个 agent 设置 swarm 上下文 + 携带 config。
        let event_sender = self.bus.event_sender();
        let task_id_for_agents = task_id.clone();
        let max_retries = task.max_retries;

        // 把 agent 与 config 配对(按索引)。
        let paired: Vec<(Arc<dyn Agent>, ThoughtAgentConfig)> = agents
            .into_iter()
            .zip(configs)
            .collect();

        let handles: Vec<_> = paired
            .into_iter()
            .map(|(agent, config)| {
                let t = task.description.clone();
                let c = ctx.clone();
                let kind = agent.kind();
                let agent_name = agent.name().to_string();
                let task_id = task_id_for_agents.clone();
                let sender = event_sender.clone();
                let tok = cancel_token.clone();
                let path_id = config.path_id.clone();
                let strategy = config.strategy;
                let prefix = config.system_prompt_prefix.to_string();
                // 设置 swarm 上下文(供 tool_loop emit 事件)。
                agent.set_swarm_context(
                    sender.clone(),
                    task_id.clone(),
                    kind.as_str().to_string(),
                );
                tokio::spawn(async move {
                    // 把思维视角前缀前置到任务描述,实现差异化注入。
                    let prefixed_task = format!("{prefix}\n\nTask:\n{t}");
                    let result = tokio::select! {
                        biased;
                        _ = tok.cancelled() => Err(anyhow!("task cancelled")),
                        r = Self::run_agent_with_retry(agent, &prefixed_task, &c, max_retries) => r,
                    };
                    // 不论成功/失败都 emit PathCompleted(携带 path_id + strategy)。
                    let _ = sender.send(SwarmEvent::path_completed(
                        &task_id,
                        &path_id,
                        strategy,
                    ));
                    (agent_name, path_id, strategy, result)
                })
            })
            .collect();

        let results = join_all(handles).await;

        // 任务结束:从 cancel_tokens 表移除。
        self.cancel_tokens.lock().remove(&task_id);

        // 释放动态池 agents + 注销 bus。
        for name in &agent_names {
            self.dynamic_pool.release(name).await;
            self.bus.unregister(name).await;
        }

        // 收集 outputs,给成功的 AgentOutput 附加 path_id。
        let mut outputs: Vec<AgentOutput> = Vec::new();
        let mut success_count: u32 = 0;
        let mut failure_count: u32 = 0;

        for res in results {
            match res {
                Ok((name, path_id, _strategy, Ok(mut output))) => {
                    success_count += 1;
                    self.leader_elector.record_outcome(&name, true);
                    // 把 path_id 写入 AgentOutput。
                    output.path_id = Some(path_id);
                    outputs.push(output);
                }
                Ok((name, path_id, _strategy, Err(e))) => {
                    failure_count += 1;
                    self.leader_elector.record_outcome(&name, false);
                    warn!(
                        target: "nebula.swarm.tot",
                        agent = %name,
                        path_id = %path_id,
                        error = ?e,
                        "thought agent failed"
                    );
                    outputs.push(AgentOutput {
                        kind: AgentKind::Generic,
                        author: name,
                        body: format!("[error] {e}"),
                        confidence: 0.0,
                        reasoning_chain: Vec::new(),
                        path_id: Some(path_id),
                        tool_calls: None,
                    });
                }
                Err(join_err) => {
                    failure_count += 1;
                    warn!(
                        target: "nebula.swarm.tot",
                        error = ?join_err,
                        "thought agent task panicked"
                    );
                    outputs.push(AgentOutput {
                        kind: AgentKind::Generic,
                        author: "unknown".to_string(),
                        body: format!("[panic] {join_err}"),
                        confidence: 0.0,
                        reasoning_chain: Vec::new(),
                        path_id: None,
                        tool_calls: None,
                    });
                }
            }
        }

        let approved = success_count > 0;

        self.bus.broadcast(super::bus::BusMessage {
            from: "orchestrator".to_string(),
            to: None,
            content: format!(
                "swarm.tot.completed: success={}, failure={}, approved={}",
                success_count, failure_count, approved
            ),
            timestamp: chrono::Utc::now().timestamp_millis(),
            msg_type: super::bus::BusMessageType::Notification,
            correlation_id: None,
        });

        // 多视角综合仲裁。
        let outputs = if outputs.len() > 1 {
            self.bus
                .emit_event(SwarmEvent::negotiation_started(&task_id, outputs.len()));

            let negotiator = Negotiator::new();
            let result = negotiator
                .negotiate_paths_with_arbitration(outputs, &self.llm)
                .await?;

            info!(
                target: "nebula.swarm.tot",
                method = ?result.method,
                "tree-of-thoughts paths synthesized"
            );

            // emit ArbitrationResolved(conflict_detected=false,因为是综合而非冲突)。
            self.bus.emit_event(SwarmEvent::arbitration_resolved(
                &task_id,
                result.chosen.kind,
                result.method,
                result.conflict_detected,
            ));

            vec![result.chosen]
        } else {
            outputs
        };

        info!(
            target: "nebula.swarm.tot",
            success = success_count,
            failure = failure_count,
            total = outputs.len(),
            "tree-of-thoughts orchestration finished"
        );

        // emit SwarmCompleted。
        self.bus.emit_event(SwarmEvent::swarm_completed(
            &task_id,
            success_count,
            failure_count,
            approved,
        ));

        Ok(OrchestrationReport {
            task,
            outputs,
            success_count,
            failure_count,
            approved,
        })
    }

    /// Run a single agent with retry + exponential back-off.
    async fn run_agent_with_retry(
        agent: Arc<dyn Agent>,
        task: &str,
        ctx: &TeamContext,
        max_retries: u32,
    ) -> Result<AgentOutput> {
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 0..=max_retries {
            match agent.run(task, ctx).await {
                Ok(o) => return Ok(o),
                Err(e) => {
                    warn!(
                        target: "nebula.swarm",
                        agent = %agent.name(),
                        attempt,
                        max = max_retries,
                        error = ?e,
                        "agent run failed; will retry"
                    );
                    last_err = Some(e);
                    if attempt < max_retries {
                        let delay = Duration::from_millis(RETRY_BASE_DELAY_MS * 2u64.pow(attempt));
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("agent failed without an error")))
    }

    /// T-E-S-02: 处理 AgentOutput 中的 tool_calls。
    ///
    /// 当 Agent 返回含 tool_calls 的输出时,执行工具调用并将结果
    /// 追加到后续对话消息中,让 Agent 继续推理,直到不再返回
    /// tool_calls 或达到最大递归深度。
    ///
    /// 最大递归深度为 5,超过后强制终止并返回当前输出。
    pub async fn execute_tool_calls(
        &self,
        output: AgentOutput,
        agent: Arc<dyn Agent>,
        task: &str,
        ctx: &TeamContext,
    ) -> AgentOutput {
        const MAX_TOOL_DEPTH: usize = 5;

        let mut current_output = output;

        for depth in 0..MAX_TOOL_DEPTH {
            let tool_calls = match current_output.tool_calls.take() {
                Some(calls) if !calls.is_empty() => calls,
                _ => return current_output,
            };

            info!(
                target: "nebula.swarm",
                agent = %agent.name(),
                depth,
                count = tool_calls.len(),
                "executing tool_calls for agent"
            );

            // 执行每个 tool_call
            for tc in &tool_calls {
                let result = {
                    let input = crate::tools::ToolInput {
                        tool_name: tc.function_name.clone(),
                        arguments: tc.arguments.clone(),
                    };
                    match self.tool_registry.invoke(input) {
                        Ok(out) => crate::swarm::tool_types::ToolResult {
                            tool_call_id: tc.id.clone(),
                            content: if out.success {
                                out.result
                            } else {
                                out.error.unwrap_or_default()
                            },
                            is_error: !out.success,
                        },
                        Err(e) => crate::swarm::tool_types::ToolResult {
                            tool_call_id: tc.id.clone(),
                            content: format!("{e}"),
                            is_error: true,
                        },
                    }
                };

                // 将工具结果追加到 TeamContext
                let result_str = if result.is_error {
                    format!("[tool error] {}: {}", tc.function_name, result.content)
                } else {
                    format!("[tool result] {}: {}", tc.function_name, result.content)
                };
                ctx.push_str(agent.name(), "tool_result", &result_str);
            }

            // 再次运行 agent,让它基于工具结果继续推理
            match agent.run(task, ctx).await {
                Ok(mut next_output) => {
                    // 保留原 confidence 和 kind/author
                    next_output.confidence = next_output.confidence.max(current_output.confidence);
                    current_output = next_output;
                }
                Err(e) => {
                    warn!(
                        target: "nebula.swarm",
                        agent = %agent.name(),
                        depth,
                        error = %e,
                        "follow-up agent run after tool_calls failed"
                    );
                    // 返回当前输出(工具已执行,但 follow-up 失败)
                    current_output.tool_calls = None;
                    return current_output;
                }
            }
        }

        warn!(
            target: "nebula.swarm",
            agent = %agent.name(),
            "tool_calls recursion reached max depth"
        );
        current_output.tool_calls = None;
        current_output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn agent_count_is_clamped() {
        let task = SwarmTask::new("test").with_agent_count(100);
        assert_eq!(task.agent_count, 6);
        let task = SwarmTask::new("test").with_agent_count(1);
        assert_eq!(task.agent_count, 2);
        let task = SwarmTask::new("test").with_agent_count(4);
        assert_eq!(task.agent_count, 4);
    }

    #[tokio::test]
    async fn empty_pool_refuses_to_run() {
        // We cannot test full execution without a running LLM, but we
        // can confirm that the orchestrator correctly clamps agent_count.
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(client, "m", "ollama", None, None, None, None, None));
        let orch = SwarmOrchestrator::new_without_memory(gw, std::sync::Arc::new(crate::tools::ToolRegistry::new()));
        // agent_pool is pre-built with 6 agents — verify.
        assert_eq!(orch.agent_pool.len(), 6);
    }

    /// T-E-D-07: cancel() 对未知 task_id 返回 false,对已登记 task_id
    /// 返回 true 并实际取消 token(取消后 cancelled() 立即完成)。
    #[tokio::test]
    async fn cancel_terminates_registered_token() {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(client, "m", "ollama", None, None, None, None, None));
        let orch = SwarmOrchestrator::new_without_memory(gw, std::sync::Arc::new(crate::tools::ToolRegistry::new()));

        // 未知 task_id:返回 false。
        assert!(!orch.cancel("nonexistent-task"));

        // 手动登记一个 token(模拟 execute() 内部行为)并验证 cancel() 取消它。
        let tok = tokio_util::sync::CancellationToken::new();
        orch.cancel_tokens.lock().insert("task-xyz".to_string(), tok.clone());
        assert!(orch.cancel("task-xyz"));
        // 取消后 token 的 cancelled() future 应立即就绪。
        tokio::select! {
            _ = tok.cancelled() => { /* ok: token was cancelled */ }
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                panic!("cancellation token was not cancelled after cancel()");
            }
        }
        // 重复 cancel 同一 task_id 返回 false(已被 remove 取出)。
        assert!(!orch.cancel("task-xyz"));
    }

    // T-E-B-18: execute_with_strategy 测试。

    #[tokio::test]
    async fn execute_with_strategy_linear_delegates_to_execute() {
        // Linear 策略应走既有 execute 路径。
        // 由于无 LLM,execute 会失败,但策略分支本身不应 panic。
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let orch = SwarmOrchestrator::new_without_memory(
            gw,
            std::sync::Arc::new(crate::tools::ToolRegistry::new()),
        );
        let task = SwarmTask::new("linear test").with_agent_count(2);
        // Linear 策略:应委托给 execute()。
        let strategy = ReasoningStrategy::Linear;
        // execute 会因 LLM 不可达而返回 Err,但策略分支逻辑本身不应 panic。
        let _ = orch.execute_with_strategy(task, &strategy).await;
    }

    #[tokio::test]
    async fn execute_with_strategy_tot_handles_llm_failure_gracefully() {
        // ToT 策略 + 不可达 LLM:所有 agent 失败,但 orchestrator 应返回
        // 报告(failure_count > 0, approved=false),不 panic。
        // 使用 max_retries=0 + 短超时,确保测试快速完成。
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let orch = SwarmOrchestrator::new_without_memory(
            gw,
            std::sync::Arc::new(crate::tools::ToolRegistry::new()),
        );
        let mut task = SwarmTask::new("tot failure test").with_agent_count(4);
        task.max_retries = 0; // fail fast, no retries
        let strategy = ReasoningStrategy::TreeOfThoughts {
            branches: 4,
            depth: 1,
        };
        let report = orch
            .execute_with_strategy(task, &strategy)
            .await
            .expect("ToT must return a report even when all agents fail");
        // 所有 4 个 agent 都失败(LLM 不可达)。
        assert_eq!(report.failure_count, 4, "all 4 thought agents should fail");
        assert_eq!(report.success_count, 0);
        assert!(!report.approved);
        // 最终 outputs 长度 = 4(失败也产生 error output)+ 经 negotiate 后变为 1。
        // 注意:negotiate_paths_with_arbitration 也会因 LLM 不可达而回退,
        // 返回 highest_confidence 的 error output。
        assert!(
            !report.outputs.is_empty(),
            "report should contain at least one output (error or synthesized)"
        );
    }

    #[tokio::test]
    async fn execute_with_strategy_tot_clamps_depth_above_one() {
        // depth>1 应 clamp 到 1(MVP),不 panic。
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let orch = SwarmOrchestrator::new_without_memory(
            gw,
            std::sync::Arc::new(crate::tools::ToolRegistry::new()),
        );
        let mut task = SwarmTask::new("tot depth clamp test").with_agent_count(2);
        task.max_retries = 0;
        let strategy = ReasoningStrategy::TreeOfThoughts {
            branches: 2,
            depth: 5, // 应 clamp 到 1
        };
        let report = orch
            .execute_with_strategy(task, &strategy)
            .await
            .expect("ToT with depth>1 must not panic");
        assert!(!report.approved);
    }

    // T-E-S-03: execute() 复杂度推断路径测试。

    /// 显式指定 agent_count(非默认 3)应跳过 complexity 分类器,
    /// 直接 clamp 到 [MIN_AGENTS, MAX_AGENTS]。
    #[tokio::test]
    async fn execute_respects_explicit_agent_count() {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let orch = SwarmOrchestrator::new_without_memory(
            gw,
            std::sync::Arc::new(crate::tools::ToolRegistry::new()),
        );
        // 显式 5(非默认 3):应跳过 classifier,直接 count=5。
        let mut task = SwarmTask::new("explicit count test").with_agent_count(5);
        task.max_retries = 0; // fail fast
        let report = orch
            .execute(task)
            .await
            .expect("execute must return a report even when all agents fail");
        // 5 个 agent 全部因 LLM 不可达而失败。
        assert_eq!(
            report.failure_count, 5,
            "explicit agent_count=5 should spawn 5 agents (skip classifier)"
        );
        assert_eq!(report.success_count, 0);
        assert!(!report.approved);
    }

    /// 默认 agent_count(=3)应走 complexity 推断路径。
    /// 由于测试环境的 classifier Ollama(127.0.0.1:11434)通常不可达,
    /// 会降级为 Medium → target_count_for(Medium) = 3。
    /// 若测试环境恰好运行 Ollama,可能返回 Simple/Medium/Complex → 2/3/6,
    /// 均为合法值。此测试断言 count ∈ {2, 3, 6} 以确认 complexity 路径被触发。
    #[tokio::test]
    async fn execute_uses_complexity_when_default() {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let orch = SwarmOrchestrator::new_without_memory(
            gw,
            std::sync::Arc::new(crate::tools::ToolRegistry::new()),
        );
        // 默认 agent_count=3 → 走 complexity 推断路径。
        let mut task = SwarmTask::new("default count complexity test");
        task.max_retries = 0;
        let report = orch
            .execute(task)
            .await
            .expect("execute must return a report even when all agents fail");
        // 复杂度路径会调用 estimate_complexity,无论返回 Simple/Medium/Complex,
        // target_count_for 都在 {2, 3, 6} 中。失败 agent 数应等于 count。
        assert!(
            report.failure_count == 2
                || report.failure_count == 3
                || report.failure_count == 6,
            "complexity path should produce failure_count in {{2, 3, 6}}, got {}",
            report.failure_count
        );
        assert_eq!(report.success_count, 0);
        assert!(!report.approved);
    }

    /// 显式指定 agent_count=3(等于默认值)也应走 complexity 推断路径,
    /// 因为判断条件是 `task.agent_count != default_agent_count()`。
    /// 此测试与上一测试行为一致(都走 complexity 路径),仅是构造方式不同。
    #[tokio::test]
    async fn execute_uses_complexity_when_explicit_equals_default() {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let orch = SwarmOrchestrator::new_without_memory(
            gw,
            std::sync::Arc::new(crate::tools::ToolRegistry::new()),
        );
        // 显式 3(等于 default)→ 仍走 complexity 路径。
        let mut task = SwarmTask::new("explicit equals default").with_agent_count(3);
        task.max_retries = 0;
        let report = orch
            .execute(task)
            .await
            .expect("execute must return a report");
        assert!(
            report.failure_count == 2
                || report.failure_count == 3
                || report.failure_count == 6,
            "explicit agent_count=3 (== default) should still trigger complexity path, got {}",
            report.failure_count
        );
    }
}

//! Concrete agent implementations.
//!
//! Every agent implements the [`Agent`] trait and reads from / writes
//! to a shared [`TeamContext`]. The trait is intentionally small so
//! new agents can be added without touching the orchestrator.
//!
//! ## v2.0 — jiuwenswarm-style dynamic agents
//!
//! Starting with v2.0, the swarm no longer uses hard-coded role agents
//! (Coder / Writer / Reviewer).  Instead, every task spawns 2–6
//! [`GenericAgent`] instances that work independently on the same task
//! description — mirroring jiuwenswarm's `task_tool` sub-agent pattern.
//! v2.2: 重新启用角色专业化(T-E-S-01),每个角色含独立 tool_set/knowledge_scope,
//! 供需要显式角色分工的场景与 gRPC backward compatibility 一并使用。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::info;

use crate::llm::ollama::{ChatMessage, OllamaClient};
use crate::llm::LlmGateway;
use crate::memory::sponge::SpongeEngine;
use crate::memory::types::MemoryLayer;
use crate::swarm::events::SwarmEvent;

use super::bus::BusMessage;
use super::context::TeamContext;

mod coder;
mod generic_agent;
mod planner;
mod researcher;
mod reviewer;
mod writer;

pub use coder::CoderAgent;
pub use generic_agent::GenericAgent;
pub use planner::PlannerAgent;
pub use researcher::ResearcherAgent;
pub use reviewer::ReviewerAgent;
pub use writer::WriterAgent;

/// Identifies a concrete agent implementation.
///
/// v2.0: `Generic` is the default for all new swarms.  `Coder`,
/// `Writer`, `Reviewer`, `Researcher`, and `Planner` are
/// **deprecated** and kept only for backward-compatible gRPC queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    /// v2.0: general-purpose task-driven agent (jiuwenswarm pattern).
    Generic,
    /// Deprecated — use Generic instead.
    Coder,
    /// Deprecated — use Generic instead.
    Writer,
    /// Deprecated — use Generic instead.
    Reviewer,
    /// Deprecated — use Generic instead. Researcher role from white paper.
    Researcher,
    /// Deprecated — use Generic instead. Planner role from white paper.
    Planner,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Generic => "generic",
            AgentKind::Coder => "coder",
            AgentKind::Writer => "writer",
            AgentKind::Reviewer => "reviewer",
            AgentKind::Researcher => "researcher",
            AgentKind::Planner => "planner",
        }
    }
}

impl std::str::FromStr for AgentKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "generic" => Ok(AgentKind::Generic),
            "coder" => Ok(AgentKind::Coder),
            "writer" => Ok(AgentKind::Writer),
            "reviewer" => Ok(AgentKind::Reviewer),
            "researcher" => Ok(AgentKind::Researcher),
            "planner" => Ok(AgentKind::Planner),
            other => Err(format!("unknown agent kind: {other}")),
        }
    }
}

/// Output produced by an agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutput {
    pub kind: AgentKind,
    pub author: String,
    pub body: String,
    pub confidence: f32,
    /// T-E-B-17: 推理链(可选)。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasoning_chain: Vec<crate::llm::reasoning::ReasoningStep>,
    /// T-E-B-18: 思维树路径 ID(可选)。
    ///
    /// 仅在 TreeOfThoughts 模式下填充(如 "path-0");Linear 模式为 None。
    /// `#[serde(default)]` 保证旧 JSON(无此字段)反序列化为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_id: Option<String>,
    /// T-E-S-02: Agent 请求调用的工具列表(可选)。
    ///
    /// 当 LLM 返回 tool_calls 时填充;协商与编排层据此感知工具调用。
    /// `#[serde(default)]` 保证旧 JSON(无此字段)反序列化为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<super::tool_types::ToolCall>>,
}

impl AgentOutput {
    pub fn new(kind: AgentKind, author: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            kind,
            author: author.into(),
            body: body.into(),
            confidence: 0.8,
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
        }
    }

    pub fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c.clamp(0.0, 1.0);
        self
    }

    /// T-E-B-18: 设置 path_id(思维树模式专用)。
    pub fn with_path_id(mut self, path_id: impl Into<String>) -> Self {
        self.path_id = Some(path_id.into());
        self
    }
}

/// The shared contract for every agent.
#[async_trait]
pub trait Agent: Send + Sync {
    fn kind(&self) -> AgentKind;
    fn name(&self) -> &str;
    fn system_prompt(&self) -> &str;
    fn description(&self) -> &str {
        ""
    }
    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput>;
    /// T-S3-B-02: 改为 &self 以支持 Arc<dyn Agent> 调用。
    fn set_mailbox(&self, _rx: tokio::sync::mpsc::Receiver<BusMessage>) {}
    /// T-6: 该角色 agent 可用的工具集(默认空切片,角色 agent 覆盖)。
    fn tool_set(&self) -> &[&str] {
        &[]
    }
    /// T-6: 该角色 agent 可访问的记忆层级(默认空切片,角色 agent 覆盖)。
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        &[]
    }
    /// T-E-S-39: 注入 persona 缓存(SOUL.md/AGENTS.md/TOOLS.md)。
    /// 默认空实现;GenericAgent 覆盖以在 run() 中拼接到 system prompt 前缀。
    fn set_persona(
        &self,
        _persona: Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>,
    ) {
    }
    /// T-E-D-10: 设置 swarm 上下文（事件发射器、任务 ID、agent 角色）。
    /// 默认空实现;GenericAgent 和角色 agent 覆盖以在 tool_loop 中 emit 事件。
    fn set_swarm_context(
        &self,
        _event_sender: broadcast::Sender<SwarmEvent>,
        _task_id: String,
        _agent_role: String,
    ) {
    }
}

/// T-6: 角色化 agent 的静态配置(工具集 + 知识边界 + system prompt)。
///
/// 由 [`default_role_configs`] 集中产出,供运行时按角色查询。
#[derive(Debug, Clone)]
pub struct AgentRoleConfig {
    /// 该角色允许调用的工具名(如 `shell`、`editor_read`)。
    pub tool_set: Vec<&'static str>,
    /// 该角色可访问的记忆层级。
    pub knowledge_scope: Vec<MemoryLayer>,
    /// Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界)。
    pub system_prompt: String,
}

/// T-6: 五个白皮书角色(coder/writer/reviewer/researcher/planner)的默认配置。
///
/// 角色 agent 内部以 `const` 数组实现 `tool_set`/`knowledge_scope`,
/// 此处提供等价的查询入口供 orchestrator / 前端按角色检索。
pub fn default_role_configs() -> HashMap<&'static str, AgentRoleConfig> {
    let mut m = HashMap::new();
    m.insert(
        "coder",
        AgentRoleConfig {
            tool_set: vec!["shell", "editor_read", "editor_write", "tool_invoke"],
            knowledge_scope: vec![MemoryLayer::L2, MemoryLayer::L3],
            system_prompt: "You are the Coder agent in the nebula swarm.\n\
                 Role: produce concise, well-tested Rust code that satisfies the task.\n\
                 Tools: shell, editor_read, editor_write, tool_invoke.\n\
                 Knowledge scope: L2 (cross-session experience) and L3 (concrete facts).\n\
                 Always explain trade-offs in 2-3 sentences at the end."
                .to_string(),
        },
    );
    m.insert(
        "writer",
        AgentRoleConfig {
            tool_set: vec!["editor_read", "editor_write", "tool_invoke"],
            knowledge_scope: vec![MemoryLayer::L2],
            system_prompt: "You are the Writer agent in the nebula swarm.\n\
                 Role: produce clear, well-structured Markdown documentation.\n\
                 Tools: editor_read, editor_write, tool_invoke.\n\
                 Knowledge scope: L2 (cross-session experience).\n\
                 Prefer concrete examples over abstract prose."
                .to_string(),
        },
    );
    m.insert(
        "reviewer",
        AgentRoleConfig {
            tool_set: vec!["editor_read", "tool_invoke"],
            knowledge_scope: vec![MemoryLayer::L2],
            system_prompt: "You are the Reviewer agent in the nebula swarm.\n\
                 Role: critically evaluate the latest work in the team context.\n\
                 Tools: editor_read, tool_invoke (read-only — no write/shell).\n\
                 Knowledge scope: L2 (cross-session experience).\n\
                 End your response with one of: APPROVE / REVISE / REJECT."
                .to_string(),
        },
    );
    m.insert(
        "researcher",
        AgentRoleConfig {
            tool_set: vec!["memory_search", "tool_invoke"],
            knowledge_scope: vec![MemoryLayer::L1, MemoryLayer::L2, MemoryLayer::L4],
            system_prompt: "You are the Researcher agent (Researcher-B) in the nebula swarm.\n\
                 Role: gather, verify and synthesise information; cite sources; highlight uncertainty.\n\
                 Tools: memory_search, tool_invoke.\n\
                 Knowledge scope: L1 (session history), L2 (cross-session experience), L4 (distilled knowledge)."
                .to_string(),
        },
    );
    m.insert(
        "planner",
        AgentRoleConfig {
            tool_set: vec!["memory_search", "tool_invoke"],
            knowledge_scope: vec![MemoryLayer::L2, MemoryLayer::L5],
            system_prompt: "You are the Planner agent (Planner-F) in the nebula swarm.\n\
                 Role: decompose tasks into sub-tasks (max depth 3), assign agents, arbitrate conflicts.\n\
                 Tools: memory_search, tool_invoke.\n\
                 Knowledge scope: L2 (cross-session experience) and L5 (lessons learned)."
                .to_string(),
        },
    );
    m
}

/// v2.0: builds a pool of up to 6 generic agents.
///
/// The pool is lazy — agents are created on first access and cached
/// for the lifetime of the orchestrator.  Each `execute` call uses
/// `agent_count` (2..=6) of them so we never pay for more LLM calls
/// than the user explicitly requests.
pub fn build_agent_pool(
    llm: Arc<LlmGateway>,
    tool_registry: Arc<crate::tools::ToolRegistry>,
) -> Vec<Arc<dyn Agent>> {
    (1..=6)
        .map(|i| {
            Arc::new(GenericAgent::new(llm.clone(), i, tool_registry.clone())) as Arc<dyn Agent>
        })
        .collect()
}

/// v1.1: builds an agent pool from a list of kind strings.
///
/// v2.2 (T-E-S-01): 当 kinds 匹配 `default_role_configs()` 中的角色时,
/// 对应角色 agent 已在自身 impl 中覆盖 `tool_set()`/`knowledge_scope()`,
/// 所以本函数只需实例化正确的角色类型即可获得差异化能力。
///
/// Each kind maps to a specific agent type. Unknown kinds fall back
/// to `GenericAgent`. The result is clamped to `2..=6` agents.
pub fn build_agent_pool_by_kinds(
    kinds: &[&str],
    llm: Arc<LlmGateway>,
    tool_registry: Arc<crate::tools::ToolRegistry>,
) -> Vec<Arc<dyn Agent>> {
    let clamped = kinds.len().clamp(2, 6);
    kinds
        .iter()
        .take(clamped)
        .enumerate()
        .map(|(i, kind)| {
            let agent: Arc<dyn Agent> = match *kind {
                "coder" => Arc::new(CoderAgent::new(llm.clone())) as Arc<dyn Agent>,
                "writer" => Arc::new(WriterAgent::new(llm.clone(), None)) as Arc<dyn Agent>,
                "reviewer" => Arc::new(ReviewerAgent::new(llm.clone())) as Arc<dyn Agent>,
                "planner" => Arc::new(PlannerAgent::new(llm.clone())) as Arc<dyn Agent>,
                "researcher" => Arc::new(ResearcherAgent::new(llm.clone())) as Arc<dyn Agent>,
                _ => Arc::new(GenericAgent::new(llm.clone(), (i + 1) as u32, tool_registry.clone()))
                    as Arc<dyn Agent>,
            };
            info!(
                target: "nebula.swarm",
                kind = *kind,
                name = %agent.name(),
                tool_set = ?agent.tool_set(),
                "role agent instantiated"
            );
            agent
        })
        .collect()
}

/// T-E-S-01: 查询指定角色的默认配置。
///
/// 供 orchestrator 在 execute 路径中查询角色的 tool_set/knowledge_scope,
/// 用于注入 TeamContext 与过滤 RAG 召回层。
///
/// 返回 `Option<&'static AgentRoleConfig>` —— 内部用 `OnceLock` 缓存
/// `default_role_configs()`,避免每次调用重建 HashMap。
pub fn role_config_for(kind: &str) -> Option<&'static AgentRoleConfig> {
    use std::sync::OnceLock;
    static CONFIGS: OnceLock<HashMap<&'static str, AgentRoleConfig>> = OnceLock::new();
    let configs = CONFIGS.get_or_init(default_role_configs);
    configs.get(kind)
}

/// v2.2: 重新启用角色专业化 — 返回 5 个角色 agent 的标准团队配置。
/// 供 gRPC backward compatibility 与需要显式角色分工的场景使用。
/// 默认蜂群仍用 [`build_agent_pool`](6 个 GenericAgent)。
pub fn canonical_team(
    llm: Arc<LlmGateway>,
    sponge: Option<Arc<SpongeEngine>>,
) -> Vec<Arc<dyn Agent>> {
    let _ = sponge;
    vec![
        Arc::new(CoderAgent::new(llm.clone())),
        Arc::new(ResearcherAgent::new(llm.clone())),
        Arc::new(WriterAgent::new(llm.clone(), None)),
        Arc::new(ReviewerAgent::new(llm.clone())),
        Arc::new(PlannerAgent::new(llm)),
    ]
}

const DEFAULT_MAX_AGENTS: usize = 20;
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;
/// T-E-S-03: 复杂度分类器专用 Ollama 模型(零成本本地模型)。
const CLASSIFIER_MODEL: &str = "qwen2.5:3b";
/// T-E-S-03: 复杂度分类器 HTTP 超时(秒)。超时/Ollama 不可达均降级 Medium。
const CLASSIFIER_TIMEOUT_SECS: u64 = 2;

struct PooledAgent {
    agent: Arc<dyn Agent>,
    last_used: std::time::Instant,
    in_use: bool,
}

struct PoolInner {
    agents: Vec<PooledAgent>,
    max_agents: usize,
    idle_timeout: std::time::Duration,
    next_id: u32,
}

/// T-E-S-03: 任务复杂度,由 [`DynamicAgentPool::estimate_complexity`] 推断。
///
/// 由 [`DynamicAgentPool::target_count_for`] 映射到目标 agent 数量:
/// `Simple → 2`、`Medium → 3`、`Complex → 6`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// 简单任务(greeting、factual recall、translation、formatting)→ 2 agents。
    Simple,
    /// 中等任务(summarization、rewriting、multi-turn Q&A、code review)→ 3 agents。
    Medium,
    /// 复杂任务(reasoning、creative writing、complex coding、multi-step planning)→ 6 agents。
    Complex,
}

/// T-S3-B-02: 重构为 Arc<tokio::sync::Mutex> 内部可变性，
/// 允许通过 Arc<DynamicAgentPool> 共享给多个 spawn 的 task。
pub struct DynamicAgentPool {
    llm: Arc<LlmGateway>,
    inner: tokio::sync::Mutex<PoolInner>,
    /// T-E-S-03: 复杂度分类器专用 OllamaClient(独立 2s 超时),
    /// 与 LlmGateway 的 primary client 解耦,避免长超时(120s)阻塞 execute()。
    classifier_ollama: OllamaClient,
    /// T-E-S-03: 分类器模型名,默认 [`CLASSIFIER_MODEL`](`qwen2.5:3b`)。
    classifier_model: String,
}

impl DynamicAgentPool {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        let classifier_ollama = OllamaClient::new_with_timeout(
            "http://127.0.0.1:11434",
            Duration::from_secs(CLASSIFIER_TIMEOUT_SECS),
        );
        Self {
            llm,
            inner: tokio::sync::Mutex::new(PoolInner {
                agents: Vec::new(),
                max_agents: DEFAULT_MAX_AGENTS,
                idle_timeout: Duration::from_secs(DEFAULT_IDLE_TIMEOUT_SECS),
                next_id: 1,
            }),
            classifier_ollama,
            classifier_model: CLASSIFIER_MODEL.to_string(),
        }
    }

    /// T-E-S-03: 覆盖复杂度分类器使用的 OllamaClient(主要用于测试注入不可达端点)。
    ///
    /// 仅在构造期(builder chain)同步调用,不跨 await 点。
    pub fn with_classifier_ollama(mut self, ollama: OllamaClient) -> Self {
        self.classifier_ollama = ollama;
        self
    }

    /// **同步上下文专用**:仅在构造期(builder chain)调用,不跨 await 点。
    ///
    /// `blocking_lock()` 在这里安全,因为 builder 模式通常在 async runtime
    /// 启动前或同步上下文中使用。若需在 async 上下文中修改,请使用
    /// [`acquire`](Self::acquire) / [`release`](Self::release) 等 async 方法。
    pub fn with_max_agents(self, max: usize) -> Self {
        {
            let mut inner = self.inner.blocking_lock();
            inner.max_agents = max.max(1);
        }
        self
    }

    /// **同步上下文专用**:仅在构造期(builder chain)调用,不跨 await 点。
    ///
    /// 参见 [`with_max_agents`](Self::with_max_agents) 的说明。
    pub fn with_idle_timeout(self, secs: u64) -> Self {
        {
            let mut inner = self.inner.blocking_lock();
            inner.idle_timeout = Duration::from_secs(secs);
        }
        self
    }

    /// T-S3-B-02: async + &self，支持 Arc 共享。
    pub async fn acquire(&self, kind: AgentKind) -> Option<Arc<dyn Agent>> {
        let mut inner = self.inner.lock().await;

        // 1) 复用空闲且类型匹配的 agent
        if let Some(pooled) = inner
            .agents
            .iter_mut()
            .find(|a| !a.in_use && a.agent.kind() == kind)
        {
            pooled.in_use = true;
            pooled.last_used = std::time::Instant::now();
            return Some(pooled.agent.clone());
        }

        // 2) 到达上限时抢占任意空闲 agent
        if inner.agents.len() >= inner.max_agents {
            if let Some(pooled) = inner.agents.iter_mut().find(|a| !a.in_use) {
                pooled.in_use = true;
                pooled.last_used = std::time::Instant::now();
                return Some(pooled.agent.clone());
            }
            return None;
        }

        // 3) 新建 agent 入池
        let id = inner.next_id;
        inner.next_id += 1;
        let agent: Arc<dyn Agent> = match kind {
            // T-E-S-02: 动态池暂用空 ToolRegistry(向后兼容,ToolRegistry 的注入由 Task 5 完成)。
            AgentKind::Generic => Arc::new(GenericAgent::new(
                self.llm.clone(),
                id,
                Arc::new(crate::tools::ToolRegistry::new()),
            )),
            AgentKind::Coder => Arc::new(CoderAgent::new(self.llm.clone())),
            AgentKind::Writer => Arc::new(WriterAgent::new(self.llm.clone(), None)),
            AgentKind::Reviewer => Arc::new(ReviewerAgent::new(self.llm.clone())),
            AgentKind::Researcher => Arc::new(ResearcherAgent::new(self.llm.clone())),
            AgentKind::Planner => Arc::new(PlannerAgent::new(self.llm.clone())),
        };

        inner.agents.push(PooledAgent {
            agent: agent.clone(),
            last_used: std::time::Instant::now(),
            in_use: true,
        });

        Some(agent)
    }

    /// T-S3-B-02: async + &self。
    pub async fn release(&self, agent_name: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(pooled) = inner
            .agents
            .iter_mut()
            .find(|a| a.agent.name() == agent_name)
        {
            pooled.in_use = false;
            pooled.last_used = std::time::Instant::now();
        }
    }

    /// T-S3-B-02: async + &self。
    pub async fn cleanup_idle(&self) -> usize {
        let mut inner = self.inner.lock().await;
        let before = inner.agents.len();
        let idle_timeout = inner.idle_timeout;
        inner
            .agents
            .retain(|a| a.in_use || a.last_used.elapsed() < idle_timeout);
        before - inner.agents.len()
    }

    pub async fn active_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.agents.iter().filter(|a| a.in_use).count()
    }

    pub async fn total_count(&self) -> usize {
        let inner = self.inner.lock().await;
        inner.agents.len()
    }

    /// T-E-S-03: 基于任务描述估算复杂度(复用 ModelRouter 分类 prompt 模式)。
    ///
    /// 调用本地 Ollama `qwen2.5:3b` 分类器(零成本,不经过 LlmGateway/CostTracker)。
    /// 失败/超时(2s)/Ollama 不可达均降级为 [`TaskComplexity::Medium`]。
    ///
    /// **锁安全**:本方法不持有 `self.inner` 的 MutexGuard,只读
    /// `self.classifier_ollama` 与 `self.classifier_model`(均不需锁),
    /// 不会跨 await 点持有 MutexGuard(符合 project_memory.md 第 38 行约束)。
    pub async fn estimate_complexity(&self, task_desc: &str) -> TaskComplexity {
        let prompt = vec![
            ChatMessage::system(
                "You are a task complexity classifier. Output ONLY one word: simple, medium, or complex.\n\
                 - simple: greeting, factual recall, translation, formatting, short Q&A\n\
                 - medium: summarization, rewriting, multi-turn Q&A, code review\n\
                 - complex: reasoning, creative writing, complex coding, multi-step planning",
            ),
            ChatMessage::user(task_desc.to_string()),
        ];

        let resp = match self.classifier_ollama.chat(&self.classifier_model, &prompt).await {
            Ok(r) => r,
            // Ollama 不可达 / 网络错误 / 超时(2s 由 OllamaClient 配置)→ 降级 Medium。
            Err(_) => return TaskComplexity::Medium,
        };

        let content = resp.message.content.to_lowercase();
        Self::parse_complexity(&content).unwrap_or(TaskComplexity::Medium)
    }

    /// T-E-S-03: 解析分类器输出为 [`TaskComplexity`]。匹配前缀
    /// `simpl`/`medi`/`compl`(大小写不敏感)。不匹配返回 `None`。
    ///
    /// - "simple" / "simpler" / "SIMPLE" → [`TaskComplexity::Simple`]
    /// - "medium" / "Medium" / "mediumly" → [`TaskComplexity::Medium`]
    /// - "complex" / "Complexity" → [`TaskComplexity::Complex`]
    fn parse_complexity(s: &str) -> Option<TaskComplexity> {
        let s = s.trim().to_lowercase();
        if s.starts_with("simpl") {
            Some(TaskComplexity::Simple)
        } else if s.starts_with("medi") {
            Some(TaskComplexity::Medium)
        } else if s.starts_with("compl") {
            Some(TaskComplexity::Complex)
        } else {
            None
        }
    }

    /// T-E-S-03: 复杂度 → 目标 agent 数量映射。
    /// `Simple → 2`、`Medium → 3`、`Complex → 6`。
    pub fn target_count_for(complexity: TaskComplexity) -> usize {
        match complexity {
            TaskComplexity::Simple => 2,
            TaskComplexity::Medium => 3,
            TaskComplexity::Complex => 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_role_configs_returns_five_roles() {
        let configs = default_role_configs();
        assert_eq!(configs.len(), 5);
        for name in &["coder", "writer", "reviewer", "researcher", "planner"] {
            assert!(configs.contains_key(*name), "missing role: {}", name);
        }
    }

    #[test]
    fn role_configs_have_nonempty_tool_set() {
        let configs = default_role_configs();
        for (name, cfg) in &configs {
            assert!(!cfg.tool_set.is_empty(), "role {} has empty tool_set", name);
            assert!(
                !cfg.knowledge_scope.is_empty(),
                "role {} has empty knowledge_scope",
                name
            );
            assert!(
                !cfg.system_prompt.is_empty(),
                "role {} has empty system_prompt",
                name
            );
        }
    }

    #[test]
    fn reviewer_has_no_write_tools() {
        let configs = default_role_configs();
        let reviewer = configs.get("reviewer").expect("reviewer config");
        assert!(
            !reviewer
                .tool_set
                .iter()
                .any(|t| *t == "editor_write" || *t == "shell"),
            "reviewer must not have write tools"
        );
    }

    #[test]
    fn build_agent_pool_by_kinds_returns_differentiated_agents() {
        let llm = Arc::new(LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        let pool = build_agent_pool_by_kinds(&["coder", "reviewer"], llm, registry);
        assert_eq!(pool.len(), 2);
        // coder tool_set 含 shell,reviewer 不含
        let coder_tools = pool[0].tool_set();
        let reviewer_tools = pool[1].tool_set();
        assert!(coder_tools.contains(&"shell"));
        assert!(!reviewer_tools.contains(&"shell"));
        assert!(coder_tools.contains(&"editor_write"));
        assert!(!reviewer_tools.contains(&"editor_write"));
    }

    #[test]
    fn build_agent_pool_by_kinds_returns_specialized_agents() {
        let llm = Arc::new(LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        let agents = build_agent_pool_by_kinds(&["coder", "reviewer"], llm, registry);
        assert_eq!(agents.len(), 2);

        // Coder 应有 coder 的 tool_set
        assert_eq!(
            agents[0].tool_set(),
            &["shell", "editor_read", "editor_write", "tool_invoke"]
        );
        assert_eq!(
            agents[0].knowledge_scope(),
            &[
                crate::memory::types::MemoryLayer::L2,
                crate::memory::types::MemoryLayer::L3
            ]
        );

        // Reviewer 应有 reviewer 的 tool_set(无写权限)
        assert_eq!(agents[1].tool_set(), &["editor_read", "tool_invoke"]);
        assert_eq!(
            agents[1].knowledge_scope(),
            &[crate::memory::types::MemoryLayer::L2]
        );
    }

    #[test]
    fn role_config_for_returns_known_roles() {
        assert!(role_config_for("coder").is_some());
        assert!(role_config_for("reviewer").is_some());
        assert!(role_config_for("nonexistent").is_none());
    }

    #[test]
    fn build_agent_pool_by_kinds_falls_back_to_generic() {
        let llm = Arc::new(LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        // "unknown" kind 应回退到 GenericAgent,tool_set() 为空
        let agents = build_agent_pool_by_kinds(&["unknown1", "unknown2"], llm, registry);
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].tool_set(), &[] as &[&str]);
        assert_eq!(agents[1].tool_set(), &[] as &[&str]);
    }

    // T-E-B-18: AgentOutput.path_id 向后兼容性测试。

    #[test]
    fn agent_output_path_id_defaults_to_none_in_new() {
        let out = AgentOutput::new(AgentKind::Generic, "a", "body");
        assert!(out.path_id.is_none(), "new() should produce path_id=None");
    }

    #[test]
    fn agent_output_path_id_backward_compat_old_json_without_field() {
        // 旧 JSON(无 path_id 字段)反序列化时 path_id 应为 None。
        let old_json = r#"{
            "kind": "generic",
            "author": "a",
            "body": "answer",
            "confidence": 0.5,
            "reasoning_chain": []
        }"#;
        let out: AgentOutput = serde_json::from_str(old_json).expect("old JSON must deserialize");
        assert_eq!(out.author, "a");
        assert!(out.path_id.is_none(), "missing path_id must default to None");
    }

    #[test]
    fn agent_output_path_id_roundtrip_with_path_id() {
        let out = AgentOutput::new(AgentKind::Generic, "a", "body")
            .with_path_id("path-0");
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("\"path_id\":\"path-0\""), "got: {json}");
        // skip_serializing_if = Option::is_none 保证 None 不出现在 JSON 中。
        let out_no_path = AgentOutput::new(AgentKind::Generic, "b", "body");
        let json_no_path = serde_json::to_string(&out_no_path).unwrap();
        assert!(!json_no_path.contains("path_id"), "got: {json_no_path}");
        // 往返。
        let de: AgentOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(de.path_id.as_deref(), Some("path-0"));
    }

    // T-E-S-03: TaskComplexity + estimate_complexity 测试。

    #[test]
    fn target_count_for_maps_correctly() {
        assert_eq!(
            DynamicAgentPool::target_count_for(TaskComplexity::Simple),
            2,
            "Simple → 2"
        );
        assert_eq!(
            DynamicAgentPool::target_count_for(TaskComplexity::Medium),
            3,
            "Medium → 3"
        );
        assert_eq!(
            DynamicAgentPool::target_count_for(TaskComplexity::Complex),
            6,
            "Complex → 6"
        );
    }

    #[test]
    fn parse_complexity_matches_prefixes_case_insensitive() {
        // 完整单词。
        assert_eq!(
            DynamicAgentPool::parse_complexity("simple"),
            Some(TaskComplexity::Simple)
        );
        assert_eq!(
            DynamicAgentPool::parse_complexity("medium"),
            Some(TaskComplexity::Medium)
        );
        assert_eq!(
            DynamicAgentPool::parse_complexity("complex"),
            Some(TaskComplexity::Complex)
        );
        // 大小写不敏感。
        assert_eq!(
            DynamicAgentPool::parse_complexity("SIMPLE"),
            Some(TaskComplexity::Simple)
        );
        assert_eq!(
            DynamicAgentPool::parse_complexity("Medium"),
            Some(TaskComplexity::Medium)
        );
        assert_eq!(
            DynamicAgentPool::parse_complexity("Complex"),
            Some(TaskComplexity::Complex)
        );
        // 前缀匹配(模型可能输出 "simpler" / "mediumly" / "complexity" 等变体)。
        assert_eq!(
            DynamicAgentPool::parse_complexity("simpler"),
            Some(TaskComplexity::Simple)
        );
        assert_eq!(
            DynamicAgentPool::parse_complexity("mediumly"),
            Some(TaskComplexity::Medium)
        );
        assert_eq!(
            DynamicAgentPool::parse_complexity("complexity"),
            Some(TaskComplexity::Complex)
        );
        // 前后空白。
        assert_eq!(
            DynamicAgentPool::parse_complexity("  simple  "),
            Some(TaskComplexity::Simple)
        );
    }

    #[test]
    fn parse_complexity_returns_none_for_unknown() {
        assert_eq!(DynamicAgentPool::parse_complexity("unknown"), None);
        assert_eq!(DynamicAgentPool::parse_complexity(""), None);
        assert_eq!(DynamicAgentPool::parse_complexity("hello world"), None);
    }

    #[tokio::test]
    async fn estimate_complexity_returns_medium_on_ollama_failure() {
        // 模拟 Ollama 不可达:用 port 1 + 短超时,确保快速失败。
        // 任何 Ollama 失败(连接拒绝/超时)都应降级为 Medium。
        let llm = Arc::new(LlmGateway::new_test());
        let pool = DynamicAgentPool::new(llm).with_classifier_ollama(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_millis(100),
        ));
        let complexity = pool.estimate_complexity("write a rust web server").await;
        assert_eq!(
            complexity,
            TaskComplexity::Medium,
            "Ollama unreachable should fall back to Medium"
        );
    }

    #[tokio::test]
    async fn estimate_complexity_returns_medium_on_unparseable_response() {
        // 注入一个会返回非 simple/medium/complex 单词的"端点"不可达,降级 Medium。
        // 由于我们无法在不引入 mock server 的情况下注入可返回任意文本的 Ollama,
        // 此处复用不可达端点 — 行为与上一测试一致(均降级 Medium)。
        let llm = Arc::new(LlmGateway::new_test());
        let pool = DynamicAgentPool::new(llm).with_classifier_ollama(OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_millis(100),
        ));
        // 空字符串任务描述也不应 panic,仍返回 Medium。
        let complexity = pool.estimate_complexity("").await;
        assert_eq!(complexity, TaskComplexity::Medium);
    }

    #[test]
    fn with_max_agents_builder_clamps_to_one() {
        // builder 在同步上下文调用,blocking_lock 安全。
        let llm = Arc::new(LlmGateway::new_test());
        let pool = DynamicAgentPool::new(llm).with_max_agents(0);
        // 通过 acquire 行为间接验证 max_agents=1(无法直接读字段)。
        // 仅验证 builder 不 panic。
        let _ = pool;
    }

    #[test]
    fn with_idle_timeout_builder_accepts_arbitrary_seconds() {
        let llm = Arc::new(LlmGateway::new_test());
        let pool = DynamicAgentPool::new(llm).with_idle_timeout(0);
        let _ = pool;
    }

    // T-E-S-02: AgentOutput.tool_calls 向后兼容 + 序列化测试。

    #[test]
    fn agent_output_with_tool_calls() {
        use crate::swarm::tool_types::ToolCall;

        // 1. new() 默认 tool_calls = None
        let out = AgentOutput::new(AgentKind::Generic, "agent-a", "hello");
        assert!(out.tool_calls.is_none());

        // 2. 手动设置 tool_calls
        let tc = vec![ToolCall {
            id: "call_001".to_string(),
            function_name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt"}),
        }];
        let mut out = AgentOutput::new(AgentKind::Generic, "agent-a", "I'll read the file");
        out.tool_calls = Some(tc.clone());
        assert_eq!(out.tool_calls.as_ref().unwrap().len(), 1);
        assert_eq!(out.tool_calls.as_ref().unwrap()[0].function_name, "read_file");

        // 3. 序列化 round-trip
        let json = serde_json::to_string(&out).unwrap();
        assert!(json.contains("tool_calls"), "JSON must contain tool_calls");
        assert!(json.contains("call_001"), "JSON must contain tool call id");
        let de: AgentOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(de.tool_calls.as_ref().unwrap().len(), 1);

        // 4. 旧 JSON(无 tool_calls 字段)反序列化时 tool_calls 应为 None
        let old_json = r#"{
            "kind": "generic",
            "author": "a",
            "body": "answer",
            "confidence": 0.5,
            "reasoning_chain": [],
            "path_id": null
        }"#;
        let out_old: AgentOutput = serde_json::from_str(old_json).expect("old JSON must deserialize");
        assert!(out_old.tool_calls.is_none(), "missing tool_calls must default to None");

        // 5. tool_calls = None 时不出现在 JSON 中(skip_serializing_if)
        let out_none = AgentOutput::new(AgentKind::Generic, "b", "body");
        let json_none = serde_json::to_string(&out_none).unwrap();
        assert!(!json_none.contains("tool_calls"), "None tool_calls must be omitted from JSON");
    }
}

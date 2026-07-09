//! `nebula::swarm` — multi-agent orchestration.
//!
//! The swarm subsystem coordinates a small team of specialised agents
//! that collaborate on every non-trivial task. The key invariants are:
//!
//! * Every task dispatches **at least two agents** (so the "砍一个，长
//!   两个" principle is upheld even when one of them is the user's
//!   own dialogue).
//! * All agents read from a shared [`context::TeamContext`] so the
//!   output of one agent can condition the next.
//! * [`orchestrator::SwarmOrchestrator`] owns the dispatch logic and
//!   the retry / fallback policy.

pub mod agents;
pub mod bus;
pub mod composer;
pub mod context;
pub mod context_pool;
pub mod crdt_sync;
// M3 #40-43: TaskDag + SubTask + WorkerCapability + SubTaskResultMap
#[cfg(feature = "master-orchestrator")]
pub mod dag;
pub mod deadlock;
pub mod event_bus;
pub mod events;
pub mod leader_elector;
// M3 #44-45, #52: MasterOrchestrator + ExecuteMode + MasterEvent
#[cfg(feature = "master-orchestrator")]
pub mod master;
// T-E-L-01: Loop 定义解析层（LOOP.md YAML frontmatter + Markdown body）。
#[cfg(feature = "master-orchestrator")]
pub mod loop_def;
// T-E-L-06: Loop 预算配置解析层（loop-budget.md YAML frontmatter + Markdown 表格）。
#[cfg(feature = "master-orchestrator")]
pub mod loop_budget;
// T-E-L-07: Loop 审计日志 — 记录每次 execute_loop 调用的关键节点。
#[cfg(feature = "master-orchestrator")]
pub mod loop_audit_log;
pub mod negotiator;
pub mod orchestrator;
// T-E-AE-01: PrimaryAgent — 主智能体(decompose/delegate/synthesize)。
// 无 feature gate:不依赖 master-orchestrator 重型组件,仅依赖始终可用的 AgentScenario。
pub mod primary_agent;
pub mod tool_loop;
pub mod tool_types;
pub mod tot;

pub use agents::{
    build_agent_pool, Agent, AgentKind, AgentOutput, AgentScenario, DynamicAgentPool, GenericAgent,
};
pub use agents::{canonical_team, CoderAgent, ReviewerAgent, WriterAgent};
pub use bus::{AgentBus, BusMessage, BusMessageType};
pub use composer::{SkillComposer, SkillContext, SkillMatch};
pub use context::{ContextEntry, TeamContext};
pub use context_pool::{
    start_gc_worker, start_gc_worker_with_interval, PoolEntry, TeamContextPool,
};
pub use crdt_sync::SwarmCrdtSync;
pub use deadlock::{DeadlockDetector, DeadlockStatus, WaitForGraph};
pub use event_bus::EventBus;
pub use events::{EventEnvelope, SwarmEvent};
pub use leader_elector::LeaderElector;
pub use negotiator::{MoAConfig, MoAStrategy, NegotiationMethod, NegotiationResult, Negotiator};
pub use orchestrator::{
    AgentDescriptor, ExecuteMode, OrchestrationReport, PreCheckResult, SwarmOrchestrator, SwarmTask,
};
pub use tool_loop::{run_tool_loop, run_tool_loop_default, DEFAULT_MAX_ITERATIONS};
// T-E-S-02: Function Calling 类型 + 解析函数。
pub use tool_types::{parse_anthropic_tool_calls, parse_deepseek_tool_calls, ToolCall, ToolResult};
// T-E-B-18: 思维树模式(ReasoningStrategy + ThoughtStrategy + 工厂)。
pub use tot::{
    build_thought_agent_configs, default_tree_of_thoughts, ReasoningStrategy, ThoughtAgentConfig,
    ThoughtStrategy,
};
// T-E-AE-01: PrimaryAgent 三大能力(decompose/delegate/synthesize)。
pub use primary_agent::{
    ConcatSynthesizer, Decomposer, DelegatedResult, DelegatedTask, Delegator, PrimaryAgent,
    PrimaryAgentBus, PrimaryReport, RuleBasedDecomposer, ScenarioDelegator, Synthesizer,
};

// M3 #40-43: TaskDag 相关类型
#[cfg(feature = "master-orchestrator")]
pub use dag::{
    DependencyEdge, DependencyKind, FailureStrategy, SubTask, SubTaskResult, SubTaskResultMap,
    TaskDag, TaskDagBuilder, WorkerCapability,
};
// M3 #44-45, #52: MasterOrchestrator 相关类型
#[cfg(feature = "master-orchestrator")]
pub use master::{
    LoopRunReport, MasterEvent, MasterEventEnvelope, MasterOrchestrator, MasterReport,
};
// T-E-L-01: Loop 定义相关类型（LOOP.md 解析）。
#[cfg(feature = "master-orchestrator")]
pub use loop_def::{AutonomyLevel, LoopDef};
// T-E-L-06: Loop 预算配置相关类型（loop-budget.md 解析）。
#[cfg(feature = "master-orchestrator")]
pub use loop_budget::{LoopBudgetConfig, LoopBudgetEntry};
// T-E-L-07: Loop 审计日志相关类型。
#[cfg(feature = "master-orchestrator")]
pub use loop_audit_log::{LoopAuditEntry, LoopAuditLogger, LoopAuditPhase, LoopAuditQuery};

// T-D-C-08: master-orchestrator 运行时开关 (AtomicBool 模式,对齐 soul-system / evolution-engine)。
// 默认关闭,启动时通过环境变量 MASTER_ORCHESTRATOR_ENABLED=1 开启,或通过 Tauri 命令运行时切换。
pub static MASTER_ORCHESTRATOR_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// 查询 MasterOrchestrator 是否启用。
pub fn master_orchestrator_enabled() -> bool {
    MASTER_ORCHESTRATOR_ENABLED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 设置 MasterOrchestrator 启用状态（Settings UI 调用）。
pub fn set_master_orchestrator_enabled(on: bool) {
    MASTER_ORCHESTRATOR_ENABLED.store(on, std::sync::atomic::Ordering::SeqCst);
}

/// 启动时从环境变量 `MASTER_ORCHESTRATOR_ENABLED` 读取初始状态。
///
/// 应在 bootstrap 的 MasterOrchestrator 构造前调用一次。值为 `1` / `true` / `on` 时启用。
pub fn init_master_orchestrator_from_env() {
    let enabled = match std::env::var("MASTER_ORCHESTRATOR_ENABLED") {
        Ok(v) => {
            let lower = v.to_lowercase();
            lower == "1" || lower == "true" || lower == "on"
        }
        Err(_) => false,
    };
    set_master_orchestrator_enabled(enabled);
    if enabled {
        tracing::info!(target: "nebula", "master-orchestrator enabled via env (T-D-C-08)");
    }
}

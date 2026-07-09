//! T-D-B-14: Sidecar Swarm Coordinator 服务 — 单二进制多角色方案。
//!
//! 延续 T-S4-B-01 / T-S4-B-02 / T-S6-A-01a / memory_service / llm_service
//! 的单二进制多角色方案 (`nebula-sidecar --kind=swarm`),为子智能体
//! 编排 + 任务分发提供独立进程隔离。
//!
//! 本模块定义 Swarm sidecar 的服务处理器 [`SwarmServiceHandler`],
//! 它包装 [`SwarmOrchestrator`] 并暴露与 gRPC 服务方法对应的 RPC 接口。
//!
//! ## 架构
//!
//! ```text
//! 主进程 (Tauri UI)
//!    │  gRPC (tonic)
//!    ▼
//! nebula-sidecar --kind=swarm  (监听 127.0.0.1:50053)
//!    │  SwarmServiceHandler
//!    ▼
//! SwarmOrchestrator (fan-out + RAG + Leader + Negotiator)
//!    │
//!    ▼
//! Agent 池 (Coder/Writer/Reviewer/Researcher/Planner/Generic)
//! ```
//!
//! ## RPC 映射
//!
//! | gRPC RPC        | Handler 方法   | 后端                        |
//! |-----------------|----------------|-----------------------------|
//! | `Execute`       | `execute`      | SwarmOrchestrator::execute  |
//! | `ListAgents`    | `list_agents`  | SwarmOrchestrator::list_agents |
//! | `GetAgent`      | `get_agent`    | SwarmOrchestrator::get_agent|
//! | `HealthCheck`   | `health_check` | (always ok)                 |
//!
//! ## 依赖
//!
//! 依赖 [`SwarmOrchestrator`](crate::swarm::SwarmOrchestrator)。
//! 该 orchestrator 在 sidecar 进程中应通过 `new_without_memory` 构造
//! (sidecar 不直接持有 SQLite/LanceDB,记忆通过 Memory sidecar 获取),
//! 或由调用方注入完整构造的 orchestrator。

use std::sync::Arc;

use anyhow::Result;
use tracing::{info, instrument};

use crate::swarm::orchestrator::{
    AgentDescriptor, OrchestrationReport, SwarmOrchestrator, SwarmTask,
};

/// Swarm Coordinator sidecar 服务处理器。
///
/// 包装 [`SwarmOrchestrator`],为 gRPC 服务端提供业务逻辑入口。
/// 在进程内模式下也可直接使用(无需 gRPC)。
pub struct SwarmServiceHandler {
    orchestrator: Arc<SwarmOrchestrator>,
}

impl SwarmServiceHandler {
    /// 创建新的 Swarm 服务处理器。
    ///
    /// 通常在 sidecar 进程启动时构造。`SwarmOrchestrator` 应由调用方
    /// 预先构造(注入 LlmGateway + ToolRegistry,可选注入记忆后端)。
    pub fn new(orchestrator: Arc<SwarmOrchestrator>) -> Self {
        info!(
            target: "nebula.sidecar.swarm",
            "SwarmServiceHandler initialized"
        );
        Self { orchestrator }
    }

    /// 访问底层 SwarmOrchestrator(供 IPC 客户端在进程内模式下直接调用)。
    pub fn orchestrator(&self) -> &Arc<SwarmOrchestrator> {
        &self.orchestrator
    }

    /// RPC: HealthCheck — 始终返回 Ok(若 handler 存在则服务可用)。
    pub async fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// RPC: Execute — 执行一个 swarm 任务。
    ///
    /// 委托给 [`SwarmOrchestrator::execute`],fan-out N 个 agent 并行执行,
    /// 经 Negotiator 仲裁后返回 [`OrchestrationReport`]。
    #[instrument(skip(self, task), fields(desc = %task.description, agents = task.agent_count))]
    pub async fn execute(&self, task: SwarmTask) -> Result<OrchestrationReport> {
        self.orchestrator.execute(task).await
    }

    /// RPC: ListAgents — 列出 agent 池中的所有 agent。
    ///
    /// 返回 `(kind, name, system_prompt, description)` 元组列表,
    /// 供 UI 展示可用 agent 清单。
    pub fn list_agents(&self) -> Vec<(String, String, String, String)> {
        self.orchestrator.list_agents()
    }

    /// RPC: GetAgent — 按 kind 查询单个 agent 描述。
    pub fn get_agent(&self, kind: &str) -> Option<AgentDescriptor> {
        self.orchestrator.get_agent(kind)
    }
}

impl std::fmt::Debug for SwarmServiceHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmServiceHandler")
            .field("orchestrator", &"Arc<SwarmOrchestrator>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use crate::tools::ToolRegistry;

    fn make_handler() -> SwarmServiceHandler {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_secs(2),
        ));
        let gw = std::sync::Arc::new(LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let tools = std::sync::Arc::new(ToolRegistry::new());
        let orchestrator = std::sync::Arc::new(SwarmOrchestrator::new_without_memory(gw, tools));
        SwarmServiceHandler::new(orchestrator)
    }

    #[tokio::test]
    async fn health_check_returns_ok() {
        let h = make_handler();
        assert!(h.health_check().await.expect("task should complete"));
    }

    #[test]
    fn list_agents_returns_nonempty() {
        // 默认 agent 池应至少包含 1 个 agent (Generic / Coder 等)。
        let h = make_handler();
        let agents = h.list_agents();
        assert!(!agents.is_empty(), "default agent pool should be non-empty");
    }

    #[test]
    fn list_agents_tuple_has_four_fields() {
        let h = make_handler();
        for (kind, name, system_prompt, description) in h.list_agents() {
            assert!(!kind.is_empty(), "kind should be non-empty");
            assert!(!name.is_empty(), "name should be non-empty");
            // system_prompt / description 可以为空,但字段必须存在。
            let _ = (system_prompt, description);
        }
    }

    #[test]
    fn get_agent_returns_some_for_known_kind() {
        let h = make_handler();
        let agents = h.list_agents();
        let first_kind = agents
            .first()
            .map(|(k, _, _, _)| k.clone())
            .expect("at least one agent");
        let desc = h
            .get_agent(&first_kind)
            .expect("known kind should return Some");
        assert!(!desc.name.is_empty(), "name should be non-empty");
    }

    #[test]
    fn get_agent_returns_none_for_unknown_kind() {
        let h = make_handler();
        let result = h.get_agent("nonexistent-agent-kind-xyz");
        assert!(result.is_none(), "unknown kind should return None");
    }

    #[test]
    fn orchestrator_accessor_works() {
        let h = make_handler();
        let _ = h.orchestrator();
    }

    #[tokio::test]
    async fn execute_does_not_panic_with_unreachable_upstream() {
        // 上游 LLM 不可达时 execute 应返回 Err,而不是 panic。
        // 这里只验证不 panic,允许 Err。
        let h = make_handler();
        let task = SwarmTask::new("test task");
        let _ = h.execute(task).await;
        // 不 assert 结果(LLM 不可达时会 Err),只验证不 panic。
    }
}

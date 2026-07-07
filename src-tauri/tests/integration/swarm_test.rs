//! Swarm integration test: pipeline validation + output contract.
//!
//! Validates the v2.0 swarm: single-agent and empty-pipeline tasks
//! execute gracefully (they no longer return errors — the orchestrator
//! falls back to default behavior). A real LLM round-trip is out of
//! scope for the integration test (covered by unit tests).

use nebula_lib::llm::LlmGateway;
use nebula_lib::llm::OllamaClient;
use nebula_lib::swarm::orchestrator::{SwarmOrchestrator, SwarmTask};
// M7b #91: new_without_memory 签名变更,需 ToolRegistry 第二参数。
use nebula_lib::tools::ToolRegistry;
use std::sync::Arc;
use std::time::Duration;

fn mock_gateway() -> Arc<LlmGateway> {
    let client = Arc::new(OllamaClient::new_with_timeout(
        "http://127.0.0.1:1",
        Duration::from_secs(2),
    ));
    Arc::new(LlmGateway::new(
        client, "m", "ollama", None, None, None, None, None,
    ))
}

/// M7b #91: 构造空 ToolRegistry 供 new_without_memory 第二参数使用。
fn mock_tool_registry() -> Arc<ToolRegistry> {
    Arc::new(ToolRegistry::new())
}

#[tokio::test]
async fn swarm_single_agent_by_kind_executes() {
    let gw = mock_gateway();
    let orch = SwarmOrchestrator::new_without_memory(gw, mock_tool_registry());
    let mut task = SwarmTask::new("hi");
    // M7b #91: AgentKind::from_str 大小写敏感,需用小写 "coder" 才能解析。
    // 单 kind 不足 MIN_AGENTS=2 时补齐 Generic,故 dispatch 2 个 agent。
    task.agents = vec!["coder".to_string()];

    // v2.0: coder + Generic padding = 2 agents (mock LLM → 全失败)。
    let res = orch.execute(task).await;
    assert!(res.is_ok(), "single-agent by kind should execute");
    let report = res.unwrap();
    assert_eq!(
        report.failure_count, 2,
        "coder + Generic padding = 2 agents"
    );
}

#[tokio::test]
async fn swarm_empty_agents_falls_back_to_default_pool() {
    let gw = mock_gateway();
    let orch = SwarmOrchestrator::new_without_memory(gw, mock_tool_registry());
    let mut task = SwarmTask::new("hi");
    task.agents = vec![];

    // v2.0: empty agents falls back to default agent_count (3).
    let res = orch.execute(task).await;
    assert!(res.is_ok(), "empty agents should fall back to default pool");
    let report = res.unwrap();
    assert_eq!(report.failure_count, 3, "default 3 agents dispatched");
}

#[tokio::test]
async fn swarm_canonical_pipeline_is_well_formed() {
    // We do not exercise the network: this test asserts that the
    // canonical task is constructed correctly.
    let gw = mock_gateway();
    let orch = SwarmOrchestrator::new_without_memory(gw, mock_tool_registry());
    let task = SwarmTask::new("design a snake");
    assert!(task.agents.is_empty());
    assert_eq!(task.max_retries, 1);
    // The orchestrator is constructable; verify the pool is non-empty.
    assert_eq!(orch.list_agents().len(), 6);
}

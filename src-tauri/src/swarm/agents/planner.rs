//! Planner agent — task decomposition, scheduling, and arbitration.
//!
//! ## 白皮书设计规格
//! - **角色**：Planner-F（任务拆解）
//! - **核心能力**：任务规划、调度、仲裁
//! - **v1.0 MVP**：半自动 DAG 拆解，深度 ≤3 层（P0-1）
//! - **v1.1**：完整 Plan 模式上线

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::types::MemoryLayer;
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput, AgentScenario};

/// T-6: Planner 可用工具集。
const PLANNER_TOOL_SET: [&str; 2] = ["memory_search", "tool_invoke"];
/// T-6: Planner 可访问的记忆层级。
const PLANNER_KNOWLEDGE_SCOPE: [MemoryLayer; 2] = [MemoryLayer::L2, MemoryLayer::L5];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界)。
const PLANNER_SYSTEM_PROMPT: &str = "You are the Planner agent (Planner-F) in the nebula swarm.\n\
     Role: decompose tasks into sub-tasks (max depth 3), assign agents, arbitrate conflicts.\n\
     Tools: memory_search, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L5 (lessons learned).";

pub struct PlannerAgent {
    llm: Arc<LlmGateway>,
}

impl PlannerAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self { llm }
    }
}

#[async_trait]
// T-D-B-17: 角色 agent 保留以支持向后兼容(scenarios.json / gRPC / 旧 API)。
// kind() 与 run() 引用废弃的 AgentKind::Planner,此处显式放行废弃警告。
#[allow(deprecated)]
impl Agent for PlannerAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Planner
    }
    fn name(&self) -> &str {
        "Planner"
    }
    fn system_prompt(&self) -> &str {
        PLANNER_SYSTEM_PROMPT
    }
    fn description(&self) -> &str {
        "Decomposes tasks, plans execution order, and arbitrates agent conflicts."
    }
    fn tool_set(&self) -> &[&str] {
        &PLANNER_TOOL_SET
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        &PLANNER_KNOWLEDGE_SCOPE
    }

    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let msgs = vec![
            ChatMessage::system(self.system_prompt()),
            ChatMessage::user(format!(
                "Plan the following task. Break it into sub-tasks (max depth 3) \
                 and assign each to the most suitable agent in the team \
                 (Coder / Writer / Reviewer / Researcher / Planner).\n\n\
                 Task:\n{task}\n\n\
                 Previous team context:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        ctx.push_str(self.name(), "plan", &body);
        info!(target: "nebula.swarm", agent = %self.name(), "planner finished");
        // T-D-B-17: 同时填充 scenario 字段,供新代码读取场景标签。
        Ok(AgentOutput::new(AgentKind::Planner, self.name(), body)
            .with_scenario(AgentScenario::Planning))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use std::sync::Arc;

    #[test]
    fn planner_tool_set_and_knowledge_scope() {
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()));
        assert_eq!(agent.tool_set(), &["memory_search", "tool_invoke"]);
        assert_eq!(agent.knowledge_scope(), &[MemoryLayer::L2, MemoryLayer::L5]);
    }

    #[test]
    fn planner_system_prompt_mentions_tools_and_scope() {
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Planner"));
        assert!(prompt.contains("memory_search"));
        assert!(prompt.contains("L2"));
        assert!(prompt.contains("L5"));
    }
}

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
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

pub struct PlannerAgent {
    llm: Arc<LlmGateway>,
}

impl PlannerAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl Agent for PlannerAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Planner
    }
    fn name(&self) -> &str {
        "Planner"
    }
    fn system_prompt(&self) -> &str {
        "You are the Planner agent (Planner-F) in the nine-snake swarm. \
         Your role is to decompose complex tasks into executable sub-tasks \
         (max depth 3), assign them to the appropriate agents, and resolve \
         conflicts. Output a structured plan with dependencies, then \
         monitor execution and arbitrate when agents disagree."
    }
    fn description(&self) -> &str {
        "Decomposes tasks, plans execution order, and arbitrates agent conflicts."
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
        info!(target: "nine_snake.swarm", agent = %self.name(), "planner finished");
        Ok(AgentOutput::new(AgentKind::Planner, self.name(), body))
    }
}

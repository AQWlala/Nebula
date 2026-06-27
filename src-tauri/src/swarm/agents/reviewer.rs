//! Reviewer agent — reviews the most recent work in the team context.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

pub struct ReviewerAgent {
    llm: Arc<LlmGateway>,
}

impl ReviewerAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl Agent for ReviewerAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Reviewer
    }
    fn name(&self) -> &str {
        "Reviewer"
    }
    fn system_prompt(&self) -> &str {
        "You are the Reviewer agent in the nine-snake swarm. \
         Critically evaluate the latest piece of work in the team context. \
         End your response with one of: APPROVE / REVISE / REJECT."
    }
    fn description(&self) -> &str {
        "Reviews the team's work and emits an APPROVE / REVISE / REJECT verdict."
    }

    async fn run(&self, _task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let msgs = vec![
            ChatMessage::system(self.system_prompt()),
            ChatMessage::user(format!(
                "Review the most recent work in this team context:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        ctx.push_str(self.name(), "review", &body);
        let verdict = if body.contains("APPROVE") {
            0.9
        } else if body.contains("REVISE") {
            0.6
        } else {
            0.2
        };
        info!(target: "nine_snake.swarm", agent = %self.name(), verdict, "reviewer finished");
        Ok(AgentOutput::new(AgentKind::Reviewer, self.name(), body).with_confidence(verdict))
    }
}

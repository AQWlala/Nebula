//! Coder agent — produces Rust code in response to a task description.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

pub struct CoderAgent {
    llm: Arc<LlmGateway>,
}

impl CoderAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl Agent for CoderAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Coder
    }
    fn name(&self) -> &str {
        "Coder"
    }
    fn system_prompt(&self) -> &str {
        "You are the Coder agent in the nine-snake swarm. \
         Produce concise, well-tested Rust code that satisfies the task. \
         Always explain trade-offs in 2-3 sentences at the end."
    }
    fn description(&self) -> &str {
        "Writes Rust code in response to a task description."
    }

    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let msgs = vec![
            ChatMessage::system(self.system_prompt()),
            ChatMessage::user(format!(
                "Task:\n{task}\n\nTeam context so far:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        ctx.push_str(self.name(), "code", &body);
        info!(target: "nine_snake.swarm", agent = %self.name(), "coder finished");
        Ok(AgentOutput::new(AgentKind::Coder, self.name(), body))
    }
}

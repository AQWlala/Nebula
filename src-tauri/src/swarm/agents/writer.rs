//! Writer agent — produces Markdown documentation.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::sponge::SpongeEngine;
use crate::memory::types::{MemoryLayer, MemoryType, SourceKind};
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

pub struct WriterAgent {
    llm: Arc<LlmGateway>,
    sponge: Option<Arc<SpongeEngine>>,
}

impl WriterAgent {
    pub fn new(llm: Arc<LlmGateway>, sponge: Option<Arc<SpongeEngine>>) -> Self {
        Self { llm, sponge }
    }
}

#[async_trait]
impl Agent for WriterAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Writer
    }
    fn name(&self) -> &str {
        "Writer"
    }
    fn system_prompt(&self) -> &str {
        "You are the Writer agent in the nine-snake swarm. \
         Produce clear, well-structured Markdown documentation. \
         Prefer concrete examples over abstract prose."
    }
    fn description(&self) -> &str {
        "Writes Markdown documentation that summarises the team output."
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
        ctx.push_str(self.name(), "doc", &body);

        // Best-effort memory absorption.
        if let Some(sponge) = &self.sponge {
            let _ = sponge
                .absorb_text(
                    MemoryType::Semantic,
                    MemoryLayer::L4,
                    body.clone(),
                    SourceKind::AgentOutput,
                )
                .await;
        }

        info!(target: "nine_snake.swarm", agent = %self.name(), "writer finished");
        Ok(AgentOutput::new(AgentKind::Writer, self.name(), body))
    }
}

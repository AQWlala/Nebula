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

/// T-6: Writer 可用工具集。
const WRITER_TOOL_SET: [&str; 3] = ["editor_read", "editor_write", "tool_invoke"];
/// T-6: Writer 可访问的记忆层级。
const WRITER_KNOWLEDGE_SCOPE: [MemoryLayer; 1] = [MemoryLayer::L2];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界)。
const WRITER_SYSTEM_PROMPT: &str = "You are the Writer agent in the nebula swarm.\n\
     Role: produce clear, well-structured Markdown documentation.\n\
     Tools: editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience).\n\
     Prefer concrete examples over abstract prose.";

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
        WRITER_SYSTEM_PROMPT
    }
    fn description(&self) -> &str {
        "Writes Markdown documentation that summarises the team output."
    }
    fn tool_set(&self) -> &[&str] {
        &WRITER_TOOL_SET
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        &WRITER_KNOWLEDGE_SCOPE
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
                    None, // T-E-B-04: tool 由主 agent 集成时传入
                )
                .await;
        }

        info!(target: "nebula.swarm", agent = %self.name(), "writer finished");
        Ok(AgentOutput::new(AgentKind::Writer, self.name(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use std::sync::Arc;

    #[test]
    fn writer_tool_set_and_knowledge_scope() {
        let agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None);
        assert_eq!(
            agent.tool_set(),
            &["editor_read", "editor_write", "tool_invoke"]
        );
        assert_eq!(agent.knowledge_scope(), &[MemoryLayer::L2]);
    }

    #[test]
    fn writer_system_prompt_mentions_tools_and_scope() {
        let agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None);
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Writer"));
        assert!(prompt.contains("editor_write"));
        assert!(prompt.contains("L2"));
        // writer 无 shell 工具
        assert!(!agent.tool_set().contains(&"shell"));
    }
}

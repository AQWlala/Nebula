//! Reviewer agent — reviews the most recent work in the team context.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::types::MemoryLayer;
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

/// T-6: Reviewer 可用工具集(只读,无写权限)。
const REVIEWER_TOOL_SET: [&str; 2] = ["editor_read", "tool_invoke"];
/// T-6: Reviewer 可访问的记忆层级。
const REVIEWER_KNOWLEDGE_SCOPE: [MemoryLayer; 1] = [MemoryLayer::L2];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界)。
const REVIEWER_SYSTEM_PROMPT: &str = "You are the Reviewer agent in the nebula swarm.\n\
     Role: critically evaluate the latest work in the team context.\n\
     Tools: editor_read, tool_invoke (read-only — no write/shell).\n\
     Knowledge scope: L2 (cross-session experience).\n\
     End your response with one of: APPROVE / REVISE / REJECT.";

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
        REVIEWER_SYSTEM_PROMPT
    }
    fn description(&self) -> &str {
        "Reviews the team's work and emits an APPROVE / REVISE / REJECT verdict."
    }
    fn tool_set(&self) -> &[&str] {
        &REVIEWER_TOOL_SET
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        &REVIEWER_KNOWLEDGE_SCOPE
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
        info!(target: "nebula.swarm", agent = %self.name(), verdict, "reviewer finished");
        Ok(AgentOutput::new(AgentKind::Reviewer, self.name(), body).with_confidence(verdict))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use std::sync::Arc;

    #[test]
    fn reviewer_tool_set_and_knowledge_scope() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        assert_eq!(agent.tool_set(), &["editor_read", "tool_invoke"]);
        assert_eq!(agent.knowledge_scope(), &[MemoryLayer::L2]);
    }

    #[test]
    fn reviewer_system_prompt_mentions_tools_and_scope() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Reviewer"));
        assert!(prompt.contains("editor_read"));
        assert!(prompt.contains("L2"));
        // reviewer 无写权限
        assert!(!agent.tool_set().contains(&"shell"));
        assert!(!agent.tool_set().contains(&"editor_write"));
    }
}

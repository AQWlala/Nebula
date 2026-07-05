//! Coder agent — produces Rust code in response to a task description.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::types::MemoryLayer;
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

/// T-6: Coder 可用工具集。
const CODER_TOOL_SET: [&str; 4] = ["shell", "editor_read", "editor_write", "tool_invoke"];
/// T-6: Coder 可访问的记忆层级。
const CODER_KNOWLEDGE_SCOPE: [MemoryLayer; 2] = [MemoryLayer::L2, MemoryLayer::L3];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界)。
const CODER_SYSTEM_PROMPT: &str = "You are the Coder agent in the nebula swarm.\n\
     Role: produce concise, well-tested Rust code that satisfies the task.\n\
     Tools: shell, editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L3 (concrete facts).\n\
     Always explain trade-offs in 2-3 sentences at the end.";

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
        CODER_SYSTEM_PROMPT
    }
    fn description(&self) -> &str {
        "Writes Rust code in response to a task description."
    }
    fn tool_set(&self) -> &[&str] {
        &CODER_TOOL_SET
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        &CODER_KNOWLEDGE_SCOPE
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
        info!(target: "nebula.swarm", agent = %self.name(), "coder finished");
        Ok(AgentOutput::new(AgentKind::Coder, self.name(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use std::sync::Arc;

    #[test]
    fn coder_tool_set_and_knowledge_scope() {
        let agent = CoderAgent::new(Arc::new(LlmGateway::new_test()));
        assert_eq!(
            agent.tool_set(),
            &["shell", "editor_read", "editor_write", "tool_invoke"]
        );
        assert_eq!(
            agent.knowledge_scope(),
            &[MemoryLayer::L2, MemoryLayer::L3]
        );
    }

    #[test]
    fn coder_system_prompt_mentions_tools_and_scope() {
        let agent = CoderAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Coder"));
        assert!(prompt.contains("shell"));
        assert!(prompt.contains("L2"));
    }
}

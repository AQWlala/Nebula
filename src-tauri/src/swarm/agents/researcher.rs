//! Researcher agent — web search, literature analysis, and information retrieval.
//!
//! ## 白皮书设计规格
//! - **角色**：Researcher-B（资料检索）
//! - **核心能力**：网络搜索、文献分析
//! - **v1.0 MVP**：基础搜索 + 结果摘要，不使用外部 API（本地优先）

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::types::MemoryLayer;
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

/// T-6: Researcher 可用工具集。
const RESEARCHER_TOOL_SET: [&str; 2] = ["memory_search", "tool_invoke"];
/// T-6: Researcher 可访问的记忆层级。
const RESEARCHER_KNOWLEDGE_SCOPE: [MemoryLayer; 3] =
    [MemoryLayer::L1, MemoryLayer::L2, MemoryLayer::L4];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界)。
const RESEARCHER_SYSTEM_PROMPT: &str = "You are the Researcher agent (Researcher-B) in the nebula swarm.\n\
     Role: gather, verify and synthesise information; cite sources; highlight uncertainty.\n\
     Tools: memory_search, tool_invoke.\n\
     Knowledge scope: L1 (session history), L2 (cross-session experience), L4 (distilled knowledge).";

pub struct ResearcherAgent {
    llm: Arc<LlmGateway>,
}

impl ResearcherAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self { llm }
    }
}

#[async_trait]
impl Agent for ResearcherAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Researcher
    }
    fn name(&self) -> &str {
        "Researcher"
    }
    fn system_prompt(&self) -> &str {
        RESEARCHER_SYSTEM_PROMPT
    }
    fn description(&self) -> &str {
        "Gathers, verifies and synthesises information from available sources."
    }
    fn tool_set(&self) -> &[&str] {
        &RESEARCHER_TOOL_SET
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        &RESEARCHER_KNOWLEDGE_SCOPE
    }

    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let msgs = vec![
            ChatMessage::system(self.system_prompt()),
            ChatMessage::user(format!(
                "Research task:\n{task}\n\nExisting team context (build on this, do not repeat):\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        ctx.push_str(self.name(), "research", &body);
        info!(target: "nebula.swarm", agent = %self.name(), "researcher finished");
        Ok(AgentOutput::new(AgentKind::Researcher, self.name(), body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use std::sync::Arc;

    #[test]
    fn researcher_tool_set_and_knowledge_scope() {
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()));
        assert_eq!(agent.tool_set(), &["memory_search", "tool_invoke"]);
        assert_eq!(
            agent.knowledge_scope(),
            &[MemoryLayer::L1, MemoryLayer::L2, MemoryLayer::L4]
        );
    }

    #[test]
    fn researcher_system_prompt_mentions_tools_and_scope() {
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Researcher"));
        assert!(prompt.contains("memory_search"));
        assert!(prompt.contains("L1"));
        assert!(prompt.contains("L4"));
    }
}

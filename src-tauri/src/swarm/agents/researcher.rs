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
use crate::swarm::context::TeamContext;

use super::{Agent, AgentKind, AgentOutput};

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
        "You are the Researcher agent (Researcher-B) in the nine-snake swarm. \
         Your role is to gather, verify and synthesise information. \
         When a task requires factual grounding, you search for relevant \
         data, cite sources where possible, and produce a concise research \
         brief. Always highlight uncertainty and knowledge gaps."
    }
    fn description(&self) -> &str {
        "Gathers, verifies and synthesises information from available sources."
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
        info!(target: "nine_snake.swarm", agent = %self.name(), "researcher finished");
        Ok(AgentOutput::new(AgentKind::Researcher, self.name(), body))
    }
}

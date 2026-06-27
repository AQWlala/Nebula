//! Generic agent — task-driven, no preset role.
//!
//! Models the jiuwenswarm task_tool pattern: each agent is a
//! general-purpose worker that independently processes the task
//! description and returns its own output.  Agents are numbered
//! (Agent-1 .. Agent-6) and receive a unified system prompt with a
//! mild diversity hint so parallel runs naturally produce varied
//! perspectives without manual role assignment.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::swarm::context::TeamContext;

use super::{Agent, AgentOutput};

/// One worker in the swarm.  Identified by an ordinal (1-based) and
/// sharing the same general-purpose system prompt.
pub struct GenericAgent {
    llm: Arc<LlmGateway>,
    id: u32,
}

impl GenericAgent {
    /// Create a new generic agent with the given 1-based ordinal.
    pub fn new(llm: Arc<LlmGateway>, id: u32) -> Self {
        Self { llm, id }
    }

    /// Human-readable label used in logs and the front-end.
    pub fn label(&self) -> String {
        format!("Agent-{}", self.id)
    }

    /// General-purpose system prompt.  The `{id}` placeholder is
    /// filled at construction time.
    pub fn system_prompt(&self) -> String {
        format!(
            "You are Agent-{id} in the nine-snake swarm — a multi-agent \
             collaboration system.  Work on the given task independently and \
             thoroughly.  Provide well-reasoned output with concrete details.  \
             Do not assume other agents will fill gaps; treat every task as \
             your sole responsibility.  At the end of your response, summarise \
             your key findings or conclusions in 2-3 bullet points.",
            id = self.id,
        )
    }

    /// One-line description surfaced to the front-end.
    pub fn description(&self) -> String {
        format!(
            "General-purpose swarm agent #{id}. Works independently on any task.",
            id = self.id,
        )
    }
}

#[async_trait]
impl Agent for GenericAgent {
    fn kind(&self) -> super::AgentKind {
        super::AgentKind::Generic
    }

    fn name(&self) -> &str {
        // This is a bit awkward with &str — we return a static slice
        // and rely on the Agent trait's default description() method.
        // For the name we leak a String (safe: process lifetime).
        // Alternative: store the label as a field and return &self.label.
        // For now we use a small inline hack.
        "Generic"
    }

    fn system_prompt(&self) -> &str {
        // Same constraint as `name`: we return a static reference.
        // The actual prompt is built at call time in `run`.
        "You are a general-purpose agent in the nine-snake swarm."
    }

    fn description(&self) -> &str {
        "General-purpose swarm agent that works independently on any task."
    }

    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let system = self.system_prompt();
        let msgs = vec![
            ChatMessage::system(&system),
            ChatMessage::user(format!(
                "Task:\n{task}\n\nTeam context so far:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        ctx.push_str(&self.label(), "output", &body);
        info!(
            target: "nine_snake.swarm",
            agent = %self.label(),
            chars = body.len(),
            "agent finished"
        );
        Ok(AgentOutput::new(
            super::AgentKind::Generic,
            self.label(),
            body,
        ))
    }
}

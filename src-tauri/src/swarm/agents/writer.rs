//! Writer agent — produces Markdown documentation.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::sponge::SpongeEngine;
use crate::memory::types::{MemoryLayer, MemoryType, SourceKind};
use crate::swarm::context::TeamContext;

use super::{writing_role_profile, Agent, AgentKind, AgentOutput, AgentScenario};

/// T-6: Writer 可用工具集(编程场景:Markdown 文档)。
const WRITER_TOOL_SET: [&str; 3] = ["editor_read", "editor_write", "tool_invoke"];
/// T-6: Writer 可访问的记忆层级(编程场景)。
const WRITER_KNOWLEDGE_SCOPE: [MemoryLayer; 1] = [MemoryLayer::L2];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界,编程场景)。
const WRITER_SYSTEM_PROMPT: &str = "You are the Writer agent in the nebula swarm.\n\
     Role: produce clear, well-structured Markdown documentation.\n\
     Tools: editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience).\n\
     Prefer concrete examples over abstract prose.";

pub struct WriterAgent {
    llm: Arc<LlmGateway>,
    sponge: Option<Arc<SpongeEngine>>,
    /// T-D-B-19: 场景标签(None = 编程场景,Some(Writing) = 写作场景)。
    scenario: Option<AgentScenario>,
}

impl WriterAgent {
    pub fn new(llm: Arc<LlmGateway>, sponge: Option<Arc<SpongeEngine>>) -> Self {
        Self {
            llm,
            sponge,
            scenario: None,
        }
    }

    /// T-D-B-19: Builder — 注入场景标签,切换到对应场景行为。
    ///
    /// - `AgentScenario::Writing` → 主笔(写作场景)
    /// - 其他值或未调用 → Writer(编程场景,向后兼容)
    pub fn with_scenario(mut self, scenario: AgentScenario) -> Self {
        self.scenario = Some(scenario);
        self
    }

    /// T-D-B-19: 当前场景标签(主要用于测试与诊断)。
    pub fn current_scenario(&self) -> Option<AgentScenario> {
        self.scenario
    }

    /// T-D-B-19: 当前场景下的 system prompt。
    /// Writing 场景返回写作提示词(主笔),其他返回编程提示词(Markdown 文档)。
    fn effective_system_prompt(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("writer")
                .map(|p| p.system_prompt)
                .unwrap_or(WRITER_SYSTEM_PROMPT),
            _ => WRITER_SYSTEM_PROMPT,
        }
    }

    /// T-D-B-19: 当前场景下的 tool_set。
    fn effective_tool_set(&self) -> &[&str] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("writer")
                .map(|p| p.tool_set)
                .unwrap_or(&WRITER_TOOL_SET),
            _ => &WRITER_TOOL_SET,
        }
    }

    /// T-D-B-19: 当前场景下的 knowledge_scope。
    fn effective_knowledge_scope(&self) -> &[MemoryLayer] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("writer")
                .map(|p| p.knowledge_scope)
                .unwrap_or(&WRITER_KNOWLEDGE_SCOPE),
            _ => &WRITER_KNOWLEDGE_SCOPE,
        }
    }

    /// T-D-B-19: 当前场景下写入 TeamContext 的 label。
    fn effective_context_label(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("writer")
                .map(|p| p.context_label)
                .unwrap_or("doc"),
            _ => "doc",
        }
    }
}

#[async_trait]
// T-D-B-17: 角色 agent 保留以支持向后兼容(scenarios.json / gRPC / 旧 API)。
// kind() 与 run() 引用废弃的 AgentKind::Writer,此处显式放行废弃警告。
#[allow(deprecated)]
impl Agent for WriterAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Writer
    }
    fn name(&self) -> &str {
        "Writer"
    }
    fn system_prompt(&self) -> &str {
        // T-D-B-19: 根据场景返回对应提示词。
        self.effective_system_prompt()
    }
    fn description(&self) -> &str {
        "Writes Markdown documentation (coding) or prose (writing) for the team."
    }
    fn tool_set(&self) -> &[&str] {
        // T-D-B-19: 根据场景返回对应工具集。
        self.effective_tool_set()
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        // T-D-B-19: 根据场景返回对应知识边界。
        self.effective_knowledge_scope()
    }

    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let msgs = vec![
            ChatMessage::system(self.effective_system_prompt()),
            ChatMessage::user(format!(
                "Task:\n{task}\n\nTeam context so far:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        // T-D-B-19: 根据场景选择 TeamContext label(写作场景用 "prose")。
        ctx.push_str(self.name(), self.effective_context_label(), &body);

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
        // T-D-B-17: 同时填充 scenario 字段,供新代码读取场景标签。
        // T-D-B-19: 写作场景下打 Writing 标签,其他打 Writing(Writer 始终偏写作,
        // 但编程场景下用原 Writing 标签保持向后兼容;显式场景注入时也用 Writing)。
        let scenario_tag = match self.scenario {
            Some(AgentScenario::Writing) => AgentScenario::Writing,
            _ => AgentScenario::Writing,
        };
        Ok(AgentOutput::new(AgentKind::Writer, self.name(), body).with_scenario(scenario_tag))
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

    // ---- T-D-B-19: 写作场景(主笔)测试 ----

    #[test]
    fn writer_default_scenario_is_none() {
        // 默认(未注入场景)→ scenario = None,行为为编程场景(Markdown 文档)。
        let agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None);
        assert!(agent.current_scenario().is_none());
    }

    #[test]
    fn writer_with_writing_scenario_switches_system_prompt() {
        // Writing 场景 → system_prompt 应切换到主笔提示词(含 Lead Writer)。
        let agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None)
            .with_scenario(AgentScenario::Writing);
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
        let prompt = agent.system_prompt();
        assert!(
            prompt.to_lowercase().contains("lead writer"),
            "writing scenario prompt should mention Lead Writer: {prompt}"
        );
        // 编程场景的 Markdown 文档指引不应出现在写作提示词中。
        assert!(
            !prompt.contains("well-structured Markdown documentation"),
            "writing prompt should not mention Markdown documentation: {prompt}"
        );
    }

    #[test]
    fn writer_with_writing_scenario_knowledge_scope_includes_l3() {
        // 写作场景知识边界应含 L3(主笔需引用具体事实)。
        // 编程场景 Writer 仅 L2;写作场景扩展到 L2+L3。
        let coding_agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None);
        let writing_agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None)
            .with_scenario(AgentScenario::Writing);
        assert_eq!(coding_agent.knowledge_scope(), &[MemoryLayer::L2]);
        assert!(writing_agent.knowledge_scope().contains(&MemoryLayer::L3));
    }

    #[test]
    fn writer_with_writing_scenario_keeps_editor_write() {
        // 主笔需要 editor_write(撰写正文),写作场景应保留。
        let agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None)
            .with_scenario(AgentScenario::Writing);
        assert!(
            agent.tool_set().contains(&"editor_write"),
            "lead writer should keep editor_write"
        );
        assert!(!agent.tool_set().contains(&"shell"));
    }

    #[test]
    fn writer_with_coding_scenario_keeps_coding_behavior() {
        // Coding 场景应保持编程场景行为(Markdown 文档)。
        let agent = WriterAgent::new(Arc::new(LlmGateway::new_test()), None)
            .with_scenario(AgentScenario::Coding);
        let prompt = agent.system_prompt();
        assert!(
            prompt.contains("Writer"),
            "Coding scenario should keep writer prompt: {prompt}"
        );
    }
}

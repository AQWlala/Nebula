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

use super::{writing_role_profile, Agent, AgentKind, AgentOutput, AgentScenario};

/// T-6: Researcher 可用工具集(编程场景)。
const RESEARCHER_TOOL_SET: [&str; 2] = ["memory_search", "tool_invoke"];
/// T-6: Researcher 可访问的记忆层级(编程场景)。
const RESEARCHER_KNOWLEDGE_SCOPE: [MemoryLayer; 3] =
    [MemoryLayer::L1, MemoryLayer::L2, MemoryLayer::L4];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界,编程场景)。
const RESEARCHER_SYSTEM_PROMPT: &str = "You are the Researcher agent (Researcher-B) in the nebula swarm.\n\
     Role: gather, verify and synthesise information; cite sources; highlight uncertainty.\n\
     Tools: memory_search, tool_invoke.\n\
     Knowledge scope: L1 (session history), L2 (cross-session experience), L4 (distilled knowledge).";

pub struct ResearcherAgent {
    llm: Arc<LlmGateway>,
    /// T-D-B-19: 场景标签(None = 编程场景,Some(Writing) = 写作场景)。
    scenario: Option<AgentScenario>,
}

impl ResearcherAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self {
            llm,
            scenario: None,
        }
    }

    /// T-D-B-19: Builder — 注入场景标签,切换到对应场景行为。
    ///
    /// - `AgentScenario::Writing` → 素材收集(写作场景)
    /// - 其他值或未调用 → Researcher(编程场景,向后兼容)
    pub fn with_scenario(mut self, scenario: AgentScenario) -> Self {
        self.scenario = Some(scenario);
        self
    }

    /// T-D-B-19: 当前场景标签(主要用于测试与诊断)。
    pub fn current_scenario(&self) -> Option<AgentScenario> {
        self.scenario
    }

    /// T-D-B-19: 当前场景下的 system prompt。
    /// Writing 场景返回写作提示词(素材收集),其他返回编程提示词。
    fn effective_system_prompt(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("researcher")
                .map(|p| p.system_prompt)
                .unwrap_or(RESEARCHER_SYSTEM_PROMPT),
            _ => RESEARCHER_SYSTEM_PROMPT,
        }
    }

    /// T-D-B-19: 当前场景下的 tool_set。
    fn effective_tool_set(&self) -> &[&str] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("researcher")
                .map(|p| p.tool_set)
                .unwrap_or(&RESEARCHER_TOOL_SET),
            _ => &RESEARCHER_TOOL_SET,
        }
    }

    /// T-D-B-19: 当前场景下的 knowledge_scope。
    fn effective_knowledge_scope(&self) -> &[MemoryLayer] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("researcher")
                .map(|p| p.knowledge_scope)
                .unwrap_or(&RESEARCHER_KNOWLEDGE_SCOPE),
            _ => &RESEARCHER_KNOWLEDGE_SCOPE,
        }
    }

    /// T-D-B-19: 当前场景下写入 TeamContext 的 label。
    fn effective_context_label(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("researcher")
                .map(|p| p.context_label)
                .unwrap_or("research"),
            _ => "research",
        }
    }
}

#[async_trait]
// T-D-B-17: 角色 agent 保留以支持向后兼容(scenarios.json / gRPC / 旧 API)。
// kind() 与 run() 引用废弃的 AgentKind::Researcher,此处显式放行废弃警告。
#[allow(deprecated)]
impl Agent for ResearcherAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Researcher
    }
    fn name(&self) -> &str {
        "Researcher"
    }
    fn system_prompt(&self) -> &str {
        // T-D-B-19: 根据场景返回对应提示词。
        self.effective_system_prompt()
    }
    fn description(&self) -> &str {
        "Gathers information (coding) or writing material (writing) from available sources."
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
                "Research task:\n{task}\n\nExisting team context (build on this, do not repeat):\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        // T-D-B-19: 根据场景选择 TeamContext label(写作场景用 "material")。
        ctx.push_str(self.name(), self.effective_context_label(), &body);
        info!(target: "nebula.swarm", agent = %self.name(), "researcher finished");
        // T-D-B-17: 同时填充 scenario 字段,供新代码读取场景标签。
        // T-D-B-19: 写作场景下打 Writing 标签,其他打 Research。
        let scenario_tag = match self.scenario {
            Some(AgentScenario::Writing) => AgentScenario::Writing,
            _ => AgentScenario::Research,
        };
        Ok(AgentOutput::new(AgentKind::Researcher, self.name(), body).with_scenario(scenario_tag))
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

    // ---- T-D-B-19: 写作场景(素材收集)测试 ----

    #[test]
    fn researcher_default_scenario_is_none() {
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()));
        assert!(agent.current_scenario().is_none());
    }

    #[test]
    fn researcher_with_writing_scenario_switches_system_prompt() {
        // Writing 场景 → system_prompt 应切换到素材收集提示词。
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
        let prompt = agent.system_prompt();
        // 写作场景提示词应提及素材收集(material/background)。
        assert!(
            prompt.to_lowercase().contains("material")
                || prompt.to_lowercase().contains("background"),
            "writing scenario prompt should mention material/background: {prompt}"
        );
    }

    #[test]
    fn researcher_with_writing_scenario_keeps_memory_search() {
        // 素材收集需要 memory_search(检索背景资料),写作场景应保留。
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert!(
            agent.tool_set().contains(&"memory_search"),
            "writing researcher should keep memory_search"
        );
    }

    #[test]
    fn researcher_with_writing_scenario_knowledge_scope_includes_l4() {
        // 写作场景知识边界应含 L4(蒸馏知识,供考证事实)。
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert!(agent.knowledge_scope().contains(&MemoryLayer::L4));
    }

    #[test]
    fn researcher_with_coding_scenario_keeps_coding_behavior() {
        // Coding 场景应保持编程场景行为。
        let agent = ResearcherAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Coding);
        let prompt = agent.system_prompt();
        assert!(
            prompt.contains("Researcher"),
            "Coding scenario should keep researcher prompt: {prompt}"
        );
    }
}

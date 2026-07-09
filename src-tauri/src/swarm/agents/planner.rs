//! Planner agent — task decomposition, scheduling, and arbitration.
//!
//! ## 白皮书设计规格
//! - **角色**：Planner-F（任务拆解）
//! - **核心能力**：任务规划、调度、仲裁
//! - **v1.0 MVP**：半自动 DAG 拆解，深度 ≤3 层（P0-1）
//! - **v1.1**：完整 Plan 模式上线

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::types::MemoryLayer;
use crate::swarm::context::TeamContext;

use super::{writing_role_profile, Agent, AgentKind, AgentOutput, AgentScenario};

/// T-6: Planner 可用工具集(编程场景)。
const PLANNER_TOOL_SET: [&str; 2] = ["memory_search", "tool_invoke"];
/// T-6: Planner 可访问的记忆层级(编程场景)。
const PLANNER_KNOWLEDGE_SCOPE: [MemoryLayer; 2] = [MemoryLayer::L2, MemoryLayer::L5];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界,编程场景)。
const PLANNER_SYSTEM_PROMPT: &str = "You are the Planner agent (Planner-F) in the nebula swarm.\n\
     Role: decompose tasks into sub-tasks (max depth 3), assign agents, arbitrate conflicts.\n\
     Tools: memory_search, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L5 (lessons learned).";

pub struct PlannerAgent {
    llm: Arc<LlmGateway>,
    /// T-D-B-19: 场景标签(None = 编程场景,Some(Writing) = 写作场景)。
    scenario: Option<AgentScenario>,
}

impl PlannerAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self {
            llm,
            scenario: None,
        }
    }

    /// T-D-B-19: Builder — 注入场景标签,切换到对应场景行为。
    ///
    /// - `AgentScenario::Writing` → 大纲规划(写作场景)
    /// - 其他值或未调用 → Planner(编程场景,向后兼容)
    pub fn with_scenario(mut self, scenario: AgentScenario) -> Self {
        self.scenario = Some(scenario);
        self
    }

    /// T-D-B-19: 当前场景标签(主要用于测试与诊断)。
    pub fn current_scenario(&self) -> Option<AgentScenario> {
        self.scenario
    }

    /// T-D-B-19: 当前场景下的 system prompt。
    /// Writing 场景返回写作提示词(大纲规划),其他返回编程提示词。
    fn effective_system_prompt(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("planner")
                .map(|p| p.system_prompt)
                .unwrap_or(PLANNER_SYSTEM_PROMPT),
            _ => PLANNER_SYSTEM_PROMPT,
        }
    }

    /// T-D-B-19: 当前场景下的 tool_set。
    fn effective_tool_set(&self) -> &[&str] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("planner")
                .map(|p| p.tool_set)
                .unwrap_or(&PLANNER_TOOL_SET),
            _ => &PLANNER_TOOL_SET,
        }
    }

    /// T-D-B-19: 当前场景下的 knowledge_scope。
    fn effective_knowledge_scope(&self) -> &[MemoryLayer] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("planner")
                .map(|p| p.knowledge_scope)
                .unwrap_or(&PLANNER_KNOWLEDGE_SCOPE),
            _ => &PLANNER_KNOWLEDGE_SCOPE,
        }
    }

    /// T-D-B-19: 当前场景下写入 TeamContext 的 label。
    fn effective_context_label(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("planner")
                .map(|p| p.context_label)
                .unwrap_or("plan"),
            _ => "plan",
        }
    }

    /// T-D-B-19: 当前场景下的角色分工提示(嵌入 user prompt)。
    /// 写作场景用写作角色名,编程场景用编程角色名。
    fn effective_role_lineup(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => "Writer / Reviewer / Researcher / Formatter",
            _ => "Coder / Writer / Reviewer / Researcher / Planner",
        }
    }
}

#[async_trait]
// T-D-B-17: 角色 agent 保留以支持向后兼容(scenarios.json / gRPC / 旧 API)。
// kind() 与 run() 引用废弃的 AgentKind::Planner,此处显式放行废弃警告。
#[allow(deprecated)]
impl Agent for PlannerAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Planner
    }
    fn name(&self) -> &str {
        "Planner"
    }
    fn system_prompt(&self) -> &str {
        // T-D-B-19: 根据场景返回对应提示词。
        self.effective_system_prompt()
    }
    fn description(&self) -> &str {
        "Decomposes tasks (coding) or structures the outline (writing) and arbitrates agent conflicts."
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
        // T-D-B-19: 根据场景选择角色分工提示(写作场景用写作角色名)。
        let role_lineup = self.effective_role_lineup();
        let msgs = vec![
            ChatMessage::system(self.effective_system_prompt()),
            ChatMessage::user(format!(
                "Plan the following task. Break it into sub-tasks (max depth 3) \
                 and assign each to the most suitable agent in the team \
                 ({role_lineup}).\n\n\
                 Task:\n{task}\n\n\
                 Previous team context:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        // T-D-B-19: 根据场景选择 TeamContext label(写作场景用 "outline")。
        ctx.push_str(self.name(), self.effective_context_label(), &body);
        info!(target: "nebula.swarm", agent = %self.name(), "planner finished");
        // T-D-B-17: 同时填充 scenario 字段,供新代码读取场景标签。
        // T-D-B-19: 写作场景下打 Writing 标签,其他打 Planning。
        let scenario_tag = match self.scenario {
            Some(AgentScenario::Writing) => AgentScenario::Writing,
            _ => AgentScenario::Planning,
        };
        Ok(AgentOutput::new(AgentKind::Planner, self.name(), body).with_scenario(scenario_tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmGateway;
    use std::sync::Arc;

    #[test]
    fn planner_tool_set_and_knowledge_scope() {
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()));
        assert_eq!(agent.tool_set(), &["memory_search", "tool_invoke"]);
        assert_eq!(agent.knowledge_scope(), &[MemoryLayer::L2, MemoryLayer::L5]);
    }

    #[test]
    fn planner_system_prompt_mentions_tools_and_scope() {
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Planner"));
        assert!(prompt.contains("memory_search"));
        assert!(prompt.contains("L2"));
        assert!(prompt.contains("L5"));
    }

    // ---- T-D-B-19: 写作场景(大纲规划)测试 ----

    #[test]
    fn planner_default_scenario_is_none() {
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()));
        assert!(agent.current_scenario().is_none());
    }

    #[test]
    fn planner_with_writing_scenario_switches_system_prompt() {
        // Writing 场景 → system_prompt 应切换到大纲规划提示词(含 Outline Planner)。
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
        let prompt = agent.system_prompt();
        assert!(
            prompt.to_lowercase().contains("outline planner"),
            "writing scenario prompt should mention Outline Planner: {prompt}"
        );
    }

    #[test]
    fn planner_with_writing_scenario_knowledge_scope_includes_l5() {
        // 写作场景知识边界应含 L5(经验教训,供大纲借鉴过往结构)。
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert!(agent.knowledge_scope().contains(&MemoryLayer::L5));
    }

    #[test]
    fn planner_with_writing_scenario_keeps_memory_search() {
        // 大纲规划需要 memory_search,写作场景应保留。
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert!(
            agent.tool_set().contains(&"memory_search"),
            "outline planner should keep memory_search"
        );
    }

    #[test]
    fn planner_with_coding_scenario_keeps_coding_behavior() {
        // Coding 场景应保持编程场景行为。
        let agent = PlannerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Coding);
        let prompt = agent.system_prompt();
        assert!(
            prompt.contains("Planner"),
            "Coding scenario should keep planner prompt: {prompt}"
        );
    }
}

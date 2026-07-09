//! Coder agent — produces Rust code in response to a task description.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::memory::types::MemoryLayer;
use crate::swarm::context::TeamContext;

use super::{writing_role_profile, Agent, AgentKind, AgentOutput, AgentScenario};

/// T-6: Coder 可用工具集(编程场景)。
const CODER_TOOL_SET: [&str; 4] = ["shell", "editor_read", "editor_write", "tool_invoke"];
/// T-6: Coder 可访问的记忆层级(编程场景)。
const CODER_KNOWLEDGE_SCOPE: [MemoryLayer; 2] = [MemoryLayer::L2, MemoryLayer::L3];
/// T-6: Dify 风格 system prompt(角色定位 + 工具指引 + 知识边界,编程场景)。
const CODER_SYSTEM_PROMPT: &str = "You are the Coder agent in the nebula swarm.\n\
     Role: produce concise, well-tested Rust code that satisfies the task.\n\
     Tools: shell, editor_read, editor_write, tool_invoke.\n\
     Knowledge scope: L2 (cross-session experience) and L3 (concrete facts).\n\
     Always explain trade-offs in 2-3 sentences at the end.";

pub struct CoderAgent {
    llm: Arc<LlmGateway>,
    /// T-D-B-19: 场景标签(None = 编程场景,Some(Writing) = 写作场景)。
    scenario: Option<AgentScenario>,
}

impl CoderAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self {
            llm,
            scenario: None,
        }
    }

    /// T-D-B-19: Builder — 注入场景标签,切换到对应场景行为。
    ///
    /// - `AgentScenario::Writing` → 排版格式化师(写作场景)
    /// - 其他值或未调用 → Coder(编程场景,向后兼容)
    pub fn with_scenario(mut self, scenario: AgentScenario) -> Self {
        self.scenario = Some(scenario);
        self
    }

    /// T-D-B-19: 当前场景标签(主要用于测试与诊断)。
    pub fn current_scenario(&self) -> Option<AgentScenario> {
        self.scenario
    }

    /// T-D-B-19: 当前场景下的 system prompt。
    /// Writing 场景返回写作提示词(排版格式化),其他返回编程提示词。
    fn effective_system_prompt(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("coder")
                .map(|p| p.system_prompt)
                .unwrap_or(CODER_SYSTEM_PROMPT),
            _ => CODER_SYSTEM_PROMPT,
        }
    }

    /// T-D-B-19: 当前场景下的 tool_set。
    fn effective_tool_set(&self) -> &[&str] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("coder")
                .map(|p| p.tool_set)
                .unwrap_or(&CODER_TOOL_SET),
            _ => &CODER_TOOL_SET,
        }
    }

    /// T-D-B-19: 当前场景下的 knowledge_scope。
    fn effective_knowledge_scope(&self) -> &[MemoryLayer] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("coder")
                .map(|p| p.knowledge_scope)
                .unwrap_or(&CODER_KNOWLEDGE_SCOPE),
            _ => &CODER_KNOWLEDGE_SCOPE,
        }
    }

    /// T-D-B-19: 当前场景下写入 TeamContext 的 label。
    fn effective_context_label(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("coder")
                .map(|p| p.context_label)
                .unwrap_or("code"),
            _ => "code",
        }
    }
}

#[async_trait]
// T-D-B-17: 角色 agent 保留以支持向后兼容(scenarios.json / gRPC / 旧 API)。
// kind() 与 run() 引用废弃的 AgentKind::Coder,此处显式放行废弃警告。
#[allow(deprecated)]
impl Agent for CoderAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Coder
    }
    fn name(&self) -> &str {
        "Coder"
    }
    fn system_prompt(&self) -> &str {
        // T-D-B-19: 根据场景返回对应提示词。
        self.effective_system_prompt()
    }
    fn description(&self) -> &str {
        "Writes Rust code (coding) or formats Markdown (writing) in response to a task."
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
        // T-D-B-19: 根据场景选择 TeamContext label(写作场景用 "formatting")。
        ctx.push_str(self.name(), self.effective_context_label(), &body);
        info!(target: "nebula.swarm", agent = %self.name(), "coder finished");
        // T-D-B-17: 同时填充 scenario 字段,供新代码读取场景标签。
        // T-D-B-19: 写作场景下打 Writing 标签,其他打 Coding。
        let scenario_tag = match self.scenario {
            Some(AgentScenario::Writing) => AgentScenario::Writing,
            _ => AgentScenario::Coding,
        };
        Ok(AgentOutput::new(AgentKind::Coder, self.name(), body).with_scenario(scenario_tag))
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
        assert_eq!(agent.knowledge_scope(), &[MemoryLayer::L2, MemoryLayer::L3]);
    }

    #[test]
    fn coder_system_prompt_mentions_tools_and_scope() {
        let agent = CoderAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.system_prompt();
        assert!(prompt.contains("Coder"));
        assert!(prompt.contains("shell"));
        assert!(prompt.contains("L2"));
    }

    // ---- T-D-B-19: 写作场景(排版格式化)测试 ----

    #[test]
    fn coder_default_scenario_is_none() {
        // 默认(未注入场景)→ scenario = None,行为为编程场景。
        let agent = CoderAgent::new(Arc::new(LlmGateway::new_test()));
        assert!(agent.current_scenario().is_none());
    }

    #[test]
    fn coder_with_writing_scenario_switches_system_prompt() {
        // Writing 场景 → system_prompt 应切换到排版格式化提示词(含 Formatter)。
        let agent =
            CoderAgent::new(Arc::new(LlmGateway::new_test())).with_scenario(AgentScenario::Writing);
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
        let prompt = agent.system_prompt();
        assert!(
            prompt.to_lowercase().contains("formatter"),
            "writing scenario prompt should mention Formatter: {prompt}"
        );
        // 编程场景的 shell 工具指引不应出现在写作提示词中。
        assert!(
            !prompt.contains("produce concise, well-tested Rust code"),
            "writing prompt should not mention Rust code: {prompt}"
        );
    }

    #[test]
    fn coder_with_writing_scenario_tool_set_differs_from_coding() {
        // 写作场景工具集不应含 shell(排版格式化不需要执行命令)。
        let coding_agent = CoderAgent::new(Arc::new(LlmGateway::new_test()));
        let writing_agent =
            CoderAgent::new(Arc::new(LlmGateway::new_test())).with_scenario(AgentScenario::Writing);
        assert!(coding_agent.tool_set().contains(&"shell"));
        assert!(
            !writing_agent.tool_set().contains(&"shell"),
            "formatter should not have shell"
        );
    }

    #[test]
    fn coder_with_writing_scenario_knowledge_scope_includes_l3() {
        // 写作场景知识边界应含 L3(具体事实,供排版引用)。
        let agent =
            CoderAgent::new(Arc::new(LlmGateway::new_test())).with_scenario(AgentScenario::Writing);
        assert!(agent.knowledge_scope().contains(&MemoryLayer::L3));
    }

    #[test]
    fn coder_with_writing_scenario_returns_to_coding_when_not_writing() {
        // 非 Writing 场景(如 Coding)应回退到编程场景行为。
        let agent =
            CoderAgent::new(Arc::new(LlmGateway::new_test())).with_scenario(AgentScenario::Coding);
        let prompt = agent.system_prompt();
        assert!(
            prompt.contains("Coder"),
            "Coding scenario should keep coding prompt: {prompt}"
        );
        assert!(agent.tool_set().contains(&"shell"));
    }

    #[test]
    fn coder_with_scenario_builder_returns_self_for_chaining() {
        // builder 应返回 Self 供链式调用。
        let agent =
            CoderAgent::new(Arc::new(LlmGateway::new_test())).with_scenario(AgentScenario::Writing);
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
    }
}

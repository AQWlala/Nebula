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
use tokio::sync::{broadcast, Mutex};
use tracing::info;

use crate::llm::{ChatMessage, LlmGateway};
use crate::swarm::bus::BusMessage;
use crate::swarm::context::TeamContext;
use crate::swarm::events::SwarmEvent;

use crate::llm::dispatcher::{UnifiedModelDispatcher, WorkType};

use super::{Agent, AgentOutput};

/// One worker in the swarm.  Identified by an ordinal (1-based) and
/// sharing the same general-purpose system prompt.
pub struct GenericAgent {
    llm: Arc<LlmGateway>,
    id: u32,
    /// T-E-S-02: 工具注册表，用于 function calling。
    tool_registry: Arc<crate::tools::ToolRegistry>,
    /// T-S3-B-02: 唯一名称 (Agent-{id})，用 Box::leak 获得 &'static str。
    /// 这样 AgentBus 注册时每个 agent 有独立 key，避免 P2P 消息冲突。
    name: &'static str,
    /// T-S3-B-02: P2P 消息接收端，由 orchestrator 通过 set_mailbox 注入。
    mailbox: Mutex<Option<tokio::sync::mpsc::Receiver<BusMessage>>>,
    /// T-E-S-39: persona 缓存(SOUL.md/AGENTS.md/TOOLS.md)，由 orchestrator
    /// 通过 set_persona 注入。run() 时拼接到 system prompt 前缀。
    persona: Mutex<Option<Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>>>,
    /// T-E-D-10: SwarmEvent 广播发送器，用于 emit 工具调用等事件。
    event_sender: Mutex<Option<broadcast::Sender<SwarmEvent>>>,
    /// T-E-D-10: 当前任务 ID，emit 事件时使用。
    task_id: Mutex<Option<String>>,
    /// T-E-D-10: agent 角色，emit 事件时使用。
    agent_role: Mutex<Option<String>>,
    /// M3 #48: 注入的统一调度器(可选)。
    ///
    /// 启用后,无工具路径的 LLM 调用会走 `dispatch(SwarmWorker, msgs)`,
    /// 享受 ModelPolicy + 断路器 + 限流 + 缓存 + 成本统计。
    /// SwarmWorker 默认走本地 Ollama(可由 ModelRouter 动态升级到远端)。
    ///
    /// 未注入时回退到 `self.llm.chat(msgs)`(向后兼容)。
    /// 带工具路径(tool_loop)仍走 LlmGateway,等 Phase 3 完整迁移。
    dispatcher: Mutex<Option<Arc<UnifiedModelDispatcher>>>,
}

impl GenericAgent {
    /// Create a new generic agent with the given 1-based ordinal.
    pub fn new(
        llm: Arc<LlmGateway>,
        id: u32,
        tool_registry: Arc<crate::tools::ToolRegistry>,
    ) -> Self {
        let name: &'static str = Box::leak(format!("Agent-{id}").into_boxed_str());
        Self {
            llm,
            id,
            tool_registry,
            name,
            mailbox: Mutex::new(None),
            persona: Mutex::new(None),
            event_sender: Mutex::new(None),
            task_id: Mutex::new(None),
            agent_role: Mutex::new(None),
            dispatcher: Mutex::new(None),
        }
    }

    /// M3 #48: 注入 `UnifiedModelDispatcher`。
    ///
    /// 启用后,无工具路径的 LLM 调用会走 `dispatch(SwarmWorker, msgs)`。
    /// 未注入时回退到 `self.llm.chat(msgs)`(向后兼容)。
    pub async fn set_dispatcher(&self, dispatcher: Arc<UnifiedModelDispatcher>) {
        *self.dispatcher.lock().await = Some(dispatcher);
    }

    /// M3 #48: Builder-style 注入 dispatcher。
    pub fn with_dispatcher(mut self, dispatcher: Arc<UnifiedModelDispatcher>) -> Self {
        // Mutex::new 是 const fn,可在非 async 上下文调用。
        self.dispatcher = Mutex::new(Some(dispatcher));
        self
    }

    /// M3 #48: 是否已注入 dispatcher(主要用于测试与诊断)。
    pub async fn has_dispatcher(&self) -> bool {
        self.dispatcher.lock().await.is_some()
    }

    /// Human-readable label used in logs and the front-end.
    pub fn label(&self) -> String {
        self.name.to_string()
    }

    /// General-purpose system prompt.  The `{id}` placeholder is
    /// filled at construction time.
    pub fn system_prompt(&self) -> String {
        format!(
            "You are Agent-{id} in the nebula swarm — a multi-agent \
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

    /// T-E-S-02: 按 tool_set() 过滤 ToolRegistry，构造 LLM tools 参数。
    ///
    /// 过滤规则:
    /// - tool_set() 为空 → 返回空(向后兼容，GenericAgent 默认无 tool_set)
    /// - tool_set() 含精确工具名 → 只含该工具
    /// - tool_set() 含 "tool_invoke" → 匹配所有 mcp_* 工具
    fn build_tools_for_llm(&self) -> Vec<crate::llm::ToolSpec> {
        let tool_set = self.tool_set();
        if tool_set.is_empty() {
            return Vec::new();
        }

        let has_tool_invoke = tool_set.contains(&"tool_invoke");
        let exact_set: std::collections::HashSet<&str> = tool_set.iter().copied().collect();

        self.tool_registry
            .list_all()
            .into_iter()
            .filter(|(name, _, _)| {
                exact_set.contains(name.as_str()) || (has_tool_invoke && name.starts_with("mcp_"))
            })
            .map(|(name, description, schema)| {
                crate::llm::ToolSpec::function(name, description, schema)
            })
            .collect()
    }
}

#[async_trait]
impl Agent for GenericAgent {
    fn kind(&self) -> super::AgentKind {
        super::AgentKind::Generic
    }

    fn name(&self) -> &str {
        // T-S3-B-02: 返回唯一名称 "Agent-{id}"，用于 AgentBus 注册。
        self.name
    }

    fn system_prompt(&self) -> &str {
        // Same constraint as `name`: we return a static reference.
        // The actual prompt is built at call time in `run`.
        "You are a general-purpose agent in the nebula swarm."
    }

    fn description(&self) -> &str {
        "General-purpose swarm agent that works independently on any task."
    }

    async fn run(&self, task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        // T-E-S-39: 若 persona 已注入且非空,拼接到 system prompt 最前。
        let sys_prompt = {
            let persona_prefix = self.persona.lock().await.as_ref().and_then(|pc| {
                let guard = pc.read();
                if guard.is_empty() {
                    None
                } else {
                    Some(guard.to_system_prefix())
                }
            });
            match persona_prefix {
                Some(pp) => format!("{pp}\n{}", self.system_prompt()),
                None => self.system_prompt().to_string(),
            }
        };

        let msgs = vec![
            ChatMessage::system(&sys_prompt),
            ChatMessage::user(format!(
                "Task:\n{task}\n\nTeam context so far:\n{}",
                ctx.render()
            )),
        ];

        // T-E-S-02: 按 tool_set() 过滤 ToolRegistry 构造 tools 参数。
        let tools = self.build_tools_for_llm();

        let body = if tools.is_empty() {
            // M3 #48: 优先走 dispatcher(若注入),否则回退直连 llm。
            {
                let dispatcher_opt = self.dispatcher.lock().await.clone();
                if let Some(dispatcher) = dispatcher_opt {
                    let resp = dispatcher.dispatch(WorkType::SwarmWorker, msgs).await?;
                    resp.message.content
                } else {
                    let resp = self.llm.chat(msgs).await?;
                    resp.message.content
                }
            }
        } else {
            // 有工具:进入 tool_loop。
            // T-E-D-10: 尝试获取 swarm 上下文（event_sender、task_id、agent_role）
            let event_sender = self.event_sender.lock().await.clone();
            let task_id = self.task_id.lock().await.clone();
            let agent_role = self.agent_role.lock().await.clone();

            if let (Some(sender), Some(tid), Some(role)) = (event_sender, task_id, agent_role) {
                // 在 swarm 上下文中，使用带事件的 tool_loop
                crate::swarm::tool_loop::run_tool_loop_with_events(
                    &self.llm,
                    &self.tool_registry,
                    msgs,
                    tools,
                    sender,
                    self.name(),
                    &role,
                    &tid,
                )
                .await?
            } else {
                // 不在 swarm 上下文中，使用默认的 tool_loop（向后兼容）
                crate::swarm::run_tool_loop_default(&self.llm, &self.tool_registry, msgs, tools)
                    .await?
            }
        };

        ctx.push_str(self.name(), "generic", &body);
        info!(
            target: "nebula.swarm",
            agent = %self.name(),
            "generic agent finished"
        );
        Ok(AgentOutput::new(
            super::AgentKind::Generic,
            self.name(),
            body,
        ))
    }

    /// T-S3-B-02: 覆写 set_mailbox，接收 P2P 消息通道。
    fn set_mailbox(&self, rx: tokio::sync::mpsc::Receiver<BusMessage>) {
        // 使用 blocking_lock 因为 set_mailbox 不是 async
        // 这在 agent 构造阶段调用是安全的
        if let Ok(mut guard) = self.mailbox.try_lock() {
            *guard = Some(rx);
        }
    }

    /// T-E-S-39: 覆写 set_persona，缓存 persona Arc 供 run() 使用。
    fn set_persona(&self, persona: Arc<parking_lot::RwLock<crate::llm::persona::PersonaConfig>>) {
        if let Ok(mut guard) = self.persona.try_lock() {
            *guard = Some(persona);
        }
    }

    /// T-E-D-10: 设置 SwarmEvent 发射器、task_id 和 agent_role。
    fn set_swarm_context(
        &self,
        event_sender: broadcast::Sender<SwarmEvent>,
        task_id: String,
        agent_role: String,
    ) {
        if let Ok(mut guard) = self.event_sender.try_lock() {
            *guard = Some(event_sender);
        }
        if let Ok(mut guard) = self.task_id.try_lock() {
            *guard = Some(task_id);
        }
        if let Ok(mut guard) = self.agent_role.try_lock() {
            *guard = Some(agent_role);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Tool, ToolOutput};

    struct DummyTool;
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "dummy"
        }
        fn description(&self) -> &str {
            "A dummy tool for testing"
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object" })
        }
        fn call(&self, _args: serde_json::Value) -> anyhow::Result<ToolOutput> {
            Ok(ToolOutput {
                success: true,
                result: "ok".to_string(),
                error: None,
            })
        }
    }

    #[test]
    fn build_tools_for_llm_empty_when_tool_set_empty() {
        // GenericAgent 默认 tool_set() 为空,build_tools_for_llm 返回空。
        let llm = Arc::new(crate::llm::LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        // 即使注册了工具,因 tool_set() 为空,build_tools_for_llm 仍返回空。
        registry.register(Arc::new(DummyTool));
        let agent = GenericAgent::new(llm, 1, registry);
        assert!(agent.build_tools_for_llm().is_empty());
    }

    // M3 #48: dispatcher 未注入时 has_dispatcher 返回 false。
    #[tokio::test]
    async fn test_has_dispatcher_returns_false_without_dispatcher() {
        let llm = Arc::new(crate::llm::LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        let agent = GenericAgent::new(llm, 1, registry);
        assert!(!agent.has_dispatcher().await);
    }

    // M3 #48: with_dispatcher 注入后 has_dispatcher 返回 true。
    #[tokio::test]
    async fn test_with_dispatcher_sets_dispatcher() {
        use crate::llm::dispatcher::{ModelPolicy, UnifiedModelDispatcher};

        let llm = Arc::new(crate::llm::LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        let policy = ModelPolicy::new(
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            std::collections::HashMap::new(),
        );
        let dispatcher = Arc::new(UnifiedModelDispatcher::new(
            llm.clone(),
            policy,
            None,
            None,
            2,
        ));
        let agent = GenericAgent::new(llm, 2, registry).with_dispatcher(dispatcher);
        assert!(agent.has_dispatcher().await);
    }

    // M3 #48: set_dispatcher 异步注入。
    #[tokio::test]
    async fn test_set_dispatcher_injects_async() {
        use crate::llm::dispatcher::{ModelPolicy, UnifiedModelDispatcher};

        let llm = Arc::new(crate::llm::LlmGateway::new_test());
        let registry = Arc::new(crate::tools::ToolRegistry::new());
        let policy = ModelPolicy::new(
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "ollama".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            "qwen2.5:3b".to_string(),
            "qwen2.5:7b".to_string(),
            std::collections::HashMap::new(),
        );
        let dispatcher = Arc::new(UnifiedModelDispatcher::new(
            llm.clone(),
            policy,
            None,
            None,
            2,
        ));
        let agent = GenericAgent::new(llm, 3, registry);
        assert!(!agent.has_dispatcher().await);
        agent.set_dispatcher(dispatcher).await;
        assert!(agent.has_dispatcher().await);
    }
}

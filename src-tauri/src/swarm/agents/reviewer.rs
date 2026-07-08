//! Reviewer agent — reviews the most recent work in the team context.
//!
//! T-E-L-03: ReviewerAgent 升级为 CheckerAgent — 直接升级现有 reviewer.rs,
//! 不新建 maker_checker.rs。新增能力(分 4 个 commit):
//! - Commit 1: 模型同质检测(detect_model_homogeneity)
//! - Commit 2: 对抗 prompt(adversarial_prompt)
//! - Commit 3: 独立 Context 通道(Checker 用独立 TeamContext + ShadowWorkspaceEngine worktree 隔离)
//! - Commit 4: 自动降级 L4→L2(检测到模型同质时,enforce_homogeneity_policy 自动降级)
//!
//! 数据主权红线:Checker 不能用闭源模型(Claude/GPT),必须用本地 Ollama。
//! 当 Maker 与 Checker 用同一闭源模型时,模型同质检测会触发自动降级 L4→L2,
//! 因为同模型自审无意义。

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
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

/// T-E-L-03: 模型描述符 — (provider, model_name) 二元组,用于模型同质检测。
///
/// `provider` 来自 `LlmGateway::provider()`(如 `"ollama"` / `"deepseek"` /
/// `"anthropic"` / `"openai-compat"`),`model_name` 来自
/// `LlmGateway::default_model()`(如 `"qwen2.5:7b"` / `"deepseek-chat"` /
/// `"claude-3-5-haiku-20241022"`)。
///
/// 两个描述符相等(provider+model_name 均相同)即视为「模型同质」。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// LLM provider 名(如 `"ollama"` / `"deepseek"` / `"anthropic"`)。
    pub provider: String,
    /// 模型名(如 `"qwen2.5:7b"` / `"deepseek-chat"`)。
    pub model_name: String,
}

impl ModelDescriptor {
    pub fn new(provider: impl Into<String>, model_name: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model_name: model_name.into(),
        }
    }
}

/// T-E-L-03: 模型同质警告 — 当 Maker 和 Checker 用同一模型时返回。
///
/// 同模型自审无意义(模型不会发现自己生成的输出的盲点,
/// confirmation bias),触发此警告时调用方应将自主度从 L4 自动降级到 L2
/// (Commit 4 的 `enforce_homogeneity_policy` 实现此降级逻辑)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomogeneityWarning {
    /// Maker 使用的模型描述符。
    pub maker: ModelDescriptor,
    /// Checker 使用的模型描述符。
    pub checker: ModelDescriptor,
    /// 触发原因(人类可读,用于 warn 日志)。
    pub reason: String,
}

pub struct ReviewerAgent {
    llm: Arc<LlmGateway>,
    /// T-E-L-03: Maker 使用的模型描述符(可选,未设置时不做同质检测)。
    /// 由 `with_maker_model()` 注入;Checker 据此与自身模型比较。
    maker_model: Option<ModelDescriptor>,
}

impl ReviewerAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self {
            llm,
            maker_model: None,
        }
    }

    /// T-E-L-03: Builder — 注入 Maker 的模型描述符,启用模型同质检测。
    ///
    /// 调用方(MasterAgent / Loop Orchestrator)在生成 Maker 输出后,
    /// 把 Maker 实际使用的 (provider, model_name) 传给 Checker,
    /// Checker 据此在 [`detect_model_homogeneity`] 中与自身模型比较。
    pub fn with_maker_model(mut self, maker: ModelDescriptor) -> Self {
        self.maker_model = Some(maker);
        self
    }

    /// T-E-L-03: 返回 Checker 自身的模型描述符。
    ///
    /// 从 `LlmGateway::provider()` 与 `LlmGateway::default_model()` 读取,
    /// 不需要调用方注入。
    pub fn checker_model(&self) -> ModelDescriptor {
        ModelDescriptor::new(self.llm.provider(), self.llm.default_model())
    }

    /// T-E-L-03: 检测 Maker 与 Checker 是否使用同一模型。
    ///
    /// 比较维度:`provider` + `model_name`(大小写敏感,精确匹配)。
    /// 当两者完全相等时返回 [`HomogeneityWarning`],否则返回 `None`。
    ///
    /// **触发条件**:
    /// - `maker_model` 未注入(旧调用方)→ 永远返回 `None`(向后兼容)
    /// - `maker_model` 已注入且与 Checker 自身模型相等 → 返回警告
    /// - 不等 → 返回 `None`
    ///
    /// **设计依据**:同模型自审无意义 — 模型很难发现自己生成输出的盲点
    /// (confirmation bias),Maker-Checker 模式要求 Checker 用不同模型
    /// (理想情况:不同家族,如 Maker 用 DeepSeek、Checker 用 Qwen)。
    pub fn detect_model_homogeneity(&self) -> Option<HomogeneityWarning> {
        let maker = self.maker_model.as_ref()?;
        let checker = self.checker_model();
        if maker == &checker {
            Some(HomogeneityWarning {
                maker: maker.clone(),
                checker: checker.clone(),
                reason: "Maker and Checker use the same provider+model — \
                         self-review is meaningless (confirmation bias)"
                    .to_string(),
            })
        } else {
            None
        }
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

    // ---- T-E-L-03 Commit 1: 模型同质检测测试 ----

    #[test]
    fn checker_model_reads_provider_and_default_model_from_gateway() {
        // new_test() 用 provider="ollama" + default_model="test-model"
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let desc = agent.checker_model();
        assert_eq!(desc.provider, "ollama");
        assert_eq!(desc.model_name, "test-model");
    }

    #[test]
    fn detect_model_homogeneity_returns_none_when_maker_model_not_injected() {
        // 向后兼容:旧调用方未调用 with_maker_model() 时,不触发同质检测。
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        assert!(agent.detect_model_homogeneity().is_none());
    }

    #[test]
    fn detect_model_homogeneity_returns_warning_when_maker_equals_checker() {
        // Maker 与 Checker 都用 ollama + test-model → 同质
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let warning = agent
            .detect_model_homogeneity()
            .expect("same model should trigger warning");
        assert_eq!(warning.maker.provider, "ollama");
        assert_eq!(warning.maker.model_name, "test-model");
        assert_eq!(warning.checker.provider, "ollama");
        assert_eq!(warning.checker.model_name, "test-model");
        assert!(
            warning.reason.contains("self-review"),
            "reason should mention self-review: {}",
            warning.reason
        );
    }

    #[test]
    fn detect_model_homogeneity_returns_none_when_models_differ() {
        // Maker 用 deepseek,Checker 用 ollama → 不同质
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("deepseek", "deepseek-chat"));
        assert!(
            agent.detect_model_homogeneity().is_none(),
            "different providers should not trigger homogeneity"
        );
    }

    #[test]
    fn detect_model_homogeneity_returns_none_when_only_model_name_differs() {
        // 同 provider 不同 model_name → 不算同质(模型能力可能差异很大)
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "qwen2.5:32b"));
        // Checker 是 ollama + test-model,Maker 是 ollama + qwen2.5:32b
        assert!(
            agent.detect_model_homogeneity().is_none(),
            "same provider but different model_name should not trigger homogeneity"
        );
    }

    #[test]
    fn detect_model_homogeneity_case_sensitive() {
        // provider/model_name 大小写敏感(精确匹配,避免 Ollama 与 OLLAMA 误判)
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("OLLAMA", "test-model"));
        assert!(
            agent.detect_model_homogeneity().is_none(),
            "case-sensitive comparison: OLLAMA != ollama"
        );
    }

    #[test]
    fn model_descriptor_equality_uses_provider_and_model_name() {
        let a = ModelDescriptor::new("ollama", "qwen2.5:7b");
        let b = ModelDescriptor::new("ollama", "qwen2.5:7b");
        let c = ModelDescriptor::new("ollama", "qwen2.5:32b");
        let d = ModelDescriptor::new("deepseek", "qwen2.5:7b");
        assert_eq!(a, b, "same provider+model → equal");
        assert_ne!(a, c, "different model_name → not equal");
        assert_ne!(a, d, "different provider → not equal");
    }

    #[test]
    fn model_descriptor_serializes_to_json() {
        // 序列化兼容性:可被 serde_json 序列化(供前端 / 日志使用)
        let desc = ModelDescriptor::new("ollama", "qwen2.5:7b");
        let json = serde_json::to_string(&desc).expect("serialize");
        assert!(json.contains("\"provider\":\"ollama\""), "got: {json}");
        assert!(
            json.contains("\"model_name\":\"qwen2.5:7b\""),
            "got: {json}"
        );
        let de: ModelDescriptor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de, desc);
    }

    #[test]
    fn homogeneity_warning_serializes_to_json() {
        let warning = HomogeneityWarning {
            maker: ModelDescriptor::new("deepseek", "deepseek-chat"),
            checker: ModelDescriptor::new("ollama", "test-model"),
            reason: "test reason".to_string(),
        };
        let json = serde_json::to_string(&warning).expect("serialize");
        assert!(json.contains("\"maker\""));
        assert!(json.contains("\"checker\""));
        assert!(json.contains("\"reason\":\"test reason\""));
        let de: HomogeneityWarning = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de, warning);
    }

    #[test]
    fn with_maker_model_builder_returns_self_for_chaining() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        assert!(agent.maker_model.is_some());
        assert!(agent.detect_model_homogeneity().is_some());
    }
}

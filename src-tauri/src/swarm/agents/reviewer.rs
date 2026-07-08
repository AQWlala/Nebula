//! Reviewer agent — reviews the most recent work in the team context.
//!
//! T-E-L-03: ReviewerAgent 升级为 CheckerAgent — 直接升级现有 reviewer.rs,
//! 不新建 maker_checker.rs。新增能力(分 4 个 commit):
//! - Commit 1: 模型同质检测(detect_model_homogeneity) ✅
//! - Commit 2: 对抗 prompt(adversarial_prompt) ✅
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

    /// T-E-L-03: 生成对抗性审查 prompt。
    ///
    /// 与默认的「请审查以下工作」不同,对抗 prompt 显式要求 Checker
    /// **假设 Maker 的输出存在致命缺陷**,并尽力找出。这能突破模型
    /// 默认的「顺从/附和」倾向(sycophancy),迫使 Checker 主动寻找:
    ///
    /// - 隐藏的假设与边界条件
    /// - 安全漏洞与权限提升路径
    /// - 错误的推理链与逻辑谬误
    /// - 性能瓶颈与资源泄漏
    /// - 与既有代码/规范的冲突
    ///
    /// **设计依据**(7 专家评审):普通审查 prompt 容易让 Checker 沦为
    /// 「橡皮图章」(rubber-stamp),尤其在 Maker 输出看起来合理时。
    /// 对抗 prompt 把 Checker 的默认立场从「验证正确性」翻转为
    /// 「证伪」(falsification),这是科学方法的核心 — 一个无法被证伪
    /// 的方案才是稳妥的。
    ///
    /// **降级策略**:即使检测到模型同质(Commit 1),对抗 prompt 仍会
    /// 发给 Checker — 同模型虽难以发现自己输出的盲点,但在对抗 prompt
    /// 引导下仍可能发现部分问题(降级到 L2 后由人类最终裁决)。
    ///
    /// `work` 是 Maker 的输出文本(代码 / 文档 / 方案)。
    /// 返回的字符串可直接作为 `ChatMessage::user()` 的内容。
    pub fn adversarial_prompt(&self, work: &str) -> String {
        format!(
            "You are the Checker agent in a Maker-Checker pipeline.\n\
             Your role is ADVERSARIAL review — assume the work below has FATAL flaws.\n\n\
             Probe aggressively for:\n\
             1. Hidden assumptions and unstated edge cases\n\
             2. Security vulnerabilities and privilege escalation paths\n\
             3. Logical fallacies and incorrect reasoning chains\n\
             4. Performance bottlenecks and resource leaks\n\
             5. Conflicts with existing code, specs, or conventions\n\
             6. Missing error handling and recovery paths\n\
             7. Test coverage gaps for failure modes\n\n\
             --- WORK TO REVIEW ---\n{work}\n--- END WORK ---\n\n\
             Falsify the work: try hard to break it. Only if you genuinely\n\
             cannot find any flaw after thorough analysis, emit APPROVE.\n\
             Otherwise emit REVISE (fixable) or REJECT (fundamentally broken).\n\
             End your response with one of: APPROVE / REVISE / REJECT."
        )
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

    // ---- T-E-L-03 Commit 2: 对抗 prompt 测试 ----

    #[test]
    fn adversarial_prompt_includes_work_content() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.adversarial_prompt("fn add(a, b) { a + b }");
        assert!(
            prompt.contains("fn add(a, b) { a + b }"),
            "adversarial prompt must include the work content verbatim"
        );
    }

    #[test]
    fn adversarial_prompt_instructs_adversarial_review() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.adversarial_prompt("some work");
        // 核心指令:假设有致命缺陷 + 对抗审查
        assert!(
            prompt.to_lowercase().contains("fatal"),
            "prompt should assume FATAL flaws: {prompt}"
        );
        assert!(
            prompt.to_lowercase().contains("adversarial"),
            "prompt should mention ADVERSARIAL review: {prompt}"
        );
    }

    #[test]
    fn adversarial_prompt_probes_specific_flaw_categories() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.adversarial_prompt("work").to_lowercase();
        // 7 个探测维度(至少检查关键的几个)
        assert!(
            prompt.contains("security"),
            "should probe for security vulnerabilities"
        );
        assert!(prompt.contains("edge case"), "should probe for edge cases");
        assert!(
            prompt.contains("error handling"),
            "should probe for missing error handling"
        );
        assert!(
            prompt.contains("performance"),
            "should probe for performance issues"
        );
    }

    #[test]
    fn adversarial_prompt_ends_with_verdict_instruction() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.adversarial_prompt("work");
        // 必须以 APPROVE / REVISE / REJECT 三选一结尾(与原 system prompt 一致)
        assert!(
            prompt.contains("APPROVE"),
            "prompt must mention APPROVE verdict"
        );
        assert!(
            prompt.contains("REVISE"),
            "prompt must mention REVISE verdict"
        );
        assert!(
            prompt.contains("REJECT"),
            "prompt must mention REJECT verdict"
        );
    }

    #[test]
    fn adversarial_prompt_uses_falsification_language() {
        // 对抗 prompt 的核心是「证伪」而非「验证」
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.adversarial_prompt("work").to_lowercase();
        assert!(
            prompt.contains("falsif") || prompt.contains("break"),
            "prompt should use falsification/break language: {prompt}"
        );
    }

    #[test]
    fn adversarial_prompt_handles_empty_work() {
        // 空工作内容不应 panic
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let prompt = agent.adversarial_prompt("");
        assert!(
            !prompt.is_empty(),
            "prompt should not be empty even for empty work"
        );
        assert!(
            prompt.contains("APPROVE"),
            "verdict instruction must still be present"
        );
    }

    #[test]
    fn adversarial_prompt_is_deterministic() {
        // 相同输入应产生相同输出(无随机性)
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let p1 = agent.adversarial_prompt("work X");
        let p2 = agent.adversarial_prompt("work X");
        assert_eq!(p1, p2, "adversarial_prompt must be deterministic");
    }
}

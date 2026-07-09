//! Reviewer agent — reviews the most recent work in the team context.
//!
//! T-E-L-03: ReviewerAgent 升级为 CheckerAgent — 直接升级现有 reviewer.rs,
//! 不新建 maker_checker.rs。新增能力(分 4 个 commit):
//! - Commit 1: 模型同质检测(detect_model_homogeneity) ✅
//! - Commit 2: 对抗 prompt(adversarial_prompt) ✅
//! - Commit 3: 独立 Context 通道(Checker 用独立 TeamContext + ShadowWorkspaceEngine worktree 隔离) ✅
//! - Commit 4: 自动降级 L4→L2(检测到模型同质时,enforce_homogeneity_policy 自动降级) ✅
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
use crate::shadow_workspace::engine::ShadowWorkspaceEngine;
use crate::swarm::context::TeamContext;

use super::{writing_role_profile, Agent, AgentKind, AgentOutput, AgentScenario};

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

/// T-E-L-03 Commit 3: 独立审查的预备上下文。
///
/// 由 [`ReviewerAgent::prepare_independent_review`] 生成,包含 Checker 独立审查
/// 所需的全部信息(独立 context + LLM messages + 可选的 worktree id)。
/// 供 [`ReviewerAgent::run_independent`] 使用,也可单独测试(无需 LLM / git)。
//
// 注:不 derive Debug,因为 TeamContext 未实现 Debug(避免修改 context.rs)。
pub struct IndependentReviewSetup {
    /// 独立的 TeamContext(不引用 Maker 的 context,避免被锚定)。
    pub context: TeamContext,
    /// 传给 LLM 的 messages(system + adversarial prompt)。
    pub messages: Vec<ChatMessage>,
    /// 如果创建了 Shadow Workspace worktree,这是其 id(供 cleanup)。
    /// `None` 表示未创建 worktree(降级模式 — 无 shadow_engine 或 create 失败)。
    pub workspace_id: Option<String>,
}

/// T-E-L-03 Commit 4: 模型同质策略执行结果。
///
/// 由 [`ReviewerAgent::enforce_homogeneity_policy`] 返回,告知调用方
/// (MasterAgent / LoopEngine)是否需要降级自主度。
///
/// **设计依据**(7 专家评审):同模型自审无意义 — 当 Maker 与 Checker 用
/// 同一闭源模型时,模型不会发现自己生成输出的盲点(confirmation bias),
/// 此时 L4 蜂群(Maker-Checker 双 Agent)模式形同虚设,应自动降级到
/// L2 对话模式(由人类最终裁决)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomogeneityPolicy {
    /// 检测到模型同质,自主度已从 L4 降级到 L2。
    ///
    /// 调用方应据此将实际执行自主度降为 L2,并记审计日志 +
    /// 向用户发送降级通知(IM webhook / 通知中心)。
    Enforced {
        /// 原始自主度(应为 L4)。
        original_level: crate::autonomy::AutonomyLevel,
        /// 降级后的自主度(L2 对话模式,由人类最终裁决)。
        downgraded_to: crate::autonomy::AutonomyLevel,
        /// 触发降级的同质警告(含 maker/checker 模型描述 + reason)。
        warning: HomogeneityWarning,
    },
    /// 当前自主度为 L4 但未检测到模型同质,无需降级。
    ///
    /// 可能原因:`maker_model` 未注入(旧调用方),或 Maker 与 Checker
    /// 用不同模型(理想情况 — 不同家族,如 Maker 用 DeepSeek、Checker 用 Qwen)。
    NoHomogeneity {
        /// 当前自主度(未变更,L4)。
        level: crate::autonomy::AutonomyLevel,
    },
    /// 当前自主度非 L4,不涉及 Maker-Checker 双 Agent 模式,无需同质检测。
    ///
    /// 同质检测仅在 L4(蜂群 + ApprovalGate,Maker + Checker 双 Agent)下
    /// 有意义。L0-L3 是单 Agent 模式,L5 后台自动化有更严格的 Checker 要求
    /// (需强 Checker + 审计日志),不在本策略范围。
    NotCheckerMode {
        /// 当前自主度。
        level: crate::autonomy::AutonomyLevel,
    },
}

pub struct ReviewerAgent {
    llm: Arc<LlmGateway>,
    /// T-E-L-03: Maker 使用的模型描述符(可选,未设置时不做同质检测)。
    /// 由 `with_maker_model()` 注入;Checker 据此与自身模型比较。
    maker_model: Option<ModelDescriptor>,
    /// T-E-L-03 Commit 3: Shadow Workspace 引擎(可选,启用 worktree 隔离)。
    /// 由 `with_shadow_workspace()` 注入;Checker 在独立 worktree 中执行,
    /// 避免污染主仓库。未注入时降级为无 worktree 模式。
    shadow_engine: Option<Arc<ShadowWorkspaceEngine>>,
    /// T-D-B-19: 场景标签(None = 编程场景,Some(Writing) = 写作场景)。
    scenario: Option<AgentScenario>,
}

// T-D-B-17: run_independent 引用废弃的 AgentKind::Reviewer,显式放行废弃警告。
#[allow(deprecated)]
impl ReviewerAgent {
    pub fn new(llm: Arc<LlmGateway>) -> Self {
        Self {
            llm,
            maker_model: None,
            shadow_engine: None,
            scenario: None,
        }
    }

    /// T-D-B-19: Builder — 注入场景标签,切换到对应场景行为。
    ///
    /// - `AgentScenario::Writing` → 校对编辑(写作场景)
    /// - 其他值或未调用 → Reviewer(编程场景,向后兼容)
    pub fn with_scenario(mut self, scenario: AgentScenario) -> Self {
        self.scenario = Some(scenario);
        self
    }

    /// T-D-B-19: 当前场景标签(主要用于测试与诊断)。
    pub fn current_scenario(&self) -> Option<AgentScenario> {
        self.scenario
    }

    /// T-D-B-19: 当前场景下的 system prompt。
    /// Writing 场景返回写作提示词(校对编辑),其他返回编程提示词(代码审查)。
    fn effective_system_prompt(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("reviewer")
                .map(|p| p.system_prompt)
                .unwrap_or(REVIEWER_SYSTEM_PROMPT),
            _ => REVIEWER_SYSTEM_PROMPT,
        }
    }

    /// T-D-B-19: 当前场景下的 tool_set。
    fn effective_tool_set(&self) -> &[&str] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("reviewer")
                .map(|p| p.tool_set)
                .unwrap_or(&REVIEWER_TOOL_SET),
            _ => &REVIEWER_TOOL_SET,
        }
    }

    /// T-D-B-19: 当前场景下的 knowledge_scope。
    fn effective_knowledge_scope(&self) -> &[MemoryLayer] {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("reviewer")
                .map(|p| p.knowledge_scope)
                .unwrap_or(&REVIEWER_KNOWLEDGE_SCOPE),
            _ => &REVIEWER_KNOWLEDGE_SCOPE,
        }
    }

    /// T-D-B-19: 当前场景下写入 TeamContext 的 label。
    fn effective_context_label(&self) -> &str {
        match self.scenario {
            Some(AgentScenario::Writing) => writing_role_profile("reviewer")
                .map(|p| p.context_label)
                .unwrap_or("review"),
            _ => "review",
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

    /// T-E-L-03 Commit 3: Builder — 注入 ShadowWorkspaceEngine,启用 worktree 隔离。
    ///
    /// 启用后,[`run_independent`] 会在独立的 git worktree 中执行 Checker,
    /// 避免污染主仓库。未注入时降级为无 worktree 模式(记 warn 日志)。
    pub fn with_shadow_workspace(mut self, engine: Arc<ShadowWorkspaceEngine>) -> Self {
        self.shadow_engine = Some(engine);
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

    // ---- T-E-L-03 Commit 3: 独立 Context 通道 + worktree 隔离 ----

    /// T-E-L-03 Commit 3: 准备独立审查上下文。
    ///
    /// 与默认 [`Agent::run`] 的关键区别:
    /// 1. 创建**独立的空 TeamContext**(不引用 Maker 的 context,避免被锚定)
    /// 2. 用 [`adversarial_prompt`] 而非默认的「请审查以下工作」
    /// 3. 如果有 shadow_engine,创建临时 worktree 供 Checker 隔离执行
    ///
    /// **降级策略**:无 shadow_engine 或 `create()` 失败时,
    /// `workspace_id` 为 `None`,Checker 仍在独立 context 中执行,
    /// 只是无 worktree 隔离(记 warn 日志)。
    pub fn prepare_independent_review(&self, maker_work: &str) -> Result<IndependentReviewSetup> {
        let context = TeamContext::new();

        // 尝试创建 worktree(可选,失败降级)。
        // worktree 让 Checker 能在隔离的 git 分支中 read 文件验证 Maker 的代码,
        // 不污染主仓库。create() 失败的常见原因:repo_root 未设置 / 非 git 仓库。
        let workspace_id = if let Some(engine) = &self.shadow_engine {
            match engine.create(
                "checker-independent-review".to_string(),
                None, // base = current branch
            ) {
                Ok(ws) => {
                    // 把 worktree path 写入独立 context,让 Checker 知道在哪读文件。
                    context.push_str(
                        "Checker",
                        "workspace",
                        &format!(
                            "Independent review worktree created at: {}\n\
                             Checker may read files in this path to verify Maker's work.",
                            ws.path
                        ),
                    );
                    Some(ws.id)
                }
                Err(e) => {
                    tracing::warn!(
                        target: "nebula.swarm.checker",
                        error = %e,
                        "shadow workspace create failed, degrading to no-worktree mode"
                    );
                    None
                }
            }
        } else {
            tracing::warn!(
                target: "nebula.swarm.checker",
                "no shadow_engine injected, Checker running without worktree isolation"
            );
            None
        };

        let messages = vec![
            ChatMessage::system(self.system_prompt()),
            ChatMessage::user(self.adversarial_prompt(maker_work)),
        ];

        Ok(IndependentReviewSetup {
            context,
            messages,
            workspace_id,
        })
    }

    /// T-E-L-03 Commit 3: 清理独立审查资源(abort worktree)。
    ///
    /// 如果 `workspace_id` 为 `Some`,调用 `shadow_engine.abort()` 清理 worktree。
    /// abort 失败仅记 warn(不阻断主流程 — worktree 会泄漏但 Checker 仍完成)。
    pub fn cleanup_independent_review(&self, workspace_id: &Option<String>) {
        if let (Some(id), Some(engine)) = (workspace_id, &self.shadow_engine) {
            if let Err(e) = engine.abort(id) {
                tracing::warn!(
                    target: "nebula.swarm.checker",
                    workspace_id = %id,
                    error = %e,
                    "failed to abort checker worktree (leaking workspace)"
                );
            }
        }
    }

    /// T-E-L-03 Commit 3: 独立审查执行模式。
    ///
    /// 与默认 [`Agent::run`] 的区别:
    /// - 用**独立 TeamContext**(不引用 Maker context,避免锚定)
    /// - 用 [`adversarial_prompt`](对抗 prompt)而非默认 prompt
    /// - 在 Shadow Workspace worktree 中隔离执行(可选)
    ///
    /// 流程:`prepare → llm.chat → cleanup → 返回 AgentOutput`
    /// 审查结果写入独立 context(不污染 Maker 的 context)。
    pub async fn run_independent(&self, maker_work: &str) -> Result<AgentOutput> {
        let setup = self.prepare_independent_review(maker_work)?;

        let resp = self.llm.chat(setup.messages).await?;
        let body = resp.message.content;

        // 把审查结果写入独立 context(不污染 Maker 的 context)。
        setup
            .context
            .push_str(self.name(), "adversarial_review", &body);

        self.cleanup_independent_review(&setup.workspace_id);

        let verdict = if body.contains("APPROVE") {
            0.9
        } else if body.contains("REVISE") {
            0.6
        } else {
            0.2
        };
        info!(
            target: "nebula.swarm",
            agent = %self.name(),
            verdict,
            "checker independent review finished"
        );
        Ok(AgentOutput::new(AgentKind::Reviewer, self.name(), body)
            .with_confidence(verdict)
            .with_scenario(AgentScenario::Review))
    }

    // ---- T-E-L-03 Commit 4: 自动降级 L4→L2 ----

    /// T-E-L-03 Commit 4: 根据模型同质检测结果决定是否降级自主度 L4→L2。
    ///
    /// 调用方(MasterAgent / LoopEngine)在执行 L4 蜂群任务前应先调用此方法:
    /// - 若返回 `Enforced`,实际执行自主度应降为 L2(由人类最终裁决),
    ///   并记审计日志 + 向用户发送降级通知。
    /// - 若返回 `NoHomogeneity`,L4 模式可正常进行(Checker 与 Maker 不同模型)。
    /// - 若返回 `NotCheckerMode`,当前非 L4,无需同质检测。
    ///
    /// **降级目标固定为 L2**(而非 L3)的设计依据:
    /// - L3 仍是单 Agent Plan 模式,无 Checker 独立审查;
    /// - L2 对话模式让人类直接介入裁决,是最安全的降级目标;
    /// - 7 专家评审一致同意:L4 失效时退到 L2 而非 L3。
    ///
    /// **幂等性**:此方法是纯函数(只读 `self.maker_model` + 调用
    /// `detect_model_homogeneity`),无副作用,可重复调用。
    pub fn enforce_homogeneity_policy(
        &self,
        current_level: crate::autonomy::AutonomyLevel,
    ) -> HomogeneityPolicy {
        use crate::autonomy::AutonomyLevel;
        match current_level {
            AutonomyLevel::L4Swarm => {
                if let Some(warning) = self.detect_model_homogeneity() {
                    HomogeneityPolicy::Enforced {
                        original_level: current_level,
                        downgraded_to: AutonomyLevel::L2Chat,
                        warning,
                    }
                } else {
                    HomogeneityPolicy::NoHomogeneity {
                        level: current_level,
                    }
                }
            }
            other => HomogeneityPolicy::NotCheckerMode { level: other },
        }
    }
}

#[async_trait]
// T-D-B-17: 角色 agent 保留以支持向后兼容(scenarios.json / gRPC / 旧 API)。
// kind() 与 run() 引用废弃的 AgentKind::Reviewer,此处显式放行废弃警告。
#[allow(deprecated)]
impl Agent for ReviewerAgent {
    fn kind(&self) -> AgentKind {
        AgentKind::Reviewer
    }
    fn name(&self) -> &str {
        "Reviewer"
    }
    fn system_prompt(&self) -> &str {
        // T-D-B-19: 根据场景返回对应提示词。
        self.effective_system_prompt()
    }
    fn description(&self) -> &str {
        "Reviews the team's work (coding) or proofreads the manuscript (writing) and emits an APPROVE / REVISE / REJECT verdict."
    }
    fn tool_set(&self) -> &[&str] {
        // T-D-B-19: 根据场景返回对应工具集。
        self.effective_tool_set()
    }
    fn knowledge_scope(&self) -> &[MemoryLayer] {
        // T-D-B-19: 根据场景返回对应知识边界。
        self.effective_knowledge_scope()
    }

    async fn run(&self, _task: &str, ctx: &TeamContext) -> Result<AgentOutput> {
        let msgs = vec![
            ChatMessage::system(self.effective_system_prompt()),
            ChatMessage::user(format!(
                "Review the most recent work in this team context:\n{}",
                ctx.render()
            )),
        ];
        let resp = self.llm.chat(msgs).await?;
        let body = resp.message.content;
        // T-D-B-19: 根据场景选择 TeamContext label(写作场景用 "copyedit")。
        ctx.push_str(self.name(), self.effective_context_label(), &body);
        let verdict = if body.contains("APPROVE") {
            0.9
        } else if body.contains("REVISE") {
            0.6
        } else {
            0.2
        };
        info!(target: "nebula.swarm", agent = %self.name(), verdict, "reviewer finished");
        // T-D-B-17: 同时填充 scenario 字段,供新代码读取场景标签。
        // T-D-B-19: 写作场景下打 Writing 标签,其他打 Review。
        let scenario_tag = match self.scenario {
            Some(AgentScenario::Writing) => AgentScenario::Writing,
            _ => AgentScenario::Review,
        };
        Ok(AgentOutput::new(AgentKind::Reviewer, self.name(), body)
            .with_confidence(verdict)
            .with_scenario(scenario_tag))
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

    // ---- T-E-L-03 Commit 3: 独立 Context 通道 + worktree 隔离测试 ----

    #[test]
    fn prepare_independent_review_creates_empty_context() {
        // 独立 context 应初始为空(不包含 Maker 的 context 条目)
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let setup = agent
            .prepare_independent_review("some work")
            .expect("prepare should succeed without shadow_engine");
        assert!(
            setup.context.snapshot().is_empty(),
            "independent context should be empty initially (no shadow_engine)"
        );
    }

    #[test]
    fn prepare_independent_review_uses_adversarial_prompt() {
        // user message 应该是对抗 prompt(包含 FATAL / adversarial 指令)
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let setup = agent
            .prepare_independent_review("fn foo() {}")
            .expect("prepare should succeed");
        let user_msg = setup
            .messages
            .iter()
            .find(|m| m.role == "user")
            .expect("should have a user message");
        assert!(
            user_msg.content.to_lowercase().contains("fatal"),
            "user message should be adversarial (FATAL): {}",
            user_msg.content
        );
        assert!(
            user_msg.content.to_lowercase().contains("adversarial"),
            "user message should mention ADVERSARIAL: {}",
            user_msg.content
        );
        assert!(
            user_msg.content.contains("fn foo() {}"),
            "user message should include maker work verbatim"
        );
    }

    #[test]
    fn prepare_independent_review_without_shadow_engine_has_no_workspace() {
        // 不注入 shadow_engine → workspace_id 应为 None
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let setup = agent
            .prepare_independent_review("work")
            .expect("prepare should succeed");
        assert!(
            setup.workspace_id.is_none(),
            "no shadow_engine → no workspace"
        );
    }

    #[test]
    fn prepare_independent_review_with_unconfigured_shadow_engine_degrades() {
        // shadow_engine 存在但 repo_root 未设置 → create() 失败 → 降级
        use crate::shadow_workspace::engine::ShadowWorkspaceEngine;
        let engine = Arc::new(ShadowWorkspaceEngine::with_default());
        // repo_root 默认 None(未调用 set_repo_root)
        let agent =
            ReviewerAgent::new(Arc::new(LlmGateway::new_test())).with_shadow_workspace(engine);
        let setup = agent
            .prepare_independent_review("work")
            .expect("prepare should succeed even when worktree create fails");
        assert!(
            setup.workspace_id.is_none(),
            "unconfigured shadow_engine should degrade to no-workspace"
        );
        // context 应仍然为空(worktree 没创建,没写 path)
        assert!(
            setup.context.snapshot().is_empty(),
            "context should be empty when worktree creation failed"
        );
    }

    #[test]
    fn prepare_independent_review_does_not_include_maker_context() {
        // 关键不变式:Checker 的 context 不应包含 Maker 的 context 条目
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let setup = agent
            .prepare_independent_review("maker output")
            .expect("prepare should succeed");
        for entry in setup.context.snapshot() {
            assert!(
                !entry.author.contains("Maker") && !entry.label.contains("maker"),
                "independent context should not include Maker entries, found: {:?}",
                entry
            );
        }
    }

    #[test]
    fn prepare_independent_review_has_system_and_user_messages() {
        // messages 应包含 system + user 两条(对抗 prompt)
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let setup = agent
            .prepare_independent_review("work")
            .expect("prepare should succeed");
        assert_eq!(
            setup.messages.len(),
            2,
            "should have system + user messages"
        );
        assert_eq!(setup.messages[0].role, "system");
        assert_eq!(setup.messages[1].role, "user");
    }

    #[test]
    fn cleanup_independent_review_without_workspace_is_noop() {
        // workspace_id = None → cleanup 是 noop,不应 panic
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        agent.cleanup_independent_review(&None);
    }

    #[test]
    fn cleanup_independent_review_without_shadow_engine_is_noop() {
        // 无 shadow_engine 时,即使 workspace_id = Some(不可能但防御性测试),
        // cleanup 不应 panic
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let fake_id = Some("fake-id".to_string());
        agent.cleanup_independent_review(&fake_id);
    }

    #[test]
    fn with_shadow_workspace_builder_returns_self_for_chaining() {
        use crate::shadow_workspace::engine::ShadowWorkspaceEngine;
        let engine = Arc::new(ShadowWorkspaceEngine::with_default());
        let agent =
            ReviewerAgent::new(Arc::new(LlmGateway::new_test())).with_shadow_workspace(engine);
        assert!(agent.shadow_engine.is_some(), "shadow_engine should be set");
    }

    // ---- T-E-L-03 Commit 4: 自动降级 L4→L2 测试 ----

    #[test]
    fn enforce_policy_l4_with_homogeneity_returns_enforced() {
        // L4 + Maker 与 Checker 同模型 → Enforced,降级到 L2
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        match &policy {
            HomogeneityPolicy::Enforced {
                original_level,
                downgraded_to,
                warning,
            } => {
                assert_eq!(*original_level, AutonomyLevel::L4Swarm);
                assert_eq!(*downgraded_to, AutonomyLevel::L2Chat);
                assert_eq!(warning.maker.provider, "ollama");
                assert_eq!(warning.checker.model_name, "test-model");
            }
            other => panic!("expected Enforced, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l4_without_maker_model_returns_no_homogeneity() {
        // L4 + 未注入 maker_model(旧调用方)→ NoHomogeneity(向后兼容)
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        match policy {
            HomogeneityPolicy::NoHomogeneity { level } => {
                assert_eq!(level, AutonomyLevel::L4Swarm);
            }
            other => panic!("expected NoHomogeneity, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l4_with_different_models_returns_no_homogeneity() {
        // L4 + Maker 与 Checker 不同模型 → NoHomogeneity(理想情况)
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("deepseek", "deepseek-chat"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        match policy {
            HomogeneityPolicy::NoHomogeneity { level } => {
                assert_eq!(level, AutonomyLevel::L4Swarm);
            }
            other => panic!("expected NoHomogeneity, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l0_returns_not_checker_mode() {
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L0InlineCompletion);
        match policy {
            HomogeneityPolicy::NotCheckerMode { level } => {
                assert_eq!(level, AutonomyLevel::L0InlineCompletion);
            }
            other => panic!("expected NotCheckerMode, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l1_returns_not_checker_mode() {
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L1DirectedEdit);
        match policy {
            HomogeneityPolicy::NotCheckerMode { level } => {
                assert_eq!(level, AutonomyLevel::L1DirectedEdit);
            }
            other => panic!("expected NotCheckerMode, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l2_returns_not_checker_mode() {
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L2Chat);
        match policy {
            HomogeneityPolicy::NotCheckerMode { level } => {
                assert_eq!(level, AutonomyLevel::L2Chat);
            }
            other => panic!("expected NotCheckerMode, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l3_returns_not_checker_mode() {
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L3Plan);
        match policy {
            HomogeneityPolicy::NotCheckerMode { level } => {
                assert_eq!(level, AutonomyLevel::L3Plan);
            }
            other => panic!("expected NotCheckerMode, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_l5_returns_not_checker_mode() {
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L5Background);
        match policy {
            HomogeneityPolicy::NotCheckerMode { level } => {
                assert_eq!(level, AutonomyLevel::L5Background);
            }
            other => panic!("expected NotCheckerMode, got {other:?}"),
        }
    }

    #[test]
    fn enforce_policy_enforced_downgraded_to_is_always_l2() {
        // 关键不变式:降级目标永远是 L2Chat(而非 L3 Plan)
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        if let HomogeneityPolicy::Enforced { downgraded_to, .. } = policy {
            assert_eq!(
                downgraded_to,
                AutonomyLevel::L2Chat,
                "downgraded_to must always be L2Chat (human final adjudication)"
            );
        } else {
            panic!("expected Enforced when models are homogeneous");
        }
    }

    #[test]
    fn enforce_policy_enforced_preserves_warning_details() {
        // Enforced 分支应完整保留 warning(maker + checker 描述符 + reason)
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let policy = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        if let HomogeneityPolicy::Enforced { warning, .. } = policy {
            assert_eq!(
                warning.maker, warning.checker,
                "homogeneous → maker == checker"
            );
            assert!(!warning.reason.is_empty(), "reason should not be empty");
            assert!(
                warning.reason.contains("self-review") || warning.reason.contains("confirmation"),
                "reason should explain confirmation bias: {}",
                warning.reason
            );
        } else {
            panic!("expected Enforced");
        }
    }

    #[test]
    fn enforce_policy_is_idempotent() {
        // 幂等性:重复调用结果一致(纯函数,无副作用)
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        let p1 = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        let p2 = agent.enforce_homogeneity_policy(AutonomyLevel::L4Swarm);
        assert_eq!(p1, p2, "enforce_homogeneity_policy must be idempotent");
    }

    #[test]
    fn enforce_policy_all_six_levels_covered() {
        // 遍历所有 6 个等级,确保每个都有明确的策略结果
        use crate::autonomy::AutonomyLevel;
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        for &level in AutonomyLevel::all() {
            let policy = agent.enforce_homogeneity_policy(level);
            match policy {
                HomogeneityPolicy::Enforced { .. } => {
                    assert_eq!(level, AutonomyLevel::L4Swarm, "Enforced only for L4");
                }
                HomogeneityPolicy::NoHomogeneity { .. } => {
                    assert_eq!(
                        level,
                        AutonomyLevel::L4Swarm,
                        "NoHomogeneity only for L4 (without maker_model or diff models)"
                    );
                }
                HomogeneityPolicy::NotCheckerMode { .. } => {
                    assert_ne!(
                        level,
                        AutonomyLevel::L4Swarm,
                        "NotCheckerMode for non-L4 levels"
                    );
                }
            }
        }
    }

    // ---- T-D-B-19: 写作场景(校对编辑)测试 ----

    #[test]
    fn reviewer_default_scenario_is_none() {
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()));
        assert!(agent.current_scenario().is_none());
    }

    #[test]
    fn reviewer_with_writing_scenario_switches_system_prompt() {
        // Writing 场景 → system_prompt 应切换到校对编辑提示词(含 Copy Editor)。
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
        let prompt = agent.system_prompt();
        assert!(
            prompt.to_lowercase().contains("copy editor"),
            "writing scenario prompt should mention Copy Editor: {prompt}"
        );
        // 编程场景的代码审查提示不应出现在写作提示词中。
        assert!(
            !prompt.contains("critically evaluate the latest work"),
            "writing prompt should not mention code review: {prompt}"
        );
    }

    #[test]
    fn reviewer_with_writing_scenario_keeps_readonly_tools() {
        // 校对编辑应保持只读(无 editor_write/shell),与编程场景一致。
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert!(!agent.tool_set().contains(&"shell"));
        assert!(!agent.tool_set().contains(&"editor_write"));
        assert!(agent.tool_set().contains(&"editor_read"));
    }

    #[test]
    fn reviewer_with_writing_scenario_knowledge_scope_is_l2() {
        // 校对编辑知识边界为 L2(与编程场景一致,只读审查无需深层记忆)。
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing);
        assert_eq!(agent.knowledge_scope(), &[MemoryLayer::L2]);
    }

    #[test]
    fn reviewer_with_scenario_and_maker_model_chains() {
        // with_scenario 与 with_maker_model 应可链式组合。
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Writing)
            .with_maker_model(ModelDescriptor::new("ollama", "test-model"));
        assert_eq!(agent.current_scenario(), Some(AgentScenario::Writing));
        assert!(agent.maker_model.is_some());
        // 链式组合后同质检测仍应工作。
        assert!(agent.detect_model_homogeneity().is_some());
    }

    #[test]
    fn reviewer_with_coding_scenario_keeps_coding_behavior() {
        // Coding 场景应保持编程场景行为(代码审查)。
        let agent = ReviewerAgent::new(Arc::new(LlmGateway::new_test()))
            .with_scenario(AgentScenario::Coding);
        let prompt = agent.system_prompt();
        assert!(
            prompt.contains("Reviewer"),
            "Coding scenario should keep reviewer prompt: {prompt}"
        );
    }
}

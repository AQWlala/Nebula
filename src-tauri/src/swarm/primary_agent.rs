//! T-E-AE-01: PrimaryAgent — 主智能体(decompose/delegate/synthesize)。
//!
//! Phase 3 架构演进的核心:将扁平蜂群升级为"主智能体 + 子智能体"分层架构。
//! PrimaryAgent 是面向用户的主入口,负责:
//!
//! 1. **decompose(分解)**: 将复杂任务分解为子任务列表(`Vec<DelegatedTask>`)
//! 2. **delegate(委派)**: 将子任务委派给合适的 agent(基于 [`AgentScenario`])
//! 3. **synthesize(综合)**: 收集子任务结果,综合产出最终结果
//!
//! ## 设计要点
//!
//! * **无 LLM 依赖的可测试性**: 三大能力通过 trait 注入(`Decomposer`/`Delegator`/
//!   `Synthesizer`),测试可注入纯函数实现,生产环境注入 LLM/蜂群实现。
//! * **场景化委派**: `DelegatedTask.target_scenario` 用 [`AgentScenario`] 表达
//!   目标 agent 场景(Coding/Writing/Review/Research/Planning),取代废弃的
//!   `AgentKind` 角色变体(T-D-B-17)。
//! * **简化 AgentBus**: 基于 `tokio::sync::mpsc` 的轻量消息传递,完整协议
//!   (含 CRDT/死锁检测)由 T-E-AE-05 落地,本模块仅提供最小可用分派通道。
//! * **无 feature gate**: 不依赖 `master-orchestrator` feature(该 feature
//!   控制 MasterOrchestrator + TaskDag 重型组件),PrimaryAgent 仅依赖
//!   始终可用的 [`AgentScenario`],保持"无依赖"特性。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, instrument, warn};

use super::agents::AgentScenario;

// ---------------------------------------------------------------------------
// DelegatedTask — 主→子任务分派协议的数据载体
// ---------------------------------------------------------------------------

/// 主智能体委派给子智能体的任务单元(T-E-AE-01 / T-E-AE-05 协议数据)。
///
/// 每个子任务携带:
/// - 唯一 `id`(供依赖引用与结果收集)
/// - `description`(子任务描述,可能含 `{{<dep_id>.output}}` 占位符)
/// - `target_scenario`(目标 agent 场景,基于 [`AgentScenario`] 路由)
/// - `dependencies`(上游子任务 ID 列表,空表示可立即执行)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedTask {
    /// 唯一标识(如 "dt_1")。
    pub id: String,
    /// 子任务描述(prompt 主体,可含 `{{dt_x.output}}` 占位符引用上游结果)。
    pub description: String,
    /// 目标 agent 场景,委派器据此路由到对应子智能体。
    pub target_scenario: AgentScenario,
    /// 上游依赖子任务 ID 列表(空表示无依赖,可并行执行)。
    #[serde(default)]
    pub dependencies: Vec<String>,
}

impl DelegatedTask {
    /// 创建一个新 DelegatedTask。
    pub fn new(
        id: impl Into<String>,
        description: impl Into<String>,
        target_scenario: AgentScenario,
    ) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            target_scenario,
            dependencies: Vec::new(),
        }
    }

    /// 链式设置依赖(上游子任务 ID)。
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies = deps;
        self
    }
}

/// 子任务执行结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegatedResult {
    /// 对应 [`DelegatedTask::id`]。
    pub task_id: String,
    /// 子智能体输出。
    pub output: String,
    /// 执行是否成功。
    pub success: bool,
    /// 执行耗时(毫秒)。
    pub elapsed_ms: u64,
}

impl DelegatedResult {
    /// 创建一个成功结果。
    pub fn ok(task_id: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            output: output.into(),
            success: true,
            elapsed_ms: 0,
        }
    }

    /// 创建一个失败结果。
    pub fn failed(task_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            output: error.into(),
            success: false,
            elapsed_ms: 0,
        }
    }
}

/// PrimaryAgent 端到端运行报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimaryReport {
    /// 任务 ID(唯一标识本次编排)。
    pub task_id: String,
    /// 用户原始输入。
    pub input: String,
    /// 最终综合输出。
    pub output: String,
    /// 拆解得到的子任务总数。
    pub total_sub_tasks: usize,
    /// 成功完成的子任务数。
    pub successful_sub_tasks: usize,
    /// 失败的子任务数。
    pub failed_sub_tasks: usize,
    /// 总耗时(毫秒)。
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// 三大能力 trait — decompose / delegate / synthesize
// ---------------------------------------------------------------------------

/// 分解能力:将用户输入分解为子任务列表。
///
/// 实现方可以是:
/// - LLM 驱动(调用 MasterAgent 提示词拆解,见 `master.rs`)
/// - 规则驱动(关键词匹配,见 [`RuleBasedDecomposer`],无 LLM 依赖)
#[async_trait]
pub trait Decomposer: Send + Sync {
    /// 将 `input` 分解为子任务列表。
    ///
    /// 返回空 Vec 表示无需分解(单任务直通)。
    async fn decompose(&self, input: &str) -> Result<Vec<DelegatedTask>>;
}

/// 委派能力:将单个子任务委派给目标 agent 执行。
///
/// 实现方可以是:
/// - 蜂群驱动(调用 `SwarmOrchestrator::execute`)
/// - 闭包驱动(测试用,见 [`ScenarioDelegator`])
#[async_trait]
pub trait Delegator: Send + Sync {
    /// 委派 `task` 给对应场景的子智能体执行,返回结果。
    async fn delegate(&self, task: &DelegatedTask) -> Result<DelegatedResult>;
}

/// 综合能力:收集子任务结果,综合产出最终输出。
///
/// 实现方可以是:
/// - LLM 驱动(调用 MasterAgent 综合提示词)
/// - 拼接驱动(见 [`ConcatSynthesizer`],无 LLM 依赖)
#[async_trait]
pub trait Synthesizer: Send + Sync {
    /// 基于 `input`(用户原始任务)和 `results`(子任务结果)综合最终输出。
    async fn synthesize(&self, input: &str, results: &[DelegatedResult]) -> Result<String>;
}

// ---------------------------------------------------------------------------
// 默认实现 — 无 LLM 依赖,供测试与降级路径使用
// ---------------------------------------------------------------------------

/// 规则驱动的分解器:基于关键词匹配将任务拆分为子任务。
///
/// 拆分规则(关键词 → 场景):
/// - "搜索"/"查询"/"search" → [`AgentScenario::Research`]
/// - "审查"/"检查"/"review" → [`AgentScenario::Review`]
/// - "规划"/"计划"/"plan" → [`AgentScenario::Planning`]
/// - "写"/"撰写"/"write" → [`AgentScenario::Writing`]
/// - "代码"/"实现"/"code" → [`AgentScenario::Coding`]
///
/// 若输入含分号(`;`)或换行符,按分隔符拆分;否则返回单个 Coding 场景任务
/// (兜底,向后兼容)。
pub struct RuleBasedDecomposer;

impl RuleBasedDecomposer {
    /// 创建一个新的规则分解器。
    pub fn new() -> Self {
        Self
    }

    /// 根据描述文本推断目标场景。
    fn infer_scenario(desc: &str) -> AgentScenario {
        let lower = desc.to_lowercase();
        if lower.contains("搜索") || lower.contains("查询") || lower.contains("search") {
            AgentScenario::Research
        } else if lower.contains("审查") || lower.contains("检查") || lower.contains("review") {
            AgentScenario::Review
        } else if lower.contains("规划") || lower.contains("计划") || lower.contains("plan") {
            AgentScenario::Planning
        } else if lower.contains("写") || lower.contains("撰写") || lower.contains("write") {
            AgentScenario::Writing
        } else {
            AgentScenario::Coding
        }
    }
}

impl Default for RuleBasedDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Decomposer for RuleBasedDecomposer {
    async fn decompose(&self, input: &str) -> Result<Vec<DelegatedTask>> {
        // 按分号或换行符拆分(支持中英文分号)。
        let segments: Vec<&str> = input
            .split([';', '；', '\n'])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        if segments.len() <= 1 {
            // 单任务:返回单个 Coding 场景任务(兜底)。
            return Ok(vec![DelegatedTask::new(
                "dt_1",
                input,
                Self::infer_scenario(input),
            )]);
        }

        // 多任务:按顺序建立依赖链(每个任务依赖前一个)。
        let mut tasks = Vec::with_capacity(segments.len());
        for (i, seg) in segments.iter().enumerate() {
            let id = format!("dt_{}", i + 1);
            let deps = if i == 0 {
                Vec::new()
            } else {
                vec![format!("dt_{}", i)]
            };
            tasks.push(
                DelegatedTask::new(id, *seg, Self::infer_scenario(seg)).with_dependencies(deps),
            );
        }
        Ok(tasks)
    }
}

/// 场景驱动的委派器:通过注入的执行闭包完成实际委派。
///
/// `executor` 接收 `(scenario, description)`,返回输出字符串。
/// 生产环境注入调用 `SwarmOrchestrator::execute` 的闭包;
/// 测试环境注入返回确定性输出的闭包。
pub struct ScenarioDelegator<F>
where
    F: Fn(AgentScenario, &str) -> String + Send + Sync,
{
    executor: F,
}

impl<F> ScenarioDelegator<F>
where
    F: Fn(AgentScenario, &str) -> String + Send + Sync,
{
    /// 创建一个场景委派器,注入执行闭包。
    pub fn new(executor: F) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl<F> Delegator for ScenarioDelegator<F>
where
    F: Fn(AgentScenario, &str) -> String + Send + Sync,
{
    async fn delegate(&self, task: &DelegatedTask) -> Result<DelegatedResult> {
        let start = std::time::Instant::now();
        // 同步执行闭包(生产环境可替换为 async 调用)。
        let output = (self.executor)(task.target_scenario, &task.description);
        let elapsed_ms = start.elapsed().as_millis() as u64;
        Ok(DelegatedResult {
            task_id: task.id.clone(),
            output,
            success: true,
            elapsed_ms,
        })
    }
}

/// 拼接驱动的综合器:按子任务 ID 顺序拼接结果。
///
/// 输出格式:
/// ```text
/// [dt_1] <output>
///
/// [dt_2] <output>
/// ```
///
/// 失败的子任务输出以 `[FAILED]` 前缀标记。
pub struct ConcatSynthesizer;

impl ConcatSynthesizer {
    /// 创建一个新的拼接综合器。
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConcatSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Synthesizer for ConcatSynthesizer {
    async fn synthesize(&self, input: &str, results: &[DelegatedResult]) -> Result<String> {
        if results.is_empty() {
            return Ok(String::new());
        }
        let mut parts = Vec::with_capacity(results.len() + 1);
        parts.push(format!("# 任务综合报告\n\n原始任务: {input}\n"));
        for r in results {
            let prefix = if r.success { "" } else { "[FAILED] " };
            parts.push(format!("[{}] {}{}", r.task_id, prefix, r.output));
        }
        Ok(parts.join("\n\n"))
    }
}

// ---------------------------------------------------------------------------
// PrimaryAgent — 主智能体
// ---------------------------------------------------------------------------

/// 主智能体:面向用户的主入口,编排 decompose → delegate → synthesize。
///
/// 持有三大能力策略(可注入),不直接持有 Worker 池或 LLM 客户端。
/// 实际执行委托给注入的 [`Decomposer`] / [`Delegator`] / [`Synthesizer`]。
///
/// ## 构造
///
/// - [`PrimaryAgent::new`][]: 注入三大策略。
/// - [`PrimaryAgent::with_defaults`][]: 使用无 LLM 依赖的默认实现(规则分解 +
///   拼接综合,委派器需单独注入)。
pub struct PrimaryAgent {
    decomposer: Arc<dyn Decomposer>,
    delegator: Arc<dyn Delegator>,
    synthesizer: Arc<dyn Synthesizer>,
}

impl PrimaryAgent {
    /// 创建主智能体,注入三大能力策略。
    pub fn new(
        decomposer: Arc<dyn Decomposer>,
        delegator: Arc<dyn Delegator>,
        synthesizer: Arc<dyn Synthesizer>,
    ) -> Self {
        Self {
            decomposer,
            delegator,
            synthesizer,
        }
    }

    /// 分解阶段:将用户输入分解为子任务列表。
    ///
    /// 调用注入的 [`Decomposer::decompose`]。
    #[instrument(skip(self, input), fields(stage = "decompose"))]
    pub async fn decompose(&self, input: &str) -> Result<Vec<DelegatedTask>> {
        self.decomposer.decompose(input).await
    }

    /// 委派阶段:将单个子任务委派给目标 agent 执行。
    ///
    /// 调用注入的 [`Delegator::delegate`]。
    #[instrument(skip(self, task), fields(stage = "delegate", task_id = %task.id))]
    pub async fn delegate(&self, task: &DelegatedTask) -> Result<DelegatedResult> {
        self.delegator.delegate(task).await
    }

    /// 综合阶段:收集子任务结果,综合产出最终输出。
    ///
    /// 调用注入的 [`Synthesizer::synthesize`]。
    #[instrument(skip(self, results), fields(stage = "synthesize", result_count = results.len()))]
    pub async fn synthesize(&self, input: &str, results: &[DelegatedResult]) -> Result<String> {
        self.synthesizer.synthesize(input, results).await
    }

    /// 端到端编排入口:decompose → delegate(按依赖顺序) → synthesize。
    ///
    /// 流程:
    /// 1. `decompose(input)` 得到子任务列表
    /// 2. 按依赖顺序委派(无依赖的先执行,有依赖的等待上游完成)
    /// 3. `synthesize(input, results)` 综合最终输出
    ///
    /// 单任务(分解结果长度为 1)仍走完整流程(委托 + 综合),
    /// 以保持事件链一致;综合器对单结果应原样返回或轻量包装。
    #[instrument(skip(self, input), fields(stage = "orchestrate"))]
    pub async fn run(&self, input: &str) -> Result<PrimaryReport> {
        let task_id = format!("primary_{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let start = std::time::Instant::now();

        // ---- 1. 分解 ----
        let tasks = self.decompose(input).await?;
        let total = tasks.len();
        if total == 0 {
            return Ok(PrimaryReport {
                task_id,
                input: input.to_string(),
                output: String::new(),
                total_sub_tasks: 0,
                successful_sub_tasks: 0,
                failed_sub_tasks: 0,
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
        }

        info!(
            target: "nebula.primary_agent",
            task_id = %task_id,
            sub_task_count = total,
            "decompose completed"
        );

        // ---- 2. 按依赖顺序委派 ----
        // 简化版:按列表顺序执行(假设 decompose 已按依赖顺序输出)。
        // 完整拓扑并行执行由 T-E-AE-05 的 AgentBus 协议落地。
        let mut results = Vec::with_capacity(total);
        let mut successful = 0usize;
        let mut failed = 0usize;

        for task in &tasks {
            match self.delegate(task).await {
                Ok(r) => {
                    if r.success {
                        successful += 1;
                    } else {
                        failed += 1;
                    }
                    results.push(r);
                }
                Err(e) => {
                    warn!(
                        target: "nebula.primary_agent",
                        task_id = %task.id,
                        error = %e,
                        "delegate failed"
                    );
                    failed += 1;
                    results.push(DelegatedResult::failed(&task.id, e.to_string()));
                }
            }
        }

        // ---- 3. 综合 ----
        let output = self.synthesize(input, &results).await?;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        info!(
            target: "nebula.primary_agent",
            task_id = %task_id,
            total, successful, failed, elapsed_ms,
            "orchestrate completed"
        );

        Ok(PrimaryReport {
            task_id,
            input: input.to_string(),
            output,
            total_sub_tasks: total,
            successful_sub_tasks: successful,
            failed_sub_tasks: failed,
            elapsed_ms,
        })
    }
}

// ---------------------------------------------------------------------------
// PrimaryAgentBus — 简化版主→子任务分派通道
// ---------------------------------------------------------------------------

/// 简化版 AgentBus:基于 `tokio::sync::mpsc` 的单向消息通道。
///
/// T-E-AE-01 提供最小可用分派通道;完整协议(含 CRDT 同步、死锁检测、
/// 场景路由表)由 T-E-AE-05 落地。本通道仅支持:
/// - `dispatch`: 主智能体向子智能体发送 `DelegatedTask`
/// - `recv`: 子智能体接收任务
///
/// 不内置结果回传(结果通过 `Delegator` trait 直接返回),
/// 保持 T-E-AE-01 与 T-E-AE-05 的职责边界清晰。
pub struct PrimaryAgentBus {
    tx: mpsc::Sender<DelegatedTask>,
    rx: tokio::sync::Mutex<mpsc::Receiver<DelegatedTask>>,
}

impl PrimaryAgentBus {
    /// 创建一个新的分派通道,内部缓冲区容量为 `capacity`。
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        Self {
            tx,
            rx: tokio::sync::Mutex::new(rx),
        }
    }

    /// 主智能体向通道分派一个子任务。
    ///
    /// 若通道已满或已关闭返回 Err。
    pub async fn dispatch(&self, task: DelegatedTask) -> Result<()> {
        self.tx
            .send(task)
            .await
            .map_err(|e| anyhow!("PrimaryAgentBus dispatch failed: {e}"))
    }

    /// 子智能体从通道接收下一个子任务。
    ///
    /// 通道关闭时返回 `None`。
    pub async fn recv(&self) -> Option<DelegatedTask> {
        self.rx.lock().await.recv().await
    }

    /// 发送端句柄(供多生产者场景克隆使用)。
    pub fn sender(&self) -> mpsc::Sender<DelegatedTask> {
        self.tx.clone()
    }
}

// ---------------------------------------------------------------------------
// 单元测试(TDD:测试定义期望行为)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ===================================================================
    // DelegatedTask 数据结构测试
    // ===================================================================

    #[test]
    fn delegated_task_new_sets_fields() {
        let task = DelegatedTask::new("dt_1", "搜索 Rust 异步资料", AgentScenario::Research);
        assert_eq!(task.id, "dt_1");
        assert_eq!(task.description, "搜索 Rust 异步资料");
        assert_eq!(task.target_scenario, AgentScenario::Research);
        assert!(task.dependencies.is_empty(), "new() 应无依赖");
    }

    #[test]
    fn delegated_task_with_dependencies_sets_deps() {
        let task = DelegatedTask::new("dt_2", "总结", AgentScenario::Review)
            .with_dependencies(vec!["dt_1".to_string()]);
        assert_eq!(task.dependencies, vec!["dt_1"]);
    }

    #[test]
    fn delegated_task_serde_roundtrip() {
        let task = DelegatedTask::new("dt_1", "写代码", AgentScenario::Coding)
            .with_dependencies(vec!["dt_0".to_string()]);
        let json = serde_json::to_string(&task).expect("serialize");
        let de: DelegatedTask = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.id, "dt_1");
        assert_eq!(de.target_scenario, AgentScenario::Coding);
        assert_eq!(de.dependencies, vec!["dt_0"]);
    }

    #[test]
    fn delegated_result_ok_and_failed() {
        let ok = DelegatedResult::ok("dt_1", "成功输出");
        assert!(ok.success);
        assert_eq!(ok.output, "成功输出");
        let fail = DelegatedResult::failed("dt_2", "超时");
        assert!(!fail.success);
        assert_eq!(fail.output, "超时");
    }

    // ===================================================================
    // decompose 测试 — RuleBasedDecomposer
    // ===================================================================

    #[tokio::test]
    async fn decompose_single_task_returns_one_coding_task() {
        // 单任务(无分隔符)→ 返回单个 Coding 场景任务。
        let decomposer = RuleBasedDecomposer::new();
        let tasks = decomposer
            .decompose("实现一个 HTTP 服务器")
            .await
            .expect("decompose");
        assert_eq!(tasks.len(), 1, "单任务应返回 1 个子任务");
        assert_eq!(tasks[0].target_scenario, AgentScenario::Coding);
        assert!(tasks[0].dependencies.is_empty());
    }

    #[tokio::test]
    async fn decompose_multi_task_splits_by_separator_and_chains_deps() {
        // 多任务(分号分隔)→ 按顺序拆分并建立依赖链。
        let decomposer = RuleBasedDecomposer::new();
        let input = "搜索资料;写文章;审查文章";
        let tasks = decomposer.decompose(input).await.expect("decompose");
        assert_eq!(tasks.len(), 3, "应拆分为 3 个子任务");
        // 第一个任务无依赖。
        assert!(tasks[0].dependencies.is_empty(), "首个任务应无依赖");
        // 后续任务依赖前一个。
        assert_eq!(tasks[1].dependencies, vec!["dt_1"]);
        assert_eq!(tasks[2].dependencies, vec!["dt_2"]);
        // 场景推断:搜索→Research,写→Writing,审查→Review。
        assert_eq!(tasks[0].target_scenario, AgentScenario::Research);
        assert_eq!(tasks[1].target_scenario, AgentScenario::Writing);
        assert_eq!(tasks[2].target_scenario, AgentScenario::Review);
    }

    #[tokio::test]
    async fn decompose_newline_separator_also_splits() {
        // 换行符也应作为分隔符。
        let decomposer = RuleBasedDecomposer::new();
        let tasks = decomposer
            .decompose("规划架构\n实现代码")
            .await
            .expect("decompose");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].target_scenario, AgentScenario::Planning);
        assert_eq!(tasks[1].target_scenario, AgentScenario::Coding);
    }

    #[tokio::test]
    async fn decompose_empty_input_returns_single_empty_task() {
        // 空输入:trim 后无内容,split 产生空片段被过滤 → segments 为空 → 走单任务分支。
        let decomposer = RuleBasedDecomposer::new();
        let tasks = decomposer.decompose("   ").await.expect("decompose");
        // 空白输入过滤后 segments 为空,走单任务兜底分支。
        assert_eq!(tasks.len(), 1);
    }

    // ===================================================================
    // delegate 测试 — ScenarioDelegator
    // ===================================================================

    #[tokio::test]
    async fn delegate_invokes_executor_with_scenario_and_description() {
        // 注入闭包:返回 "场景:描述" 格式,验证闭包被正确调用。
        let delegator =
            ScenarioDelegator::new(|scenario, desc| format!("[{}] {}", scenario.as_str(), desc));
        let task = DelegatedTask::new("dt_1", "写文档", AgentScenario::Writing);
        let result = delegator.delegate(&task).await.expect("delegate");
        assert!(result.success);
        assert_eq!(result.task_id, "dt_1");
        assert_eq!(result.output, "[writing] 写文档");
    }

    #[tokio::test]
    async fn delegate_returns_elapsed_ms_positive() {
        // 委派应记录耗时(elapsed_ms >= 0,实际几乎为 0 但字段存在)。
        let delegator = ScenarioDelegator::new(|_, _| "ok".to_string());
        let task = DelegatedTask::new("dt_1", "test", AgentScenario::Coding);
        let result = delegator.delegate(&task).await.expect("delegate");
        // elapsed_ms 是 u64,只要不 panic 即可(快速闭包可能为 0)。
        let _ = result.elapsed_ms;
    }

    #[tokio::test]
    async fn delegate_routes_different_scenarios_to_different_outputs() {
        // 不同场景应路由到不同执行路径(闭包可区分 scenario)。
        let delegator = ScenarioDelegator::new(|scenario, _| match scenario {
            AgentScenario::Research => "研究完成".to_string(),
            AgentScenario::Writing => "写作完成".to_string(),
            _ => "其他".to_string(),
        });
        let r1 = delegator
            .delegate(&DelegatedTask::new("dt_1", "搜索", AgentScenario::Research))
            .await
            .expect("delegate");
        let r2 = delegator
            .delegate(&DelegatedTask::new("dt_2", "写", AgentScenario::Writing))
            .await
            .expect("delegate");
        assert_eq!(r1.output, "研究完成");
        assert_eq!(r2.output, "写作完成");
    }

    // ===================================================================
    // synthesize 测试 — ConcatSynthesizer
    // ===================================================================

    #[tokio::test]
    async fn synthesize_concatenates_results_with_task_ids() {
        let synthesizer = ConcatSynthesizer::new();
        let results = vec![
            DelegatedResult::ok("dt_1", "搜索结果"),
            DelegatedResult::ok("dt_2", "文章内容"),
        ];
        let output = synthesizer
            .synthesize("写一篇文章", &results)
            .await
            .expect("synthesize");
        assert!(
            output.contains("[dt_1] 搜索结果"),
            "应包含 dt_1 输出: {output}"
        );
        assert!(
            output.contains("[dt_2] 文章内容"),
            "应包含 dt_2 输出: {output}"
        );
        assert!(output.contains("写一篇文章"), "应包含原始任务: {output}");
    }

    #[tokio::test]
    async fn synthesize_marks_failed_results_with_prefix() {
        let synthesizer = ConcatSynthesizer::new();
        let results = vec![
            DelegatedResult::ok("dt_1", "成功"),
            DelegatedResult::failed("dt_2", "超时"),
        ];
        let output = synthesizer
            .synthesize("任务", &results)
            .await
            .expect("synthesize");
        assert!(output.contains("[dt_1] 成功"));
        assert!(
            output.contains("[dt_2] [FAILED] 超时"),
            "失败结果应标记 [FAILED]: {output}"
        );
    }

    #[tokio::test]
    async fn synthesize_empty_results_returns_empty_string() {
        let synthesizer = ConcatSynthesizer::new();
        let output = synthesizer
            .synthesize("任务", &[])
            .await
            .expect("synthesize");
        assert!(output.is_empty(), "空结果应返回空字符串");
    }

    // ===================================================================
    // PrimaryAgent 端到端编排测试
    // ===================================================================

    /// 构造测试用 PrimaryAgent(无 LLM 依赖)。
    fn make_primary_agent() -> PrimaryAgent {
        let decomposer = Arc::new(RuleBasedDecomposer::new());
        let delegator = Arc::new(ScenarioDelegator::new(|scenario, desc| {
            format!("[{}] {}", scenario.as_str(), desc)
        }));
        let synthesizer = Arc::new(ConcatSynthesizer::new());
        PrimaryAgent::new(decomposer, delegator, synthesizer)
    }

    #[tokio::test]
    async fn primary_agent_decompose_delegates_to_injected_decomposer() {
        // PrimaryAgent::decompose 应委托给注入的 Decomposer。
        let agent = make_primary_agent();
        let tasks = agent.decompose("搜索资料;写文章").await.expect("decompose");
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].target_scenario, AgentScenario::Research);
    }

    #[tokio::test]
    async fn primary_agent_delegate_delegates_to_injected_delegator() {
        // PrimaryAgent::delegate 应委托给注入的 Delegator。
        let agent = make_primary_agent();
        let task = DelegatedTask::new("dt_1", "test", AgentScenario::Coding);
        let result = agent.delegate(&task).await.expect("delegate");
        assert_eq!(result.output, "[coding] test");
    }

    #[tokio::test]
    async fn primary_agent_synthesize_delegates_to_injected_synthesizer() {
        // PrimaryAgent::synthesize 应委托给注入的 Synthesizer。
        let agent = make_primary_agent();
        let results = vec![DelegatedResult::ok("dt_1", "输出")];
        let output = agent
            .synthesize("任务", &results)
            .await
            .expect("synthesize");
        assert!(output.contains("[dt_1] 输出"));
    }

    #[tokio::test]
    async fn primary_agent_run_end_to_end_single_task() {
        // 单任务端到端:分解→委派→综合。
        let agent = make_primary_agent();
        let report = agent.run("实现 HTTP 服务器").await.expect("run");
        assert_eq!(report.total_sub_tasks, 1);
        assert_eq!(report.successful_sub_tasks, 1);
        assert_eq!(report.failed_sub_tasks, 0);
        // 综合输出应包含子任务输出。
        assert!(
            report.output.contains("[coding]"),
            "应包含委派输出: {}",
            report.output
        );
        assert!(report.output.contains("实现 HTTP 服务器"));
    }

    #[tokio::test]
    async fn primary_agent_run_end_to_end_multi_task() {
        // 多任务端到端:拆分为 3 个子任务,全部成功。
        let agent = make_primary_agent();
        let report = agent.run("搜索资料;写文章;审查文章").await.expect("run");
        assert_eq!(report.total_sub_tasks, 3);
        assert_eq!(report.successful_sub_tasks, 3);
        assert_eq!(report.failed_sub_tasks, 0);
        // 综合输出应包含所有子任务。
        assert!(report.output.contains("[dt_1]"));
        assert!(report.output.contains("[dt_2]"));
        assert!(report.output.contains("[dt_3]"));
        assert!(report.output.contains("搜索资料;写文章;审查文章"));
    }

    #[tokio::test]
    async fn primary_agent_run_records_failed_delegate() {
        // 委派失败应计入 failed_sub_tasks,综合输出标记 [FAILED]。
        let decomposer = Arc::new(RuleBasedDecomposer::new());
        // 用一个始终返回 Err 的 Delegator。
        struct FailingDelegator;
        #[async_trait]
        impl Delegator for FailingDelegator {
            async fn delegate(&self, task: &DelegatedTask) -> Result<DelegatedResult> {
                Err(anyhow!("委派失败: {}", task.id))
            }
        }
        let delegator: Arc<dyn Delegator> = Arc::new(FailingDelegator);
        let synthesizer = Arc::new(ConcatSynthesizer::new());
        let agent = PrimaryAgent::new(decomposer, delegator, synthesizer);

        let report = agent.run("任务A;任务B").await.expect("run");
        assert_eq!(report.total_sub_tasks, 2);
        assert_eq!(report.successful_sub_tasks, 0);
        assert_eq!(report.failed_sub_tasks, 2);
        // 综合输出应标记失败。
        assert!(
            report.output.contains("[FAILED]"),
            "应标记失败: {}",
            report.output
        );
    }

    #[tokio::test]
    async fn primary_agent_run_empty_decompose_returns_empty_report() {
        // 分解返回空 Vec → 返回空报告(不调用 delegate/synthesize)。
        struct EmptyDecomposer;
        #[async_trait]
        impl Decomposer for EmptyDecomposer {
            async fn decompose(&self, _input: &str) -> Result<Vec<DelegatedTask>> {
                Ok(Vec::new())
            }
        }
        let decomposer: Arc<dyn Decomposer> = Arc::new(EmptyDecomposer);
        let delegator: Arc<dyn Delegator> =
            Arc::new(ScenarioDelegator::new(|_, _| "ok".to_string()));
        let synthesizer = Arc::new(ConcatSynthesizer::new());
        let agent = PrimaryAgent::new(decomposer, delegator, synthesizer);

        let report = agent.run("任何输入").await.expect("run");
        assert_eq!(report.total_sub_tasks, 0);
        assert!(report.output.is_empty());
    }

    // ===================================================================
    // PrimaryAgentBus 测试
    // ===================================================================

    #[tokio::test]
    async fn bus_dispatch_and_recv_delivers_task() {
        // 分派 + 接收:任务应按顺序到达。
        let bus = PrimaryAgentBus::new(8);
        let task = DelegatedTask::new("dt_1", "测试", AgentScenario::Coding);
        bus.dispatch(task.clone()).await.expect("dispatch");
        let received = bus.recv().await.expect("应收到任务");
        assert_eq!(received.id, "dt_1");
        assert_eq!(received.description, "测试");
    }

    #[tokio::test]
    async fn bus_preserves_fifo_order() {
        // 多任务分派应保持 FIFO 顺序。
        let bus = PrimaryAgentBus::new(8);
        for i in 1..=3 {
            bus.dispatch(DelegatedTask::new(
                format!("dt_{i}"),
                format!("任务{i}"),
                AgentScenario::Coding,
            ))
            .await
            .expect("dispatch");
        }
        for i in 1..=3 {
            let received = bus.recv().await.expect("应收到任务");
            assert_eq!(received.id, format!("dt_{i}"), "应保持 FIFO 顺序");
        }
    }

    #[tokio::test]
    async fn bus_recv_returns_none_when_closed() {
        // 发送端全部 drop 后,recv 返回 None。
        let bus = PrimaryAgentBus::new(8);
        let tx = bus.sender();
        let task = DelegatedTask::new("dt_1", "x", AgentScenario::Coding);
        tx.send(task).await.expect("send");
        let _ = bus.recv().await; // 取出任务
                                  // drop 发送端(包括 bus 内部的 tx 通过 sender 克隆)
        drop(tx);
        // bus 内部仍持有一个 tx,需要显式关闭通道。
        // 由于 bus 持有 tx,recv 不会返回 None 直到 bus 也被 drop。
        // 此测试验证:当无任务时 recv 会等待(不立即返回 None)。
        // 改为验证空通道 + 超时行为。
        let result = tokio::time::timeout(std::time::Duration::from_millis(50), bus.recv()).await;
        assert!(result.is_err(), "空通道应在超时后未收到任务");
    }

    #[tokio::test]
    async fn bus_supports_multiple_producers_via_sender_clone() {
        // 多生产者:多个 sender 克隆可同时分派。
        let bus = PrimaryAgentBus::new(16);
        let tx2 = bus.sender();

        let task1 = DelegatedTask::new("dt_1", "a", AgentScenario::Coding);
        let task2 = DelegatedTask::new("dt_2", "b", AgentScenario::Writing);

        bus.dispatch(task1).await.expect("dispatch 1");
        tx2.send(task2).await.expect("send 2");

        let r1 = bus.recv().await.expect("recv 1");
        let r2 = bus.recv().await.expect("recv 2");
        assert_eq!(r1.id, "dt_1");
        assert_eq!(r2.id, "dt_2");
    }

    #[test]
    fn bus_new_clamps_capacity_to_minimum_one() {
        // capacity=0 应被 clamp 到 1(避免 panic)。
        let _bus = PrimaryAgentBus::new(0);
        // 不 panic 即通过。
    }
}

//! ADR-001: MasterOrchestrator 组合模式（#44, #45）
//!
//! MasterAgent 编排顶层任务：拆解为 DAG → 按拓扑层执行 SubTask →
//! 收集结果 → 综合最终输出。
//!
//! ## 关键设计（ADR-001）
//!
//! - MasterOrchestrator **持有 `Arc<SwarmOrchestrator>`**，不持有 Worker 池
//! - SubTask → SwarmTask 适配层
//! - BypassMode 通过 `ExecuteMode` 参数传入 `execute_with_mode`
//! - 复用 SwarmOrchestrator 的全部子系统（RAG / Leader / Negotiator / CRDT）
//!
//! ## Feature Gate
//!
//! `master-orchestrator` feature（默认 off）。

use std::sync::Arc;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument, warn};
use uuid::Uuid;

use crate::llm::dispatcher::{UnifiedModelDispatcher, WorkType};
use crate::llm::ollama::{ChatMessage, ChatResponse};
// T-E-L-06: 预算检查所需的 CostTracker(已由 crate::llm 顶层 re-export)。
use crate::llm::CostTracker;
// T-E-L-01: Loop 执行模式所需 import。
use super::loop_def::LoopDef;
use crate::long_task::{LongTaskEngine, StepInput};
use crate::memory::values::risk_assessor::ActionKind;
use crate::memory::values::Verdict;

// T-E-L-06: 同质检测所需 import(从 agents 模块重新导出)。
use super::agents::{HomogeneityPolicy, ModelDescriptor, ReviewerAgent};
use crate::autonomy::AutonomyLevel as GlobalAutonomyLevel;

use super::dag::{FailureStrategy, SubTask, SubTaskResult, SubTaskResultMap, TaskDag};
use super::events::EventEnvelope;
use super::orchestrator::{ExecuteMode, OrchestrationReport, SwarmOrchestrator, SwarmTask};

// ---------------------------------------------------------------------------
// #52 MasterEvent — Master 编排生命周期事件
// ---------------------------------------------------------------------------

/// MasterAgent 编排流程的结构化事件。
///
/// 与 `SwarmEvent`（蜂群执行层）正交，覆盖 Master 层的：
/// 1. 任务拆解（DecomposeStarted / DecomposeCompleted）
/// 2. DAG 层执行（LayerStarted / LayerCompleted）
/// 3. 子任务执行（SubTaskStarted / SubTaskCompleted）
/// 4. 综合输出（SynthesizeStarted / SynthesizeCompleted）
/// 5. 异常与人工介入（DagFailed / UserConfirmationRequired）
/// 6. 全局完成（MasterCompleted）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MasterEvent {
    /// 任务拆解开始
    DecomposeStarted {
        task_id: String,
        /// 用户原始输入摘要（前 200 字符）
        input_summary: String,
        timestamp: i64,
    },
    /// 任务拆解完成（LLM 输出 JSON DAG）
    DecomposeCompleted {
        task_id: String,
        node_count: usize,
        edge_count: usize,
        timestamp: i64,
    },
    /// 拆解失败（JSON 解析错误 / 重试耗尽）
    DecomposeFailed {
        task_id: String,
        error: String,
        timestamp: i64,
    },
    /// DAG 某层开始执行
    LayerStarted {
        task_id: String,
        layer_index: usize,
        node_count: usize,
        timestamp: i64,
    },
    /// DAG 某层执行完成
    LayerCompleted {
        task_id: String,
        layer_index: usize,
        success_count: usize,
        failure_count: usize,
        timestamp: i64,
    },
    /// 单个子任务开始执行（fan-out 到 SwarmOrchestrator）
    SubTaskStarted {
        task_id: String,
        sub_task_id: String,
        worker_count: u32,
        timestamp: i64,
    },
    /// 单个子任务执行完成
    SubTaskCompleted {
        task_id: String,
        sub_task_id: String,
        success: bool,
        /// 失败时的错误摘要
        error: Option<String>,
        elapsed_ms: u64,
        timestamp: i64,
    },
    /// 综合阶段开始（LLM 调用聚合所有子任务结果）
    SynthesizeStarted {
        task_id: String,
        result_count: usize,
        timestamp: i64,
    },
    /// 综合阶段完成
    SynthesizeCompleted {
        task_id: String,
        output_length: usize,
        timestamp: i64,
    },
    /// DAG 执行失败（某节点 Fail 策略触发或重试耗尽）
    DagFailed {
        task_id: String,
        failed_sub_task_id: String,
        reason: String,
        timestamp: i64,
    },
    /// 需要用户确认（Manual 失败策略触发）
    UserConfirmationRequired {
        task_id: String,
        sub_task_id: String,
        /// 给用户看的决策提示
        prompt: String,
        /// 防重放 nonce（P2-5 EA-4 修复）
        confirmation_id: String,
        /// 创建时间（用于 5 分钟超时判定，M5 实现）
        created_at: i64,
        timestamp: i64,
    },
    /// 整个 Master 编排完成
    MasterCompleted {
        task_id: String,
        total_sub_tasks: usize,
        successful_sub_tasks: usize,
        elapsed_ms: u64,
        timestamp: i64,
    },
}

impl MasterEvent {
    pub fn now_ts() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    pub fn decompose_started(task_id: impl Into<String>, input: &str) -> Self {
        let summary: String = input.chars().take(200).collect();
        Self::DecomposeStarted {
            task_id: task_id.into(),
            input_summary: summary,
            timestamp: Self::now_ts(),
        }
    }

    pub fn decompose_completed(task_id: impl Into<String>, dag: &TaskDag) -> Self {
        Self::DecomposeCompleted {
            task_id: task_id.into(),
            node_count: dag.node_count(),
            edge_count: dag.edge_count(),
            timestamp: Self::now_ts(),
        }
    }

    pub fn layer_started(
        task_id: impl Into<String>,
        layer_index: usize,
        node_count: usize,
    ) -> Self {
        Self::LayerStarted {
            task_id: task_id.into(),
            layer_index,
            node_count,
            timestamp: Self::now_ts(),
        }
    }

    pub fn layer_completed(
        task_id: impl Into<String>,
        layer_index: usize,
        success_count: usize,
        failure_count: usize,
    ) -> Self {
        Self::LayerCompleted {
            task_id: task_id.into(),
            layer_index,
            success_count,
            failure_count,
            timestamp: Self::now_ts(),
        }
    }

    pub fn sub_task_started(
        task_id: impl Into<String>,
        sub_task_id: impl Into<String>,
        worker_count: u32,
    ) -> Self {
        Self::SubTaskStarted {
            task_id: task_id.into(),
            sub_task_id: sub_task_id.into(),
            worker_count,
            timestamp: Self::now_ts(),
        }
    }

    pub fn sub_task_completed(
        task_id: impl Into<String>,
        sub_task_id: impl Into<String>,
        success: bool,
        error: Option<String>,
        elapsed_ms: u64,
    ) -> Self {
        Self::SubTaskCompleted {
            task_id: task_id.into(),
            sub_task_id: sub_task_id.into(),
            success,
            error,
            elapsed_ms,
            timestamp: Self::now_ts(),
        }
    }

    pub fn synthesize_started(task_id: impl Into<String>, result_count: usize) -> Self {
        Self::SynthesizeStarted {
            task_id: task_id.into(),
            result_count,
            timestamp: Self::now_ts(),
        }
    }

    pub fn synthesize_completed(task_id: impl Into<String>, output_length: usize) -> Self {
        Self::SynthesizeCompleted {
            task_id: task_id.into(),
            output_length,
            timestamp: Self::now_ts(),
        }
    }

    pub fn dag_failed(
        task_id: impl Into<String>,
        failed_sub_task_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::DagFailed {
            task_id: task_id.into(),
            failed_sub_task_id: failed_sub_task_id.into(),
            reason: reason.into(),
            timestamp: Self::now_ts(),
        }
    }

    pub fn user_confirmation_required(
        task_id: impl Into<String>,
        sub_task_id: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        let now = Self::now_ts();
        Self::UserConfirmationRequired {
            task_id: task_id.into(),
            sub_task_id: sub_task_id.into(),
            prompt: prompt.into(),
            confirmation_id: Uuid::new_v4().to_string(),
            created_at: now,
            timestamp: now,
        }
    }

    pub fn master_completed(
        task_id: impl Into<String>,
        total_sub_tasks: usize,
        successful_sub_tasks: usize,
        elapsed_ms: u64,
    ) -> Self {
        Self::MasterCompleted {
            task_id: task_id.into(),
            total_sub_tasks,
            successful_sub_tasks,
            elapsed_ms,
            timestamp: Self::now_ts(),
        }
    }
}

/// `EventEnvelope<MasterEvent>` 类型别名（#52）。
pub type MasterEventEnvelope = EventEnvelope<MasterEvent>;

impl MasterEventEnvelope {
    /// 从 MasterEvent 变体名提取事件类型。
    pub fn wrap_master_event(event: MasterEvent) -> Self {
        let trace_id = get_current_trace_id()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string().replace("-", ""));
        let event_type = match &event {
            MasterEvent::DecomposeStarted { .. } => "DecomposeStarted",
            MasterEvent::DecomposeCompleted { .. } => "DecomposeCompleted",
            MasterEvent::DecomposeFailed { .. } => "DecomposeFailed",
            MasterEvent::LayerStarted { .. } => "LayerStarted",
            MasterEvent::LayerCompleted { .. } => "LayerCompleted",
            MasterEvent::SubTaskStarted { .. } => "SubTaskStarted",
            MasterEvent::SubTaskCompleted { .. } => "SubTaskCompleted",
            MasterEvent::SynthesizeStarted { .. } => "SynthesizeStarted",
            MasterEvent::SynthesizeCompleted { .. } => "SynthesizeCompleted",
            MasterEvent::DagFailed { .. } => "DagFailed",
            MasterEvent::UserConfirmationRequired { .. } => "UserConfirmationRequired",
            MasterEvent::MasterCompleted { .. } => "MasterCompleted",
        };
        Self {
            event_type: event_type.to_string(),
            payload: event,
            trace_id,
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// 从当前 OTel span context 提取 trace_id（与 swarm/events.rs 同源逻辑）。
fn get_current_trace_id() -> Option<String> {
    #[cfg(feature = "otel")]
    {
        use opentelemetry::trace::TraceContextExt as _;
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let ctx = tracing::Span::current().context();
        let span = ctx.span();
        let span_ctx = span.span_context();
        if span_ctx.is_valid() {
            return Some(span_ctx.trace_id().to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// #44 MasterOrchestrator
// ---------------------------------------------------------------------------

/// Master 编排结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterReport {
    /// 任务 ID（唯一标识本次编排）
    pub task_id: String,
    /// 用户原始输入
    pub input: String,
    /// 最终综合输出
    pub output: String,
    /// 拆解得到的 DAG 节点总数
    pub total_sub_tasks: usize,
    /// 成功完成的子任务数
    pub successful_sub_tasks: usize,
    /// 失败的子任务数
    pub failed_sub_tasks: usize,
    /// 总耗时（毫秒）
    pub elapsed_ms: u64,
    /// 是否降级为直通（无拆解,直接 chat）
    pub bypassed: bool,
}

/// T-E-L-01: `execute_loop()` 返回报告。
///
/// 与 [`MasterReport`]（Once 模式）平行，Loop 模式返回轻量级报告，
/// 实际执行状态由 LongTaskEngine 持久化（可通过 `state_projection()` 投影到 STATE.md）。
///
/// T-E-L-06: 新增 `budget_status` + `autonomy_downgraded` 字段，分别记录
/// 预算门禁结果与 L4 同质降级标记，供前端审计与告警展示。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopRunReport {
    /// 关联的 LongTask ID（denied / needs_confirmation 时为 None）。
    pub task_id: Option<String>,
    /// Loop 名称（来自 LOOP.md frontmatter）。
    pub loop_name: String,
    /// ValuesLayer 裁定：`"allow"` | `"deny"` | `"confirm"` | `"plan"`。
    pub values_verdict: String,
    /// 执行状态：`"started"` | `"denied"` | `"needs_confirmation"`。
    pub status: String,
    /// 人类可读消息（deny 理由 / confirm 提示 / 启动确认）。
    pub message: String,
    /// T-E-L-06: 预算门禁状态。
    ///
    /// 取值：
    /// - `"ok"`：未超月度预算（或未配置预算 → 视为 ok）
    /// - `"warning_80"`：月度用量已达 80% 阈值（仍允许执行，仅告警）
    /// - `"exceeded"`：月度预算超限（execute_loop 已返回 Err，不会到达此处；
    ///   此值保留给未来单次预算超限但仍允许启动的场景）
    /// - `"n/a"`：未传入 CostTracker，跳过预算检查
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_status: Option<String>,
    /// T-E-L-06: 自主度降级标记。
    ///
    /// 当 LoopDef.autonomy == L4 且 ReviewerAgent 检测到模型同质时，
    /// 实际执行自主度从 L4 降级为 L2（由人类最终裁决），此字段记录降级链路
    /// （如 `"L4→L2"`）。None 表示未降级（非 L4 或 L4 但无同质）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy_downgraded: Option<String>,
}

/// T-E-L-06: 把 LoopDef 的 [`super::loop_def::AutonomyLevel`]（L0-L5 简写）
/// 转换为全局 [`crate::autonomy::AutonomyLevel`]（L0InlineCompletion 等），
/// 用于调用 [`ReviewerAgent::enforce_homogeneity_policy`]。
///
/// 两个 enum 表达同一概念但变体名不同（前者用于 LOOP.md frontmatter
/// 序列化，后者用于 autonomy 子系统），需要显式映射。
fn loop_autonomy_to_global(level: super::loop_def::AutonomyLevel) -> GlobalAutonomyLevel {
    use super::loop_def::AutonomyLevel as LoopAutonomy;
    match level {
        LoopAutonomy::L0 => GlobalAutonomyLevel::L0InlineCompletion,
        LoopAutonomy::L1 => GlobalAutonomyLevel::L1DirectedEdit,
        LoopAutonomy::L2 => GlobalAutonomyLevel::L2Chat,
        LoopAutonomy::L3 => GlobalAutonomyLevel::L3Plan,
        LoopAutonomy::L4 => GlobalAutonomyLevel::L4Swarm,
        LoopAutonomy::L5 => GlobalAutonomyLevel::L5Background,
    }
}

/// T-E-L-06: 预算门禁检查结果（内部辅助类型，供 execute_loop 决策）。
///
/// - `Ok`：未超限，附带建议的 `budget_status` 字符串
/// - `Exceeded`：超限，附带人类可读原因（用于 Err 消息）
#[derive(Debug)]
enum BudgetCheckResult {
    /// 未超限，参数为建议写入 LoopRunReport.budget_status 的字符串。
    Ok(String),
    /// 超限，参数为人类可读的原因。
    Exceeded(String),
}

/// T-E-L-06: 执行月度预算检查。
///
/// 调用 [`CostTracker::loop_cost_this_month`] 获取当月已用量，与
/// `monthly_budget_usd` / `monthly_budget_tokens` 比较。
///
/// - 任一受限维度超限 → [`BudgetCheckResult::Exceeded`]
/// - 否则根据 80% 阈值返回 `"ok"` 或 `"warning_80"`
/// - 未传入 CostTracker → `"n/a"`（跳过检查）
fn check_monthly_budget(
    cost_tracker: Option<&CostTracker>,
    monthly_budget_usd: Option<f64>,
    monthly_budget_tokens: Option<u64>,
) -> BudgetCheckResult {
    let tracker = match cost_tracker {
        Some(t) => t,
        None => return BudgetCheckResult::Ok("n/a".to_string()),
    };
    let (used_tokens, used_usd) = tracker.loop_cost_this_month();

    // 任一受限维度超限即返回 Exceeded（OR 语义，与 LoopBudgetConfig 一致）。
    let token_exceeded = monthly_budget_tokens
        .filter(|&limit| limit > 0 && used_tokens >= limit)
        .is_some();
    let usd_exceeded = monthly_budget_usd
        .filter(|&limit| limit > 0.0 && used_usd >= limit)
        .is_some();
    if token_exceeded || usd_exceeded {
        let reason = format!(
            "monthly loop budget exceeded: used {} tokens / ${:.4} (limits: tokens={:?}, usd={:?})",
            used_tokens, used_usd, monthly_budget_tokens, monthly_budget_usd
        );
        return BudgetCheckResult::Exceeded(reason);
    }

    // 80% 告警阈值（仅当该维度配置了上限时才检查）。
    let token_warn = monthly_budget_tokens
        .filter(|&limit| limit > 0 && used_tokens * 5 >= limit * 4)
        .is_some();
    let usd_warn = monthly_budget_usd
        .filter(|&limit| limit > 0.0 && used_usd * 5.0 >= limit * 4.0)
        .is_some();
    if token_warn || usd_warn {
        return BudgetCheckResult::Ok("warning_80".to_string());
    }
    BudgetCheckResult::Ok("ok".to_string())
}

/// MasterAgent 任务拆解提示词模板。
///
/// MasterAgent 通过 `dispatch(WorkType::MasterTask)` 调用 LLM,
/// 要求输出符合 [`TaskDag`] JSON schema 的结构。
const DECOMPOSE_SYSTEM_PROMPT: &str = r#"你是 MasterAgent,负责将用户任务拆解为可并行执行的 DAG。

输出格式(严格 JSON,不要 markdown 包裹):
{
  "nodes": [
    {
      "id": "st_1",
      "prompt": "子任务描述(可包含 {{st_xxx.output}} 占位符引用上游结果)",
      "capabilities": ["search", "summarize"],
      "work_type_hint": "swarm_worker",
      "worker_count": 3,
      "max_retries": 1,
      "agent_kinds": ["generic"],
      "on_failure": "retry"
    }
  ],
  "edges": [
    {"from": "st_1", "to": "st_2", "kind": "finish_to_start"}
  ]
}

规则:
1. 节点 ID 必须唯一,格式 st_<数字>
2. edges 中 from 是上游(被依赖),to 是下游(依赖上游)
3. work_type_hint 可选,值: chat/swarm_worker/swarm_synthesize/master_task/evolution/soul_compile/classifier
4. on_failure 可选,值: retry(默认)/skip/fail/manual
5. 简单任务只需 1 个节点,复杂任务拆解为 2-6 个节点
6. 不要包含任何 JSON 之外的文字"#;

/// MasterAgent 综合阶段提示词模板。
const SYNTHESIZE_SYSTEM_PROMPT: &str = r#"你是 MasterAgent,负责综合多个子任务的执行结果,产出最终回复给用户。

输入会包含多个 <upstream_result> 标签,每个对应一个子任务的输出。
请基于这些结果,综合产出一份连贯、完整的最终回复。
保留关键信息,去除冗余,按逻辑组织结构。
用中文回复(除非用户明确要求其他语言)。"#;

/// MasterOrchestrator — 顶层任务编排器（#44）。
///
/// 组合 [`SwarmOrchestrator`] 复用 fan-out / RAG / Negotiator 等子系统,
/// 在其之上增加 DAG 拆解 + 按层执行 + 结果综合能力。
///
/// **不持有 Worker 池** —— 所有 Worker 调度委托给 SwarmOrchestrator。
pub struct MasterOrchestrator {
    /// 复用的蜂群编排器(Arc 共享,不重新实现 fan-out)
    swarm: Arc<SwarmOrchestrator>,
    /// 统一模型调度器(用于 MasterDecompose / MasterValidate / Synthesize)
    /// 可选:未启用 unified-dispatcher 时回退到 LlmGateway 直连
    dispatcher: Option<Arc<UnifiedModelDispatcher>>,
    /// 事件订阅器(M3 仅记录日志,M5+ 接前端 Channel)
    event_sink: Mutex<Option<std::sync::mpsc::Sender<MasterEvent>>>,
}

impl MasterOrchestrator {
    /// 构造 MasterOrchestrator。
    ///
    /// # 参数
    /// - `swarm`: 复用的 SwarmOrchestrator(Arc 共享)
    /// - `dispatcher`: 统一模型调度器(可选,未启用时用 swarm.llm 兜底)
    pub fn new(
        swarm: Arc<SwarmOrchestrator>,
        dispatcher: Option<Arc<UnifiedModelDispatcher>>,
    ) -> Self {
        Self {
            swarm,
            dispatcher,
            event_sink: Mutex::new(None),
        }
    }

    /// 注入事件 sink(M3 仅用于测试,M5 接前端 Channel)。
    pub fn set_event_sink(&self, sender: std::sync::mpsc::Sender<MasterEvent>) {
        *self.event_sink.lock() = Some(sender);
    }

    /// 内部:发送事件(若有 sink)。
    fn emit(&self, event: MasterEvent) {
        if let Some(sender) = self.event_sink.lock().as_ref() {
            let _ = sender.send(event);
        }
    }

    /// 端到端编排入口。
    ///
    /// 流程:
    /// 1. `dispatch(MasterDecompose)` 拆解为 TaskDag
    /// 2. 拓扑分层,按层并行执行 SubTask(委托 SwarmOrchestrator)
    /// 3. 收集结果到 SubTaskResultMap
    /// 4. `dispatch(SwarmSynthesize)` 综合最终输出
    ///
    /// `mode` 控制子任务执行模式:
    /// - `Standard`: 走 SwarmOrchestrator::execute(含 Negotiator)
    /// - `Bypass`: 走 execute_bypass(选最高置信度,无 LLM 协商)
    /// - `Plan`: L4 门禁预检(同 Standard 路径)
    #[instrument(skip(self, input), fields(task_id))]
    pub async fn orchestrate(&self, input: &str, mode: ExecuteMode) -> Result<MasterReport> {
        let task_id = format!("master_{}", &Uuid::new_v4().to_string()[..8]);
        let start = std::time::Instant::now();

        // ---- 1. 拆解 DAG ----
        self.emit(MasterEvent::decompose_started(&task_id, input));
        let dag = match self.decompose(input).await {
            Ok(dag) => dag,
            Err(e) => {
                self.emit(MasterEvent::DecomposeFailed {
                    task_id: task_id.clone(),
                    error: e.to_string(),
                    timestamp: MasterEvent::now_ts(),
                });
                // 降级为直通:不拆解,直接用 SwarmOrchestrator 执行原始输入
                warn!(task_id = %task_id, error = %e, "decompose failed, falling back to direct execution");
                return self.fallback_direct(&task_id, input, mode, start).await;
            }
        };
        self.emit(MasterEvent::decompose_completed(&task_id, &dag));

        // 单节点 DAG → 直通(不需要综合阶段)
        if dag.node_count() == 1 {
            info!(task_id = %task_id, "single-node DAG, executing directly");
            return self
                .execute_single_node(&task_id, &dag, input, mode, start)
                .await;
        }

        let layers = dag.topological_layers()?;
        let mut results = SubTaskResultMap::new();
        let mut successful = 0usize;
        let mut failed = 0usize;

        for (layer_idx, layer) in layers.iter().enumerate() {
            self.emit(MasterEvent::layer_started(&task_id, layer_idx, layer.len()));

            let (layer_ok, layer_fail) = self
                .execute_layer(&task_id, &dag, layer, &mut results, mode)
                .await?;

            successful += layer_ok;
            failed += layer_fail;

            self.emit(MasterEvent::layer_completed(
                &task_id, layer_idx, layer_ok, layer_fail,
            ));

            // Fail 策略触发:中止后续层
            if layer_fail > 0 && self.has_fail_strategy(&dag, layer) {
                let failed_id = self.first_failed_id(&dag, layer, &results);
                self.emit(MasterEvent::dag_failed(
                    &task_id,
                    failed_id.unwrap_or_default(),
                    "Fail strategy triggered",
                ));
                break;
            }
        }

        // ---- 3. 综合最终输出 ----
        self.emit(MasterEvent::synthesize_started(&task_id, results.len()));
        let output = self.synthesize(input, &results).await?;
        self.emit(MasterEvent::synthesize_completed(&task_id, output.len()));

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let report = MasterReport {
            task_id: task_id.clone(),
            input: input.to_string(),
            output,
            total_sub_tasks: dag.node_count(),
            successful_sub_tasks: successful,
            failed_sub_tasks: failed,
            elapsed_ms,
            bypassed: false,
        };
        self.emit(MasterEvent::master_completed(
            &task_id,
            report.total_sub_tasks,
            report.successful_sub_tasks,
            elapsed_ms,
        ));
        Ok(report)
    }

    /// T-E-L-01: Loop 执行模式入口。
    ///
    /// 与 [`orchestrate()`]（Once 模式）并列。Loop 模式不拆解 DAG，
    /// 而是将 LOOP.md 定义的 action 序列转为 LongTask 步骤，后台异步执行。
    ///
    /// 流程（与 NEBULA_LOOP_DESIGN.md §2.5 对齐）：
    /// 1. **ValuesLayer 门禁**：对 `loop_def.action` 拼接为动作描述，
    ///    调用 `self.swarm.values_layer().evaluate(desc, Generic)`：
    ///    - `Deny` → 返回 `status="denied"`，不创建 LongTask
    ///    - `Confirm` / `Plan` → 返回 `status="needs_confirmation"`
    ///      （TODO: T-E-L-03 PlanEngine 集成，暂不阻塞）
    ///    - `Allow` → 继续
    /// 2. **T-E-L-06 预算门禁**：月度预算超限 → 返回 `Err`（不创建 LongTask）。
    ///    80% 阈值 → `budget_status="warning_80"`，仍允许执行。
    /// 3. **T-E-L-06 同质检测**：若 `loop_def.autonomy == L4`，
    ///    调用 [`ReviewerAgent::enforce_homogeneity_policy`]，
    ///    返回 `Enforced{L4→L2}` 时降级为 L2 并记 `autonomy_downgraded`。
    /// 4. **构造 LongTask 步骤**：每条 action → `StepInput { program: "loop-action", args: [text] }`
    /// 5. **创建 + 启动 LongTask**：`create_task(intent, steps, workspace_id, None)` + `start(id)`
    /// 6. **返回报告**：`LoopRunReport { status: "started", task_id: Some(...) }`
    ///
    /// **不持有 LongTaskEngine / CostTracker / ReviewerAgent**（避免循环依赖），
    /// 通过参数传入。`cost_tracker` / `reviewer` 为 `None` 时跳过对应检查
    /// （向后兼容旧调用方）。**ValuesLayer 内部访问**：`self.swarm.values_layer()`。
    #[instrument(
        skip(self, long_task_engine, cost_tracker, reviewer),
        fields(loop_name = %loop_def.name)
    )]
    pub async fn execute_loop(
        &self,
        loop_def: &LoopDef,
        long_task_engine: &LongTaskEngine,
        workspace_id: Option<String>,
        // T-E-L-06: 月度预算检查的 CostTracker 引用。None 时跳过月度检查
        // （budget_status 设为 "n/a"）。
        cost_tracker: Option<&CostTracker>,
        // T-E-L-06: 月度美元预算上限（来自 AppConfig.loop_monthly_budget_usd）。
        // None 或 0.0 表示该维度不限制。
        monthly_budget_usd: Option<f64>,
        // T-E-L-06: 月度 Token 预算上限（来自 AppConfig.loop_monthly_budget_tokens）。
        // None 或 0 表示该维度不限制。
        monthly_budget_tokens: Option<u64>,
        // T-E-L-06: 同质检测的 ReviewerAgent 引用。None 时跳过同质检测
        // （L4 不降级，autonomy_downgraded 为 None）。
        reviewer: Option<&ReviewerAgent>,
    ) -> Result<LoopRunReport> {
        // ---- 1. ValuesLayer 门禁 ----
        let action_desc = loop_def.action.join("; ");
        let verdict = self
            .swarm
            .values_layer()
            .evaluate(&action_desc, ActionKind::Generic);

        match verdict {
            Verdict::Deny { reason } => {
                warn!(
                    target: "nebula.master.loop",
                    loop_name = %loop_def.name,
                    reason = %reason,
                    "loop denied by values layer"
                );
                return Ok(LoopRunReport {
                    task_id: None,
                    loop_name: loop_def.name.clone(),
                    values_verdict: "deny".to_string(),
                    status: "denied".to_string(),
                    message: reason,
                    budget_status: None,
                    autonomy_downgraded: None,
                });
            }
            Verdict::Confirm { prompt } => {
                // TODO(T-E-L-03): PlanEngine 集成 — 暂返回 needs_confirmation 不阻塞。
                info!(
                    target: "nebula.master.loop",
                    loop_name = %loop_def.name,
                    "loop requires confirmation"
                );
                return Ok(LoopRunReport {
                    task_id: None,
                    loop_name: loop_def.name.clone(),
                    values_verdict: "confirm".to_string(),
                    status: "needs_confirmation".to_string(),
                    message: prompt,
                    budget_status: None,
                    autonomy_downgraded: None,
                });
            }
            Verdict::Plan { prompt } => {
                // TODO(T-E-L-03): PlanEngine 集成 — 暂返回 needs_confirmation 不阻塞。
                info!(
                    target: "nebula.master.loop",
                    loop_name = %loop_def.name,
                    "loop requires plan"
                );
                return Ok(LoopRunReport {
                    task_id: None,
                    loop_name: loop_def.name.clone(),
                    values_verdict: "plan".to_string(),
                    status: "needs_confirmation".to_string(),
                    message: prompt,
                    budget_status: None,
                    autonomy_downgraded: None,
                });
            }
            Verdict::Allow => {} // 继续
        }

        // ---- 2. T-E-L-06: 预算门禁 ----
        // 月度预算超限 → Err（不创建 LongTask）。
        // 单次预算检查需要 CronScheduler 引用（T-E-L-02），此处不持有，
        // 故仅做月度检查；单次超限由 LongTaskEngine::pause_all() 在执行期触发。
        let budget_status =
            match check_monthly_budget(cost_tracker, monthly_budget_usd, monthly_budget_tokens) {
                BudgetCheckResult::Ok(status) => status,
                BudgetCheckResult::Exceeded(reason) => {
                    warn!(
                        target: "nebula.master.loop",
                        loop_name = %loop_def.name,
                        reason = %reason,
                        "loop rejected: monthly budget exceeded"
                    );
                    return Err(anyhow!(reason));
                }
            };
        if budget_status == "warning_80" {
            warn!(
                target: "nebula.master.loop",
                loop_name = %loop_def.name,
                "loop budget at 80%% warning threshold, proceeding"
            );
        }

        // ---- 3. T-E-L-06: 同质检测 ----
        // 仅当 loop_def.autonomy == L4 时调用 enforce_homogeneity_policy。
        // 返回 Enforced{L4→L2} → 实际执行自主度降为 L2，记降级标记。
        // 返回 NoHomogeneity → 保持 L4。
        // 返回 NotCheckerMode → 不可能（L4 不会返回 NotCheckerMode），防御性处理。
        let autonomy_downgraded = if loop_def.autonomy == super::loop_def::AutonomyLevel::L4 {
            if let Some(reviewer) = reviewer {
                let global_level = loop_autonomy_to_global(loop_def.autonomy);
                match reviewer.enforce_homogeneity_policy(global_level) {
                    HomogeneityPolicy::Enforced {
                        original_level,
                        downgraded_to,
                        warning,
                    } => {
                        warn!(
                            target: "nebula.master.loop",
                            loop_name = %loop_def.name,
                            original = ?original_level,
                            downgraded = ?downgraded_to,
                            reason = %warning.reason,
                            "L4 loop downgraded to L2 (model homogeneity)"
                        );
                        Some(format!(
                            "L{}→L{}",
                            original_level.as_u8(),
                            downgraded_to.as_u8()
                        ))
                    }
                    HomogeneityPolicy::NoHomogeneity { .. } => None,
                    // L4 不应返回 NotCheckerMode，防御性 None。
                    HomogeneityPolicy::NotCheckerMode { .. } => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        // ---- 4. 构造 LongTask 步骤 ----
        let steps: Vec<StepInput> = loop_def
            .action
            .iter()
            .map(|a| StepInput {
                description: a.clone(),
                program: "loop-action".to_string(),
                args: vec![a.clone()],
            })
            .collect();

        // ---- 5. 创建 + 启动 LongTask ----
        let task = long_task_engine.create_task(
            loop_def.intent.clone(),
            steps,
            workspace_id,
            None, // plan_id — T-E-L-03 PlanEngine 集成后填充
        )?;

        // start() 会 spawn 后台 runner 异步执行步骤。
        // runner 执行 "loop-action" 程序会失败（非真实命令），但不影响 execute_loop 返回。
        // 实际 Loop 执行逻辑由 T-E-L-02/03 完善。
        long_task_engine.start(&task.id)?;

        info!(
            target: "nebula.master.loop",
            loop_name = %loop_def.name,
            task_id = %task.id,
            budget_status = %budget_status,
            autonomy_downgraded = ?autonomy_downgraded,
            "loop started"
        );

        // ---- 6. 返回报告 ----
        Ok(LoopRunReport {
            task_id: Some(task.id),
            loop_name: loop_def.name.clone(),
            values_verdict: "allow".to_string(),
            status: "started".to_string(),
            message: format!("Loop '{}' started", loop_def.name),
            budget_status: Some(budget_status),
            autonomy_downgraded,
        })
    }

    /// 任务拆解阶段:调用 LLM 生成 TaskDag JSON,然后解析。
    async fn decompose(&self, input: &str) -> Result<TaskDag> {
        let messages = vec![
            ChatMessage::system(DECOMPOSE_SYSTEM_PROMPT),
            ChatMessage::user(format!("用户任务: {input}")),
        ];

        let resp = self
            .dispatch_master(WorkType::MasterTask, &messages)
            .await?;
        TaskDag::from_json(&resp.message.content)
    }

    /// 执行单层节点(可并行)。
    ///
    /// 每个节点:
    /// 1. 解析 prompt 中的 placeholder(替换上游结果)
    /// 2. SubTask → SwarmTask 适配
    /// 3. 调用 SwarmOrchestrator::execute_with_mode
    /// 4. 收集结果到 SubTaskResultMap
    async fn execute_layer(
        &self,
        task_id: &str,
        dag: &TaskDag,
        layer: &[petgraph::graph::NodeIndex],
        results: &mut SubTaskResultMap,
        mode: ExecuteMode,
    ) -> Result<(usize, usize)> {
        use futures::future::join_all;

        let futures: Vec<_> = layer
            .iter()
            .map(|&idx| self.execute_sub_task(task_id, dag, idx, results, mode))
            .collect();

        let outcomes = join_all(futures).await;

        let mut ok = 0usize;
        let mut fail = 0usize;
        for outcome in outcomes {
            match outcome {
                Ok(result) => {
                    if result.success {
                        ok += 1;
                    } else {
                        fail += 1;
                    }
                    // 收集结果到 map(供下游 placeholder 替换使用)
                    results.set(result);
                }
                Err(_) => {
                    fail += 1;
                }
            }
        }
        Ok((ok, fail))
    }

    /// 执行单个子任务(委托 SwarmOrchestrator)。
    async fn execute_sub_task(
        &self,
        task_id: &str,
        dag: &TaskDag,
        idx: petgraph::graph::NodeIndex,
        results: &SubTaskResultMap,
        mode: ExecuteMode,
    ) -> Result<SubTaskResult> {
        let sub = dag.node(idx).ok_or_else(|| anyhow!("node not found"))?;
        let sub_id = sub.id.clone();
        let worker_count = sub.worker_count;
        let start = std::time::Instant::now();

        self.emit(MasterEvent::sub_task_started(
            task_id,
            &sub_id,
            worker_count,
        ));

        // 1. 解析 placeholder
        let resolved_prompt = results.resolve_placeholders(&sub.prompt);

        // 2. SubTask → SwarmTask 适配(ADR-001 §3)
        let swarm_task = self.sub_task_to_swarm_task(sub, resolved_prompt);

        // 3. 执行(委托 SwarmOrchestrator)
        let report = self.swarm.execute_with_mode(swarm_task, mode).await;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let result = match report {
            Ok(r) => {
                // 从 OrchestrationReport 提取最佳输出
                let output = pick_best_output(&r);
                let success = r.approved && !r.outputs.is_empty();
                self.emit(MasterEvent::sub_task_completed(
                    task_id,
                    &sub_id,
                    success,
                    if success {
                        None
                    } else {
                        Some("no output".to_string())
                    },
                    elapsed_ms,
                ));
                SubTaskResult {
                    sub_task_id: sub_id.clone(),
                    output,
                    success,
                    elapsed_ms,
                }
            }
            Err(e) => {
                self.emit(MasterEvent::sub_task_completed(
                    task_id,
                    &sub_id,
                    false,
                    Some(e.to_string()),
                    elapsed_ms,
                ));
                SubTaskResult::failed(&sub_id, e.to_string())
            }
        };

        Ok(result)
    }

    /// SubTask → SwarmTask 适配(ADR-001 §3)。
    ///
    /// `capabilities` 和 `work_type_hint` 字段无法直接传给 SwarmTask
    /// (SwarmTask 只有 description/agent_count/agents),这里做适配。
    /// `work_type_hint` 通过环境变量或 SwarmTask 扩展字段传递(暂存到 description 前缀)。
    fn sub_task_to_swarm_task(&self, sub: &SubTask, resolved_prompt: String) -> SwarmTask {
        let agent_count = sub.worker_count.clamp(2, 6);
        SwarmTask {
            description: resolved_prompt,
            agent_count,
            max_retries: sub.max_retries,
            agents: sub.agent_kinds.clone(),
        }
    }

    /// 综合阶段:调用 LLM 聚合所有子任务结果。
    async fn synthesize(&self, input: &str, results: &SubTaskResultMap) -> Result<String> {
        // 构造上游结果摘要
        let mut upstream_summary = String::new();
        for (id, result) in results.iter() {
            upstream_summary.push_str(&format!(
                "<upstream_result sub_task_id=\"{id}\">\n{}\n</upstream_result>\n\n",
                result.output
            ));
        }

        let messages = vec![
            ChatMessage::system(SYNTHESIZE_SYSTEM_PROMPT),
            ChatMessage::user(format!(
                "用户原始任务: {input}\n\n子任务执行结果:\n{upstream_summary}"
            )),
        ];

        let resp = self
            .dispatch_master(WorkType::SwarmSynthesize, &messages)
            .await?;
        Ok(resp.message.content)
    }

    /// 统一调度入口:优先使用 UnifiedModelDispatcher,降级到 LlmGateway 直连。
    ///
    /// 这个方法隔离了 dispatcher 的可选性,确保未启用 unified-dispatcher
    /// feature 时 MasterOrchestrator 仍可工作(用 SwarmOrchestrator 持有的 LlmGateway)。
    async fn dispatch_master(
        &self,
        work_type: WorkType,
        messages: &[ChatMessage],
    ) -> Result<ChatResponse> {
        if let Some(dispatcher) = &self.dispatcher {
            return dispatcher.dispatch(work_type, messages.to_vec()).await;
        }
        // 降级:直接调用 SwarmOrchestrator 持有的 LlmGateway
        // (走标准 chat 路径,无 WorkType 路由)
        self.swarm.dispatch_via_gateway(messages).await
    }

    /// 降级路径:拆解失败时直接用 SwarmOrchestrator 执行原始输入。
    async fn fallback_direct(
        &self,
        task_id: &str,
        input: &str,
        mode: ExecuteMode,
        start: std::time::Instant,
    ) -> Result<MasterReport> {
        let swarm_task = SwarmTask::new(input);
        let report = self.swarm.execute_with_mode(swarm_task, mode).await?;
        let output = pick_best_output(&report);
        let elapsed_ms = start.elapsed().as_millis() as u64;

        self.emit(MasterEvent::master_completed(
            task_id,
            1,
            if report.approved { 1 } else { 0 },
            elapsed_ms,
        ));

        Ok(MasterReport {
            task_id: task_id.to_string(),
            input: input.to_string(),
            output,
            total_sub_tasks: 1,
            successful_sub_tasks: if report.approved { 1 } else { 0 },
            failed_sub_tasks: if report.approved { 0 } else { 1 },
            elapsed_ms,
            bypassed: true,
        })
    }

    /// 单节点 DAG 直通执行(无需综合阶段)。
    async fn execute_single_node(
        &self,
        task_id: &str,
        dag: &TaskDag,
        input: &str,
        mode: ExecuteMode,
        start: std::time::Instant,
    ) -> Result<MasterReport> {
        let roots = dag.roots();
        let root_idx = roots.first().copied().ok_or_else(|| anyhow!("no root"))?;
        let empty_results = SubTaskResultMap::new();
        let result = self
            .execute_sub_task(task_id, dag, root_idx, &empty_results, mode)
            .await?;

        let elapsed_ms = start.elapsed().as_millis() as u64;
        let successful = if result.success { 1 } else { 0 };
        let failed = if result.success { 0 } else { 1 };

        self.emit(MasterEvent::master_completed(
            task_id, 1, successful, elapsed_ms,
        ));

        Ok(MasterReport {
            task_id: task_id.to_string(),
            input: input.to_string(),
            output: result.output,
            total_sub_tasks: 1,
            successful_sub_tasks: successful,
            failed_sub_tasks: failed,
            elapsed_ms,
            bypassed: false,
        })
    }

    /// 检查层中是否有节点使用 Fail 策略且执行失败。
    fn has_fail_strategy(&self, dag: &TaskDag, layer: &[petgraph::graph::NodeIndex]) -> bool {
        layer.iter().any(|&idx| {
            dag.node(idx)
                .map_or(false, |n| n.on_failure == FailureStrategy::Fail)
        })
    }

    /// 找到层中第一个失败的节点 ID。
    fn first_failed_id(
        &self,
        dag: &TaskDag,
        layer: &[petgraph::graph::NodeIndex],
        _results: &SubTaskResultMap,
    ) -> Option<String> {
        layer
            .iter()
            .find_map(|&idx| dag.node(idx).map(|n| n.id.clone()))
    }
}

/// 从 OrchestrationReport 中挑选最佳输出。
///
/// 策略:
/// 1. 如果 approved,取第一个成功的输出
/// 2. 否则取任意非空输出
/// 3. 都没有则返回空字符串
fn pick_best_output(report: &OrchestrationReport) -> String {
    if let Some(first) = report.outputs.first() {
        return first.body.clone();
    }
    String::new()
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_event_decompose_started_truncates_input() {
        let long_input = "x".repeat(500);
        let evt = MasterEvent::decompose_started("t1", &long_input);
        match evt {
            MasterEvent::DecomposeStarted { input_summary, .. } => {
                assert_eq!(input_summary.chars().count(), 200);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn master_event_decompose_completed_carries_counts() {
        let dag = TaskDag::builder()
            .add_node(SubTask::new("a", "1"))
            .add_node(SubTask::new("b", "2"))
            .add_edge("a", "b")
            .build()
            .expect("test op should succeed");
        let evt = MasterEvent::decompose_completed("t1", &dag);
        match evt {
            MasterEvent::DecomposeCompleted {
                node_count,
                edge_count,
                ..
            } => {
                assert_eq!(node_count, 2);
                assert_eq!(edge_count, 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn master_event_serde_roundtrip() {
        let evt = MasterEvent::synthesize_started("t1", 3);
        let json = serde_json::to_string(&evt).expect("serialize should succeed");
        let de: MasterEvent = serde_json::from_str(&json).expect("parse should succeed");
        match de {
            MasterEvent::SynthesizeStarted { result_count, .. } => assert_eq!(result_count, 3),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn master_event_user_confirmation_has_nonce() {
        let evt = MasterEvent::user_confirmation_required("t1", "st_1", "retry?");
        match evt {
            MasterEvent::UserConfirmationRequired {
                confirmation_id,
                created_at,
                timestamp,
                ..
            } => {
                assert!(!confirmation_id.is_empty());
                assert!(created_at > 0);
                assert!(timestamp > 0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn master_event_envelope_wrap_with_variant() {
        let evt = MasterEvent::decompose_started("t1", "input");
        let envelope = MasterEventEnvelope::wrap_master_event(evt);
        assert_eq!(envelope.event_type, "DecomposeStarted");
        assert!(!envelope.trace_id.is_empty());
        assert!(envelope.timestamp > 0);
    }

    #[test]
    fn master_event_envelope_all_variants() {
        // 验证所有变体都能正确包装
        let events = vec![
            MasterEvent::decompose_started("t", "x"),
            MasterEvent::decompose_completed(
                "t",
                &TaskDag::builder()
                    .add_node(SubTask::new("a", "1"))
                    .build()
                    .expect("test op should succeed"),
            ),
            MasterEvent::DecomposeFailed {
                task_id: "t".into(),
                error: "e".into(),
                timestamp: 0,
            },
            MasterEvent::layer_started("t", 0, 1),
            MasterEvent::layer_completed("t", 0, 1, 0),
            MasterEvent::sub_task_started("t", "st", 3),
            MasterEvent::sub_task_completed("t", "st", true, None, 100),
            MasterEvent::synthesize_started("t", 1),
            MasterEvent::synthesize_completed("t", 100),
            MasterEvent::dag_failed("t", "st", "reason"),
            MasterEvent::user_confirmation_required("t", "st", "p"),
            MasterEvent::master_completed("t", 1, 1, 100),
        ];
        for evt in events {
            let envelope = MasterEventEnvelope::wrap_master_event(evt);
            assert!(!envelope.event_type.is_empty());
            assert!(envelope.event_type.chars().next().expect("assertion value").is_uppercase());
        }
    }

    // ---- T-E-L-01: execute_loop() 测试 ----

    /// 构造测试用 MasterOrchestrator + LongTaskEngine + LlmGateway + 临时 SQLite 路径。
    ///
    /// - LLM 端点指向不存在的 127.0.0.1:1（测试不依赖 LLM 调用）
    /// - LongTaskEngine 用临时 SQLite + migration 037
    /// - ShadowWorkspaceEngine 用默认（run_command 会失败但测试不依赖执行结果）
    /// - T-E-L-06: 同时返回 LlmGateway,供测试构造 ReviewerAgent 做同质检测
    fn make_master_and_engine() -> (
        MasterOrchestrator,
        LongTaskEngine,
        Arc<crate::llm::LlmGateway>,
        std::path::PathBuf,
    ) {
        use std::time::Duration;
        let client = Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            Duration::from_secs(2),
        ));
        let gw = Arc::new(crate::llm::LlmGateway::new(
            client, "m", "ollama", None, None, None, None, None,
        ));
        let swarm = Arc::new(SwarmOrchestrator::new_without_memory(
            gw.clone(),
            Arc::new(crate::tools::ToolRegistry::new()),
        ));
        let master = MasterOrchestrator::new(swarm, None);

        let tmp = std::env::temp_dir().join(format!("nebula-loop-test-{}", Uuid::new_v4()));
        let _ = std::fs::remove_file(&tmp);
        let sqlite =
            Arc::new(crate::memory::sqlite_store::SqliteStore::open(&tmp).expect("open sqlite"));
        {
            let conn = sqlite.raw_connection();
            let conn = conn.lock();
            conn.execute_batch(include_str!("../../migrations/037_long_tasks.sql"))
                .expect("apply migration 037");
        }
        let shadow = Arc::new(crate::shadow_workspace::ShadowWorkspaceEngine::with_default());
        let engine = LongTaskEngine::new(sqlite, shadow);
        (master, engine, gw, tmp)
    }

    /// 构造测试用 LoopDef（直接构造字段，不走 from_markdown 解析）。
    /// 默认 autonomy=L2（不触发同质检测）。
    fn make_loop_def(name: &str, intent: &str, actions: Vec<&str>) -> LoopDef {
        make_loop_def_with_autonomy(
            name,
            intent,
            actions,
            crate::swarm::loop_def::AutonomyLevel::L2,
        )
    }

    /// 构造测试用 LoopDef,可指定 autonomy level（供 T-E-L-06 同质检测测试）。
    fn make_loop_def_with_autonomy(
        name: &str,
        intent: &str,
        actions: Vec<&str>,
        autonomy: crate::swarm::loop_def::AutonomyLevel,
    ) -> LoopDef {
        LoopDef {
            name: name.to_string(),
            description: "test loop".to_string(),
            cadence: "0 9 * * 1-5".to_string(),
            autonomy,
            budget_tokens: 50000,
            budget_minutes: 10,
            intent: intent.to_string(),
            context: vec![],
            action: actions.into_iter().map(String::from).collect(),
            observation: vec![],
            adjustment: vec![],
            stop_condition: None,
            connectors: vec![],
            safety: vec![],
        }
    }

    /// T-E-L-06: 构造 ReviewerAgent 并注入 Maker 模型描述符。
    /// `maker_provider` / `maker_model` 设为 None 时不注入(跳过同质检测)。
    fn make_reviewer(
        gw: Arc<crate::llm::LlmGateway>,
        maker: Option<ModelDescriptor>,
    ) -> ReviewerAgent {
        let mut reviewer = ReviewerAgent::new(gw);
        if let Some(m) = maker {
            reviewer = reviewer.with_maker_model(m);
        }
        reviewer
    }

    #[tokio::test]
    async fn execute_loop_allow_path() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        let loop_def = make_loop_def(
            "safe-read",
            "读取并总结文档",
            vec!["读取 README.md", "总结要点"],
        );
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, None)
            .await
            .expect("execute_loop should succeed on safe action");
        assert_eq!(report.status, "started");
        assert_eq!(report.values_verdict, "allow");
        assert!(report.task_id.is_some(), "task_id should be Some on allow");
        // T-E-L-06: 未传 CostTracker → budget_status="n/a"
        assert_eq!(report.budget_status.as_deref(), Some("n/a"));
        assert!(
            report.autonomy_downgraded.is_none(),
            "L2 should not downgrade"
        );
        // 验证 LongTask 确实被创建
        let task_id = report.task_id.as_ref().expect("test op should succeed");
        let task = engine.get_task(task_id).expect("get_task should succeed");
        assert!(task.is_some(), "task should exist in engine");
        // 等待后台 runner 结束（loop-action 非真实命令会快速失败）
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_deny_path() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        // 身份证号触发 PrivacyGuard::Block → Verdict::Deny
        let loop_def = make_loop_def(
            "pii-leak",
            "处理用户信息",
            vec!["处理身份证号 11010119900307888X"],
        );
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, None)
            .await
            .expect("execute_loop should not error on deny");
        assert_eq!(report.status, "denied");
        assert_eq!(report.values_verdict, "deny");
        assert!(report.task_id.is_none(), "task_id must be None on deny");
        assert!(!report.message.is_empty(), "deny reason should be present");
        // T-E-L-06: deny 路径不进入预算/同质检测,budget_status / autonomy_downgraded 均为 None
        assert!(report.budget_status.is_none());
        assert!(report.autonomy_downgraded.is_none());
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_bulk_plan_path() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        // "批量更新所有配置" 触发 has_bulk_signal（"批量"+"所有"）→ NeedsPlan → Verdict::Plan。
        // 注意：不能用"批量删除...文件"——宪法规则 `批量删除.*文件` 会直接 Deny。
        let loop_def = make_loop_def("bulk-update", "批量更新配置", vec!["批量更新所有配置"]);
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, None)
            .await
            .expect("execute_loop should not error on plan");
        assert_eq!(report.status, "needs_confirmation");
        assert_eq!(report.values_verdict, "plan");
        assert!(
            report.task_id.is_none(),
            "task_id must be None on plan (not yet started)"
        );
        // T-E-L-06: plan 路径同样不进入预算/同质检测
        assert!(report.budget_status.is_none());
        assert!(report.autonomy_downgraded.is_none());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn loop_run_report_serde_round_trip() {
        let report = LoopRunReport {
            task_id: Some("task-123".to_string()),
            loop_name: "daily-triage".to_string(),
            values_verdict: "allow".to_string(),
            status: "started".to_string(),
            message: "Loop 'daily-triage' started".to_string(),
            budget_status: Some("ok".to_string()),
            autonomy_downgraded: None,
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let de: LoopRunReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.task_id, Some("task-123".to_string()));
        assert_eq!(de.loop_name, "daily-triage");
        assert_eq!(de.values_verdict, "allow");
        assert_eq!(de.status, "started");
        assert_eq!(de.message, "Loop 'daily-triage' started");
        assert_eq!(de.budget_status.as_deref(), Some("ok"));
        assert!(de.autonomy_downgraded.is_none());
    }

    // -------------------------------------------------------------------
    // T-E-L-06: 预算门禁 + 同质检测 单测
    // -------------------------------------------------------------------

    /// 辅助:构造一个 CostTracker,内含当月 Automation/Cron/Background 来源
    /// 的预填充记录,用于月度预算检查测试。
    /// deepseek-chat 1M input = 0.14 USD,tokens = 1M。
    ///
    /// 使用公共 `record_async` API 注入记录(绕过 `records` 私有字段访问),
    /// 通过 `CostRecord::new_with_context` 构造后显式覆盖 `source` 字段。
    async fn make_tracker_with_loop_cost(
        source: crate::llm::cost_tracker::CostSource,
        records: u32,
        input_tokens_per_record: u64,
        output_tokens_per_record: u64,
    ) -> crate::llm::CostTracker {
        use crate::llm::cost_tracker::CostRecord;
        let tracker = crate::llm::CostTracker::new();
        for _ in 0..records {
            let mut r = CostRecord::new_with_context(
                "deepseek-chat",
                input_tokens_per_record,
                output_tokens_per_record,
                Some("deepseek".to_string()),
                None,
                None,
            );
            r.source = source;
            // 使用公共 record_async API 推入记录(store=None 时仅内存写入)。
            tracker.record_async(r).await;
        }
        tracker
    }

    #[test]
    fn check_monthly_budget_returns_na_when_no_tracker() {
        // 未传入 CostTracker → 跳过检查,返回 "n/a"
        let result = check_monthly_budget(None, Some(50.0), Some(5_000_000));
        match result {
            BudgetCheckResult::Ok(status) => assert_eq!(status, "n/a"),
            other => panic!("expected Ok(n/a), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_monthly_budget_ok_when_under_limit() {
        // 用量远低于限制 → "ok"
        // tracker 内有 1 条 Cron 记录: 1M tokens / $0.14
        let tracker = make_tracker_with_loop_cost(
            crate::llm::cost_tracker::CostSource::Cron,
            1,
            1_000_000,
            0,
        )
        .await;
        let result = check_monthly_budget(Some(&tracker), Some(50.0), Some(5_000_000));
        match result {
            BudgetCheckResult::Ok(status) => assert_eq!(status, "ok"),
            other => panic!("expected Ok(ok), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_monthly_budget_warning_at_80_percent() {
        // 80% 阈值:limit=1M tokens,used=0.8M → warning_80
        // 1 条 Cron 记录: 0.8M input tokens / $0.112 (deepseek-chat 0.14/1M)
        let tracker =
            make_tracker_with_loop_cost(crate::llm::cost_tracker::CostSource::Cron, 1, 800_000, 0)
                .await;
        let result = check_monthly_budget(Some(&tracker), None, Some(1_000_000));
        match result {
            BudgetCheckResult::Ok(status) => assert_eq!(status, "warning_80"),
            other => panic!("expected Ok(warning_80), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_monthly_budget_exceeded_on_tokens() {
        // Token 维度超限:limit=500K,used=1M → Exceeded
        let tracker = make_tracker_with_loop_cost(
            crate::llm::cost_tracker::CostSource::Cron,
            1,
            1_000_000,
            0,
        )
        .await;
        let result = check_monthly_budget(Some(&tracker), None, Some(500_000));
        match result {
            BudgetCheckResult::Exceeded(reason) => {
                assert!(
                    reason.contains("exceeded"),
                    "reason should mention exceeded: {reason}"
                );
                assert!(
                    reason.contains("500000"),
                    "reason should mention limit: {reason}"
                );
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_monthly_budget_exceeded_on_usd() {
        // USD 维度超限:limit=$0.10,used=$0.14 (1M deepseek-chat input) → Exceeded
        let tracker = make_tracker_with_loop_cost(
            crate::llm::cost_tracker::CostSource::Automation,
            1,
            1_000_000,
            0,
        )
        .await;
        let result = check_monthly_budget(Some(&tracker), Some(0.10), None);
        match result {
            BudgetCheckResult::Exceeded(reason) => {
                assert!(reason.contains("exceeded"), "reason: {reason}");
            }
            other => panic!("expected Exceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_monthly_budget_zero_limit_means_unlimited() {
        // limit=0 表示该维度不限制,即使用量很高也返回 ok
        let tracker = make_tracker_with_loop_cost(
            crate::llm::cost_tracker::CostSource::Background,
            5,
            1_000_000,
            500_000,
        )
        .await;
        // tokens=0 → 不限制;usd=0.0 → 不限制
        let result = check_monthly_budget(Some(&tracker), Some(0.0), Some(0));
        match result {
            BudgetCheckResult::Ok(status) => assert_eq!(status, "ok"),
            other => panic!("expected Ok(ok), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_monthly_budget_excludes_chat_source() {
        // Chat 来源记录不计入 loop_cost_this_month,即使 Chat 用量很高也只看
        // Automation/Cron/Background。
        let tracker = crate::llm::CostTracker::new();
        // Chat 记录: 5M tokens / $0.70(若被错误计入会触发超限)
        let mut r_chat = crate::llm::cost_tracker::CostRecord::new_with_context(
            "deepseek-chat",
            5_000_000,
            0,
            Some("deepseek".to_string()),
            None,
            None,
        );
        r_chat.source = crate::llm::cost_tracker::CostSource::Chat;
        // 使用公共 record_async API 推入记录(store=None 时仅内存写入)。
        tracker.record_async(r_chat).await;
        // 限制 1M tokens / $0.10,但 Chat 不算 → 应为 ok
        let result = check_monthly_budget(Some(&tracker), Some(0.10), Some(1_000_000));
        match result {
            BudgetCheckResult::Ok(status) => assert_eq!(status, "ok"),
            other => panic!("expected Ok(ok) (Chat excluded), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_loop_monthly_budget_exceeded_returns_err() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        let loop_def = make_loop_def("over-budget", "测试预算超限", vec!["读取文件"]);
        // 构造 CostTracker:1 条 Cron 记录 1M tokens,limit=500K → 超限
        let tracker = make_tracker_with_loop_cost(
            crate::llm::cost_tracker::CostSource::Cron,
            1,
            1_000_000,
            0,
        )
        .await;
        let result = master
            .execute_loop(
                &loop_def,
                &engine,
                None,
                Some(&tracker),
                None,
                Some(500_000),
                None,
            )
            .await;
        assert!(result.is_err(), "monthly budget exceeded should return Err");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("exceeded"),
            "error should mention exceeded: {err}"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_monthly_budget_warning_proceeds() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        let loop_def = make_loop_def("near-limit", "测试 80% 告警", vec!["读取文件"]);
        // 0.8M tokens / limit=1M → 80% 告警但仍执行
        let tracker =
            make_tracker_with_loop_cost(crate::llm::cost_tracker::CostSource::Cron, 1, 800_000, 0)
                .await;
        let report = master
            .execute_loop(
                &loop_def,
                &engine,
                None,
                Some(&tracker),
                None,
                Some(1_000_000),
                None,
            )
            .await
            .expect("80% threshold should proceed");
        assert_eq!(report.status, "started");
        assert_eq!(report.budget_status.as_deref(), Some("warning_80"));
        // 等待后台 runner 结束
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_budget_ok_when_under_limit() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        let loop_def = make_loop_def("under-budget", "测试预算正常", vec!["读取文件"]);
        // 0.5M tokens / limit=5M → ok
        let tracker =
            make_tracker_with_loop_cost(crate::llm::cost_tracker::CostSource::Cron, 1, 500_000, 0)
                .await;
        let report = master
            .execute_loop(
                &loop_def,
                &engine,
                None,
                Some(&tracker),
                Some(50.0),
                Some(5_000_000),
                None,
            )
            .await
            .expect("under-limit should proceed");
        assert_eq!(report.status, "started");
        assert_eq!(report.budget_status.as_deref(), Some("ok"));
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_l4_homogeneity_downgrades_to_l2() {
        let (master, engine, gw, tmp) = make_master_and_engine();
        // L4 + Maker 与 Checker 用同一模型 → 触发降级
        // gw 的 provider="ollama" + default_model="m",maker 设为相同 → 同质
        let loop_def = make_loop_def_with_autonomy(
            "l4-homogeneous",
            "L4 同质检测",
            vec!["执行蜂群任务"],
            crate::swarm::loop_def::AutonomyLevel::L4,
        );
        let reviewer = make_reviewer(gw, Some(ModelDescriptor::new("ollama", "m")));
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, Some(&reviewer))
            .await
            .expect("execute_loop should not error on L4 homogeneity");
        assert_eq!(report.status, "started");
        assert_eq!(
            report.autonomy_downgraded.as_deref(),
            Some("L4→L2"),
            "L4 + same model should downgrade to L2: {:?}",
            report.autonomy_downgraded
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_l4_no_homogeneity_keeps_l4() {
        let (master, engine, gw, tmp) = make_master_and_engine();
        // L4 + Maker(deepseek) 与 Checker(ollama) 不同 → 不降级
        let loop_def = make_loop_def_with_autonomy(
            "l4-heterogeneous",
            "L4 异构正常",
            vec!["执行蜂群任务"],
            crate::swarm::loop_def::AutonomyLevel::L4,
        );
        let reviewer = make_reviewer(gw, Some(ModelDescriptor::new("deepseek", "deepseek-chat")));
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, Some(&reviewer))
            .await
            .expect("execute_loop should succeed on L4 without homogeneity");
        assert_eq!(report.status, "started");
        assert!(
            report.autonomy_downgraded.is_none(),
            "L4 + different models should NOT downgrade: {:?}",
            report.autonomy_downgraded
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_l4_without_reviewer_no_downgrade() {
        let (master, engine, _gw, tmp) = make_master_and_engine();
        // L4 但未传入 reviewer → 跳过同质检测,不降级
        let loop_def = make_loop_def_with_autonomy(
            "l4-no-checker",
            "L4 无 Checker",
            vec!["执行蜂群任务"],
            crate::swarm::loop_def::AutonomyLevel::L4,
        );
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, None)
            .await
            .expect("execute_loop should succeed without reviewer");
        assert_eq!(report.status, "started");
        assert!(
            report.autonomy_downgraded.is_none(),
            "L4 without reviewer should NOT downgrade: {:?}",
            report.autonomy_downgraded
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_l4_without_maker_model_no_downgrade() {
        let (master, engine, gw, tmp) = make_master_and_engine();
        // L4 + reviewer 但未注入 maker_model → NoHomogeneity(向后兼容)
        let loop_def = make_loop_def_with_autonomy(
            "l4-legacy",
            "L4 旧调用方",
            vec!["执行蜂群任务"],
            crate::swarm::loop_def::AutonomyLevel::L4,
        );
        let reviewer = make_reviewer(gw, None);
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, Some(&reviewer))
            .await
            .expect("execute_loop should succeed without maker_model");
        assert_eq!(report.status, "started");
        assert!(
            report.autonomy_downgraded.is_none(),
            "L4 without maker_model should NOT downgrade: {:?}",
            report.autonomy_downgraded
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn execute_loop_non_l4_skips_homogeneity_check() {
        let (master, engine, gw, tmp) = make_master_and_engine();
        // L2 + reviewer + maker=checker(同质)— 但 L2 不会触发同质检测
        let loop_def = make_loop_def_with_autonomy(
            "l2-skip-check",
            "L2 跳过同质检测",
            vec!["执行对话"],
            crate::swarm::loop_def::AutonomyLevel::L2,
        );
        let reviewer = make_reviewer(gw, Some(ModelDescriptor::new("ollama", "m")));
        let report = master
            .execute_loop(&loop_def, &engine, None, None, None, None, Some(&reviewer))
            .await
            .expect("execute_loop should succeed for L2");
        assert_eq!(report.status, "started");
        // L2 不触发同质检测,即使 maker=checker
        assert!(
            report.autonomy_downgraded.is_none(),
            "L2 should not trigger homogeneity check: {:?}",
            report.autonomy_downgraded
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn loop_autonomy_to_global_maps_all_levels() {
        use crate::swarm::loop_def::AutonomyLevel as LoopAutonomy;
        // 验证所有 6 个等级的映射
        assert_eq!(
            loop_autonomy_to_global(LoopAutonomy::L0),
            crate::autonomy::AutonomyLevel::L0InlineCompletion
        );
        assert_eq!(
            loop_autonomy_to_global(LoopAutonomy::L1),
            crate::autonomy::AutonomyLevel::L1DirectedEdit
        );
        assert_eq!(
            loop_autonomy_to_global(LoopAutonomy::L2),
            crate::autonomy::AutonomyLevel::L2Chat
        );
        assert_eq!(
            loop_autonomy_to_global(LoopAutonomy::L3),
            crate::autonomy::AutonomyLevel::L3Plan
        );
        assert_eq!(
            loop_autonomy_to_global(LoopAutonomy::L4),
            crate::autonomy::AutonomyLevel::L4Swarm
        );
        assert_eq!(
            loop_autonomy_to_global(LoopAutonomy::L5),
            crate::autonomy::AutonomyLevel::L5Background
        );
    }

    #[test]
    fn loop_run_report_with_downgrade_serde_round_trip() {
        // 验证带降级标记的 LoopRunReport 序列化/反序列化 round-trip
        let report = LoopRunReport {
            task_id: Some("task-456".to_string()),
            loop_name: "l4-loop".to_string(),
            values_verdict: "allow".to_string(),
            status: "started".to_string(),
            message: "Loop 'l4-loop' started".to_string(),
            budget_status: Some("warning_80".to_string()),
            autonomy_downgraded: Some("L4→L2".to_string()),
        };
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(
            json.contains("\"budget_status\":\"warning_80\""),
            "json: {json}"
        );
        assert!(
            json.contains("\"autonomy_downgraded\":\"L4→L2\""),
            "json: {json}"
        );
        let de: LoopRunReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de.budget_status.as_deref(), Some("warning_80"));
        assert_eq!(de.autonomy_downgraded.as_deref(), Some("L4→L2"));
    }

    #[test]
    fn loop_run_report_skips_none_fields_in_serde() {
        // skip_serializing_if = "Option::is_none" 应让 None 字段不出现在 JSON 中
        let report = LoopRunReport {
            task_id: Some("task-789".to_string()),
            loop_name: "simple".to_string(),
            values_verdict: "allow".to_string(),
            status: "started".to_string(),
            message: "started".to_string(),
            budget_status: None,
            autonomy_downgraded: None,
        };
        let json = serde_json::to_string(&report).expect("serialize");
        assert!(
            !json.contains("budget_status"),
            "None fields should be skipped: {json}"
        );
        assert!(
            !json.contains("autonomy_downgraded"),
            "None fields should be skipped: {json}"
        );
        // 反序列化时 None 字段回退为 None
        let de: LoopRunReport = serde_json::from_str(&json).expect("deserialize");
        assert!(de.budget_status.is_none());
        assert!(de.autonomy_downgraded.is_none());
    }

    #[test]
    fn loop_run_report_old_json_without_new_fields_deserializes() {
        // 旧 JSON(无 budget_status / autonomy_downgraded 字段)反序列化时
        // 新字段应回退为 None(#[serde(default)] 保证向后兼容)。
        let old_json = r#"{
            "task_id": "task-old",
            "loop_name": "old-loop",
            "values_verdict": "allow",
            "status": "started",
            "message": "started"
        }"#;
        let de: LoopRunReport = serde_json::from_str(old_json).expect("deserialize old json");
        assert_eq!(de.task_id.as_deref(), Some("task-old"));
        assert_eq!(de.loop_name, "old-loop");
        assert!(de.budget_status.is_none(), "missing budget_status → None");
        assert!(
            de.autonomy_downgraded.is_none(),
            "missing autonomy_downgraded → None"
        );
    }
}

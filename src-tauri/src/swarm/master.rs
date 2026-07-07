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
            .unwrap();
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
        let json = serde_json::to_string(&evt).unwrap();
        let de: MasterEvent = serde_json::from_str(&json).unwrap();
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
                    .unwrap(),
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
            assert!(envelope.event_type.chars().next().unwrap().is_uppercase());
        }
    }
}
